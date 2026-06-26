//! Sync talk `[llm]` thinking settings with gateway `llm_providers.*.extra_body`.

use hermes_config::GatewayConfig;

use crate::config::{DEFAULT_THINKING_BUDGET, LlmConfig};

fn map_openai_reasoning_effort(effort: &str) -> &str {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "min" => "low",
        "xhigh" | "max" => "high",
        "low" => "low",
        "medium" | "med" => "medium",
        "high" => "high",
        _ => "medium",
    }
}

fn gemini_thinking_level(effort: &str) -> &str {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "min" | "low" => "low",
        "medium" | "med" => "medium",
        "high" | "xhigh" | "max" => "high",
        _ => "medium",
    }
}

/// Resolve gateway provider key for talk / call_hermes reasoning patch.
pub fn resolve_talk_gateway_provider(gw: &GatewayConfig, llm: &LlmConfig) -> String {
    if let Some(provider) = llm
        .aipc_talk
        .provider
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return provider.to_string();
    }
    if let Some(model) = gw.model.as_deref().filter(|m| !m.is_empty()) {
        if let Some((provider, _)) = model.split_once(':') {
            if !provider.trim().is_empty() {
                return provider.trim().to_string();
            }
        }
    }
    "custom".to_string()
}

/// Patch gateway provider `extra_body` from talk `[llm]` thinking fields.
pub fn apply_talk_reasoning_to_gateway(gw: &mut GatewayConfig, llm: &LlmConfig) {
    let provider_key = resolve_talk_gateway_provider(gw, llm);
    let provider_cfg = gw.llm_providers.entry(provider_key.clone()).or_default();

    let mut body_map = provider_cfg
        .extra_body
        .take()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    if llm.thinking_enabled {
        body_map.insert("enable_thinking".to_string(), serde_json::Value::Bool(true));
        let budget = llm.thinking_budget.unwrap_or(DEFAULT_THINKING_BUDGET);
        body_map.insert(
            "budget_tokens".to_string(),
            serde_json::Value::Number(budget.into()),
        );
        body_map.insert(
            "thinking_budget".to_string(),
            serde_json::Value::Number(budget.into()),
        );
    } else {
        body_map.insert(
            "enable_thinking".to_string(),
            serde_json::Value::Bool(false),
        );
        body_map.remove("budget_tokens");
        body_map.remove("thinking_budget");
        if let Some(thinking_obj) = body_map
            .get_mut("thinking")
            .and_then(|value| value.as_object_mut())
        {
            thinking_obj.remove("budget_tokens");
            if thinking_obj.is_empty() {
                body_map.remove("thinking");
            }
        }
    }

    if let Some(effort) = llm
        .reasoning_effort
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        body_map.remove("reasoning_effort");
        let mapped = map_openai_reasoning_effort(effort);
        let mut reasoning_obj = body_map
            .get("reasoning")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        reasoning_obj.insert(
            "effort".to_string(),
            serde_json::Value::String(mapped.to_string()),
        );
        body_map.insert(
            "reasoning".to_string(),
            serde_json::Value::Object(reasoning_obj),
        );

        if provider_key.contains("gemini") || provider_key == "google" {
            let level = gemini_thinking_level(effort);
            let mut google_obj = body_map
                .get("google")
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            let mut thinking_cfg = google_obj
                .get("thinking_config")
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            thinking_cfg.insert(
                "thinking_level".to_string(),
                serde_json::Value::String(level.to_string()),
            );
            google_obj.insert(
                "thinking_config".to_string(),
                serde_json::Value::Object(thinking_cfg.clone()),
            );
            body_map.insert("google".to_string(), serde_json::Value::Object(google_obj));
            body_map.insert(
                "thinking_config".to_string(),
                serde_json::Value::Object(thinking_cfg),
            );
        }
    }

    provider_cfg.extra_body = if body_map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(body_map))
    };
}

fn extra_body_string(body: &serde_json::Value, key: &str) -> Option<String> {
    body.get(key)
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

fn extra_body_u64(body: &serde_json::Value, key: &str) -> Option<u32> {
    body.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
}

/// Fill talk `[llm]` reasoning fields from gateway when unset.
pub fn merge_gateway_reasoning_defaults(cfg: &mut LlmConfig) {
    let Ok(gw) = hermes_config::load_config(None) else {
        return;
    };
    let provider_key = resolve_talk_gateway_provider(&gw, cfg);
    let Some(body) = gw
        .llm_providers
        .get(&provider_key)
        .and_then(|p| p.extra_body.as_ref())
    else {
        return;
    };

    if cfg.reasoning_effort.is_none() {
        let effort = body
            .get("reasoning")
            .and_then(|v| v.get("effort"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .or_else(|| extra_body_string(body, "reasoning_effort"));
        if let Some(effort) = effort.filter(|e| !e.trim().is_empty()) {
            cfg.reasoning_effort = Some(effort);
        }
    }

    if cfg.thinking_budget.is_none() {
        let budget = extra_body_u64(body, "budget_tokens")
            .or_else(|| extra_body_u64(body, "thinking_budget"))
            .or_else(|| {
                body.get("thinking")
                    .and_then(|v| extra_body_u64(v, "budget_tokens"))
            });
        if let Some(budget) = budget {
            cfg.thinking_budget = Some(budget);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermes_config::LlmProviderConfig;

    fn sample_llm(thinking_enabled: bool) -> LlmConfig {
        LlmConfig {
            base_url: "http://127.0.0.1:11080/v1".to_string(),
            api_key: String::new(),
            model: "auto".to_string(),
            system_prompt: String::new(),
            max_tokens: None,
            temperature: 0.7,
            warmup_on_start: true,
            thinking_enabled,
            thinking_budget: Some(256),
            reasoning_effort: Some("low".to_string()),
            user_id: "flowy".to_string(),
            tools_enabled: true,
            execute_allowlist: vec![],
            aipc_talk: Default::default(),
        }
    }

    #[test]
    fn apply_talk_reasoning_sets_budget_and_effort() {
        let llm = sample_llm(true);
        let mut gw = GatewayConfig::default();
        apply_talk_reasoning_to_gateway(&mut gw, &llm);
        let body = gw
            .llm_providers
            .get("custom")
            .and_then(|p| p.extra_body.as_ref())
            .expect("extra_body");
        assert_eq!(body["enable_thinking"], true);
        assert_eq!(body["budget_tokens"], 256);
        assert_eq!(body["reasoning"]["effort"], "low");
    }

    #[test]
    fn apply_talk_reasoning_disables_thinking() {
        let llm = sample_llm(false);
        let mut gw = GatewayConfig {
            llm_providers: [(
                "custom".to_string(),
                LlmProviderConfig {
                    extra_body: Some(serde_json::json!({
                        "enable_thinking": true,
                        "budget_tokens": 1024
                    })),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        apply_talk_reasoning_to_gateway(&mut gw, &llm);
        let body = gw
            .llm_providers
            .get("custom")
            .and_then(|p| p.extra_body.as_ref())
            .expect("extra_body");
        assert_eq!(body["enable_thinking"], false);
        assert!(body.get("budget_tokens").is_none());
    }

    #[test]
    fn resolve_provider_from_aipc_talk() {
        let mut llm = sample_llm(true);
        llm.aipc_talk.provider = Some("openrouter".to_string());
        let gw = GatewayConfig::default();
        assert_eq!(
            resolve_talk_gateway_provider(&gw, &llm),
            "openrouter".to_string()
        );
    }
}
