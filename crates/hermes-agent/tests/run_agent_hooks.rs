//! Phase A-9: `pre_llm_call` / `post_llm_call` / tool hooks through `AgentLoop::run`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use hermes_agent::{
    agent_loop::ToolRegistry,
    plugins::{HookResult, HookType, Plugin, PluginContext, PluginManager, PluginMeta},
    AgentConfig, AgentLoop,
};
use hermes_core::{
    AgentError, FunctionCall, JsonSchema, LlmProvider, LlmResponse, Message, StreamChunk,
    ToolCall, ToolSchema,
};

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
            description: "phase-a9 hook counter".into(),
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
        ctx.on(self.hook, Arc::new(move |_ctx_val: &serde_json::Value| {
            counter.push(label);
            HookResult::Ok
        }));
    }
}

struct ToolThenStopProvider;

#[async_trait]
impl LlmProvider for ToolThenStopProvider {
    async fn chat_completion(
        &self,
        messages: &[Message],
        _tools: &[ToolSchema],
        _max_tokens: Option<u32>,
        _temperature: Option<f64>,
        _model: Option<&str>,
        _extra_body: Option<&serde_json::Value>,
    ) -> Result<LlmResponse, AgentError> {
        let saw_tool = messages.iter().any(|m| m.tool_call_id.as_deref() == Some("tc1"));
        if !saw_tool {
            Ok(LlmResponse {
                message: Message::assistant_with_tool_calls(
                    None,
                    vec![ToolCall {
                        id: "tc1".into(),
                        function: FunctionCall {
                            name: "echo_tool".into(),
                            arguments: r#"{"msg":"hi"}"#.to_string(),
                        },
                        extra_content: None,
                    }],
                ),
                usage: None,
                model: "test".into(),
                finish_reason: Some("tool_calls".into()),
            
                ..Default::default()})
        } else {
            Ok(LlmResponse {
                message: Message::assistant("done"),
                usage: None,
                model: "test".into(),
                finish_reason: Some("stop".into()),
            ..Default::default()
        })
        }
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

fn echo_registry() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools.register(
        "echo_tool",
        ToolSchema {
            name: "echo_tool".to_string(),
            description: "echo".to_string(),
            parameters: JsonSchema::new("object"),
        },
        Arc::new(|params| {
            Ok(params
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("ok")
                .to_string())
        }),
    );
    tools
}

fn agent_with_llm_and_tool_hooks(events: Arc<Mutex<Vec<String>>>) -> AgentLoop {
    let mut pm = PluginManager::new();
    let shared = HookCounter(events);
    for (hook, label) in [
        (HookType::PreLlmCall, "pre_llm_call"),
        (HookType::PostLlmCall, "post_llm_call"),
        (HookType::PreToolCall, "pre_tool_call"),
        (HookType::PostToolCall, "post_tool_call"),
    ] {
        pm.register(Arc::new(CountingHookPlugin {
            hook,
            counter: shared.clone(),
            label,
        }));
    }
    let cfg = AgentConfig {
        max_turns: 4,
        ..AgentConfig::default()
    };
    AgentLoop::new(
        cfg,
        Arc::new(echo_registry()),
        Arc::new(ToolThenStopProvider),
    )
    .with_plugins(Arc::new(Mutex::new(pm)))
}

#[tokio::test]
async fn phase_a9_run_invokes_pre_post_llm_and_tool_hooks() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let agent = agent_with_llm_and_tool_hooks(events.clone());
    let result = agent.run(vec![Message::user("go")], None).await;
    assert!(result.is_ok(), "{result:?}");

    let fired = events.lock().expect("events lock");
    for expected in [
        "pre_llm_call",
        "post_llm_call",
        "pre_tool_call",
        "post_tool_call",
    ] {
        assert!(
            fired.iter().any(|e| e == expected),
            "expected {expected} during run, got {fired:?}"
        );
    }
}
