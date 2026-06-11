//! Extension bus: optional subsystems (hooks, voice/STT, inbound preparation,
//! clarify dispatch) that augment the core routing pipeline.

use std::sync::Arc;

use tokio::sync::RwLock;

use hermes_core::InboundMessagePreparer;

use crate::hooks::HookRegistry;
use crate::tool_backends::ClarifyDispatcher;
use crate::voice::VoiceManager;

/// Optional gateway extensions: hooks, voice, STT, inbound preparer, clarify dispatcher,
/// and the Discord adapter handle used for history backfill.
pub(crate) struct ExtensionBus {
    /// Lifecycle / progress hook registry.
    pub(crate) hook_registry: RwLock<Option<Arc<HookRegistry>>>,
    /// TTS / STT manager.
    pub(crate) voice_manager: RwLock<Option<Arc<VoiceManager>>>,
    /// When `false`, inbound audio is not transcribed (Python `stt_enabled`).
    pub(crate) stt_enabled: RwLock<bool>,
    /// Agent-layer inbound preparer (vision routing, native multimodal, etc.).
    pub(crate) inbound_preparer: RwLock<Option<Arc<dyn InboundMessagePreparer>>>,
    /// Channel clarify dispatcher for IM fast-path answer delivery.
    pub(crate) clarify_dispatcher: RwLock<Option<ClarifyDispatcher>>,
    /// Concrete Discord adapter for history backfill.
    #[cfg(feature = "discord")]
    pub(crate) discord_adapter: RwLock<Option<Arc<crate::platforms::discord::DiscordAdapter>>>,
}

impl ExtensionBus {
    pub(crate) fn new() -> Self {
        Self {
            hook_registry: RwLock::new(None),
            voice_manager: RwLock::new(None),
            stt_enabled: RwLock::new(true),
            inbound_preparer: RwLock::new(None),
            clarify_dispatcher: RwLock::new(None),
            #[cfg(feature = "discord")]
            discord_adapter: RwLock::new(None),
        }
    }
}
