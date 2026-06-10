//! Shared internal types, constants, and utility functions for the provider module.

use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use reqwest::Client;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use hermes_core::{
    AgentError, FunctionCallDelta, LlmResponse, Message, MessageRole, StreamChunk, StreamDelta,
    ToolCallDelta, ToolSchema, UsageStats,
};

use crate::tool_call_args::arguments_value_to_string;

use super::{AnthropicProvider, GenericProvider, OpenAiProvider};

pub struct ChatRequestParams<'a> {
    pub messages: &'a [Message],
    pub tools: &'a [ToolSchema],
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub effective_model: &'a str,
    pub extra_body: Option<&'a Value>,
    pub stream: bool,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub(crate) const ACP_MULTIMODAL_PREFIX: &str = "__hermes_acp_parts_json__:";
pub const OPENAI_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub(crate) const CODEX_CLOUDFLARE_ORIGINATOR: &str = "codex_cli_rs";

// ---------------------------------------------------------------------------
// Rate-limit / timeout / HTTP client helpers
// ---------------------------------------------------------------------------

pub(crate) fn rate_limit_headers_json(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let mut map = HashMap::new();
    for (k, v) in headers.iter() {
        let kl = k.as_str().to_ascii_lowercase();
        if kl.starts_with("x-ratelimit-") || kl == "retry-after" {
            if let Ok(s) = v.to_str() {
                map.insert(kl, s.to_string());
            }
        }
    }
    if map.is_empty() {
        None
    } else {
        serde_json::to_string(&map).ok()
    }
}

pub(crate) fn request_timeout_duration(seconds: Option<f64>) -> Option<Duration> {
    seconds.and_then(|value| {
        if value.is_finite() && value > 0.0 {
            Duration::try_from_secs_f64(value).ok()
        } else {
            None
        }
    })
}

pub(crate) fn build_provider_http_client(request_timeout: Option<Duration>) -> Client {
    let mut builder = Client::builder();
    if let Some(timeout) = request_timeout {
        builder = builder.timeout(timeout);
    }
    builder.build().unwrap_or_else(|err| {
        tracing::warn!("failed to build provider HTTP client: {}", err);
        Client::new()
    })
}

// ---------------------------------------------------------------------------
// Codex / Cloudflare helpers
// ---------------------------------------------------------------------------

pub fn codex_cloudflare_headers(access_token: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![
        (
            "originator".to_string(),
            CODEX_CLOUDFLARE_ORIGINATOR.to_string(),
        ),
        (
            "User-Agent".to_string(),
            format!(
                "{CODEX_CLOUDFLARE_ORIGINATOR}/{}",
                env!("CARGO_PKG_VERSION")
            ),
        ),
    ];

    if let Some(account_id) = access_token.and_then(codex_chatgpt_account_id) {
        headers.push(("ChatGPT-Account-ID".to_string(), account_id));
    }

    headers
}

pub fn openai_codex_provider(
    api_key: impl Into<String>,
    model: impl Into<String>,
    base_url: Option<&str>,
) -> OpenAiProvider {
    openai_codex_provider_with_timeout(api_key, model, base_url, None)
}

pub fn openai_codex_provider_with_timeout(
    api_key: impl Into<String>,
    model: impl Into<String>,
    base_url: Option<&str>,
    request_timeout_seconds: Option<f64>,
) -> OpenAiProvider {
    let api_key = api_key.into();
    let base_url = base_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(OPENAI_CODEX_BASE_URL)
        .to_string();
    let mut provider = OpenAiProvider::new(api_key.as_str())
        .with_model(model)
        .with_base_url(base_url.as_str())
        .with_optional_request_timeout_seconds(request_timeout_seconds);
    if is_codex_cloudflare_base_url(base_url.as_str()) {
        provider = provider.with_headers(codex_cloudflare_headers(Some(api_key.as_str())));
    }
    provider
}

pub(crate) fn is_codex_cloudflare_base_url(base_url: &str) -> bool {
    base_url
        .trim()
        .to_ascii_lowercase()
        .contains("chatgpt.com/backend-api/codex")
}

pub(crate) fn codex_chatgpt_account_id(token: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .or_else(|_| URL_SAFE.decode(payload.as_bytes()))
        .ok()?;
    let claims: Value = serde_json::from_slice(&decoded).ok()?;
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// ACP multimodal parts helpers
// ---------------------------------------------------------------------------

pub(crate) fn parse_acp_multimodal_parts(content: &str) -> Option<Vec<Value>> {
    let payload = content.trim().strip_prefix(ACP_MULTIMODAL_PREFIX)?;
    let parsed: Value = serde_json::from_str(payload).ok()?;
    let parts = parsed.as_array()?.clone();
    if parts.is_empty() {
        return None;
    }
    if !parts.iter().all(|part| {
        part.as_object()
            .and_then(|obj| obj.get("type"))
            .and_then(|v| v.as_str())
            .is_some()
    }) {
        return None;
    }
    Some(parts)
}

pub(crate) fn flatten_multimodal_parts_text(parts: &[Value]) -> String {
    let mut lines = Vec::new();
    for part in parts {
        let Some(obj) = part.as_object() else {
            continue;
        };
        let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "text" => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        lines.push(text.to_string());
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
                    lines.push(format!("[Attached image]\nURL: {url}"));
                }
            }
            _ => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        lines.push(text.to_string());
                    }
                }
            }
        }
    }
    lines.join("\n")
}

pub(crate) fn anthropic_blocks_from_multimodal_parts(parts: &[Value]) -> Vec<Value> {
    let mut blocks = Vec::new();
    for part in parts {
        let Some(obj) = part.as_object() else {
            continue;
        };
        let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "text" => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        blocks.push(serde_json::json!({"type": "text", "text": text}));
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
                    let source =
                        hermes_intelligence::anthropic_adapter::image_source_from_openai_url(&url);
                    blocks.push(serde_json::json!({"type": "image", "source": source}));
                }
            }
            _ => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        blocks.push(serde_json::json!({"type": "text", "text": text}));
                    }
                }
            }
        }
    }
    blocks
}

// ---------------------------------------------------------------------------
// Model name helpers
// ---------------------------------------------------------------------------

pub(crate) fn flat_model_name(model: &str) -> String {
    let normalized = model.trim().to_ascii_lowercase();
    let parts = normalized
        .split(['/', ':'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    for part in parts.iter().rev() {
        if part.starts_with("kimi-k2")
            || part.starts_with("deepseek-v")
            || *part == "deepseek-reasoner"
        {
            return (*part).to_string();
        }
    }

    parts
        .last()
        .copied()
        .unwrap_or(normalized.as_str())
        .to_string()
}

pub(crate) fn opencode_go_kimi_reasoning_effort(effort: Option<&str>) -> Option<&'static str> {
    match effort.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" | "max" => Some("high"),
        _ => None,
    }
}

pub(crate) fn opencode_go_deepseek_reasoning_effort(effort: Option<&str>) -> Option<&'static str> {
    match effort.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" | "max" => Some("max"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Moonshot sanitizers
// ---------------------------------------------------------------------------

pub(crate) fn is_moonshot_model(model: &str) -> bool {
    let lower = model.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    flat_model_name(&lower).starts_with("kimi-k2") || lower.contains("moonshotai/")
}

pub(crate) fn format_tools_for_openai_api_with_model(
    tools: &[ToolSchema],
    effective_model: &str,
    base_url: &str,
) -> Value {
    let mut formatted = GenericProvider::format_tools_for_openai_api(tools);
    if is_moonshot_model(effective_model)
        || AnthropicProvider::is_kimi_coding_endpoint(Some(base_url))
    {
        sanitize_moonshot_tools_value(&mut formatted);
    }
    formatted
}

pub(crate) fn sanitize_moonshot_tools_value(tools: &mut Value) {
    let Some(items) = tools.as_array_mut() else {
        return;
    };
    for tool in items {
        let Some(function) = tool.get_mut("function").and_then(Value::as_object_mut) else {
            continue;
        };
        let params = function
            .remove("parameters")
            .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
        function.insert(
            "parameters".to_string(),
            sanitize_moonshot_tool_parameters(&params),
        );
    }
}

pub(crate) fn sanitize_moonshot_tool_parameters(params: &Value) -> Value {
    let mut root = match params.as_object() {
        Some(obj) => Value::Object(obj.clone()),
        None => serde_json::json!({"type": "object", "properties": {}}),
    };
    sanitize_moonshot_schema_node(&mut root, true);
    root
}

pub(crate) fn sanitize_moonshot_schema_node(node: &mut Value, top_level: bool) {
    let Some(obj) = node.as_object_mut() else {
        if top_level {
            *node = serde_json::json!({"type": "object", "properties": {}});
        }
        return;
    };

    let has_ref = obj.contains_key("$ref");

    if let Some(any_of) = obj.get_mut("anyOf").and_then(Value::as_array_mut) {
        for branch in any_of.iter_mut() {
            sanitize_moonshot_schema_node(branch, false);
        }
        any_of.retain(|branch| {
            branch
                .get("type")
                .and_then(Value::as_str)
                .is_none_or(|kind| kind != "null")
        });
    }

    if obj.contains_key("anyOf") {
        let non_null = obj
            .get("anyOf")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if non_null.len() == 1 {
            obj.remove("anyOf");
            if let Some(branch_obj) = non_null
                .into_iter()
                .next()
                .and_then(|v| v.as_object().cloned())
            {
                for (key, value) in branch_obj {
                    obj.insert(key, value);
                }
            }
        } else {
            obj.remove("type");
        }
    }

    obj.remove("nullable");

    if let Some(props) = obj.get_mut("properties").and_then(Value::as_object_mut) {
        for value in props.values_mut() {
            sanitize_moonshot_schema_node(value, false);
        }
    }

    if let Some(items) = obj.get_mut("items") {
        sanitize_moonshot_schema_node(items, false);
    }

    if let Some(any_of) = obj.get_mut("anyOf").and_then(Value::as_array_mut) {
        for branch in any_of {
            sanitize_moonshot_schema_node(branch, false);
        }
    }

    clean_moonshot_enum(obj);

    if top_level {
        obj.insert("type".to_string(), Value::String("object".to_string()));
        if !obj.get("properties").is_some_and(Value::is_object) {
            obj.insert(
                "properties".to_string(),
                Value::Object(serde_json::Map::new()),
            );
        }
        return;
    }

    if !has_ref && !obj.contains_key("type") && !obj.contains_key("anyOf") {
        let inferred = infer_moonshot_schema_type(obj);
        obj.insert("type".to_string(), Value::String(inferred.to_string()));
        clean_moonshot_enum(obj);
    }
}

pub(crate) fn clean_moonshot_enum(obj: &mut serde_json::Map<String, Value>) {
    let scalar = matches!(
        obj.get("type").and_then(Value::as_str),
        Some("string" | "integer" | "number" | "boolean")
    );
    if !scalar {
        return;
    }
    let Some(values) = obj.get_mut("enum").and_then(Value::as_array_mut) else {
        return;
    };
    values.retain(|value| !value.is_null() && !matches!(value.as_str(), Some("")));
    if values.is_empty() {
        obj.remove("enum");
    }
}

pub(crate) fn infer_moonshot_schema_type(obj: &serde_json::Map<String, Value>) -> &'static str {
    if obj.get("properties").is_some_and(Value::is_object) {
        return "object";
    }
    if obj.contains_key("items") {
        return "array";
    }
    if let Some(values) = obj.get("enum").and_then(Value::as_array) {
        if let Some(first) = values
            .iter()
            .find(|value| !value.is_null() && !matches!(value.as_str(), Some("")))
        {
            if first.is_boolean() {
                return "boolean";
            }
            if first.is_i64() || first.is_u64() {
                return "integer";
            }
            if first.is_number() {
                return "number";
            }
        }
    }
    "string"
}

// ---------------------------------------------------------------------------
// SSE and response parsing helpers
// ---------------------------------------------------------------------------

/// Parse a single SSE data JSON object into a StreamChunk (OpenAI format).
pub(crate) fn parse_sse_chunk(json: &Value) -> Option<StreamChunk> {
    let choices = json.get("choices").and_then(|c| c.as_array())?;
    let choice = choices.first()?;

    let delta_obj = choice.get("delta")?;

    let content = delta_obj
        .get("content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());

    let tool_calls = delta_obj
        .get("tool_calls")
        .and_then(|tc| tc.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|tc| {
                    let index = tc.get("index").and_then(|i| i.as_u64())? as u32;
                    let id = tc.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
                    let function = tc.get("function").map(|f| FunctionCallDelta {
                        name: f
                            .get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_string()),
                        arguments: f
                            .get("arguments")
                            .and_then(|a| a.as_str())
                            .map(|s| s.to_string()),
                    });
                    Some(ToolCallDelta {
                        index,
                        id,
                        function,
                    })
                })
                .collect::<Vec<_>>()
        });

    let delta = if content.is_some() || tool_calls.is_some() {
        Some(StreamDelta {
            content,
            tool_calls,
            extra: None,
        })
    } else {
        None
    };

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|f| f.as_str())
        .map(|s| s.to_string());

    // Usage may appear in the final chunk
    let usage = json.get("usage").and_then(|u| {
        Some(UsageStats {
            prompt_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            completion_tokens: u
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            estimated_cost: None,
            ..Default::default()
        })
    });

    Some(StreamChunk {
        delta,
        finish_reason,
        usage,
    })
}

/// Parse an OpenAI-style chat completion response.
pub(crate) fn parse_openai_response(json: &Value) -> Result<LlmResponse, AgentError> {
    let choices = json
        .get("choices")
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            AgentError::LlmApi(format!(
                "No choices in response ({})",
                summarize_openai_response_shape(json)
            ))
        })?;

    let choice = choices.first().ok_or_else(|| {
        AgentError::LlmApi(format!(
            "Empty choices array ({})",
            summarize_openai_response_shape(json)
        ))
    })?;

    let message_obj = choice
        .get("message")
        .ok_or_else(|| AgentError::LlmApi("No message in choice".to_string()))?;

    // Parse content
    let content = message_obj
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    // Parse tool calls
    let tool_calls = message_obj
        .get("tool_calls")
        .and_then(|tc| tc.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|tc| {
                    let id = tc.get("id")?.as_str()?.to_string();
                    let function = tc.get("function")?;
                    let name = function.get("name")?.as_str()?.to_string();
                    let (arguments, _) = arguments_value_to_string(function.get("arguments"));
                    let extra_content = tc.get("extra_content").filter(|v| !v.is_null()).cloned();

                    Some(hermes_core::ToolCall {
                        id,
                        function: hermes_core::FunctionCall { name, arguments },
                        extra_content,
                    })
                })
                .collect::<Vec<_>>()
        });

    // Parse usage
    let usage = json.get("usage").and_then(|u| {
        Some(UsageStats {
            prompt_tokens: u.get("prompt_tokens")?.as_u64()? as u64,
            completion_tokens: u.get("completion_tokens")?.as_u64()? as u64,
            total_tokens: u.get("total_tokens")?.as_u64()? as u64,
            estimated_cost: None,
            ..Default::default()
        })
    });

    let role = message_obj
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("assistant");

    // Extract reasoning content
    let reasoning_content = message_obj
        .get("reasoning_content")
        .and_then(|r| r.as_str())
        .map(|s| s.to_string());

    let message = Message {
        role: match role {
            "user" => MessageRole::User,
            "system" => MessageRole::System,
            "tool" => MessageRole::Tool,
            _ => MessageRole::Assistant,
        },
        content: Some(content),
        tool_calls,
        tool_call_id: None,
        name: None,
        reasoning_content,
        cache_control: None,
    };

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|f| f.as_str())
        .map(|s| s.to_string());

    let model = json
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(LlmResponse {
        message,
        usage,
        model,
        finish_reason,
        ..Default::default()
    })
}

pub(crate) fn summarize_openai_response_shape(json: &Value) -> String {
    fn truncate_chars(value: &str, max_chars: usize) -> String {
        if value.chars().count() <= max_chars {
            return value.to_string();
        }
        let mut out = value
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        out.push('…');
        out
    }

    let mut parts = Vec::new();
    if let Some(status) = json.get("status").and_then(|v| v.as_i64()) {
        parts.push(format!("status={status}"));
    }
    if let Some(message) = json.get("message").and_then(|v| v.as_str()) {
        parts.push(format!("message={}", truncate_chars(message, 240)));
    }
    if let Some(error) = json.get("error") {
        let error_text = error
            .get("message")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        parts.push(format!("error={}", truncate_chars(&error_text, 240)));
    }
    if parts.is_empty() {
        let keys = json
            .as_object()
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>().join(","))
            .unwrap_or_else(|| json.to_string());
        parts.push(format!("keys={}", truncate_chars(&keys, 240)));
    }
    parts.join("; ")
}

// ---------------------------------------------------------------------------
// OpenRouter response cache types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenRouterResponseCacheControl {
    pub(crate) enabled: bool,
    pub(crate) clear: bool,
    pub(crate) ttl_secs: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct OpenRouterResponseCacheEntry {
    pub(crate) response: LlmResponse,
    pub(crate) expires_at: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct OpenRouterResponseCache {
    pub(crate) entries: HashMap<String, OpenRouterResponseCacheEntry>,
    pub(crate) order: VecDeque<String>,
}

pub(crate) static OPENROUTER_RESPONSE_CACHE: OnceLock<Mutex<OpenRouterResponseCache>> =
    OnceLock::new();
