//! GenericProvider — a flexible, config-driven provider for any OpenAI-compatible API.

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use reqwest::Client;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hermes_core::{AgentError, LlmProvider, LlmResponse, Message, StreamChunk, ToolSchema};

use crate::credential_pool::CredentialPool;
use crate::provider_profiles;
use crate::provider_serialize_cache::ProviderSerializeCache;
use crate::rate_limit::RateLimitTracker;

use super::{
    ChatRequestParams, build_provider_http_client, flat_model_name, flatten_multimodal_parts_text,
    is_moonshot_model, opencode_go_deepseek_reasoning_effort, opencode_go_kimi_reasoning_effort,
    parse_acp_multimodal_parts, parse_openai_response, parse_sse_chunk, request_timeout_duration,
    sanitize_moonshot_tools_value,
};

/// A generic LLM provider that can be configured for any OpenAI-compatible API.
///
/// This is the primary provider used by the agent loop. It supports
/// OpenAI-compatible APIs via configuration.
#[derive(Debug, Clone)]
pub struct GenericProvider {
    /// Base URL for the API endpoint.
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
    /// Optional custom headers to send with every request.
    pub extra_headers: Vec<(String, String)>,
    /// Optional rate limit tracker.
    pub rate_limiter: Option<Arc<RateLimitTracker>>,
    /// Optional credential pool for key rotation.
    pub credential_pool: Option<Arc<CredentialPool>>,
    serialize_cache: Option<Arc<ProviderSerializeCache>>,
    /// Optional OpenAI-compatible provider profile used for request shaping.
    pub provider_profile: Option<String>,
}

impl GenericProvider {
    /// Create a new generic provider.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let request_timeout = None;
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            client: Arc::new(Mutex::new(build_provider_http_client(request_timeout))),
            request_timeout,
            client_refreshed_at: Arc::new(Mutex::new(Instant::now())),
            extra_headers: Vec::new(),
            rate_limiter: None,
            credential_pool: None,
            serialize_cache: None,
            provider_profile: None,
        }
    }

    /// Add a custom header to be sent with every request.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((key.into(), value.into()));
        self
    }

    /// Set a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Set the default model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
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

    /// Attach a Rust-native provider profile for request shaping.
    pub fn with_provider_profile(mut self, profile: impl Into<String>) -> Self {
        self.provider_profile =
            provider_profiles::canonical_provider_profile_id(&profile.into()).map(str::to_string);
        self
    }

    /// Attach a rate limit tracker.
    pub fn with_rate_limiter(mut self, tracker: Arc<RateLimitTracker>) -> Self {
        self.rate_limiter = Some(tracker);
        self
    }

    /// Attach a credential pool for API key rotation.
    pub fn with_credential_pool(mut self, pool: Arc<CredentialPool>) -> Self {
        self.credential_pool = Some(pool);
        self
    }

    /// Share per-turn sanitize/tools JSON cache (LLM retry fast path).
    pub(crate) fn with_serialize_cache(mut self, cache: Arc<ProviderSerializeCache>) -> Self {
        self.serialize_cache = Some(cache);
        self
    }

    fn sanitized_messages_for_request(
        &self,
        messages: &[Message],
        strict: bool,
        effective_model: &str,
        profile: Option<&str>,
        extra_body: Option<&Value>,
    ) -> Value {
        if let Some(cache) = &self.serialize_cache {
            cache.sanitized_openai_messages(messages, strict, effective_model, profile, extra_body)
        } else {
            Self::sanitize_messages_for_api(messages, strict, effective_model, profile, extra_body)
        }
    }

    fn formatted_tools_for_request(&self, tools: &[ToolSchema], effective_model: &str) -> Value {
        let mut formatted = if let Some(cache) = &self.serialize_cache {
            cache.formatted_openai_tools(tools)
        } else {
            Self::format_tools_for_openai_api(tools)
        };
        if is_moonshot_model(effective_model)
            || AnthropicProvider::is_kimi_coding_endpoint(Some(&self.base_url))
        {
            sanitize_moonshot_tools_value(&mut formatted);
        }
        formatted
    }

    /// Get the effective API key, using the credential pool if available.
    pub(crate) fn effective_api_key(&self) -> String {
        if let Some(ref pool) = self.credential_pool {
            pool.get_key()
        } else {
            self.api_key.clone()
        }
    }

    /// Check rate limits before making a request. Waits if needed.
    pub(crate) async fn check_rate_limit(&self) {
        if let Some(ref tracker) = self.rate_limiter {
            if let Some(wait_duration) = tracker.should_wait() {
                tracing::info!(
                    "Rate limited, waiting {:?} before next request",
                    wait_duration
                );
                tokio::time::sleep(wait_duration).await;
            }
        }
    }

    /// Update rate limit state from response headers.
    pub(crate) fn update_rate_limit(&self, headers: &reqwest::header::HeaderMap) {
        if let Some(ref tracker) = self.rate_limiter {
            tracker.update_from_headers(headers);
        }
    }

    fn capture_nous_credits_headers(&self, headers: &reqwest::header::HeaderMap) {
        let _ = hermes_core::credits::capture_nous_credits_from_pairs(headers.iter().filter_map(
            |(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (key.as_str().to_string(), value.to_string()))
            },
        ));
    }

    /// Inject optional runtime hints: reasoning effort, vision preprocessing,
    /// and service tier.
    fn apply_runtime_hints(
        &self,
        body: &mut Value,
        messages: &[Message],
        extra_body: Option<&Value>,
    ) {
        // Reasoning effort passthrough (`low|medium|high`) using extra_body.reasoning_effort.
        if let Some(eb) = extra_body
            .and_then(|v| v.get("reasoning_effort"))
            .and_then(|v| v.as_str())
        {
            body["reasoning_effort"] = serde_json::json!(eb);
        }

        // OpenAI service tier passthrough.
        if let Some(st) = extra_body
            .and_then(|v| v.get("service_tier"))
            .and_then(|v| v.as_str())
        {
            body["service_tier"] = serde_json::json!(st);
        }

        // Vision preprocessing: if user content contains local file-like paths,
        // add a hint field used by downstream adapters.
        let needs_vision_preprocess = messages.iter().any(|m| {
            m.content.as_ref().is_some_and(|c| {
                c.contains(".png") || c.contains(".jpg") || c.contains("data:image/")
            })
        });
        if needs_vision_preprocess {
            body["vision_preprocessed"] = serde_json::json!(true);
        }
    }

    pub(crate) fn apply_opencode_go_reasoning_controls(
        &self,
        body: &mut Value,
        effective_model: &str,
    ) {
        if !self
            .base_url
            .to_ascii_lowercase()
            .contains("opencode.ai/zen/go")
        {
            return;
        }

        let model = flat_model_name(effective_model);
        let is_kimi_k2 = model.starts_with("kimi-k2");
        let is_deepseek_thinking = (model.starts_with("deepseek-v")
            && !model.starts_with("deepseek-v3"))
            || model == "deepseek-reasoner";

        let reasoning = body.get("reasoning").cloned();
        let mut enabled = true;
        let mut effort = body
            .get("reasoning_effort")
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_ascii_lowercase());

        if let Some(reasoning_obj) = reasoning.as_ref().and_then(|value| value.as_object()) {
            enabled = reasoning_obj
                .get("enabled")
                .and_then(|value| value.as_bool())
                .unwrap_or(true);
            if effort.is_none() {
                effort = reasoning_obj
                    .get("effort")
                    .and_then(|value| value.as_str())
                    .map(|value| value.trim().to_ascii_lowercase());
            }
        }

        if let Some(map) = body.as_object_mut() {
            map.remove("reasoning");
            map.remove("reasoning_effort");
        }

        if is_kimi_k2 {
            if reasoning.is_none() && effort.is_none() {
                return;
            }
            body["thinking"] =
                serde_json::json!({ "type": if enabled { "enabled" } else { "disabled" } });
            if enabled {
                if let Some(mapped) = opencode_go_kimi_reasoning_effort(effort.as_deref()) {
                    body["reasoning_effort"] = serde_json::json!(mapped);
                }
            }
            return;
        }

        if is_deepseek_thinking {
            body["thinking"] =
                serde_json::json!({ "type": if enabled { "enabled" } else { "disabled" } });
            if enabled {
                if let Some(mapped) = opencode_go_deepseek_reasoning_effort(effort.as_deref()) {
                    body["reasoning_effort"] = serde_json::json!(mapped);
                }
            }
        }
    }

    /// Force-close helper for future explicit TCP cleanup hooks.
    pub fn force_close_tcp_sockets(&self) {
        // reqwest handles connection pooling internally; dropping clones and relying
        // on idle timeout is currently sufficient for our runtime.
    }

    fn current_client(&self) -> Client {
        self.client
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|_| build_provider_http_client(self.request_timeout))
    }

    pub(crate) fn refresh_client(&self, reason: &str) {
        tracing::warn!("rebuilding primary HTTP client: {}", reason);
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
                if Self::is_connection_recoverable(&e) {
                    self.refresh_client(&format!("stale connection probe failed: {e}"));
                } else if let Ok(mut t) = self.client_refreshed_at.lock() {
                    *t = Instant::now();
                }
            }
        }
    }

    pub(crate) fn is_connection_recoverable(err: &reqwest::Error) -> bool {
        if err.is_connect() || err.is_timeout() || err.is_request() {
            return true;
        }
        let msg = err.to_string().to_lowercase();
        msg.contains("connection reset")
            || msg.contains("connection closed")
            || msg.contains("broken pipe")
            || msg.contains("pool")
            || msg.contains("eof")
    }

    fn should_sanitize_tool_calls(extra_body: Option<&Value>) -> bool {
        extra_body
            .and_then(|v| {
                v.get("strict_tool_calls")
                    .or_else(|| v.get("strict_api"))
                    .or_else(|| v.get("provider_strict"))
            })
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn is_local_request_control_key(key: &str) -> bool {
        matches!(key, "strict_tool_calls" | "strict_api" | "provider_strict")
            || provider_profiles::local_control_key_for_profile(None, key)
    }

    pub(crate) fn merge_extra_body_fields(body: &mut Value, extra_body: Option<&Value>) {
        let Some(Value::Object(map)) = extra_body else {
            return;
        };
        for (k, v) in map {
            if Self::is_local_request_control_key(k) {
                continue;
            }
            body[k] = v.clone();
        }
    }

    fn profile_for_extra_body<'a>(&'a self, extra_body: Option<&'a Value>) -> Option<&'a str> {
        extra_body
            .and_then(|value| value.get("provider_profile"))
            .and_then(Value::as_str)
            .and_then(provider_profiles::canonical_provider_profile_id)
            .or(self.provider_profile.as_deref())
    }

    fn merge_extra_body_fields_for_profile(
        body: &mut Value,
        profile: Option<&str>,
        extra_body: Option<&Value>,
    ) {
        let cleaned = provider_profiles::clean_extra_body_for_profile(profile, extra_body);
        Self::merge_extra_body_fields(body, cleaned.as_ref());
    }

    pub(crate) fn sanitize_messages_for_api(
        messages: &[Message],
        enabled: bool,
        effective_model: &str,
        profile: Option<&str>,
        extra_body: Option<&Value>,
    ) -> Value {
        let model_supports_vision =
            Self::supports_multimodal_tool_results(profile, effective_model, extra_body);
        let mut out = Vec::with_capacity(messages.len());
        for msg in messages {
            let mut api_msg = serde_json::to_value(msg).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(parts) = api_msg
                .get("content")
                .and_then(|v| v.as_str())
                .and_then(parse_acp_multimodal_parts)
            {
                api_msg["content"] = if model_supports_vision {
                    Value::Array(parts)
                } else {
                    Value::String(flatten_multimodal_parts_text(&parts))
                };
            }
            if !enabled {
                out.push(api_msg);
                continue;
            }
            if let Some(tool_calls) = api_msg.get_mut("tool_calls").and_then(|v| v.as_array_mut()) {
                for tc in tool_calls.iter_mut() {
                    if let Some(obj) = tc.as_object_mut() {
                        let id = obj.get("id").cloned();
                        let function = obj.get("function").cloned().or_else(|| {
                            let name = obj.get("name").and_then(|v| v.as_str())?.to_string();
                            let args_raw = obj
                                .get("arguments")
                                .cloned()
                                .unwrap_or_else(|| Value::String("{}".to_string()));
                            let (args, _) = arguments_value_to_string(Some(&args_raw));
                            Some(serde_json::json!({
                                "name": name,
                                "arguments": args,
                            }))
                        });
                        let mut stripped = serde_json::Map::new();
                        if let Some(v) = id {
                            stripped.insert("id".to_string(), v);
                        }
                        stripped.insert(
                            "type".to_string(),
                            obj.get("type")
                                .cloned()
                                .unwrap_or_else(|| Value::String("function".to_string())),
                        );
                        if let Some(v) = function {
                            stripped.insert("function".to_string(), v);
                        }
                        *obj = stripped;
                    }
                }
            }
            out.push(api_msg);
        }
        Value::Array(out)
    }

    fn supports_multimodal_tool_results(
        profile: Option<&str>,
        effective_model: &str,
        extra_body: Option<&Value>,
    ) -> bool {
        if let Some(value) = extra_body
            .and_then(|body| body.get("supports_vision"))
            .and_then(Value::as_bool)
        {
            return value;
        }
        profile.is_some_and(provider_profiles::supports_vision) || supports_vision(effective_model)
    }

    pub(crate) fn format_tools_for_openai_api(tools: &[ToolSchema]) -> Value {
        let formatted = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    })
                })
                .collect(),
        );
        hermes_core::sanitize_tool_schemas(Some(&formatted)).unwrap_or(formatted)
    }

    pub fn chat_request_body(&self, request: ChatRequestParams<'_>) -> Value {
        let ChatRequestParams {
            messages,
            tools,
            max_tokens,
            temperature,
            effective_model,
            extra_body,
            stream,
        } = request;
        let profile = self.profile_for_extra_body(extra_body);
        let strict_tool_sanitize = Self::should_sanitize_tool_calls(extra_body);
        let mut api_messages = self.sanitized_messages_for_request(
            messages,
            strict_tool_sanitize,
            effective_model,
            profile,
            extra_body,
        );
        provider_profiles::normalize_messages_for_profile(profile, &mut api_messages);

        let mut body = serde_json::json!({
            "model": effective_model,
            "messages": api_messages,
        });
        if stream {
            body["stream"] = Value::Bool(true);
        }

        if let Some(mt) =
            max_tokens.or_else(|| profile.and_then(provider_profiles::default_max_tokens))
        {
            body["max_tokens"] = serde_json::json!(mt);
        }
        if !profile.is_some_and(provider_profiles::omit_temperature) {
            if let Some(temp) = temperature {
                body["temperature"] = serde_json::json!(temp);
            }
        }
        if !tools.is_empty() {
            body["tools"] = self.formatted_tools_for_request(tools, effective_model);
        }
        Self::merge_extra_body_fields_for_profile(&mut body, profile, extra_body);
        self.apply_runtime_hints(&mut body, messages, extra_body);
        provider_profiles::apply_profile_to_body(profile, &mut body, effective_model, extra_body);
        self.apply_opencode_go_reasoning_controls(&mut body, effective_model);
        body
    }

    pub(crate) fn build_request(
        &self,
        client: &Client,
        url: &str,
        api_key: &str,
        body: &Value,
    ) -> reqwest::RequestBuilder {
        let mut req = client
            .post(url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(body);
        if provider_profiles::is_kimi_code_base_url(&self.base_url) {
            req = req.header("User-Agent", provider_profiles::KIMI_CODE_USER_AGENT);
        }
        for (key, value) in &self.extra_headers {
            req = req.header(key.as_str(), value.as_str());
        }
        req
    }

    pub(crate) async fn send_with_dead_connection_recovery(
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
                if !Self::is_connection_recoverable(&e) {
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
}

// ---------------------------------------------------------------------------
// LlmProvider impl for GenericProvider
// ---------------------------------------------------------------------------

use super::AnthropicProvider;
use crate::tool_call_args::arguments_value_to_string;
use hermes_intelligence::supports_vision;

#[async_trait]
impl LlmProvider for GenericProvider {
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
        let body = self.chat_request_body(ChatRequestParams {
            messages,
            tools,
            max_tokens,
            temperature,
            effective_model,
            extra_body,
            stream: false,
        });

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let resp = self
            .send_with_dead_connection_recovery(&url, &api_key, &body)
            .await?;

        self.update_rate_limit(resp.headers());
        self.capture_nous_credits_headers(resp.headers());

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

        parse_openai_response(&resp_json)
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
            let mut body = provider.chat_request_body(ChatRequestParams {
                messages: &messages,
                tools: &tools,
                max_tokens,
                temperature,
                effective_model,
                extra_body: extra_body.as_ref(),
                stream: true,
            });
            // Request usage in the final streaming chunk
            body["stream_options"] = serde_json::json!({"include_usage": true});

            let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));

            let resp = match provider
                .send_with_dead_connection_recovery(&url, &api_key, &body)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };

            provider.update_rate_limit(resp.headers());
            provider.capture_nous_credits_headers(resp.headers());

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_else(|_| "<no body>".to_string());
                yield Err(AgentError::LlmApi(format!("API error {status}: {body_text}")));
                return;
            }

            // Read the SSE byte stream, using a cursor-based buffer to
            // avoid O(N²) copies from repeated `buffer[offset..].to_string()`.
            let mut byte_stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::with_capacity(8192);
            let mut cursor: usize = 0;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk_bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        yield Err(AgentError::LlmApi(format!("Stream read error: {e}")));
                        return;
                    }
                };

                buf.extend_from_slice(&chunk_bytes);

                // Process complete SSE events (separated by double newlines).
                // Search forward from cursor, not from the start of buf.
                while let Some(pos) = buf[cursor..]
                    .windows(2)
                    .position(|w| w == b"\n\n")
                {
                    let event_end = cursor + pos;
                    let event_block = &buf[cursor..event_end];

                    for line_bytes in event_block.split(|&b| b == b'\n') {
                        let line = std::str::from_utf8(line_bytes)
                            .unwrap_or("")
                            .trim();
                        if line.is_empty() || line.starts_with(':') {
                            continue;
                        }
                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim();
                            if data == "[DONE]" {
                                // Stream finished
                                return;
                            }
                            match serde_json::from_str::<Value>(data) {
                                Ok(json) => {
                                    if let Some(chunk) = parse_sse_chunk(&json) {
                                        yield Ok(chunk);
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to parse SSE data: {e}");
                                }
                            }
                        }
                    }

                    cursor = event_end + 2; // skip \n\n
                }

                // Drain processed bytes from the front of the buffer to keep
                // memory bounded (avoids unbounded growth on long streams).
                if cursor > 4096 {
                    buf.drain(..cursor);
                    cursor = 0;
                }
            }

            // Process any remaining data in the buffer
            let remaining = std::str::from_utf8(&buf[cursor..]).unwrap_or("").trim();
            if !remaining.is_empty() {
                for line in remaining.lines() {
                    let line = line.trim();
                    if let Some(data) = line.strip_prefix("data: ") {
                        let data = data.trim();
                        if data == "[DONE]" {
                            return;
                        }
                        if let Ok(json) = serde_json::from_str::<Value>(data) {
                            if let Some(chunk) = parse_sse_chunk(&json) {
                                yield Ok(chunk);
                            }
                        }
                    }
                }
            }
        }
        .boxed()
    }
}
