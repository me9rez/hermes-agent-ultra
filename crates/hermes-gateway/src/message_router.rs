//! Message routing layer: platform adapters, handler callbacks, access policies.
//!
//! Contains the types required by the Gateway routing layer and groups the
//! fields that own message dispatch into a single struct so that borrowing
//! and responsibility boundaries are visible at the type level.

use std::collections::{BTreeMap, HashMap};
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::RwLock;

use hermes_core::{errors::GatewayError, traits::PlatformAdapter, types::Message};

use crate::background::BackgroundTaskManager;
use crate::dm::DmManager;
use crate::pairing_store::DmPairingStore;

// ---------------------------------------------------------------------------
// Access-control policy types
// ---------------------------------------------------------------------------

/// Per-platform access mode for group (non-DM) channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupAccessMode {
    Open,
    Allowlist,
    Disabled,
}

impl Default for GroupAccessMode {
    fn default() -> Self {
        Self::Open
    }
}

/// Direct-message access mode (`platforms.<name>.dm_policy` / `extra.dm_policy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DmAccessMode {
    /// Anyone may DM; skip global pairing gate.
    Open,
    /// Unknown senders get pairing / approval prompt.
    #[default]
    Pairing,
    /// Only users in `allowed_users` may DM.
    Allowlist,
    /// DMs are completely disabled.
    Disabled,
}

/// Platform-level access policy (group traffic + slash-command gate + DM mode).
#[derive(Debug, Clone, Default)]
pub struct PlatformAccessPolicy {
    pub allowed_users: std::collections::HashSet<String>,
    pub admin_users: std::collections::HashSet<String>,
    pub allowed_roles: std::collections::HashSet<String>,
    pub group_mode: GroupAccessMode,
    pub slash_requires_allowlist: bool,
    pub dm_mode: DmAccessMode,
}

impl PlatformAccessPolicy {
    pub fn has_allowlist(&self) -> bool {
        !self.allowed_users.is_empty()
            || !self.admin_users.is_empty()
            || !self.allowed_roles.is_empty()
    }

    fn user_matches_any(user_id: &str, set: &std::collections::HashSet<String>) -> bool {
        let candidate = user_id.trim();
        if candidate.is_empty() {
            return false;
        }
        let candidate_no_at = candidate.strip_prefix('@').unwrap_or(candidate);
        set.iter().any(|entry| {
            let allowed = entry.trim();
            if allowed.is_empty() {
                return false;
            }
            let allowed_no_at = allowed.strip_prefix('@').unwrap_or(allowed);
            allowed.eq_ignore_ascii_case(candidate)
                || allowed.eq_ignore_ascii_case(candidate_no_at)
                || allowed_no_at.eq_ignore_ascii_case(candidate)
                || allowed_no_at.eq_ignore_ascii_case(candidate_no_at)
        })
    }

    pub fn is_user_allowed(&self, user_id: &str, role_ids: &[String]) -> bool {
        Self::user_matches_any(user_id, &self.admin_users)
            || Self::user_matches_any(user_id, &self.allowed_users)
            || role_ids
                .iter()
                .any(|role| Self::user_matches_any(role, &self.allowed_roles))
    }
}

// ---------------------------------------------------------------------------
// Runtime context passed to V2 handlers
// ---------------------------------------------------------------------------

/// Structured runtime context passed to context-aware agent handlers.
#[derive(Debug, Clone, Default)]
pub struct GatewayRuntimeContext {
    pub session_key: String,
    /// Rotatable session UUID (Python-style). Equals `session_key` for sessions
    /// that have never been reset; becomes a fresh UUID after each `/new` / `/reset`.
    pub session_id: String,
    pub platform: String,
    pub chat_id: String,
    pub user_id: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub profile: Option<String>,
    pub branch: Option<String>,
    pub personality: Option<String>,
    pub home: Option<String>,
    pub service_tier: Option<String>,
    pub tool_progress: Option<String>,
    pub verbose: bool,
    pub yolo: bool,
    pub reasoning: bool,
    pub mcp_reload_generation: u64,
    /// Messages queued by handlers to be delivered only after the main reply.
    pub deferred_post_delivery_messages: Option<Arc<StdMutex<Vec<String>>>>,
    /// Release flag shared with handlers for post-delivery gating.
    pub deferred_post_delivery_released: Option<Arc<std::sync::atomic::AtomicBool>>,
}

// ---------------------------------------------------------------------------
// Handler type aliases
// ---------------------------------------------------------------------------

/// Callback type for processing messages through the agent loop.
pub type MessageHandler = Arc<
    dyn Fn(
            Arc<Vec<Message>>,
        )
            -> Pin<Box<dyn std::future::Future<Output = Result<String, GatewayError>> + Send>>
        + Send
        + Sync,
>;

/// Context-aware callback type for processing messages through the agent loop.
pub type MessageHandlerWithContext = Arc<
    dyn Fn(
            Arc<Vec<Message>>,
            GatewayRuntimeContext,
        )
            -> Pin<Box<dyn std::future::Future<Output = Result<String, GatewayError>> + Send>>
        + Send
        + Sync,
>;

/// Callback type for streaming message processing.
pub type StreamingMessageHandler = Arc<
    dyn Fn(
            Arc<Vec<Message>>,
            Arc<dyn Fn(String) + Send + Sync>,
        )
            -> Pin<Box<dyn std::future::Future<Output = Result<String, GatewayError>> + Send>>
        + Send
        + Sync,
>;

/// Context-aware callback type for streaming message processing.
pub type StreamingMessageHandlerWithContext = Arc<
    dyn Fn(
            Arc<Vec<Message>>,
            GatewayRuntimeContext,
            Arc<dyn Fn(String) + Send + Sync>,
        )
            -> Pin<Box<dyn std::future::Future<Output = Result<String, GatewayError>> + Send>>
        + Send
        + Sync,
>;

// ---------------------------------------------------------------------------
// MessageRouter
// ---------------------------------------------------------------------------

/// Platform adapter registry, access policies, and inbound/outbound handler hooks.
pub(crate) struct MessageRouter {
    pub(crate) adapters: RwLock<HashMap<String, Arc<dyn PlatformAdapter>>>,
    pub(crate) platform_access_policies: RwLock<HashMap<String, PlatformAccessPolicy>>,
    pub(crate) dm_manager: Arc<RwLock<DmManager>>,
    pub(crate) pairing_store: Arc<StdMutex<DmPairingStore>>,
    pub(crate) message_handler: RwLock<Option<MessageHandler>>,
    pub(crate) message_handler_with_context: RwLock<Option<MessageHandlerWithContext>>,
    pub(crate) streaming_handler: RwLock<Option<StreamingMessageHandler>>,
    pub(crate) streaming_handler_with_context: RwLock<Option<StreamingMessageHandlerWithContext>>,
    pub(crate) background_tasks: Arc<BackgroundTaskManager>,
    pub(crate) mcp_reload_generation: RwLock<u64>,
    pub(crate) tool_progress_modes: RwLock<BTreeMap<String, String>>,
}

impl MessageRouter {
    pub(crate) fn new(dm_manager: DmManager) -> Self {
        Self {
            adapters: RwLock::new(HashMap::new()),
            platform_access_policies: RwLock::new(HashMap::new()),
            dm_manager: Arc::new(RwLock::new(dm_manager)),
            pairing_store: Arc::new(StdMutex::new(DmPairingStore::open_default())),
            message_handler: RwLock::new(None),
            message_handler_with_context: RwLock::new(None),
            streaming_handler: RwLock::new(None),
            streaming_handler_with_context: RwLock::new(None),
            background_tasks: Arc::new(BackgroundTaskManager::new(8)),
            mcp_reload_generation: RwLock::new(0),
            tool_progress_modes: RwLock::new(BTreeMap::new()),
        }
    }
}
