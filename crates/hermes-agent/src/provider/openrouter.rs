//! OpenRouterProvider — OpenRouter API provider with response caching and reasoning support.

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hermes_core::{AgentError, LlmProvider, LlmResponse, Message, StreamChunk, ToolSchema};

use crate::credential_pool::CredentialPool;
use crate::provider_profiles;
use crate::provider_serialize_cache::ProviderSerializeCache;

use super::{
    ChatRequestParams, GenericProvider, OPENROUTER_RESPONSE_CACHE, OpenRouterResponseCache,
    OpenRouterResponseCacheControl, OpenRouterResponseCacheEntry, parse_openai_response,
};

/// OpenRouter API provider with support for OpenRouter-specific parameters.
///
/// Adds:
/// - `HTTP-Referer` and `X-Title` headers (required by OpenRouter)
/// - Support for `transforms`, `provider` preferences, `route` in extra_body
/// - Parsing of `reasoning_details` array from responses
/// - `reasoning_content` extraction
#[derive(Debug, Clone)]
pub struct OpenRouterProvider {
    inner: GenericProvider,
    /// HTTP-Referer header value (required by OpenRouter).
    pub http_referer: Option<String>,
    /// X-Title header value (required by OpenRouter).
    pub x_title: Option<String>,
}

impl OpenRouterProvider {
    /// Create a new OpenRouter provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: GenericProvider::new("https://openrouter.ai/api/v1", api_key, "openai/gpt-4o")
                .with_provider_profile("openrouter"),
            http_referer: None,
            x_title: None,
        }
    }

    /// Set the default model.
    pub fn with_model(self, model: impl Into<String>) -> Self {
        Self {
            inner: self.inner.with_model(model),
            ..self
        }
    }

    pub fn with_base_url(self, base_url: impl Into<String>) -> Self {
        Self {
            inner: self.inner.with_base_url(base_url),
            ..self
        }
    }

    /// Set an optional total request timeout used by this provider and rebuilds.
    pub fn with_optional_request_timeout_seconds(self, seconds: Option<f64>) -> Self {
        Self {
            inner: self.inner.with_optional_request_timeout_seconds(seconds),
            ..self
        }
    }

    /// Set the HTTP-Referer header (required by OpenRouter).
    pub fn with_http_referer(mut self, referer: impl Into<String>) -> Self {
        self.http_referer = Some(referer.into());
        self
    }

    /// Set the X-Title header (required by OpenRouter).
    pub fn with_x_title(mut self, title: impl Into<String>) -> Self {
        self.x_title = Some(title.into());
        self
    }

    /// Attach a credential pool for API key rotation.
    pub fn with_credential_pool(self, pool: Arc<CredentialPool>) -> Self {
        Self {
            inner: self.inner.with_credential_pool(pool),
            ..self
        }
    }

    pub(crate) fn with_serialize_cache(self, cache: Arc<ProviderSerializeCache>) -> Self {
        Self {
            inner: self.inner.with_serialize_cache(cache),
            ..self
        }
    }

    /// Build the extra headers including OpenRouter-specific ones.
    pub(crate) fn build_headers(&self) -> Vec<(String, String)> {
        let mut headers = self.inner.extra_headers.clone();
        if let Some(ref referer) = self.http_referer {
            headers.push(("HTTP-Referer".to_string(), referer.clone()));
        }
        if let Some(ref title) = self.x_title {
            headers.push(("X-Title".to_string(), title.clone()));
        }
        headers
    }

    fn openrouter_response_cache_enabled() -> bool {
        std::env::var("HERMES_OPENROUTER_RESPONSE_CACHE")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on" | "enabled"
                )
            })
            .unwrap_or(false)
    }

    fn openrouter_response_cache_ttl_secs() -> u64 {
        std::env::var("HERMES_OPENROUTER_RESPONSE_CACHE_TTL_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(300)
    }

    fn openrouter_response_cache_max_entries() -> usize {
        std::env::var("HERMES_OPENROUTER_RESPONSE_CACHE_MAX_ENTRIES")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(256)
    }

    pub(crate) fn parse_response_cache_control(
        extra_body: Option<&Value>,
    ) -> OpenRouterResponseCacheControl {
        let mut enabled = Self::openrouter_response_cache_enabled();
        let mut clear = false;
        let mut ttl_secs = Self::openrouter_response_cache_ttl_secs();

        if let Some(Value::Object(map)) = extra_body {
            if let Some(v) = map.get("response_cache_enabled").and_then(Value::as_bool) {
                enabled = v;
            }
            if let Some(v) = map.get("response_cache_clear").and_then(Value::as_bool) {
                clear = v;
            }
            if let Some(v) = map
                .get("response_cache_ttl_secs")
                .and_then(Value::as_u64)
                .filter(|v| *v > 0)
            {
                ttl_secs = v;
            }
            if let Some(Value::Bool(flag)) = map.get("response_cache") {
                enabled = *flag;
            }
            if let Some(Value::Object(cache_cfg)) = map.get("response_cache") {
                if let Some(v) = cache_cfg.get("enabled").and_then(Value::as_bool) {
                    enabled = v;
                }
                if let Some(v) = cache_cfg.get("clear").and_then(Value::as_bool) {
                    clear = v;
                }
                if let Some(v) = cache_cfg
                    .get("ttl_secs")
                    .and_then(Value::as_u64)
                    .filter(|v| *v > 0)
                {
                    ttl_secs = v;
                }
            }
        }

        OpenRouterResponseCacheControl {
            enabled,
            clear,
            ttl_secs,
        }
    }

    pub(crate) fn merge_extra_body(extra_body: Option<&Value>) -> Option<Value> {
        let Some(Value::Object(map)) = extra_body else {
            return extra_body.cloned();
        };
        let mut cleaned = map.clone();
        cleaned.remove("response_cache");
        cleaned.remove("response_cache_enabled");
        cleaned.remove("response_cache_ttl_secs");
        cleaned.remove("response_cache_clear");
        cleaned.remove("strict_tool_calls");
        cleaned.remove("strict_api");
        cleaned.remove("provider_strict");
        if !cleaned.contains_key("reasoning") {
            if let Some(effort) = cleaned.remove("reasoning_effort") {
                cleaned.insert(
                    "reasoning".to_string(),
                    serde_json::json!({ "effort": effort }),
                );
            }
        } else {
            cleaned.remove("reasoning_effort");
        }
        Some(Value::Object(cleaned))
    }

    fn response_cache_key(model: &str, body: &Value) -> Option<String> {
        let encoded = serde_json::to_vec(body).ok()?;
        let mut hasher = Sha256::new();
        hasher.update(model.as_bytes());
        hasher.update(b"\n");
        hasher.update(encoded);
        Some(hex::encode(hasher.finalize()))
    }

    fn response_cache_get(key: &str) -> Option<LlmResponse> {
        let cache = OPENROUTER_RESPONSE_CACHE
            .get_or_init(|| Mutex::new(OpenRouterResponseCache::default()));
        let mut guard = cache.lock().expect("openrouter cache lock poisoned");
        let now = Instant::now();
        if let Some(entry) = guard.entries.get(key) {
            if now < entry.expires_at {
                return Some(entry.response.clone());
            }
        }
        guard.entries.remove(key);
        guard.order.retain(|k| k != key);
        None
    }

    fn response_cache_insert(key: String, response: &LlmResponse, ttl_secs: u64) {
        let cache = OPENROUTER_RESPONSE_CACHE
            .get_or_init(|| Mutex::new(OpenRouterResponseCache::default()));
        let mut guard = cache.lock().expect("openrouter cache lock poisoned");
        let now = Instant::now();
        guard.entries.insert(
            key.clone(),
            OpenRouterResponseCacheEntry {
                response: response.clone(),
                expires_at: now + Duration::from_secs(ttl_secs.max(1)),
            },
        );
        guard.order.retain(|k| k != &key);
        guard.order.push_back(key);
        while guard.entries.len() > Self::openrouter_response_cache_max_entries() {
            if let Some(evict) = guard.order.pop_front() {
                guard.entries.remove(&evict);
            } else {
                break;
            }
        }
    }

    pub(crate) fn parse_openrouter_response(json: &Value) -> Result<LlmResponse, AgentError> {
        let mut response = parse_openai_response(json)?;

        // Extract reasoning_content from various locations
        if let Some(reasoning) = crate::reasoning::parse_reasoning(json) {
            response.message.reasoning_content = Some(reasoning);
        }

        Ok(response)
    }
}

#[async_trait]
impl LlmProvider for OpenRouterProvider {
    async fn chat_completion(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        max_tokens: Option<u32>,
        temperature: Option<f64>,
        model: Option<&str>,
        extra_body: Option<&Value>,
    ) -> Result<LlmResponse, AgentError> {
        // Build a provider clone with OpenRouter headers
        let mut provider = self.inner.clone();
        provider.extra_headers = self.build_headers();
        let effective_model = model.unwrap_or(&self.inner.model);
        provider
            .extra_headers
            .extend(provider_profiles::extra_headers_for_profile(
                Some("openrouter"),
                effective_model,
                extra_body,
            ));
        let cache_control = Self::parse_response_cache_control(extra_body);
        if cache_control.enabled {
            provider
                .extra_headers
                .push(("X-OpenRouter-Cache".to_string(), "true".to_string()));
            if cache_control.clear {
                provider
                    .extra_headers
                    .push(("X-OpenRouter-Cache-Clear".to_string(), "true".to_string()));
            }
        }

        let merged_extra = Self::merge_extra_body(extra_body);

        // Use GenericProvider for the actual request
        provider.check_rate_limit().await;

        let api_key = provider.effective_api_key();
        let body = provider.chat_request_body(ChatRequestParams {
            messages,
            tools,
            max_tokens,
            temperature,
            effective_model,
            extra_body: merged_extra.as_ref(),
            stream: false,
        });

        let cache_key = if cache_control.enabled && !cache_control.clear {
            Self::response_cache_key(effective_model, &body)
        } else {
            None
        };
        if let Some(ref key) = cache_key {
            if let Some(hit) = Self::response_cache_get(key) {
                return Ok(hit);
            }
        }

        let url = format!(
            "{}/chat/completions",
            provider.base_url.trim_end_matches('/')
        );

        let resp = provider
            .send_with_dead_connection_recovery(&url, &api_key, &body)
            .await?;

        provider.update_rate_limit(resp.headers());

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
        let parsed = Self::parse_openrouter_response(&resp_json)?;
        if let Some(key) = cache_key {
            Self::response_cache_insert(key, &parsed, cache_control.ttl_secs);
        }
        Ok(parsed)
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
        // Use GenericProvider's streaming with OpenRouter headers
        let mut provider = self.inner.clone();
        provider.extra_headers = self.build_headers();
        let effective_model = model.unwrap_or(&self.inner.model);
        provider
            .extra_headers
            .extend(provider_profiles::extra_headers_for_profile(
                Some("openrouter"),
                effective_model,
                extra_body,
            ));
        let merged_extra = Self::merge_extra_body(extra_body);

        provider.chat_completion_stream(
            messages,
            tools,
            max_tokens,
            temperature,
            model,
            merged_extra.as_ref(),
        )
    }
}
