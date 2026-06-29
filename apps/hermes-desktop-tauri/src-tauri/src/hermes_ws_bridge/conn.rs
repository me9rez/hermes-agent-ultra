use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::reconnect::ExponentialBackoff;
use super::router::StreamRouter;
use super::types::{ClientFrame, ClientFrameKind, ServerFrame, ServerFrameKind, StreamId};

pub struct HermesWsBridge {
    pub ws_url: String,
    pub router: Arc<StreamRouter>,
    outbound: Arc<Mutex<Option<tokio::sync::mpsc::Sender<ClientFrame>>>>,
    pending_subscriptions: Arc<Mutex<Vec<(StreamId, String)>>>,
}

impl HermesWsBridge {
    pub fn new(ws_url: impl Into<String>, router: Arc<StreamRouter>) -> Self {
        Self {
            ws_url: ws_url.into(),
            router,
            outbound: Arc::new(Mutex::new(None)),
            pending_subscriptions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn connect(&self) -> Result<(), String> {
        let url = format!("{}?mode=tasks", self.ws_url.trim_end_matches('/'));
        let (ws_stream, _) = connect_async(&url)
            .await
            .map_err(|e| format!("ws connect failed: {e}"))?;
        let (write, mut read) = ws_stream.split();
        let write = Arc::new(Mutex::new(write));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ClientFrame>(64);
        *self.outbound.lock().await = Some(tx);

        for (stream_id, task_id) in self.pending_subscriptions.lock().await.drain(..) {
            self.subscribe_task(stream_id, task_id).await?;
        }

        let write_out = write.clone();
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if let Ok(text) = serde_json::to_string(&frame) {
                    let mut guard = write_out.lock().await;
                    if guard.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
        });

        let router_read = self.router.clone();
        let write_ping = write.clone();
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        handle_server_payload(&router_read, text.as_bytes()).await;
                    }
                    Ok(Message::Binary(bytes)) => {
                        handle_server_payload(&router_read, bytes.as_ref()).await;
                    }
                    Ok(Message::Ping(data)) => {
                        let mut guard = write_ping.lock().await;
                        let _ = guard.send(Message::Pong(data)).await;
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
        });

        Ok(())
    }

    pub async fn connect_with_retry(&self, max_attempts: u32) -> Result<(), String> {
        let mut backoff = ExponentialBackoff::new();
        let mut last_err = String::from("no attempts");
        for _ in 0..max_attempts {
            match self.connect().await {
                Ok(()) => {
                    backoff.reset();
                    return Ok(());
                }
                Err(err) => {
                    last_err = err;
                    tokio::time::sleep(backoff.next_delay()).await;
                }
            }
        }
        Err(last_err)
    }

    pub async fn subscribe_task(&self, stream_id: StreamId, task_id: String) -> Result<(), String> {
        if self.outbound.lock().await.is_none() {
            self.pending_subscriptions
                .lock()
                .await
                .push((stream_id.clone(), task_id));
            return Ok(());
        }
        let cmd = serde_json::json!({
            "op": "subscribe",
            "task_id": task_id,
            "stream_id": stream_id.0,
        });
        let mut frame = hermes_tasks::schema::encoding::WsFrame::encode_payload(
            hermes_tasks::schema::encoding::WsFrameKind::ClientCommand,
            &cmd,
        )
        .map_err(|e| e.to_string())?;
        frame.stream_id = Some(stream_id.0.clone());
        let bytes = hermes_tasks::schema::encoding::to_bytes(&frame);
        if let Some(tx) = self.outbound.lock().await.as_ref() {
            let client = ClientFrame {
                stream_id,
                kind: ClientFrameKind::Subscribe,
                payload: Some(serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)),
            };
            tx.send(client)
                .await
                .map_err(|e| format!("send subscribe: {e}"))
        } else {
            Err("ws bridge not connected".into())
        }
    }

    pub async fn send_client_frame(&self, frame: ClientFrame) -> Result<(), String> {
        if let Some(tx) = self.outbound.lock().await.as_ref() {
            tx.send(frame)
                .await
                .map_err(|e| format!("send client frame: {e}"))
        } else {
            Err("ws bridge not connected".into())
        }
    }

    pub fn restore_subscriptions(&self, streams: Vec<(StreamId, String)>) {
        let pending = self.pending_subscriptions.clone();
        tokio::spawn(async move {
            *pending.lock().await = streams;
        });
    }
}

async fn handle_server_payload(router: &Arc<StreamRouter>, raw: &[u8]) {
    let Ok(ws_frame) = serde_json::from_slice::<hermes_tasks::schema::encoding::WsFrame>(raw)
    else {
        return;
    };
    let Some(ref stream_id) = ws_frame.stream_id else {
        return;
    };
    let payload = ws_frame
        .payload()
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    router.publish(
        &StreamId::new(stream_id.clone()),
        ServerFrame {
            stream_id: StreamId::new(stream_id.clone()),
            kind: ServerFrameKind::Event,
            payload,
        },
    );
}
