use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamId(pub String);

impl StreamId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "channel", rename_all = "snake_case")]
pub enum ChannelKind {
    TaskEvents { task_id: String },
    GatewayCompat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientFrame {
    pub stream_id: StreamId,
    pub kind: ClientFrameKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientFrameKind {
    Subscribe,
    Unsubscribe,
    Abort,
    Heartbeat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerFrame {
    pub stream_id: StreamId,
    pub kind: ServerFrameKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerFrameKind {
    Event,
    Heartbeat,
    Error,
    Closed,
}
