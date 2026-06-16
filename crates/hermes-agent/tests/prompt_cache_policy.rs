//! Integration coverage for the prompt-cache policy override and telemetry
//! wiring. Lives in `tests/` (its own binary) so it compiles independently of
//! the in-crate `#[cfg(test)]` lib test target.

use hermes_agent::{record_prompt_cache_telemetry, resolve_prompt_cache_policy};
use serde_json::json;

#[test]
fn force_enable_override_for_custom_provider() {
    // SAFETY (Rust 2024): these env vars are touched only by this test.
    unsafe {
        std::env::remove_var("HERMES_FORCE_PROMPT_CACHING");
        std::env::remove_var("HERMES_FORCE_PROMPT_CACHE_NATIVE");
    }

    // Off by default: a custom OpenAI-compatible provider is not covered by the
    // built-in policy, so no markers are emitted.
    let (cache, native) = resolve_prompt_cache_policy(
        "custom",
        "https://my-endpoint.example/v1",
        "chat_completions",
        "custom:MiniMax-M2.7",
    );
    assert!(!cache);
    assert!(!native);

    // Opt-in force-enables the subsystem; chat_completions wire -> envelope layout.
    unsafe {
        std::env::set_var("HERMES_FORCE_PROMPT_CACHING", "1");
    }
    let (cache, native) = resolve_prompt_cache_policy(
        "custom",
        "https://my-endpoint.example/v1",
        "chat_completions",
        "custom:MiniMax-M2.7",
    );
    assert!(cache);
    assert!(!native);

    // anthropic_messages wire -> native content-block layout.
    let (cache, native) = resolve_prompt_cache_policy(
        "custom",
        "https://my-endpoint.example/v1",
        "anthropic_messages",
        "custom:some-claude-compatible",
    );
    assert!(cache);
    assert!(native);

    unsafe {
        std::env::remove_var("HERMES_FORCE_PROMPT_CACHING");
    }
}

#[test]
fn telemetry_counts_hit_and_miss() {
    let before = hermes_telemetry::snapshot();

    // Anthropic-wire cache read -> hit.
    record_prompt_cache_telemetry(&json!({
        "input_tokens": 100,
        "cache_read_input_tokens": 2000,
        "cache_creation_input_tokens": 0
    }));
    // OpenAI cached_tokens -> hit.
    record_prompt_cache_telemetry(&json!({
        "prompt_tokens": 3000,
        "completion_tokens": 200,
        "prompt_tokens_details": { "cached_tokens": 1800 }
    }));
    // Prompt present, no cache read -> miss.
    record_prompt_cache_telemetry(&json!({
        "input_tokens": 4766,
        "cache_read_input_tokens": 0
    }));
    // No prompt-side tokens -> ignored.
    record_prompt_cache_telemetry(&json!({ "completion_tokens": 200 }));

    let after = hermes_telemetry::snapshot();
    assert_eq!(after.prompt_cache_hits - before.prompt_cache_hits, 2);
    assert_eq!(after.prompt_cache_misses - before.prompt_cache_misses, 1);
}
