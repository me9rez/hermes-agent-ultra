use base64::Engine;
use serde::{Deserialize, Serialize};

pub const MSGPACK_THRESHOLD_BYTES: usize = 10 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameEncoding {
    Json,
    Msgpack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameKind {
    ClientCommand,
    ServerEvent,
    Heartbeat,
    Error,
    StreamCancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsFrame {
    pub schema_version: u32,
    pub stream_id: Option<String>,
    pub kind: WsFrameKind,
    pub encoding: WsFrameEncoding,
    pub payload_b64: String,
}

impl WsFrame {
    pub fn payload(&self) -> Result<Vec<u8>, base64::DecodeError> {
        base64::engine::general_purpose::STANDARD.decode(&self.payload_b64)
    }

    pub fn encode_payload<T: Serialize>(
        kind: WsFrameKind,
        value: &T,
    ) -> Result<Self, rmp_serde::encode::Error> {
        let json_bytes = serde_json::to_vec(value).unwrap_or_default();
        if json_bytes.len() > MSGPACK_THRESHOLD_BYTES {
            let payload = rmp_serde::to_vec_named(value)?;
            Ok(Self {
                schema_version: super::events::SCHEMA_VERSION,
                stream_id: None,
                kind,
                encoding: WsFrameEncoding::Msgpack,
                payload_b64: base64::engine::general_purpose::STANDARD.encode(payload),
            })
        } else {
            Ok(Self {
                schema_version: super::events::SCHEMA_VERSION,
                stream_id: None,
                kind,
                encoding: WsFrameEncoding::Json,
                payload_b64: base64::engine::general_purpose::STANDARD.encode(json_bytes),
            })
        }
    }

    pub fn decode_payload<T: for<'de> Deserialize<'de>>(&self) -> Result<T, DecodeError> {
        let raw = self.payload().map_err(DecodeError::Base64)?;
        match self.encoding {
            WsFrameEncoding::Json => serde_json::from_slice(&raw).map_err(DecodeError::Json),
            WsFrameEncoding::Msgpack => rmp_serde::from_slice(&raw).map_err(DecodeError::Msgpack),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("base64 decode: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("json decode: {0}")]
    Json(#[from] serde_json::Error),
    #[error("msgpack decode: {0}")]
    Msgpack(#[from] rmp_serde::decode::Error),
}

pub fn payload_marker(encoding: WsFrameEncoding) -> &'static str {
    match encoding {
        WsFrameEncoding::Json => "application/json",
        WsFrameEncoding::Msgpack => "application/msgpack",
    }
}

pub fn to_bytes(frame: &WsFrame) -> Vec<u8> {
    serde_json::to_vec(frame).unwrap_or_default()
}
