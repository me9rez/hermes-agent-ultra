//! AnthropicProvider — Anthropic Messages API provider.

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use reqwest::Client;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hermes_core::{
    AgentError, FunctionCall, FunctionCallDelta, LlmProvider, LlmResponse, Message, MessageRole,
    StreamChunk, StreamDelta, ToolCall, ToolCallDelta, ToolSchema, UsageStats,
};
use hermes_intelligence::anthropic_adapter::{
    default_anthropic_beta_header_value, forbids_sampling_params, get_anthropic_max_output,
    is_azure_anthropic_endpoint, is_oauth_token, is_third_party_endpoint, requires_bearer_auth,
    supports_fast_mode,
};

use crate::credential_pool::CredentialPool;
use crate::provider_serialize_cache::ProviderSerializeCache;
use crate::rate_limit::RateLimitTracker;

use super::{
    GenericProvider, anthropic_blocks_from_multimodal_parts, build_provider_http_client,
    flatten_multimodal_parts_text, parse_acp_multimodal_parts, request_timeout_duration,
};

/// Anthropic API provider with native Messages API support.
///
/// Uses Anthropic's own message format rather than OpenAI-compatible format:
/// - System message goes in `system` parameter, not in messages array
/// - Uses `x-api-key` header instead of `Authorization: Bearer`
/// - Content blocks use array format with typed blocks
/// - Tool use returns `type: "tool_use"` content blocks
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    /// Base URL for the Anthropic API.
    pub base_url: String,
    /// API key for authentication.
    pub api_key: String,
    /// Default model identifier.
    pub model: String,
    /// HTTP client.
    client: Arc<Mutex<Client>>,
    /// Optional total request timeout applied to newly-built clients.
    request_timeout: Option<Duration>,
    /// Last time we rebuilt the client transport.
    client_refreshed_at: Arc<Mutex<Instant>>,
    /// Anthropic API version header.
    pub api_version: String,
    /// Optional rate limit tracker.
    pub rate_limiter: Option<Arc<RateLimitTracker>>,
    /// Optional credential pool.
    pub credential_pool: Option<Arc<CredentialPool>>,
    serialize_cache: Option<Arc<ProviderSerializeCache>>,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        let request_timeout = None;
        Self {
            base_url: "https://api.anthropic.com".to_string(),
            api_key: api_key.into(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            client: Arc::new(Mutex::new(build_provider_http_client(request_timeout))),
            request_timeout,
            client_refreshed_at: Arc::new(Mutex::new(Instant::now())),
            api_version: "2023-06-01".to_string(),
            rate_limiter: None,
            credential_pool: None,
            serialize_cache: None,
        }
    }

    /// Set the default model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Set a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Set an optional total request timeout used by this provider and rebuilds.
    pub fn with_optional_request_timeout_seconds(mut self, seconds: Option<f64>) -> Self {
        self.request_timeout = request_timeout_duration(seconds);
        if let Ok(mut client) = self.client.lock() {
            *client = build_provider_http_client(self.request_timeout);
        }
        self
    }

    /// Set a total request timeout in seconds.
    pub fn with_request_timeout_seconds(self, seconds: f64) -> Self {
        self.with_optional_request_timeout_seconds(Some(seconds))
    }

    #[cfg(test)]
    pub(crate) fn configured_request_timeout(&self) -> Option<Duration> {
        self.request_timeout
    }

    /// Attach a rate limit tracker.
    pub fn with_rate_limiter(mut self, tracker: Arc<RateLimitTracker>) -> Self {
        self.rate_limiter = Some(tracker);
        self
    }

    /// Attach a credential pool.
    pub fn with_credential_pool(mut self, pool: Arc<CredentialPool>) -> Self {
        self.credential_pool = Some(pool);
        self
    }

    pub(crate) fn with_serialize_cache(mut self, cache: Arc<ProviderSerializeCache>) -> Self {
        self.serialize_cache = Some(cache);
        self
    }

    fn converted_messages_for_request(&self, messages: &[Message]) -> (Option<Value>, Vec<Value>) {
        if let Some(cache) = &self.serialize_cache {
            cache.converted_anthropic_messages(messages, &self.base_url)
        } else {
            Self::convert_messages(messages, Some(self.base_url.as_str()))
        }
    }

    fn formatted_tools_for_request(&self, tools: &[ToolSchema]) -> Value {
        if tools.is_empty() {
            return Value::Array(vec![]);
        }
        if let Some(cache) = &self.serialize_cache {
            cache.formatted_anthropic_tools(tools)
        } else {
            Value::Array(Self::convert_tools(tools))
        }
    }

    fn effective_api_key(&self) -> String {
        if let Some(ref pool) = self.credential_pool {
            pool.get_key()
        } else {
            self.api_key.clone()
        }
    }

    async fn check_rate_limit(&self) {
        if let Some(ref tracker) = self.rate_limiter {
            if let Some(wait_duration) = tracker.should_wait() {
                tracing::info!("Rate limited, waiting {:?}", wait_duration);
                tokio::time::sleep(wait_duration).await;
            }
        }
    }

    fn update_rate_limit(&self, headers: &reqwest::header::HeaderMap) {
        if let Some(ref tracker) = self.rate_limiter {
            tracker.update_from_headers(headers);
        }
    }

    fn current_client(&self) -> Client {
        self.client
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|_| build_provider_http_client(self.request_timeout))
    }

    pub(crate) fn refresh_client(&self, reason: &str) {
        tracing::warn!("rebuilding anthropic HTTP client: {}", reason);
        if let Ok(mut c) = self.client.lock() {
            *c = build_provider_http_client(self.request_timeout);
        }
        if let Ok(mut t) = self.client_refreshed_at.lock() {
            *t = Instant::now();
        }
    }

    async fn maybe_refresh_stale_client(&self, probe_url: &str) {
        const STALE_CLIENT_REFRESH_SECS: u64 = 300;
        let stale_after = Duration::from_secs(STALE_CLIENT_REFRESH_SECS);
        let should_refresh = self
            .client_refreshed_at
            .lock()
            .map(|t| t.elapsed() >= stale_after)
            .unwrap_or(false);
        if !should_refresh {
            return;
        }
        let probe_client = self.current_client();
        match probe_client
            .get(probe_url)
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            Ok(_) => {
                if let Ok(mut t) = self.client_refreshed_at.lock() {
                    *t = Instant::now();
                }
            }
            Err(e) => {
                if GenericProvider::is_connection_recoverable(&e) {
                    self.refresh_client(&format!("stale connection probe failed: {e}"));
                } else if let Ok(mut t) = self.client_refreshed_at.lock() {
                    *t = Instant::now();
                }
            }
        }
    }

    pub(crate) fn build_request(
        &self,
        client: &Client,
        url: &str,
        api_key: &str,
        body: &Value,
    ) -> reqwest::RequestBuilder {
        let base_url = self.base_url.as_str();
        let native_oauth = !is_third_party_endpoint(Some(base_url)) && is_oauth_token(api_key);
        let bearer_auth = native_oauth || requires_bearer_auth(Some(base_url));
        let beta_header = default_anthropic_beta_header_value(Some(base_url), native_oauth);

        let mut request = client
            .post(url)
            .header("anthropic-version", &self.api_version)
            .header("Content-Type", "application/json")
            .header("anthropic-beta", beta_header)
            .json(body);

        if bearer_auth {
            request = request.header("Authorization", format!("Bearer {api_key}"));
        } else {
            request = request.header("x-api-key", api_key);
        }
        if native_oauth {
            request = request
                .header("user-agent", "claude-cli/2.1.74 (external, cli)")
                .header("x-app", "cli");
        }
        request
    }

    pub(crate) fn messages_url(&self) -> String {
        Self::messages_url_for_base_url(&self.base_url)
    }

    pub(crate) fn messages_url_for_base_url(base_url: &str) -> String {
        let trimmed = base_url.trim().trim_end_matches('/');
        if let Ok(mut url) = reqwest::Url::parse(trimmed) {
            let mut path = url.path().trim_end_matches('/').to_string();
            if path.is_empty() {
                path.push_str("/v1/messages");
            } else if path.ends_with("/v1") {
                path.push_str("/messages");
            } else {
                path.push_str("/v1/messages");
            }
            url.set_path(&path);
            if is_azure_anthropic_endpoint(Some(trimmed))
                && !url
                    .query_pairs()
                    .any(|(key, _)| key.eq_ignore_ascii_case("api-version"))
            {
                url.query_pairs_mut()
                    .append_pair("api-version", "2025-04-15");
            }
            return url.to_string();
        }

        let mut url = format!("{trimmed}/v1/messages");
        if is_azure_anthropic_endpoint(Some(trimmed)) && !url.contains("api-version=") {
            let sep = if url.contains('?') { '&' } else { '?' };
            url.push(sep);
            url.push_str("api-version=2025-04-15");
        }
        url
    }

    pub(crate) fn strip_unsupported_anthropic_controls(body: &mut Value, model: &str) {
        let Some(obj) = body.as_object_mut() else {
            return;
        };
        if forbids_sampling_params(model) {
            obj.remove("temperature");
            obj.remove("top_p");
            obj.remove("top_k");
        }
        if obj.get("speed").and_then(Value::as_str) == Some("fast") && !supports_fast_mode(model) {
            obj.remove("speed");
        }
    }

    async fn send_with_dead_connection_recovery(
        &self,
        url: &str,
        api_key: &str,
        body: &Value,
    ) -> Result<reqwest::Response, AgentError> {
        self.maybe_refresh_stale_client(url).await;
        let client = self.current_client();
        match self.build_request(&client, url, api_key, body).send().await {
            Ok(resp) => Ok(resp),
            Err(e) => {
                if !GenericProvider::is_connection_recoverable(&e) {
                    return Err(AgentError::LlmApi(format!("HTTP request failed: {e}")));
                }
                self.refresh_client(&format!("recoverable transport error: {e}"));
                let retry_client = self.current_client();
                self.build_request(&retry_client, url, api_key, body)
                    .send()
                    .await
                    .map_err(|e2| {
                        AgentError::LlmApi(format!(
                            "HTTP request failed after reconnect retry: {e2}"
                        ))
                    })
            }
        }
    }

    pub(crate) fn is_kimi_coding_endpoint(base_url: Option<&str>) -> bool {
        let Some(url) = base_url else {
            return false;
        };
        let lower = url.to_lowercase();
        lower.contains("api.kimi.com")
            || lower.contains("moonshot.ai")
            || lower.contains("moonshot.cn")
    }

    pub(crate) fn resolve_messages_max_tokens(requested: Option<u32>, model: &str) -> u32 {
        if let Some(value) = requested.filter(|v| *v > 0) {
            return value;
        }
        get_anthropic_max_output(model).max(1)
    }

    fn is_native_anthropic_endpoint(base_url: Option<&str>) -> bool {
        let host = base_url
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|url| {
                let without_scheme = url.split("://").nth(1).unwrap_or(url);
                without_scheme
                    .split('/')
                    .next()
                    .map(|h| h.split(':').next().unwrap_or(h).to_ascii_lowercase())
            });
        host.as_deref() == Some("api.anthropic.com")
    }

    fn attach_cache_control(block: &mut Value, cc: &hermes_core::types::CacheControl) {
        if let Value::Object(map) = block {
            map.insert("cache_control".to_string(), cc.to_api_json());
        }
    }

    /// Convert internal messages to Anthropic format, extracting system message.
    pub(crate) fn convert_messages(
        messages: &[Message],
        base_url: Option<&str>,
    ) -> (Option<Value>, Vec<Value>) {
        let mut system_blocks: Vec<Value> = Vec::new();
        let mut anthropic_messages: Vec<Value> = Vec::new();
        let is_kimi_endpoint = Self::is_kimi_coding_endpoint(base_url);
        let native_anthropic = Self::is_native_anthropic_endpoint(base_url);

        for msg in messages {
            match msg.role {
                MessageRole::System => {
                    let content = msg.content.as_deref().unwrap_or("");
                    let mut block = serde_json::json!({"type": "text", "text": content});
                    if let Some(ref cc) = msg.cache_control {
                        Self::attach_cache_control(&mut block, cc);
                    }
                    system_blocks.push(block);
                }
                MessageRole::User => {
                    let mut content_blocks = Vec::new();
                    if let Some(ref text) = msg.content {
                        if let Some(parts) = parse_acp_multimodal_parts(text) {
                            content_blocks.extend(anthropic_blocks_from_multimodal_parts(&parts));
                            if content_blocks.is_empty() {
                                let fallback = flatten_multimodal_parts_text(&parts);
                                if !fallback.is_empty() {
                                    content_blocks.push(
                                        serde_json::json!({"type": "text", "text": fallback}),
                                    );
                                }
                            }
                        } else {
                            let mut block = serde_json::json!({"type": "text", "text": text});
                            if let Some(ref cc) = msg.cache_control {
                                Self::attach_cache_control(&mut block, cc);
                            }
                            content_blocks.push(block);
                        }
                    } else if msg.cache_control.is_some() {
                        let mut block = serde_json::json!({"type": "text", "text": ""});
                        if let Some(ref cc) = msg.cache_control {
                            Self::attach_cache_control(&mut block, cc);
                        }
                        content_blocks.push(block);
                    }
                    if !content_blocks.is_empty() {
                        anthropic_messages.push(serde_json::json!({
                            "role": "user",
                            "content": content_blocks,
                        }));
                    }
                }
                MessageRole::Assistant => {
                    let mut content_blocks = Vec::new();
                    if is_kimi_endpoint
                        && msg
                            .tool_calls
                            .as_ref()
                            .is_some_and(|calls| !calls.is_empty())
                        && msg.reasoning_content.is_some()
                    {
                        let thinking = msg.reasoning_content.as_deref().unwrap_or("");
                        content_blocks
                            .push(serde_json::json!({"type": "thinking", "thinking": thinking}));
                    }
                    if let Some(ref text) = msg.content {
                        if !text.is_empty() {
                            content_blocks.push(serde_json::json!({"type": "text", "text": text}));
                        }
                    }
                    if let Some(ref tool_calls) = msg.tool_calls {
                        for tc in tool_calls {
                            let input: Value = serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(serde_json::json!({}));
                            content_blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.function.name,
                                "input": input,
                            }));
                        }
                    }
                    if !content_blocks.is_empty() {
                        if let Some(ref cc) = msg.cache_control {
                            if let Some(last) = content_blocks.last_mut() {
                                Self::attach_cache_control(last, cc);
                            }
                        }
                        anthropic_messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": content_blocks,
                        }));
                    }
                }
                MessageRole::Tool => {
                    let mut block = serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                        "content": msg.content.as_deref().unwrap_or(""),
                    });
                    if native_anthropic {
                        if let Some(ref cc) = msg.cache_control {
                            Self::attach_cache_control(&mut block, cc);
                        }
                    }
                    anthropic_messages.push(serde_json::json!({
                        "role": "user",
                        "content": vec![block],
                    }));
                }
            }
        }

        let system = if system_blocks.is_empty() {
            None
        } else if system_blocks
            .iter()
            .any(|b| b.get("cache_control").is_some())
        {
            Some(Value::Array(system_blocks))
        } else {
            let merged = system_blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n\n");
            Some(Value::String(merged))
        };

        (system, anthropic_messages)
    }

    /// Convert tool schemas to Anthropic tool format.
    pub(crate) fn convert_tools(tools: &[ToolSchema]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect()
    }

    pub(crate) fn parse_response(json: &Value) -> Result<LlmResponse, AgentError> {
        let mut content_text = String::new();
        let mut reasoning_content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        if let Some(content_arr) = json.get("content").and_then(|c| c.as_array()) {
            for block in content_arr {
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            if !content_text.is_empty() {
                                content_text.push('\n');
                            }
                            content_text.push_str(text);
                        }
                    }
                    "tool_use" => {
                        let id = block
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                        let arguments =
                            serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
                        tool_calls.push(ToolCall {
                            id,
                            function: FunctionCall { name, arguments },
                            extra_content: None,
                        });
                    }
                    "thinking" => {
                        if let Some(thinking) = block.get("thinking").and_then(|t| t.as_str()) {
                            if !reasoning_content.is_empty() {
                                reasoning_content.push('\n');
                            }
                            reasoning_content.push_str(thinking);
                        }
                    }
                    _ => {}
                }
            }
        }

        let usage = json.get("usage").and_then(|u| {
            let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            Some(UsageStats {
                prompt_tokens: input,
                completion_tokens: output,
                total_tokens: input + output,
                estimated_cost: None,
                ..Default::default()
            })
        });

        let stop_reason = json
            .get("stop_reason")
            .and_then(|s| s.as_str())
            .map(|s| match s {
                "end_turn" => "stop".to_string(),
                "tool_use" => "tool_calls".to_string(),
                other => other.to_string(),
            });

        let model = json
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        let message = Message {
            role: MessageRole::Assistant,
            content: Some(content_text),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: None,
            name: None,
            reasoning_content: if reasoning_content.is_empty() {
                None
            } else {
                Some(reasoning_content)
            },
            cache_control: None,
        };

        Ok(LlmResponse {
            message,
            usage,
            model,
            finish_reason: stop_reason,
            ..Default::default()
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat_completion(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        max_tokens: Option<u32>,
        temperature: Option<f64>,
        model: Option<&str>,
        extra_body: Option<&Value>,
    ) -> Result<LlmResponse, AgentError> {
        self.check_rate_limit().await;

        let effective_model = model.unwrap_or(&self.model);
        let api_key = self.effective_api_key();
        let (system, anthropic_messages) = self.converted_messages_for_request(messages);
        let resolved_max_tokens = Self::resolve_messages_max_tokens(max_tokens, effective_model);

        let mut body = serde_json::json!({
            "model": effective_model,
            "messages": anthropic_messages,
            "max_tokens": resolved_max_tokens,
        });

        if let Some(sys) = system {
            body["system"] = sys;
        }
        if let Some(temp) = temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if !tools.is_empty() {
            body["tools"] = self.formatted_tools_for_request(tools);
        }
        GenericProvider::merge_extra_body_fields(&mut body, extra_body);
        Self::strip_unsupported_anthropic_controls(&mut body, effective_model);

        let url = self.messages_url();
        let resp = self
            .send_with_dead_connection_recovery(&url, &api_key, &body)
            .await?;

        self.update_rate_limit(resp.headers());

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(AgentError::LlmApi(format!(
                "API error {status}: {body_text}"
            )));
        }

        let resp_json: Value = resp
            .json()
            .await
            .map_err(|e| AgentError::LlmApi(format!("Failed to parse response: {e}")))?;

        Self::parse_response(&resp_json)
    }

    fn chat_completion_stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        max_tokens: Option<u32>,
        temperature: Option<f64>,
        model: Option<&str>,
        extra_body: Option<&Value>,
    ) -> BoxStream<'static, Result<StreamChunk, AgentError>> {
        let provider = self.clone();
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        let model = model.map(|s| s.to_string());
        let extra_body = extra_body.cloned();

        async_stream::stream! {
            provider.check_rate_limit().await;

            let effective_model = model.as_deref().unwrap_or(&provider.model);
            let api_key = provider.effective_api_key();
            let (system, anthropic_messages) = provider.converted_messages_for_request(&messages);
            let resolved_max_tokens =
                AnthropicProvider::resolve_messages_max_tokens(max_tokens, effective_model);

            let mut body = serde_json::json!({
                "model": effective_model,
                "messages": anthropic_messages,
                "max_tokens": resolved_max_tokens,
                "stream": true,
            });

            if let Some(sys) = system {
                body["system"] = sys;
            }
            if let Some(temp) = temperature {
                body["temperature"] = serde_json::json!(temp);
            }
            if !tools.is_empty() {
                body["tools"] = provider.formatted_tools_for_request(&tools);
            }
            GenericProvider::merge_extra_body_fields(&mut body, extra_body.as_ref());
            AnthropicProvider::strip_unsupported_anthropic_controls(&mut body, effective_model);

            let url = provider.messages_url();

            let resp = match provider.send_with_dead_connection_recovery(&url, &api_key, &body).await {
                Ok(r) => r,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };

            provider.update_rate_limit(resp.headers());

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_else(|_| "<no body>".to_string());
                yield Err(AgentError::LlmApi(format!("API error {status}: {body_text}")));
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut buffer = String::new();
            // Track current tool_use block index for delta accumulation
            let mut current_tool_index: u32 = 0;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk_bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        yield Err(AgentError::LlmApi(format!("Stream read error: {e}")));
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk_bytes));

                while let Some(event_end) = buffer.find("\n\n") {
                    let event_block = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();

                    let mut event_type = String::new();
                    let mut event_data = String::new();

                    for line in event_block.lines() {
                        let line = line.trim();
                        if let Some(et) = line.strip_prefix("event: ") {
                            event_type = et.trim().to_string();
                        } else if let Some(d) = line.strip_prefix("data: ") {
                            event_data = d.trim().to_string();
                        }
                    }

                    if event_data.is_empty() {
                        continue;
                    }

                    let json: Value = match serde_json::from_str(&event_data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    match event_type.as_str() {
                        "content_block_start" => {
                            let block = json.get("content_block").unwrap_or(&json);
                            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if block_type == "tool_use" {
                                let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
                                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                                let idx = json.get("index").and_then(|i| i.as_u64()).unwrap_or(current_tool_index as u64) as u32;
                                current_tool_index = idx;
                                yield Ok(StreamChunk {
                                    delta: Some(StreamDelta {
                                        content: None,
                                        tool_calls: Some(vec![ToolCallDelta {
                                            index: idx,
                                            id: Some(id),
                                            function: Some(FunctionCallDelta {
                                                name: Some(name),
                                                arguments: None,
                                            }),
                                        }]),
                                        extra: None,
                                    }),
                                    finish_reason: None,
                                    usage: None,
                                });
                            }
                        }
                        "content_block_delta" => {
                            let delta = json.get("delta").unwrap_or(&json);
                            let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            match delta_type {
                                "text_delta" => {
                                    let text = delta.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                                    yield Ok(StreamChunk {
                                        delta: Some(StreamDelta {
                                            content: Some(text),
                                            tool_calls: None,
                                            extra: None,
                                        }),
                                        finish_reason: None,
                                        usage: None,
                                    });
                                }
                                "input_json_delta" => {
                                    let partial = delta.get("partial_json").and_then(|p| p.as_str()).unwrap_or("").to_string();
                                    yield Ok(StreamChunk {
                                        delta: Some(StreamDelta {
                                            content: None,
                                            tool_calls: Some(vec![ToolCallDelta {
                                                index: current_tool_index,
                                                id: None,
                                                function: Some(FunctionCallDelta {
                                                    name: None,
                                                    arguments: Some(partial),
                                                }),
                                            }]),
                                            extra: None,
                                        }),
                                        finish_reason: None,
                                        usage: None,
                                    });
                                }
                                "thinking_delta" => {
                                    if let Some(thinking) =
                                        delta.get("thinking").and_then(|t| t.as_str())
                                    {
                                        yield Ok(StreamChunk {
                                            delta: Some(StreamDelta {
                                                content: None,
                                                tool_calls: None,
                                                extra: Some(
                                                    serde_json::json!({"thinking": thinking}),
                                                ),
                                            }),
                                            finish_reason: None,
                                            usage: None,
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                        "message_delta" => {
                            let stop_reason = json
                                .get("delta")
                                .and_then(|d| d.get("stop_reason"))
                                .and_then(|s| s.as_str())
                                .map(|s| match s {
                                    "end_turn" => "stop".to_string(),
                                    "tool_use" => "tool_calls".to_string(),
                                    other => other.to_string(),
                                });
                            let usage = json.get("usage").and_then(|u| {
                                let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                                Some(UsageStats {
                                    prompt_tokens: 0,
                                    completion_tokens: output,
                                    total_tokens: output,
        estimated_cost: None,
        ..Default::default()
    })
                            });
                            yield Ok(StreamChunk {
                                delta: None,
                                finish_reason: stop_reason,
                                usage,
                            });
                        }
                        "message_start" => {
                            // Extract usage from the initial message
                            let usage = json.get("message").and_then(|m| m.get("usage")).and_then(|u| {
                                let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                                Some(UsageStats {
                                    prompt_tokens: input,
                                    completion_tokens: 0,
                                    total_tokens: input,
        estimated_cost: None,
        ..Default::default()
    })
                            });
                            if let Some(u) = usage {
                                yield Ok(StreamChunk {
                                    delta: None,
                                    finish_reason: None,
                                    usage: Some(u),
                                });
                            }
                        }
                        "message_stop" => {
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }
        .boxed()
    }
}
