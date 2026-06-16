use hermes_agent::AgentConfig;
use hermes_config::GatewayConfig;

use super::api_keys::parse_runtime_provider_api_mode;
use super::names::normalize_runtime_provider_name;
use super::resolve::resolve_provider_and_model;

pub fn build_agent_config(config: &GatewayConfig, model: &str) -> AgentConfig {
    let (resolved_provider, _) = resolve_provider_and_model(config, model);
    let runtime_provider = normalize_runtime_provider_name(resolved_provider.as_str());
    let provider_extra_body = config
        .llm_providers
        .get(resolved_provider.as_str())
        .or_else(|| config.llm_providers.get(runtime_provider.as_str()))
        .or_else(|| {
            config.llm_providers.iter().find_map(|(name, cfg)| {
                if name.eq_ignore_ascii_case(resolved_provider.as_str())
                    || name.eq_ignore_ascii_case(runtime_provider.as_str())
                {
                    Some(cfg)
                } else {
                    None
                }
            })
        })
        .and_then(|cfg| cfg.extra_body.clone());
    let skip_memory_env = std::env::var("HERMES_SKIP_MEMORY")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    let skip_context_files_env = std::env::var("HERMES_SKIP_CONTEXT_FILES")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    let hermes_home = config
        .home_dir
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(hermes_config::hermes_home);
    let skip_memory = skip_memory_env || hermes_home.join(".memory_disabled").exists();
    let skip_context_files = config.agent.skip_context_files || skip_context_files_env;

    let mut retry_cfg = hermes_agent::agent_loop::RetryConfig::default();
    if let Ok(raw) = std::env::var("HERMES_FALLBACK_MODELS") {
        let parsed: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .collect();
        if !parsed.is_empty() {
            retry_cfg.fallback_models = parsed.clone();
            retry_cfg.fallback_model = parsed.first().cloned();
        }
    }
    if retry_cfg.fallback_model.is_none() {
        if let Ok(raw) = std::env::var("HERMES_FALLBACK_MODEL") {
            let value = raw.trim();
            if !value.is_empty() {
                retry_cfg.fallback_model = Some(value.to_string());
                if retry_cfg.fallback_models.is_empty() {
                    retry_cfg.fallback_models.push(value.to_string());
                }
            }
        }
    }

    let cache_ttl = if config.prompt_caching.cache_ttl.as_str() == "1h" {
        "1h".to_string()
    } else {
        "5m".to_string()
    };
    let provider_base_url = config
        .llm_providers
        .get(resolved_provider.as_str())
        .or_else(|| config.llm_providers.get(runtime_provider.as_str()))
        .and_then(|c| c.base_url.clone())
        .unwrap_or_default();
    let api_mode_str = if resolved_provider.eq_ignore_ascii_case("anthropic")
        || model.to_ascii_lowercase().contains("claude")
    {
        "anthropic_messages"
    } else {
        "chat_completions"
    };
    let (use_prompt_caching, use_native_cache_layout) =
        hermes_agent::prompt_caching::resolve_prompt_cache_policy(
            &resolved_provider,
            &provider_base_url,
            api_mode_str,
            model,
        );
    let max_delegate_depth = config
        .delegation
        .max_spawn_depth
        .map(|depth| depth.max(1))
        .unwrap_or_else(|| AgentConfig::default().max_delegate_depth);

    AgentConfig {
        max_turns: config.max_turns,
        budget: config.budget.clone(),
        model: model.to_string(),
        system_prompt: config.system_prompt.clone(),
        personality: config.personality.clone(),
        extra_body: provider_extra_body,
        hermes_home: config.home_dir.clone(),
        provider: Some(resolved_provider),
        stream: config.streaming.enabled,
        skip_memory,
        skip_context_files,
        platform: Some("cli".to_string()),
        enabled_skills: config.skills.enabled.clone(),
        disabled_skills: config.skills.disabled.clone(),
        pass_session_id: true,
        runtime_providers: config
            .llm_providers
            .iter()
            .map(|(name, cfg)| {
                (
                    name.clone(),
                    hermes_agent::agent_loop::RuntimeProviderConfig {
                        api_key: cfg.api_key.clone(),
                        api_key_env: cfg.api_key_env.clone(),
                        base_url: cfg.base_url.clone(),
                        command: cfg.command.clone(),
                        args: cfg.args.clone(),
                        oauth_token_url: cfg.oauth_token_url.clone(),
                        oauth_client_id: cfg.oauth_client_id.clone(),
                        request_timeout_seconds: cfg.request_timeout_seconds,
                        api_mode: cfg
                            .api_mode
                            .as_deref()
                            .and_then(parse_runtime_provider_api_mode),
                    },
                )
            })
            .collect(),
        retry: retry_cfg,
        smart_model_routing: hermes_agent::agent_loop::SmartModelRoutingConfig {
            enabled: config.smart_model_routing.enabled,
            max_simple_chars: config.smart_model_routing.max_simple_chars,
            max_simple_words: config.smart_model_routing.max_simple_words,
            cheap_model: config.smart_model_routing.cheap_model.as_ref().map(|m| {
                hermes_agent::agent_loop::CheapModelRouteConfig {
                    provider: m.provider.clone(),
                    model: m.model.clone(),
                    base_url: m.base_url.clone(),
                    api_key_env: m.api_key_env.clone(),
                }
            }),
        },
        memory_nudge_interval: config.agent.memory_nudge_interval,
        skill_creation_nudge_interval: config.agent.skill_creation_nudge_interval,
        background_review_enabled: config.agent.background_review_enabled,
        interest: config.interest.clone(),
        code_index_enabled: config.agent.code_index_enabled,
        code_index_max_files: config.agent.code_index_max_files,
        code_index_max_symbols: config.agent.code_index_max_symbols,
        lsp_context_enabled: config.agent.lsp_context_enabled,
        lsp_context_max_chars: config.agent.lsp_context_max_chars,
        cache_ttl,
        use_prompt_caching,
        use_native_cache_layout,
        web_research: config.agent.web_research.clone(),
        max_delegate_depth,
        delegation_model: config
            .delegation
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        delegation_provider: config
            .delegation
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        delegation_base_url: config
            .delegation
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        delegation_api_key: config
            .delegation
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        prefill_messages: hermes_config::load_prefill_messages(config),
        ..AgentConfig::default()
    }
}
