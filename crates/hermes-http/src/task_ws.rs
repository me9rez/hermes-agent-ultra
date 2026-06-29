use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use futures::StreamExt;
use hermes_tasks::schema::encoding::{WsFrame, WsFrameKind, to_bytes};
use hermes_tasks::types::{TaskEvent, TaskId};
use serde::Deserialize;
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, warn};

use crate::HttpServerState;
use crate::tasks::TaskApiState;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Default)]
pub struct TaskStreamHub {
    inner: Arc<Mutex<HashMap<TaskId, broadcast::Sender<TaskEvent>>>>,
}

impl TaskStreamHub {
    pub async fn subscribe(&self, task_id: TaskId) -> broadcast::Receiver<TaskEvent> {
        let mut guard = self.inner.lock().await;
        if let Some(tx) = guard.get(&task_id) {
            return tx.subscribe();
        }
        let (tx, rx) = broadcast::channel(256);
        guard.insert(task_id, tx);
        rx
    }

    pub async fn publish(&self, task_id: TaskId, event: TaskEvent) {
        let tx = {
            let guard = self.inner.lock().await;
            guard.get(&task_id).cloned()
        };
        if let Some(tx) = tx {
            let _ = tx.send(event);
        }
    }
}

fn task_state(state: &HttpServerState) -> Result<&TaskApiState, axum::http::StatusCode> {
    state
        .tasks
        .as_ref()
        .map(|t| t.as_ref())
        .ok_or(axum::http::StatusCode::SERVICE_UNAVAILABLE)
}

pub async fn task_stream_upgrade(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    State(state): State<HttpServerState>,
) -> Result<Response, axum::http::StatusCode> {
    let tasks = task_state(&state)?;
    let task_id: TaskId = id
        .parse()
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;
    if tasks
        .runtime
        .tasks()
        .get(task_id)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(axum::http::StatusCode::NOT_FOUND);
    }
    let hub = tasks.stream_hub.clone();
    let runtime = tasks.runtime.clone();
    Ok(ws.on_upgrade(move |socket| handle_task_stream(socket, task_id, hub, runtime)))
}

pub async fn multiplex_ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<HttpServerState>,
) -> Response {
    let hub = state
        .tasks
        .as_ref()
        .map(|t| t.stream_hub.clone())
        .unwrap_or_default();
    let runtime = state.tasks.as_ref().map(|t| t.runtime.clone());
    ws.on_upgrade(move |socket| handle_multiplex(socket, hub, runtime))
}

async fn handle_task_stream(
    mut socket: WebSocket,
    task_id: TaskId,
    hub: TaskStreamHub,
    runtime: Arc<hermes_tasks::TaskRuntime>,
) {
    let mut rx = hub.subscribe(task_id).await;
    let mut seen_ids: Vec<String> = Vec::new();
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);

    loop {
        tokio::select! {
            msg = socket.next() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        if let Ok(frame) = serde_json::from_str::<WsFrame>(&text) {
                            if frame.kind == WsFrameKind::StreamCancel {
                                break;
                            }
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(WsMessage::Ping(data))) => {
                        let _ = socket.send(WsMessage::Pong(data)).await;
                    }
                    _ => {}
                }
            }
            Ok(event) = rx.recv() => {
                seen_ids.push(event.id.as_str());
                if send_event_frame(&mut socket, Some(task_id.to_string()), &event).await.is_err() {
                    break;
                }
            }
            _ = interval.tick() => {
                if let Ok(events) = runtime.events().list_for_task(task_id) {
                    for event in events {
                        let id = event.id.as_str();
                        if seen_ids.iter().any(|seen| seen == &id) {
                            continue;
                        }
                        seen_ids.push(id);
                        if send_event_frame(&mut socket, Some(task_id.to_string()), &event).await.is_err() {
                            return;
                        }
                    }
                }
            }
            _ = heartbeat.tick() => {
                let hb = WsFrame {
                    schema_version: hermes_tasks::schema::events::SCHEMA_VERSION,
                    stream_id: Some(task_id.to_string()),
                    kind: WsFrameKind::Heartbeat,
                    encoding: hermes_tasks::schema::encoding::WsFrameEncoding::Json,
                    payload_b64: String::new(),
                };
                if socket.send(WsMessage::Binary(to_bytes(&hb).into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct MultiplexCommand {
    op: String,
    task_id: Option<String>,
    stream_id: Option<String>,
}

async fn handle_multiplex(
    mut socket: WebSocket,
    hub: TaskStreamHub,
    runtime: Option<Arc<hermes_tasks::TaskRuntime>>,
) {
    let mut subscriptions: HashMap<String, (TaskId, broadcast::Receiver<TaskEvent>)> =
        HashMap::new();
    let mut seen_ids: HashMap<TaskId, Vec<String>> = HashMap::new();
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);

    loop {
        tokio::select! {
            msg = socket.next() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        process_ws_payload(&mut socket, &mut subscriptions, &hub, &mut seen_ids, text.as_bytes()).await;
                    }
                    Some(Ok(WsMessage::Binary(bytes))) => {
                        process_ws_payload(&mut socket, &mut subscriptions, &hub, &mut seen_ids, bytes.as_ref()).await;
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        let _ = socket.send(WsMessage::Pong(data)).await;
                    }
                    Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            _ = interval.tick() => {
                let Some(runtime) = runtime.as_ref() else { continue };
                for (stream_id, (task_id, _rx)) in subscriptions.iter() {
                    let seen = seen_ids.entry(*task_id).or_default();
                    if let Ok(events) = runtime.events().list_for_task(*task_id) {
                        for event in events {
                            let id = event.id.as_str();
                            if seen.iter().any(|s| s == &id) {
                                continue;
                            }
                            seen.push(id);
                            if send_event_frame(&mut socket, Some(stream_id.clone()), &event).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            _ = heartbeat.tick() => {
                let hb = WsFrame {
                    schema_version: hermes_tasks::schema::events::SCHEMA_VERSION,
                    stream_id: None,
                    kind: WsFrameKind::Heartbeat,
                    encoding: hermes_tasks::schema::encoding::WsFrameEncoding::Json,
                    payload_b64: String::new(),
                };
                if socket.send(WsMessage::Binary(to_bytes(&hb).into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn process_ws_payload(
    socket: &mut WebSocket,
    subscriptions: &mut HashMap<String, (TaskId, broadcast::Receiver<TaskEvent>)>,
    hub: &TaskStreamHub,
    seen_ids: &mut HashMap<TaskId, Vec<String>>,
    raw: &[u8],
) {
    let Ok(frame) = serde_json::from_slice::<WsFrame>(raw) else {
        return;
    };
    match frame.kind {
        WsFrameKind::ClientCommand => {
            if let Ok(cmd) = frame.decode_payload::<MultiplexCommand>() {
                handle_multiplex_command(subscriptions, hub, seen_ids, cmd).await;
            }
        }
        WsFrameKind::StreamCancel => {
            if let Some(stream_id) = frame.stream_id {
                subscriptions.remove(&stream_id);
            }
        }
        WsFrameKind::Heartbeat => {
            let hb = WsFrame {
                schema_version: hermes_tasks::schema::events::SCHEMA_VERSION,
                stream_id: frame.stream_id,
                kind: WsFrameKind::Heartbeat,
                encoding: hermes_tasks::schema::encoding::WsFrameEncoding::Json,
                payload_b64: String::new(),
            };
            let _ = socket.send(WsMessage::Binary(to_bytes(&hb).into())).await;
        }
        _ => {}
    }
}

async fn handle_multiplex_command(
    subscriptions: &mut HashMap<String, (TaskId, broadcast::Receiver<TaskEvent>)>,
    hub: &TaskStreamHub,
    seen_ids: &mut HashMap<TaskId, Vec<String>>,
    cmd: MultiplexCommand,
) {
    if cmd.op != "subscribe" {
        return;
    }
    let Some(task_raw) = cmd.task_id else { return };
    let Ok(task_id) = task_raw.parse::<TaskId>() else {
        warn!(task_id = %task_raw, "invalid task id in multiplex subscribe");
        return;
    };
    let stream_id = cmd.stream_id.unwrap_or_else(|| format!("task:{task_id}"));
    let rx = hub.subscribe(task_id).await;
    seen_ids.entry(task_id).or_default();
    debug!(%task_id, stream_id = %stream_id, "multiplex subscribed");
    subscriptions.insert(stream_id, (task_id, rx));
}

async fn send_event_frame(
    socket: &mut WebSocket,
    stream_id: Option<String>,
    event: &TaskEvent,
) -> Result<(), ()> {
    let mut frame = WsFrame::encode_payload(WsFrameKind::ServerEvent, event).map_err(|_| ())?;
    frame.stream_id = stream_id;
    socket
        .send(WsMessage::Binary(to_bytes(&frame).into()))
        .await
        .map_err(|_| ())
}
