//! Parse provider usage JSON into [`hermes_core::UsageStats`] (Python `normalize_usage` parity).

use hermes_core::UsageStats;
use hermes_intelligence::usage_pricing::{normalize_usage, CanonicalUsage};
use serde_json::Value;

use crate::smart_model_routing::ApiMode;

pub fn api_mode_str(mode: &ApiMode) -> &'static str {
    match mode {
        ApiMode::ChatCompletions => "chat_completions",
        ApiMode::AnthropicMessages => "anthropic_messages",
        ApiMode::CodexResponses => "codex_responses",
        ApiMode::CodexAppServer => "codex_app_server",
        ApiMode::BedrockConverse => "bedrock_converse",
    }
}

pub fn usage_stats_from_canonical(c: &CanonicalUsage) -> UsageStats {
    UsageStats {
        prompt_tokens: c.prompt_tokens(),
        completion_tokens: c.output_tokens,
        total_tokens: c.total_tokens(),
        input_tokens: c.input_tokens,
        output_tokens: c.output_tokens,
        cache_read_tokens: c.cache_read_tokens,
        cache_write_tokens: c.cache_write_tokens,
        reasoning_tokens: c.reasoning_tokens,
        estimated_cost: None,
    }
}

/// Normalize raw provider `usage` into session/TUI buckets.
pub fn usage_stats_from_raw(
    raw: &Value,
    provider: Option<&str>,
    api_mode: Option<&str>,
) -> Option<UsageStats> {
    if raw.is_null() {
        return None;
    }
    let has_signal = [
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "input_tokens",
        "output_tokens",
    ]
    .iter()
    .any(|k| raw.get(*k).and_then(|v| v.as_u64()).unwrap_or(0) > 0)
        || [
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ]
        .iter()
        .any(|k| raw.get(*k).and_then(|v| v.as_u64()).unwrap_or(0) > 0);
    if !has_signal {
        return None;
    }
    let canonical = normalize_usage(raw, provider, api_mode);
    Some(usage_stats_from_canonical(&canonical))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_cache_buckets_from_prompt_details() {
        let raw = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 200,
            "total_tokens": 1200,
            "prompt_tokens_details": { "cached_tokens": 500 },
            "cache_creation_input_tokens": 300
        });
        let u = usage_stats_from_raw(&raw, Some("openai"), Some("chat_completions")).unwrap();
        assert_eq!(u.cache_read_tokens, 500);
        assert_eq!(u.cache_write_tokens, 300);
        assert_eq!(u.input_tokens, 200);
        assert_eq!(u.prompt_tokens, 1000);
        assert_eq!(u.completion_tokens, 200);
    }

    #[test]
    fn anthropic_cache_buckets() {
        let raw = json!({
            "input_tokens": 1000,
            "output_tokens": 500,
            "cache_read_input_tokens": 200,
            "cache_creation_input_tokens": 50
        });
        let u = usage_stats_from_raw(&raw, Some("anthropic"), Some("anthropic_messages")).unwrap();
        assert_eq!(u.cache_read_tokens, 200);
        assert_eq!(u.cache_write_tokens, 50);
        assert_eq!(u.input_tokens, 1000);
        assert_eq!(u.prompt_tokens, 1250);
    }
}
