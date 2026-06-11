//! Delivery layer: stream management, outbound file tracking, and live messaging
//! session context.

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::RwLock;

use crate::stream::{StreamConfig, StreamManager};
use hermes_tools::tools::messaging::MessagingSessionContext;

// ---------------------------------------------------------------------------
// TurnOutboundTracker
// ---------------------------------------------------------------------------

/// Tracks files delivered during a single inbound route turn (tool + MEDIA output).
/// Used to deduplicate file attachments within a turn.
#[derive(Debug, Default)]
pub(crate) struct TurnOutboundTracker {
    pub(crate) platform: String,
    pub(crate) chat_id: String,
    pub(crate) paths: StdMutex<Vec<PathBuf>>,
}

impl TurnOutboundTracker {
    pub(crate) fn new(platform: impl Into<String>, chat_id: impl Into<String>) -> Self {
        Self {
            platform: platform.into(),
            chat_id: chat_id.into(),
            paths: StdMutex::new(Vec::new()),
        }
    }

    pub(crate) fn matches(&self, platform: &str, chat_id: &str) -> bool {
        self.platform.eq_ignore_ascii_case(platform) && self.chat_id == chat_id
    }

    pub(crate) fn record(&self, path: PathBuf) {
        let key = path.to_string_lossy().to_lowercase();
        if let Ok(mut guard) = self.paths.lock() {
            if guard
                .iter()
                .any(|p| p.to_string_lossy().to_lowercase() == key)
            {
                return;
            }
            guard.push(path);
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.paths.lock().map(|g| g.len()).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// DeliveryLayer
// ---------------------------------------------------------------------------

/// Stream manager, per-turn outbound file tracking, and the live messaging context.
pub(crate) struct DeliveryLayer {
    pub(crate) stream_manager: Arc<StreamManager>,
    /// Per-session turn-level file delivery tracking (dedup within a single route turn).
    pub(crate) turn_outbound: StdMutex<std::collections::HashMap<String, TurnOutboundTracker>>,
    /// Current inbound channel for `send_message` session fallback.
    pub(crate) messaging_session: RwLock<Option<Arc<MessagingSessionContext>>>,
}

impl DeliveryLayer {
    pub(crate) fn new(stream_config: StreamConfig) -> Self {
        Self {
            stream_manager: Arc::new(StreamManager::new(stream_config)),
            turn_outbound: StdMutex::new(std::collections::HashMap::new()),
            messaging_session: RwLock::new(None),
        }
    }
}
