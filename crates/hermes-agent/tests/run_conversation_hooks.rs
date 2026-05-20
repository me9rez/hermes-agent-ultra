//! Agent-loop contract: `OnSessionEnd` and `PreApiRequest` fire through real `run()` paths.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use hermes_agent::{
    agent_loop::ToolRegistry,
    plugins::{HookResult, HookType, Plugin, PluginContext, PluginManager, PluginMeta},
    AgentConfig, AgentLoop,
};
use hermes_core::{AgentError, LlmProvider, Message, StreamChunk, ToolSchema};
use serde_json::Value;

#[derive(Clone, Default)]
struct HookCounter(Arc<Mutex<Vec<String>>>);

impl HookCounter {
    fn push(&self, label: &str) {
        self.0.lock().expect("hook counter lock").push(label.to_string());
    }
}

struct CountingHookPlugin {
    hook: HookType,
    counter: HookCounter,
    label: &'static str,
}

#[async_trait]
impl Plugin for CountingHookPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: format!("counting_{}", self.hook.as_str()),
            version: "0.0.0".into(),
            description: "hook contract counter".into(),
            author: None,
        }
    }

    async fn initialize(&self) -> Result<(), AgentError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), AgentError> {
        Ok(())
    }

    fn register(&self, ctx: &mut PluginContext) {
        let counter = self.counter.clone();
        let label = self.label;
        ctx.on(self.hook, Arc::new(move |ctx_val: &Value| {
            counter.push(label);
            if label == "on_session_end" {
                let _ = ctx_val.get("completed").and_then(Value::as_bool);
                let _ = ctx_val.get("interrupted").and_then(Value::as_bool);
            }
            if label == "pre_api_request" {
                let _ = ctx_val.get("api_call_count").and_then(Value::as_u64);
            }
            HookResult::Ok
        }));
    }
}

struct StopAssistantProvider;

#[async_trait]
impl LlmProvider for StopAssistantProvider {
    async fn chat_completion(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _max_tokens: Option<u32>,
        _temperature: Option<f64>,
        _model: Option<&str>,
        _extra_body: Option<&serde_json::Value>,
    ) -> Result<hermes_core::LlmResponse, AgentError> {
        Ok(hermes_core::LlmResponse {
            message: Message::assistant("done"),
            usage: None,
            model: "test".into(),
            finish_reason: Some("stop".into()),
        })
    }

    fn chat_completion_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _max_tokens: Option<u32>,
        _temperature: Option<f64>,
        _model: Option<&str>,
        _extra_body: Option<&serde_json::Value>,
    ) -> BoxStream<'static, Result<StreamChunk, AgentError>> {
        futures::stream::empty().boxed()
    }
}

fn agent_with_hook_plugins(counter: Arc<Mutex<Vec<String>>>) -> AgentLoop {
    let mut pm = PluginManager::new();
    let shared = HookCounter(counter);
    pm.register(Arc::new(CountingHookPlugin {
        hook: HookType::OnSessionEnd,
        counter: shared.clone(),
        label: "on_session_end",
    }));
    pm.register(Arc::new(CountingHookPlugin {
        hook: HookType::PreApiRequest,
        counter: shared,
        label: "pre_api_request",
    }));
    let cfg = AgentConfig {
        max_turns: 2,
        session_id: Some("hook-contract-sess".into()),
        platform: Some("test".into()),
        ..AgentConfig::default()
    };
    AgentLoop::new(
        cfg,
        Arc::new(ToolRegistry::new()),
        Arc::new(StopAssistantProvider),
    )
    .with_plugins(Arc::new(Mutex::new(pm)))
}

#[tokio::test]
async fn run_natural_finish_invokes_on_session_end_and_pre_api_request() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let agent = agent_with_hook_plugins(events.clone());
    let result = agent.run(vec![Message::user("hi")], None).await;
    assert!(result.is_ok(), "{result:?}");
    assert!(result.unwrap().finished_naturally);

    let fired = events.lock().expect("events lock");
    assert!(
        fired.iter().any(|e| e == "pre_api_request"),
        "expected pre_api_request during LLM call, got {fired:?}"
    );
    assert!(
        fired.iter().any(|e| e == "on_session_end"),
        "expected on_session_end after natural finish, got {fired:?}"
    );
}
