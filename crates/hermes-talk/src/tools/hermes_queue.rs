use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{info, warn};

use crate::config::AipcTalkConfig;
use crate::error::{DemoError, Result};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct HermesConnection {
    ws: WsStream,
}

impl HermesConnection {
    async fn connect(config: &AipcTalkConfig) -> Result<Self> {
        if !config.url.starts_with("ws://") && !config.url.starts_with("wss://") {
            return Err(DemoError::Tool(format!(
                "invalid hermes url '{}': must start with ws:// or wss://",
                config.url
            )));
        }
        let (ws, _response) =
            tokio::time::timeout(Duration::from_secs(10), connect_async(config.url.as_str()))
                .await
                .map_err(|_| DemoError::Tool("hermes connection timeout (>10s)".to_string()))?
                .map_err(|e| DemoError::Tool(format!("hermes WS connect failed: {e}")))?;
        Ok(Self { ws })
    }

    async fn request(
        &mut self,
        request_id: &str,
        text: &str,
        timeout_secs: Option<u64>,
        msg_tx: &mpsc::Sender<HermesMessage>,
    ) -> Result<String> {
        let req_json = serde_json::json!({
            "request_id": request_id,
            "text": text,
        })
        .to_string();

        self.ws
            .send(WsMessage::Text(req_json.into()))
            .await
            .map_err(|e| DemoError::Tool(format!("hermes send failed: {e}")))?;

        eprintln!("\n══════════ 发送给 hermes ══════════\n{text}\n══════════════════════════");

        // Read responses in a loop — forward any message whose request_id
        // does not match (e.g. cron deliveries) and keep waiting for ours.
        loop {
            let response_msg = match timeout_secs {
                Some(secs) => tokio::time::timeout(Duration::from_secs(secs), self.ws.next())
                    .await
                    .map_err(|_| DemoError::Tool(format!("hermes response timeout (>{secs}s)")))?,
                None => self.ws.next().await,
            }
            .ok_or_else(|| DemoError::Tool("hermes WS stream ended".to_string()))?
            .map_err(|e| DemoError::Tool(format!("hermes WS read error: {e}")))?;

            let response_text = match response_msg {
                WsMessage::Text(t) => t.to_string(),
                WsMessage::Close(frame) => {
                    return Err(DemoError::Tool(format!(
                        "hermes closed connection: {:?}",
                        frame.map(|f| f.reason.to_string())
                    )));
                }
                other => {
                    return Err(DemoError::Tool(format!(
                        "hermes unexpected message type: {other:?}"
                    )));
                }
            };

            let response: serde_json::Value =
                serde_json::from_str(&response_text).map_err(|e| {
                    DemoError::Tool(format!(
                        "hermes invalid JSON response: {e}, raw: {response_text}"
                    ))
                })?;

            let resp_id = response["request_id"].as_str().unwrap_or("");
            if resp_id == request_id {
                // This is our response
                let status = response["status"].as_str().unwrap_or("");
                if status != "ok" {
                    warn!(%status, %response_text, "hermes: non-ok status");
                }

                let text = response["text"].as_str().unwrap_or("").to_string();
                if text.contains("did not respond in time")
                    || text.contains("timeout")
                    || text.contains("timed out")
                {
                    return Err(DemoError::Tool(format!(
                        "hermes agent timeout, will retry: {text}"
                    )));
                }

                return Ok(text);
            }

            // Not our response — forward as unsolicited message (e.g. cron delivery)
            let text = response["text"].as_str().unwrap_or("").to_string();
            let status = response["status"].as_str().unwrap_or("ok").to_string();
            info!(%resp_id, %text, "hermes: forwarding unsolicited message");
            let _ = msg_tx
                .send(HermesMessage {
                    request_id: resp_id.to_string(),
                    text,
                    status,
                })
                .await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HermesPriority {
    Low = 0,
    Normal = 1,
    High = 2,
}

impl HermesPriority {
    pub fn from_str(s: &str) -> Self {
        match s {
            "high" => Self::High,
            "low" => Self::Low,
            _ => Self::Normal,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HermesRequest {
    pub id: String,
    pub text: String,
    pub priority: HermesPriority,
    pub created_at: Instant,
    pub model: Option<String>,
    pub provider: Option<String>,
}

impl PartialEq for HermesRequest {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for HermesRequest {}

impl PartialOrd for HermesRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HermesRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.created_at.cmp(&self.created_at))
    }
}

const MAX_QUEUE_SIZE: usize = 100;

#[derive(Debug, Clone)]
pub struct HermesMessage {
    pub request_id: String,
    pub text: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct TaskSummary {
    pub request_id: String,
    pub text: String,
    pub priority: String,
    pub created_at_secs: u64,
}

#[derive(Debug, Clone)]
pub struct CompletedTask {
    pub request_id: String,
    pub status: String,
    pub text: String,
    pub completed_at_secs: u64,
}

#[derive(Debug, Clone)]
pub struct ListResult {
    pub pending: Vec<TaskSummary>,
    pub history: Vec<CompletedTask>,
}

const MAX_HISTORY_SIZE: usize = 1000;

enum QueueCommand {
    Enqueue {
        req: HermesRequest,
        respond: oneshot::Sender<Result<String>>,
    },
    Cancel {
        request_id: String,
        respond: oneshot::Sender<Result<bool>>,
    },
    List {
        request_id: Option<String>,
        respond: oneshot::Sender<ListResult>,
    },
}

#[derive(Clone)]
pub struct HermesQueueSender {
    cmd_tx: mpsc::Sender<QueueCommand>,
}

impl HermesQueueSender {
    pub async fn add_request(
        &self,
        text: String,
        priority: HermesPriority,
        model: Option<String>,
        provider: Option<String>,
    ) -> Result<String> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let req = HermesRequest {
            id: request_id.clone(),
            text,
            priority,
            created_at: Instant::now(),
            model,
            provider,
        };
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(QueueCommand::Enqueue { req, respond: tx })
            .await
            .map_err(|_| DemoError::Tool("hermes queue closed".to_string()))?;
        rx.await
            .map_err(|_| DemoError::Tool("hermes queue response lost".to_string()))?
    }

    pub async fn cancel_request(&self, request_id: &str) -> Result<bool> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(QueueCommand::Cancel {
                request_id: request_id.to_string(),
                respond: tx,
            })
            .await
            .map_err(|_| DemoError::Tool("hermes queue closed".to_string()))?;
        rx.await
            .map_err(|_| DemoError::Tool("hermes queue response lost".to_string()))?
    }

    pub async fn list_tasks(&self, request_id: Option<String>) -> Result<ListResult> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(QueueCommand::List {
                request_id,
                respond: tx,
            })
            .await
            .map_err(|_| DemoError::Tool("hermes queue closed".to_string()))?;
        Ok(rx
            .await
            .map_err(|_| DemoError::Tool("hermes queue response lost".to_string()))?)
    }
}

pub struct HermesQueue {
    pub sender: HermesQueueSender,
}

impl HermesQueue {
    pub fn new(config: AipcTalkConfig) -> (Self, mpsc::Receiver<HermesMessage>, JoinHandle<()>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(128);
        let (msg_tx, msg_rx) = mpsc::channel(128);
        let handle = tokio::spawn(hermes_worker(config, cmd_rx, msg_tx));
        (
            Self {
                sender: HermesQueueSender { cmd_tx },
            },
            msg_rx,
            handle,
        )
    }
}

async fn hermes_worker(
    config: AipcTalkConfig,
    mut cmd_rx: mpsc::Receiver<QueueCommand>,
    msg_tx: mpsc::Sender<HermesMessage>,
) {
    let mut heap: BinaryHeap<HermesRequest> = BinaryHeap::new();
    let mut history: VecDeque<CompletedTask> = VecDeque::new();

    let mut conn = match HermesConnection::connect(&config).await {
        Ok(c) => {
            info!(url = %config.url, "hermes: connected at startup");
            Some(c)
        }
        Err(e) => {
            warn!(%e, url = %config.url, "hermes: initial connect failed, will retry on first request");
            None
        }
    };

    loop {
        let cmd = if heap.is_empty() {
            // When queue is empty, also poll the WebSocket for unsolicited
            // messages (cron deliveries) while waiting for commands.
            if let Some(ref mut c) = conn {
                tokio::select! {
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(cmd) => cmd,
                            None => break,
                        }
                    }
                    msg = c.ws.next() => {
                        match msg {
                            Some(Ok(WsMessage::Text(t))) => {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                                    let id = v["request_id"].as_str().unwrap_or("").to_string();
                                    let txt = v["text"].as_str().unwrap_or("").to_string();
                                    let st = v["status"].as_str().unwrap_or("ok").to_string();
                                    info!(%id, %txt, "hermes: unsolicited message received");
                                    let _ = msg_tx.try_send(HermesMessage {
                                        request_id: id,
                                        text: txt,
                                        status: st,
                                    });
                                }
                            }
                            Some(Ok(WsMessage::Close(_))) | None => {
                                conn = None;
                            }
                            Some(Err(e)) => {
                                warn!(%e, "hermes: WS error in idle read");
                                conn = None;
                            }
                            _ => {}
                        }
                        continue;
                    }
                }
            } else {
                // No connection — retry periodically while waiting for commands
                tokio::select! {
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(cmd) => cmd,
                            None => break,
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        match HermesConnection::connect(&config).await {
                            Ok(c) => {
                                info!(url = %config.url, "hermes: reconnected after retry");
                                conn = Some(c);
                            }
                            Err(e) => {
                                warn!(%e, "hermes: reconnect failed, will retry in 5s");
                            }
                        }
                        continue;
                    }
                }
            }
        } else {
            tokio::select! {
                Some(c) = cmd_rx.recv() => c,
                else => break,
            }
        };

        match cmd {
            QueueCommand::Enqueue { req, respond } => {
                if heap.len() >= MAX_QUEUE_SIZE {
                    let _ = respond.send(Err(DemoError::Tool(format!(
                        "hermes queue full (max {MAX_QUEUE_SIZE})"
                    ))));
                    continue;
                }
                let request_id = req.id.clone();
                info!(%request_id, text = %req.text, priority = ?req.priority, "hermes_queue: enqueue");
                heap.push(req);
                let _ = respond.send(Ok(request_id));
            }
            QueueCommand::Cancel {
                request_id,
                respond,
            } => {
                let len_before = heap.len();
                heap = heap.into_iter().filter(|r| r.id != request_id).collect();
                let found = len_before != heap.len();
                info!(%request_id, found, "hermes_queue: cancel");
                let _ = respond.send(Ok(found));
            }
            QueueCommand::List {
                request_id: filter_id,
                respond,
            } => {
                let pending: Vec<TaskSummary> = heap
                    .iter()
                    .filter(|r| filter_id.as_ref().map_or(true, |id| r.id == *id))
                    .map(|r| TaskSummary {
                        request_id: r.id.clone(),
                        text: r.text.clone(),
                        priority: format!("{:?}", r.priority).to_lowercase(),
                        created_at_secs: r.created_at.elapsed().as_secs(),
                    })
                    .collect();
                let completed: Vec<CompletedTask> = history
                    .iter()
                    .filter(|c| filter_id.as_ref().map_or(true, |id| c.request_id == *id))
                    .cloned()
                    .collect();
                info!(
                    filter = ?filter_id,
                    pending = pending.len(),
                    history = completed.len(),
                    "hermes_queue: list"
                );
                let _ = respond.send(ListResult {
                    pending,
                    history: completed,
                });
            }
        }

        if heap.is_empty() || msg_tx.is_closed() {
            continue;
        }

        if conn.is_none() {
            match HermesConnection::connect(&config).await {
                Ok(c) => {
                    info!(url = %config.url, "hermes: reconnected");
                    conn = Some(c);
                }
                Err(e) => {
                    warn!(%e, "hermes: reconnect failed, requeuing");
                    continue;
                }
            }
        }

        let req = heap.pop().unwrap();
        info!(
            id = %req.id,
            text = %req.text,
            priority = ?req.priority,
            "hermes_queue: processing"
        );

        let mut c = Some(conn.take().unwrap());
        let result;
        loop {
            let cur = c.as_mut().unwrap();
            match cur
                .request(&req.id, &req.text, config.timeout_secs, &msg_tx)
                .await
            {
                Ok(text) => {
                    result = Ok(text);
                    break;
                }
                Err(e) => {
                    warn!(id = %req.id, error = %e, "hermes_queue: request failed, reconnecting and retrying");
                    c = None; // drop old broken connection
                    match HermesConnection::connect(&config).await {
                        Ok(new_conn) => {
                            info!(url = %config.url, "hermes: reconnected for retry");
                            c = Some(new_conn);
                        }
                        Err(reconnect_err) => {
                            warn!(%reconnect_err, "hermes: reconnect for retry failed");
                            result = Err(e);
                            break;
                        }
                    }
                }
            }
        }

        match result {
            Ok(text) => {
                info!(id = %req.id, len = text.len(), "hermes_queue: got reply");
                conn = c;
                let summary = text.chars().take(200).collect::<String>();
                let summary = if text.chars().count() > 200 {
                    format!("{summary}...")
                } else {
                    summary
                };
                if history.len() >= MAX_HISTORY_SIZE {
                    history.pop_front();
                }
                history.push_back(CompletedTask {
                    request_id: req.id.clone(),
                    status: "ok".to_string(),
                    text: summary,
                    completed_at_secs: req.created_at.elapsed().as_secs(),
                });
                let _ = msg_tx
                    .send(HermesMessage {
                        request_id: req.id,
                        text,
                        status: "ok".to_string(),
                    })
                    .await;
            }
            Err(e) => {
                warn!(id = %req.id, error = %e, "hermes_queue: request failed after retries, giving up");
                if history.len() >= MAX_HISTORY_SIZE {
                    history.pop_front();
                }
                history.push_back(CompletedTask {
                    request_id: req.id.clone(),
                    status: "error".to_string(),
                    text: format!("{e}"),
                    completed_at_secs: req.created_at.elapsed().as_secs(),
                });
                let _ = msg_tx
                    .send(HermesMessage {
                        request_id: req.id,
                        text: format!("hermes request failed: {e}"),
                        status: "error".to_string(),
                    })
                    .await;
            }
        }
    }

    info!("hermes_queue: worker exiting");
}
