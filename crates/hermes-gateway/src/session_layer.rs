//! Session state layer: per-session runtime settings, usage counters, concurrency
//! locks, and the teardown lifecycle.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use hermes_core::types::Message;

use crate::session::SessionManager;

// ---------------------------------------------------------------------------
// Session teardown
// ---------------------------------------------------------------------------

/// Snapshot payload delivered to the session-teardown hook.
#[derive(Debug, Clone)]
pub struct SessionTeardownContext {
    pub session_key: String,
    pub session_id: String,
    pub platform: String,
    pub chat_id: String,
    pub user_id: String,
    pub messages: Arc<Vec<Message>>,
    pub reason: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub personality: Option<String>,
    pub home: Option<String>,
}

/// Optional hook invoked before a gateway session is reset, expired, or drained.
pub type SessionTeardownHandler = Arc<
    dyn Fn(SessionTeardownContext) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

// ---------------------------------------------------------------------------
// Private session state types
// ---------------------------------------------------------------------------

/// Per-session mutable runtime settings (model, persona, flags) driven by slash commands.
#[derive(Debug, Clone)]
pub(crate) struct SessionRuntimeState {
    pub(crate) model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) profile: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) personality: Option<String>,
    pub(crate) home: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) tool_progress: Option<String>,
    pub(crate) budget: Option<f64>,
    pub(crate) verbose: bool,
    pub(crate) yolo: bool,
    pub(crate) reasoning: bool,
}

impl Default for SessionRuntimeState {
    fn default() -> Self {
        Self {
            model: None,
            provider: None,
            profile: None,
            branch: None,
            personality: None,
            home: None,
            service_tier: None,
            tool_progress: None,
            budget: None,
            verbose: false,
            yolo: false,
            reasoning: false,
        }
    }
}

/// Basic per-session character and message-count statistics (backing `/usage`).
#[derive(Debug, Clone, Default)]
pub(crate) struct UsageStats {
    pub(crate) user_messages: u64,
    pub(crate) assistant_messages: u64,
    pub(crate) input_chars: u64,
    pub(crate) output_chars: u64,
    pub(crate) last_updated_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// SessionLayer
// ---------------------------------------------------------------------------

/// Session management, per-session state, concurrency locks, and usage tracking.
pub(crate) struct SessionLayer {
    pub(crate) session_manager: Arc<SessionManager>,
    /// Per-session mutable runtime state (model, persona, flags).
    pub(crate) runtime_state: RwLock<HashMap<String, SessionRuntimeState>>,
    /// Per-session mutex ensuring one active agent route at a time (Python `_running_agents`).
    pub(crate) session_serial: RwLock<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Abort handles for in-flight foreground routes; remove on completion or `/stop`.
    pub(crate) active_routes: RwLock<HashMap<String, futures::future::AbortHandle>>,
    /// Cumulative character-level usage per session (fallback for `/usage`).
    pub(crate) usage_stats: RwLock<HashMap<String, UsageStats>>,
    /// LLM token totals reported by the agent loop (precise `/usage` display).
    pub(crate) session_token_usage: RwLock<HashMap<String, hermes_agent::SessionUsageDisplay>>,
    /// Agent-layer hook for POI flush / memory `on_session_end` before session removal.
    pub(crate) session_teardown_handler: RwLock<Option<SessionTeardownHandler>>,
}

impl SessionLayer {
    pub(crate) fn new(session_manager: Arc<SessionManager>) -> Self {
        Self {
            session_manager,
            runtime_state: RwLock::new(HashMap::new()),
            session_serial: RwLock::new(HashMap::new()),
            active_routes: RwLock::new(HashMap::new()),
            usage_stats: RwLock::new(HashMap::new()),
            session_token_usage: RwLock::new(HashMap::new()),
            session_teardown_handler: RwLock::new(None),
        }
    }
}
