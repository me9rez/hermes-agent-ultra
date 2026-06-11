//! Model switch orchestration across core/session/state shards.

use std::path::Path;
use std::sync::Arc;

use hermes_agent::sub_agent_orchestrator::SubAgentOrchestrator;
use hermes_agent::{AgentLoop, SessionPersistence};

use super::App;
use super::provider::{
    async_tool_dispatch_for, bridge_tool_registry, build_agent_config, build_provider,
    normalize_runtime_provider_name, resolve_provider_and_model, sync_runtime_model_env,
};
use super::state::{AgentCore, ModelState, SessionState, StreamState};

impl ModelState {
    pub(super) fn switch_active(
        &mut self,
        provider_model: &str,
        core: &mut AgentCore,
        session: &SessionState,
        state_root: &Path,
        stream: &StreamState,
    ) {
        self.current_model = provider_model.to_string();
        sync_runtime_model_env(&core.config, &self.current_model);

        let provider = build_provider(&core.config, &self.current_model);
        let agent_config = build_agent_config(&core.config, &self.current_model);
        let agent_tool_registry = Arc::new(bridge_tool_registry(&core.tool_registry));

        let agent_inner = hermes_agent::attach_agent_runtime(AgentLoop::new(
            agent_config,
            agent_tool_registry,
            provider,
        ))
        .with_async_tool_dispatch(async_tool_dispatch_for(core.tool_registry.clone()))
        .with_callbacks(App::stream_callbacks(stream.stream_handle_shared.clone()));
        let orchestrator = Arc::new(SubAgentOrchestrator::from_parent(
            &agent_inner,
            state_root.to_path_buf(),
        ));
        core.agent = Arc::new(agent_inner.with_sub_agent_orchestrator(orchestrator));

        match SessionPersistence::new(state_root)
            .update_session_model(&session.session_id, &self.current_model)
        {
            Ok(true) => tracing::debug!(
                "Persisted model switch for session {} to {}",
                session.session_id,
                self.current_model
            ),
            Ok(false) => {}
            Err(err) => tracing::debug!("Failed to persist model switch to session DB: {}", err),
        }

        tracing::info!("Switched model to: {}", provider_model);
    }

    pub(super) fn switch_personality_name(&mut self, name: &str) {
        self.current_personality = Some(name.to_string());
        tracing::info!("Switched personality to: {}", name);
    }
}

impl App {
    pub fn switch_model(&mut self, provider_model: &str) {
        self.model.switch_active(
            provider_model,
            &mut self.core,
            &self.session,
            &self.state_root,
            &self.stream,
        );
    }

    pub fn switch_personality(&mut self, name: &str) {
        self.model.switch_personality_name(name);
    }

    pub fn current_runtime_provider(&self) -> String {
        let (provider_name, _) =
            resolve_provider_and_model(&self.core.config, &self.model.current_model);
        normalize_runtime_provider_name(provider_name.as_str())
    }
}
