//! Real `MoaBackend` wiring for the `mixture_of_agents` tool.
//!
//! The tool itself lives in `hermes-tools` and only knows the `MoaBackend`
//! trait; it cannot reach the provider layer. This module supplies a concrete
//! backend backed by [`build_provider`], which already resolves a
//! `provider:model` string (e.g. `"openai:gpt-4o"`) into a live
//! [`LlmProvider`] with credentials, base-url and caching handled.
//!
//! Mirrors the post-registration injection pattern used by
//! `enable_live_messaging_tool` / `wire_cron_scheduler_backend`: build the
//! handler with the real backend, then re-`register` it under the same tool
//! name to overwrite the stub installed at startup.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use hermes_config::GatewayConfig;
use hermes_core::ToolError;
use hermes_core::ToolHandler;
use hermes_core::types::Message;
use hermes_tools::ToolRegistry;
use hermes_tools::tools::mixture_of_agents::{
    MixtureOfAgentsHandler, MoaBackend, MoaConfig, MoaResponse,
};

use crate::app::build_provider;

/// Concrete `MoaBackend` that routes each `provider:model` to a real provider.
pub struct ProviderMoaBackend {
    config: Arc<GatewayConfig>,
}

impl ProviderMoaBackend {
    pub fn new(config: Arc<GatewayConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl MoaBackend for ProviderMoaBackend {
    async fn query_model(
        &self,
        model: &str,
        system_prompt: Option<&str>,
        user_prompt: &str,
        temperature: Option<f64>,
        max_tokens: Option<u32>,
    ) -> Result<MoaResponse, ToolError> {
        // `build_provider` resolves "provider:model" → live LlmProvider, with
        // its own credential/base-url/cache handling.
        let provider = build_provider(&self.config, model);

        let mut messages: Vec<Message> = Vec::with_capacity(2);
        if let Some(sys) = system_prompt {
            messages.push(Message::system(sys));
        }
        messages.push(Message::user(user_prompt));

        // Strip any "provider:" prefix: providers expect the bare model id.
        let model_id = model.split_once(':').map(|(_, m)| m).unwrap_or(model);

        let start = Instant::now();
        let resp = provider
            .chat_completion(
                &messages,
                &[], // no tools in a MoA worker call
                max_tokens,
                temperature,
                Some(model_id),
                None,
            )
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("MoA query to '{model}' failed: {e}"))
            })?;

        let latency_ms = start.elapsed().as_millis() as u64;
        let text = resp.message.content.unwrap_or_default();
        let tokens_used = resp.usage.as_ref().map(|u| u.total_tokens);

        Ok(MoaResponse {
            model: model.to_string(),
            text,
            latency_ms,
            // Cost is not derivable from the provider response here; the
            // pipeline treats 0.0 as "unknown".
            cost_usd: 0.0,
            tokens_used,
        })
    }
}

/// Replace the startup `StubMoaBackend` with a real provider-backed backend.
///
/// Safe to call after `register_builtin_tools`: re-registering under the same
/// `mixture_of_agents` name overwrites the stub entry (see
/// `ToolRegistry::register`).
pub fn wire_mixture_of_agents_backend(registry: &ToolRegistry, config: Arc<GatewayConfig>) {
    let backend = Arc::new(ProviderMoaBackend::new(config));
    let handler = MixtureOfAgentsHandler::new(backend, MoaConfig::default());
    let schema = handler.schema();
    let name = schema.name.clone();
    let desc = schema.description.clone();
    registry.register(
        name,
        "mixture_of_agents",
        schema,
        Arc::new(handler),
        Arc::new(|| true),
        vec![],
        true,
        desc,
        "🤖",
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermes_tools::tools::mixture_of_agents::StubMoaBackend;

    /// With no provider/API key configured, `build_provider` falls back to
    /// `NoBackendProvider`, so a query must surface a structured
    /// `ExecutionFailed` (proving the call reached the real provider layer,
    /// not the old stub's "backend not configured" message) — never a panic.
    #[tokio::test]
    async fn real_backend_routes_to_provider_layer_and_errors_without_keys() {
        let backend = ProviderMoaBackend::new(Arc::new(GatewayConfig::default()));
        let err = backend
            .query_model("openai:gpt-4o", None, "hello", Some(0.7), Some(256))
            .await
            .unwrap_err();
        let msg = err.to_string();
        // Routed through build_provider → our wrapper error, not the stub's.
        assert!(
            msg.contains("MoA query to 'openai:gpt-4o' failed"),
            "got: {msg}"
        );
        assert!(
            !msg.contains("backend not configured"),
            "should not hit StubMoaBackend path: {msg}"
        );
    }

    /// Re-registering overwrites the startup stub under the same tool name.
    #[test]
    fn wiring_overwrites_stub_entry() {
        let registry = ToolRegistry::new();
        // Install the stub the way startup registration does.
        let stub = MixtureOfAgentsHandler::new(Arc::new(StubMoaBackend), MoaConfig::default());
        let schema = stub.schema();
        let name = schema.name.clone();
        registry.register(
            name.clone(),
            "mixture_of_agents",
            schema,
            Arc::new(stub),
            Arc::new(|| true),
            vec![],
            true,
            "stub",
            "🤖",
            None,
        );
        assert!(registry.get_tool(&name).is_some());

        // Real wiring should overwrite, keeping the tool present.
        wire_mixture_of_agents_backend(&registry, Arc::new(GatewayConfig::default()));
        let entry = registry.get_tool(&name).expect("tool still registered");
        assert_eq!(entry.name, "mixture_of_agents");
    }
}
