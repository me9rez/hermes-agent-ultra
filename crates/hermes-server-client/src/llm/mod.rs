//! LLM provider backed by the remote server (OpenAI-compatible) — stub until wired.

use async_trait::async_trait;
use futures::stream::BoxStream;
use hermes_config::ServerConfig;
use hermes_core::{AgentError, LlmProvider, LlmResponse, Message, StreamChunk, ToolSchema};
use serde_json::Value;

use crate::error::ServerClientError;
use crate::session::ServerSession;
use crate::transport::HttpTransport;

/// Remote LLM gateway provider. Delegates to `OpenAiProvider` once server LLM docs land.
pub struct ServerLlmProvider {
    _config: ServerConfig,
    _transport: HttpTransport,
    _session: ServerSession,
}

impl ServerLlmProvider {
    pub fn new(
        config: ServerConfig,
        hermes_home: impl AsRef<std::path::Path>,
    ) -> Result<Self, ServerClientError> {
        if !config.enabled {
            return Err(ServerClientError::Disabled);
        }
        Ok(Self {
            _transport: HttpTransport::new(&config)?,
            _session: ServerSession::from_config(&config, hermes_home),
            _config: config,
        })
    }
}

#[async_trait]
impl LlmProvider for ServerLlmProvider {
    async fn chat_completion(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _max_tokens: Option<u32>,
        _temperature: Option<f64>,
        _model: Option<&str>,
        _extra_body: Option<&Value>,
    ) -> Result<LlmResponse, AgentError> {
        Err(ServerClientError::not_configured("server LLM chat/completions").into())
    }

    fn chat_completion_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _max_tokens: Option<u32>,
        _temperature: Option<f64>,
        _model: Option<&str>,
        _extra_body: Option<&Value>,
    ) -> BoxStream<'static, Result<StreamChunk, AgentError>> {
        use futures::stream;
        let err: AgentError = ServerClientError::not_configured("server LLM streaming").into();
        Box::pin(stream::once(async move { Err(err) }))
    }
}

impl From<ServerClientError> for AgentError {
    fn from(value: ServerClientError) -> Self {
        match value {
            ServerClientError::Agent(e) => e,
            other => AgentError::Config(other.to_string()),
        }
    }
}
