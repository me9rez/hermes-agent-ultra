//! ACP CLI handler.

use std::sync::Arc;

use hermes_agent::{RunConversationParams, split_messages_for_run_conversation};
use hermes_tools::tools::messaging::MessagingSessionContext;
pub(crate) const ACP_MULTIMODAL_PREFIX: &str = "__hermes_acp_parts_json__:";

fn looks_like_openai_parts(parts: &[serde_json::Value]) -> bool {
    !parts.is_empty()
        && parts.iter().all(|part| {
            part.as_object()
                .and_then(|obj| obj.get("type"))
                .and_then(|v| v.as_str())
                .is_some()
        })
}

fn flatten_openai_parts_to_text(parts: &[serde_json::Value]) -> String {
    let mut chunks: Vec<String> = Vec::new();
    for part in parts {
        let Some(obj) = part.as_object() else {
            continue;
        };
        let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "text" => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        chunks.push(text.to_string());
                    }
                }
            }
            "image_url" | "input_image" => {
                let url = obj
                    .get("image_url")
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str())
                    .or_else(|| obj.get("image_url").and_then(|v| v.as_str()))
                    .or_else(|| obj.get("url").and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !url.is_empty() {
                    chunks.push(format!("[Attached image]\nURL: {url}"));
                }
            }
            _ => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        chunks.push(text.to_string());
                    }
                }
            }
        }
    }
    chunks.join("\n")
}

pub(crate) fn acp_history_to_messages(
    history: &[serde_json::Value],
    fallback_user_text: &str,
) -> Vec<hermes_core::Message> {
    let mut messages = Vec::new();

    for item in history {
        let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content_value = item.get("content").or_else(|| item.get("text"));
        let content = match content_value {
            Some(serde_json::Value::String(s)) => s.to_string(),
            Some(serde_json::Value::Array(parts)) if looks_like_openai_parts(parts) => {
                if role == "user" {
                    match serde_json::to_string(parts) {
                        Ok(serialized) => format!("{ACP_MULTIMODAL_PREFIX}{serialized}"),
                        Err(_) => flatten_openai_parts_to_text(parts),
                    }
                } else {
                    flatten_openai_parts_to_text(parts)
                }
            }
            Some(serde_json::Value::Object(obj)) => obj
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        };

        match role {
            "system" if !content.is_empty() => messages.push(hermes_core::Message::system(content)),
            "user" if !content.is_empty() => messages.push(hermes_core::Message::user(content)),
            "assistant" => {
                if let Some(tool_calls_val) = item.get("tool_calls") {
                    if let Ok(tool_calls) =
                        serde_json::from_value::<Vec<hermes_core::ToolCall>>(tool_calls_val.clone())
                    {
                        let assistant = hermes_core::Message::assistant_with_tool_calls(
                            if content.is_empty() {
                                None
                            } else {
                                Some(content)
                            },
                            tool_calls,
                        );
                        messages.push(assistant);
                        continue;
                    }
                }
                if !content.is_empty() {
                    messages.push(hermes_core::Message::assistant(content));
                }
            }
            "tool" if !content.is_empty() => {
                let tool_call_id = item
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool_call");
                messages.push(hermes_core::Message::tool_result(tool_call_id, content));
            }
            _ => {}
        }
    }

    let has_user_tail = messages
        .last()
        .map(|m| matches!(m.role, hermes_core::MessageRole::User))
        .unwrap_or(false);
    if !has_user_tail && !fallback_user_text.trim().is_empty() {
        messages.push(hermes_core::Message::user(fallback_user_text));
    }

    messages
}

struct CliAcpPromptExecutor {
    config: Arc<hermes_config::GatewayConfig>,
    tool_registry: Arc<hermes_tools::ToolRegistry>,
    tool_schemas: Vec<hermes_core::ToolSchema>,
}

#[async_trait::async_trait]
impl hermes_acp::AcpPromptExecutor for CliAcpPromptExecutor {
    async fn execute_prompt(
        &self,
        session: &hermes_acp::SessionState,
        user_text: &str,
        history: &[serde_json::Value],
    ) -> Result<hermes_acp::PromptExecutionOutput, String> {
        let model = session
            .model
            .clone()
            .or_else(|| self.config.model.clone())
            .unwrap_or_else(|| "gpt-4o".to_string());

        let provider = crate::app::build_provider(&self.config, &model);
        let mut agent_config = crate::app::build_agent_config(&self.config, &model);
        agent_config.session_id = Some(session.session_id.clone());

        let agent_tools = Arc::new(crate::app::bridge_tool_registry(&self.tool_registry));
        let agent = hermes_agent::attach_agent_runtime(
            hermes_agent::AgentLoop::new(agent_config, agent_tools, provider)
                .with_async_tool_dispatch(crate::app::async_tool_dispatch_for(
                    self.tool_registry.clone(),
                )),
        );
        let messages = acp_history_to_messages(history, user_text);
        let (conversation_history, user_message) =
            split_messages_for_run_conversation(&messages)
                .ok_or_else(|| "ACP prompt has no user message for run_conversation".to_string())?;
        let task_id = Some(session.session_id.clone());
        let conv = agent
            .run_conversation(RunConversationParams {
                user_message,
                conversation_history,
                task_id,
                stream_callback: None,
                persist_user_message: None,
                tools: Some(self.tool_schemas.clone()),
                persist_session: false,
            })
            .await
            .map_err(|e| e.to_string())?;
        let result = conv.into_loop_result();
        let response_text = result
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, hermes_core::MessageRole::Assistant))
            .and_then(|m| m.content.clone())
            .unwrap_or_default();

        let usage = result.usage.map(|u| hermes_acp::Usage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
            thought_tokens: None,
            cached_read_tokens: None,
        });

        Ok(hermes_acp::PromptExecutionOutput {
            response_text,
            usage,
            total_turns: Some(result.total_turns),
            events: Vec::new(),
        })
    }
}

/// Handle `hermes acp [action]`.
pub async fn handle_cli_acp(action: Option<String>) -> Result<(), hermes_core::AgentError> {
    match action.as_deref().unwrap_or("status") {
        "start" => {
            let config = hermes_config::load_config(None)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;

            let model = config.model.clone().unwrap_or_else(|| "gpt-4o".to_string());
            let max_turns = config.max_turns as usize;

            println!(
                "Starting ACP server (model={}, max_turns={})...",
                model, max_turns
            );

            let tool_registry = Arc::new(hermes_tools::ToolRegistry::new());
            let terminal_backend = crate::terminal_backend::build_terminal_backend(&config);
            let skill_store = Arc::new(hermes_skills::FileSkillStore::new(
                hermes_skills::FileSkillStore::default_dir(),
            ));
            let skill_provider: Arc<dyn hermes_core::SkillProvider> =
                Arc::new(hermes_skills::SkillManager::new(skill_store));
            hermes_tools::register_builtin_tools(&tool_registry, terminal_backend, skill_provider);
            crate::runtime_tool_wiring::wire_stdio_clarify_backend(&tool_registry);
            let cron_data_dir = hermes_config::cron_dir();
            std::fs::create_dir_all(&cron_data_dir)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            let cron_scheduler = Arc::new(hermes_cron::cron_scheduler_for_data_dir(cron_data_dir));
            cron_scheduler
                .load_persisted_jobs()
                .await
                .map_err(|e| hermes_core::AgentError::Config(format!("cron load: {e}")))?;
            cron_scheduler.start().await;
            crate::runtime_tool_wiring::wire_cron_scheduler_backend(
                &tool_registry,
                cron_scheduler,
                MessagingSessionContext::new(),
            );
            let tool_schemas = crate::platform_toolsets::resolve_platform_tool_schemas(
                &config,
                "cli",
                &tool_registry,
            );

            let prompt_executor = Arc::new(CliAcpPromptExecutor {
                config: Arc::new(config.clone()),
                tool_registry,
                tool_schemas,
            });

            let session_manager = Arc::new(hermes_acp::SessionManager::new());
            let event_sink = Arc::new(hermes_acp::EventSink::default());
            let permission_store = Arc::new(hermes_acp::PermissionStore::new());
            let handler = Arc::new(
                hermes_acp::HermesAcpHandler::new(
                    session_manager.clone(),
                    event_sink.clone(),
                    permission_store.clone(),
                )
                .with_prompt_executor(prompt_executor),
            );
            let server = hermes_acp::AcpServer::with_components(
                handler,
                session_manager,
                event_sink,
                permission_store,
            );

            server
                .run()
                .await
                .map_err(|e| hermes_core::AgentError::Io(format!("ACP server error: {}", e)))?;
        }
        "status" => {
            println!("ACP server: not running");
            println!("ACP runs as a stdio JSON-RPC server in the foreground.");
            println!("Start with `hermes acp start`.");
        }
        "stop" => {
            println!("ACP stop is not a separate command in stdio mode.");
            println!("If running, stop it by closing the parent process or sending Ctrl+C.");
        }
        "restart" => {
            println!("ACP restart in stdio mode is equivalent to stop + start.");
            println!("Use:");
            println!("  1) Stop the current process (Ctrl+C)");
            println!("  2) Run `hermes acp start`");
        }
        other => {
            println!("Unknown ACP action '{}'.", other);
            println!("Available actions: start, status, stop, restart");
        }
    }
    Ok(())
}
