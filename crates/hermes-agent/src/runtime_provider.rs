//! Runtime provider construction and OAuth token refresh.
//!
//! Extracted from `impl AgentLoop` in `agent_loop.rs` to reduce the God struct.
//! All functions take `agent: &AgentLoop` instead of `&self`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;

use hermes_auth::{OAuth2Endpoints, exchange_refresh_token};
use hermes_core::{AgentError, LlmProvider};

use crate::agent_config::OAuthStoreCredential;
use crate::agent_loop::TurnRuntimeRoute;
use crate::agent_loop::{
    AgentLoop, OAUTH_REFRESH_BACKOFF_SECS, primary_runtime_from_config as primary_rt_from_cfg,
};
use crate::agent_runtime_helpers;
use crate::api_bridge::CodexProvider;
use crate::bedrock::{BedrockProvider, resolve_bedrock_region};
use crate::credential_pool::CredentialPool;
use crate::provider::{AnthropicProvider, GenericProvider, OpenAiProvider, OpenRouterProvider};
use crate::providers_extra::{
    CopilotProvider, KimiProvider, MiniMaxProvider, NousProvider, QwenProvider,
};
use crate::smart_model_routing::detect_api_mode_for_url;
use crate::smart_model_routing::{ApiMode, PrimaryRuntime};

// ---------------------------------------------------------------------------
// Provider construction
// ---------------------------------------------------------------------------

pub(crate) fn build_runtime_provider(
    agent: &AgentLoop,
    provider: &str,
    model_name: &str,
    route_base_url: Option<&str>,
    api_key_env_override: Option<&str>,
    explicit_api_key: Option<&str>,
    api_mode: Option<&ApiMode>,
    credential_pool: Option<&Arc<CredentialPool>>,
) -> Result<Arc<dyn LlmProvider>, AgentError> {
    let api_key = resolve_runtime_api_key(agent, provider, api_key_env_override, explicit_api_key)
        .ok_or_else(|| {
            AgentError::Config(format!(
                "No API key configured for runtime-routed provider '{}'",
                provider
            ))
        })?;
    let base_url = resolve_runtime_base_url(agent, provider, route_base_url);
    let request_timeout_seconds = resolve_runtime_request_timeout_seconds(agent, provider);
    let cfg = agent.config();
    let cfg_api_mode = cfg.api_mode.clone();
    let mode = api_mode.unwrap_or(&cfg_api_mode);
    let normalized_model_name =
        crate::model_normalize::normalize_model_for_provider(model_name, provider);
    let model_name = normalized_model_name.as_str();

    let provider_obj: Arc<dyn LlmProvider> = match provider {
        "openai" | "codex" | "openai-codex" => {
            if matches!(mode, ApiMode::CodexResponses) {
                let mut p = CodexProvider::new(&api_key)
                    .with_model(model_name)
                    .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
                    .with_optional_request_timeout_seconds(request_timeout_seconds);
                if let Some(ref url) = base_url {
                    p = p.with_base_url(url.clone());
                }
                if let Some(pool) = credential_pool {
                    p = p.with_credential_pool(pool.clone());
                }
                Arc::new(p)
            } else {
                let mut p = OpenAiProvider::new(&api_key)
                    .with_model(model_name)
                    .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
                    .with_optional_request_timeout_seconds(request_timeout_seconds);
                if let Some(url) = base_url {
                    p = p.with_base_url(url);
                }
                if let Some(pool) = credential_pool {
                    p = p.with_credential_pool(pool.clone());
                }
                Arc::new(p)
            }
        }
        "anthropic" => {
            let mut p = AnthropicProvider::new(&api_key)
                .with_model(model_name)
                .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
                .with_optional_request_timeout_seconds(request_timeout_seconds);
            if let Some(url) = base_url {
                p = p.with_base_url(url);
            }
            if let Some(pool) = credential_pool {
                p = p.with_credential_pool(pool.clone());
            }
            Arc::new(p)
        }
        "openrouter" => {
            let mut p = OpenRouterProvider::new(&api_key)
                .with_model(model_name)
                .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
                .with_optional_request_timeout_seconds(request_timeout_seconds);
            if let Some(url) = base_url {
                p = p.with_base_url(url);
            }
            if let Some(pool) = credential_pool {
                p = p.with_credential_pool(pool.clone());
            }
            Arc::new(p)
        }
        "qwen" | "qwen-oauth" => {
            let mut p = QwenProvider::new(&api_key)
                .with_model(model_name)
                .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
                .with_optional_request_timeout_seconds(request_timeout_seconds);
            if let Some(url) = base_url {
                p = p.with_base_url(url);
            }
            Arc::new(p)
        }
        "kimi" | "moonshot" => {
            let mut p = KimiProvider::new(&api_key)
                .with_model(model_name)
                .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
                .with_optional_request_timeout_seconds(request_timeout_seconds);
            if let Some(url) = base_url {
                p = p.with_base_url(url);
            }
            Arc::new(p)
        }
        "minimax" => {
            let mut p = MiniMaxProvider::new(&api_key)
                .with_model(model_name)
                .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
                .with_optional_request_timeout_seconds(request_timeout_seconds);
            if let Some(url) = base_url {
                p = p.with_base_url(url);
            }
            Arc::new(p)
        }
        "stepfun" => {
            let url = base_url.unwrap_or_else(|| "https://api.stepfun.ai/step_plan/v1".to_string());
            Arc::new(runtime_generic_provider(
                agent,
                GenericProvider::new(url, &api_key, model_name)
                    .with_optional_request_timeout_seconds(request_timeout_seconds)
                    .with_provider_profile(provider),
            ))
        }
        "nous" => {
            let mut p = NousProvider::new(&api_key)
                .with_model(model_name)
                .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
                .with_optional_request_timeout_seconds(request_timeout_seconds);
            if let Some(url) = base_url {
                p = p.with_base_url(url);
            }
            Arc::new(p)
        }
        "copilot" | "copilot-acp" => {
            let p = CopilotProvider::new(
                base_url.unwrap_or_else(|| "https://api.github.com/copilot".to_string()),
                &api_key,
            )
            .with_model(model_name)
            .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
            .with_optional_request_timeout_seconds(request_timeout_seconds);
            Arc::new(p)
        }
        "bedrock" | "aws" | "aws-bedrock" | "amazon-bedrock" | "amazon" => {
            let mut p = BedrockProvider::new()
                .with_region(resolve_bedrock_region())
                .with_model(model_name);
            if let Some(url) = base_url {
                p = p.with_base_url(url);
            }
            Arc::new(p)
        }
        _ => {
            let url = base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            let mut g = runtime_generic_provider(
                agent,
                GenericProvider::new(url, &api_key, model_name)
                    .with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
                    .with_optional_request_timeout_seconds(request_timeout_seconds)
                    .with_provider_profile(provider),
            );
            if let Some(pool) = credential_pool {
                g = g.with_credential_pool(pool.clone());
            }
            Arc::new(g)
        }
    };
    Ok(provider_obj)
}

pub(crate) fn build_delegation_runtime_provider(
    agent: &AgentLoop,
    provider: &str,
    model_name: &str,
    route_base_url: Option<&str>,
    explicit_api_key: Option<&str>,
) -> Result<Arc<dyn LlmProvider>, AgentError> {
    let api_mode = runtime_provider_api_mode(agent, provider)
        .or_else(|| route_base_url.and_then(detect_api_mode_for_url));
    build_runtime_provider(
        agent,
        provider,
        model_name,
        route_base_url,
        None,
        explicit_api_key,
        api_mode.as_ref(),
        agent.primary_credential_pool.as_ref(),
    )
}

/// Build an LLM provider from a full [`PrimaryRuntime`] snapshot (failover / fallback).
pub(crate) fn build_llm_provider_for_runtime(
    agent: &AgentLoop,
    runtime: &PrimaryRuntime,
) -> Result<Arc<dyn LlmProvider>, AgentError> {
    let (inferred_provider, model_name) =
        crate::route_learning::extract_provider_and_model(agent, runtime.model.as_str());
    let provider = runtime
        .provider
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(inferred_provider.as_str());
    let pool = runtime
        .credential_pool
        .as_ref()
        .or(agent.primary_credential_pool.as_ref());
    build_runtime_provider(
        agent,
        provider,
        model_name,
        runtime.base_url.as_deref(),
        None,
        None,
        Some(&runtime.api_mode),
        pool,
    )
}

/// Effective provider for API calls: rebuild from active runtime when fallback is active.
pub(crate) fn effective_llm_provider(agent: &AgentLoop) -> Arc<dyn LlmProvider> {
    let fallback_active = agent
        .state
        .lock()
        .map(|state| state.turn_fallback.is_fallback_activated())
        .unwrap_or(false);
    if fallback_active {
        if let Ok(state) = agent.state.lock() {
            if let Ok(provider) = build_llm_provider_for_runtime(agent, &state.active_runtime) {
                return provider;
            }
        }
    }
    agent.llm_provider.clone()
}

fn runtime_provider_api_mode(agent: &AgentLoop, provider: &str) -> Option<ApiMode> {
    let provider = provider.trim();
    if provider.is_empty() {
        return None;
    }
    let config = agent.config();
    let lookup = |key: &str| {
        config
            .runtime_providers
            .get(key)
            .and_then(|cfg| cfg.api_mode.clone())
    };
    if let Some(mode) = lookup(provider) {
        return Some(mode);
    }

    let lower = provider.to_ascii_lowercase();
    if let Some(mode) = lookup(lower.as_str()) {
        return Some(mode);
    }

    let canonical = hermes_core::providers::canonical_provider_id(provider);
    if let Some(mode) = lookup(canonical.as_str()) {
        return Some(mode);
    }

    if let Some(profile) = crate::provider_profiles::canonical_provider_profile_id(provider) {
        if let Some(mode) = lookup(profile) {
            return Some(mode);
        }
    }

    config
        .runtime_providers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(provider))
        .and_then(|(_, cfg)| cfg.api_mode.clone())
}

fn runtime_generic_provider(
    agent: &AgentLoop,
    provider: crate::provider::GenericProvider,
) -> crate::provider::GenericProvider {
    provider.with_serialize_cache(Arc::clone(&agent.provider_serialize_cache))
}

fn resolve_runtime_api_key(
    agent: &AgentLoop,
    provider: &str,
    api_key_env_override: Option<&str>,
    explicit_api_key: Option<&str>,
) -> Option<String> {
    if provider == "copilot-acp" {
        return Some("copilot-acp".to_string());
    }
    if let Some(token) = resolve_oauth_store_api_key(agent, provider) {
        return Some(token);
    }
    if let Some(key) = explicit_api_key.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(key.to_string());
    }
    if let Some(env_name) = api_key_env_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Ok(v) = std::env::var(env_name) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    if let Some(cfg) = agent.config().runtime_providers.get(provider) {
        if let Some(ref key) = cfg.api_key {
            let trimmed = key.trim();
            if let Some(env_ref) = trimmed.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
                if let Ok(v) = std::env::var(env_ref) {
                    if !v.trim().is_empty() {
                        return Some(v);
                    }
                }
            } else if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(env_name) = cfg
            .api_key_env
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Ok(v) = std::env::var(env_name) {
                if !v.trim().is_empty() {
                    return Some(v);
                }
            }
        }
    }
    if matches!(provider, "openai" | "codex" | "openai-codex") {
        return std::env::var("HERMES_OPENAI_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .filter(|v| !v.trim().is_empty());
    }
    if provider == "stepfun" {
        return std::env::var("HERMES_STEPFUN_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| std::env::var("STEPFUN_API_KEY").ok())
            .filter(|v| !v.trim().is_empty());
    }
    match provider {
        "anthropic" | "claude" | "claude-code" => std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| std::env::var("ANTHROPIC_TOKEN").ok())
            .filter(|v| !v.trim().is_empty())
            .or_else(|| std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok())
            .filter(|v| !v.trim().is_empty()),
        "google-gemini-cli" | "gemini-cli" | "gemini-oauth" => {
            std::env::var("HERMES_GEMINI_OAUTH_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty())
        }
        "openrouter" => std::env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        "qwen" | "qwen-oauth" => std::env::var("DASHSCOPE_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        "kimi" | "moonshot" => std::env::var("MOONSHOT_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        "minimax" => std::env::var("MINIMAX_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        "nous" => std::env::var("NOUS_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        "copilot" | "copilot-acp" => std::env::var("GITHUB_COPILOT_TOKEN")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        _ => None,
    }
}

pub(crate) fn resolve_runtime_base_url(
    agent: &AgentLoop,
    provider: &str,
    route_base_url: Option<&str>,
) -> Option<String> {
    if let Some(b) = route_base_url.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(b.to_string());
    }
    agent
        .config()
        .runtime_providers
        .get(provider)
        .and_then(|c| c.base_url.as_ref())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if provider == "copilot-acp" {
                std::env::var("COPILOT_ACP_BASE_URL")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| Some("acp://copilot".to_string()))
            } else if provider == "openai-codex" || provider == "codex" {
                Some("https://api.openai.com/v1".to_string())
            } else if provider == "qwen-oauth" {
                Some("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string())
            } else if provider == "google-gemini-cli" {
                Some("cloudcode-pa://google".to_string())
            } else if provider == "stepfun" {
                Some("https://api.stepfun.ai/step_plan/v1".to_string())
            } else {
                None
            }
        })
}

pub(crate) fn resolve_runtime_command_args(
    agent: &AgentLoop,
    provider: Option<&str>,
) -> (Option<String>, Vec<String>) {
    let mut command = agent
        .config()
        .acp_command
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut args: Vec<String> = agent
        .config()
        .acp_args
        .iter()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();

    if let Some(provider) = provider {
        if let Some(cfg) = agent.config().runtime_providers.get(provider) {
            if let Some(cmd) = cfg
                .command
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                command = Some(cmd.to_string());
            }
            if !cfg.args.is_empty() {
                args = cfg
                    .args
                    .iter()
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect();
            }
        }
        if provider == "copilot-acp" {
            if command.is_none() {
                command = std::env::var("HERMES_COPILOT_ACP_COMMAND")
                    .ok()
                    .or_else(|| std::env::var("COPILOT_CLI_PATH").ok())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| Some("copilot".to_string()));
            }
            if args.is_empty() {
                args = std::env::var("HERMES_COPILOT_ACP_ARGS")
                    .ok()
                    .and_then(|raw| shlex::split(raw.trim()))
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| vec!["--acp".to_string(), "--stdio".to_string()]);
            }
            if let Some(cmd) = command.as_deref() {
                if let Ok(resolved) = which::which(cmd) {
                    command = Some(resolved.to_string_lossy().to_string());
                }
            }
        }
    }
    (command, args)
}

fn resolve_runtime_request_timeout_seconds(agent: &AgentLoop, provider: &str) -> Option<f64> {
    agent
        .config()
        .runtime_providers
        .get(provider)
        .and_then(|c| c.request_timeout_seconds)
        .or_else(|| {
            let alias = match provider {
                "codex" => "openai-codex",
                "openai-codex" => "codex",
                "qwen" => "qwen-oauth",
                "qwen-oauth" => "qwen",
                "kimi" => "moonshot",
                "moonshot" => "kimi",
                _ => return None,
            };
            agent
                .config()
                .runtime_providers
                .get(alias)
                .and_then(|c| c.request_timeout_seconds)
        })
}

// ---------------------------------------------------------------------------
// OAuth store helpers
// ---------------------------------------------------------------------------

fn resolve_oauth_store_api_key(agent: &AgentLoop, provider: &str) -> Option<String> {
    let provider_key = match provider {
        "openai" => "openai",
        "openai-codex" | "codex" => "openai-codex",
        "nous" => "nous",
        "qwen-oauth" => "qwen-oauth",
        "anthropic" | "claude" | "claude-code" => "anthropic",
        "google-gemini-cli" | "gemini-cli" | "gemini-oauth" => "google-gemini-cli",
        _ => return None,
    };
    let path = auth_tokens_path(agent);
    let raw = std::fs::read_to_string(path).ok()?;
    let entries: HashMap<String, OAuthStoreCredential> = serde_json::from_str(&raw).ok()?;
    let cred = entries.get(provider_key)?;
    if cred.access_token.trim().is_empty() {
        return None;
    }
    if cred
        .expires_at
        .map(|exp| exp <= Utc::now())
        .unwrap_or(false)
    {
        return None;
    }
    Some(cred.access_token.clone())
}

pub(crate) async fn refresh_oauth_store_tokens_if_needed(agent: &AgentLoop) {
    // Keep this list explicit so behavior is deterministic and parity-scoped.
    refresh_single_oauth_store_token_if_needed(agent, "openai").await;
    refresh_single_oauth_store_token_if_needed(agent, "openai-codex").await;
    refresh_single_oauth_store_token_if_needed(agent, "nous").await;
    refresh_single_oauth_store_token_if_needed(agent, "qwen-oauth").await;
    refresh_single_oauth_store_token_if_needed(agent, "anthropic").await;
}

async fn refresh_single_oauth_store_token_if_needed(agent: &AgentLoop, provider_key: &str) {
    if !can_attempt_oauth_refresh(agent, provider_key) {
        return;
    }
    let path = auth_tokens_path(agent);
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(v) => v,
        Err(_) => return,
    };
    let mut entries: HashMap<String, OAuthStoreCredential> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return,
    };
    let Some(current) = entries.get(provider_key).cloned() else {
        return;
    };
    let Some(expires_at) = current.expires_at else {
        return;
    };
    if expires_at > Utc::now() {
        return;
    }
    let Some(refresh_token) = current.refresh_token.clone() else {
        return;
    };
    let Some((token_url, client_id)) = oauth_refresh_config(agent, provider_key) else {
        return;
    };
    let refreshed = match exchange_oauth_refresh_token(
        agent,
        provider_key,
        token_url.as_str(),
        client_id.as_str(),
        refresh_token.as_str(),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            mark_oauth_refresh_failure(agent, provider_key);
            tracing::warn!(
                provider = provider_key,
                error = %e,
                "oauth token refresh failed for runtime provider"
            );
            return;
        }
    };
    entries.insert(provider_key.to_string(), refreshed);
    let Ok(content) = serde_json::to_string_pretty(&entries) else {
        mark_oauth_refresh_success(agent, provider_key);
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(path, content).await;
    mark_oauth_refresh_success(agent, provider_key);
}

fn oauth_refresh_config(agent: &AgentLoop, provider_key: &str) -> Option<(String, String)> {
    // Preferred source: unified provider config centre (runtime_providers).
    let cfg_token_url = agent
        .config()
        .runtime_providers
        .get(provider_key)
        .and_then(|c| c.oauth_token_url.as_deref())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let cfg_client_id = agent
        .config()
        .runtime_providers
        .get(provider_key)
        .and_then(|c| c.oauth_client_id.as_deref())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Env fallback - keeps previous behavior working when config centre is empty.
    let (token_url_env, client_id_env) = match provider_key {
        "openai" => (
            "HERMES_OPENAI_OAUTH_TOKEN_URL",
            "HERMES_OPENAI_OAUTH_CLIENT_ID",
        ),
        "openai-codex" => (
            "HERMES_OPENAI_CODEX_OAUTH_TOKEN_URL",
            "HERMES_OPENAI_CODEX_OAUTH_CLIENT_ID",
        ),
        "nous" => ("HERMES_NOUS_OAUTH_TOKEN_URL", "HERMES_NOUS_OAUTH_CLIENT_ID"),
        "qwen-oauth" => ("HERMES_QWEN_OAUTH_TOKEN_URL", "HERMES_QWEN_OAUTH_CLIENT_ID"),
        "anthropic" => (
            "HERMES_ANTHROPIC_OAUTH_TOKEN_URL",
            "HERMES_ANTHROPIC_OAUTH_CLIENT_ID",
        ),
        _ => return cfg_token_url.zip(cfg_client_id),
    };
    let token_url = cfg_token_url
        .or_else(|| {
            std::env::var(token_url_env)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| match provider_key {
            "openai" => std::env::var("HERMES_OPENAI_CODEX_OAUTH_TOKEN_URL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| Some("https://auth.openai.com/oauth/token".to_string())),
            "nous" => std::env::var("NOUS_PORTAL_BASE_URL")
                .ok()
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .map(|base| format!("{base}/api/oauth/token"))
                .or_else(|| Some("https://portal.nousresearch.com/api/oauth/token".to_string())),
            "anthropic" => Some("https://console.anthropic.com/v1/oauth/token".to_string()),
            _ => None,
        })?;
    let client_id = cfg_client_id
        .or_else(|| {
            std::env::var(client_id_env)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| match provider_key {
            "openai" => std::env::var("HERMES_OPENAI_CODEX_OAUTH_CLIENT_ID")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| Some("app_EMoamEEZ73f0CkXaXp7hrann".to_string())),
            "nous" => std::env::var("NOUS_CLIENT_ID")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| Some("hermes-cli".to_string())),
            "anthropic" => Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e".to_string()),
            _ => None,
        })?;
    Some((token_url, client_id))
}

fn can_attempt_oauth_refresh(agent: &AgentLoop, provider_key: &str) -> bool {
    let Ok(state) = agent.state.lock() else {
        return true;
    };
    let Some(last_fail) = state.oauth_refresh_backoff.get(provider_key) else {
        return true;
    };
    last_fail.elapsed().as_secs() >= OAUTH_REFRESH_BACKOFF_SECS
}

fn mark_oauth_refresh_failure(agent: &AgentLoop, provider_key: &str) {
    if let Ok(mut state) = agent.state.lock() {
        state
            .oauth_refresh_backoff
            .insert(provider_key.to_string(), std::time::Instant::now());
    }
}

fn mark_oauth_refresh_success(agent: &AgentLoop, provider_key: &str) {
    if let Ok(mut state) = agent.state.lock() {
        state.oauth_refresh_backoff.remove(provider_key);
    }
}

fn auth_tokens_path(agent: &AgentLoop) -> PathBuf {
    let hermes_home = agent
        .config()
        .hermes_home
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| std::env::var("HERMES_HOME").ok().map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|h| h.join(".hermes")))
        .unwrap_or_else(|| PathBuf::from(".hermes"));
    hermes_home.join("auth").join("tokens.json")
}

async fn exchange_oauth_refresh_token(
    _agent: &AgentLoop,
    provider_key: &str,
    token_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<OAuthStoreCredential, AgentError> {
    let endpoints = OAuth2Endpoints {
        authorize_url: "http://127.0.0.1/oauth/authorize-unused".to_string(),
        token_url: token_url.to_string(),
        client_id: client_id.to_string(),
        redirect_uri: "http://127.0.0.1/unused".to_string(),
        scopes: vec![],
    };
    let cred = exchange_refresh_token(provider_key, &endpoints, refresh_token)
        .await
        .map_err(|e| AgentError::AuthFailed(e.to_string()))?;
    Ok(OAuthStoreCredential {
        provider: Some(provider_key.to_string()),
        access_token: cred.access_token,
        refresh_token: cred
            .refresh_token
            .or_else(|| Some(refresh_token.to_string())),
        token_type: Some(cred.token_type),
        scope: cred.scope,
        expires_at: cred.expires_at,
    })
}

// ---------------------------------------------------------------------------
// Other provider helpers
// ---------------------------------------------------------------------------

pub(crate) fn credentials_pool_for_route<'a>(
    agent: &'a AgentLoop,
    rt: &'a TurnRuntimeRoute,
) -> Option<&'a Arc<CredentialPool>> {
    if rt.credential_pool_fallback {
        rt.credential_pool
            .as_ref()
            .or(agent.primary_credential_pool.as_ref())
    } else {
        rt.credential_pool.as_ref()
    }
}

/// Recompute prompt-cache policy from current route (Python `_anthropic_prompt_cache_policy`).
pub fn refresh_prompt_cache_policy(
    agent: &AgentLoop,
    provider: &str,
    base_url: &str,
    api_mode: &str,
) {
    let (should_cache, native) = agent_runtime_helpers::resolve_prompt_cache_policy(
        provider,
        base_url,
        api_mode,
        &agent.config().model,
    );
    agent
        .use_prompt_caching
        .store(should_cache, std::sync::atomic::Ordering::Relaxed);
    agent
        .use_native_cache_layout
        .store(native, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn note_primary_rate_limited_if_applicable(agent: &AgentLoop) {
    let already = agent
        .state
        .lock()
        .map(|state| state.turn_fallback.is_fallback_activated())
        .unwrap_or(false);
    let primary_prov = agent
        .router
        .stored_primary_runtime
        .provider
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let active_prov = crate::route_learning::primary_runtime_snapshot(agent)
        .provider
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if already && !primary_prov.is_empty() && active_prov != primary_prov {
        return;
    }
    if let Ok(mut state) = agent.state.lock() {
        state.turn_fallback.note_primary_rate_limited();
    }
}

pub(crate) fn primary_runtime_for_failover_model(
    agent: &AgentLoop,
    model_id: &str,
) -> PrimaryRuntime {
    // Need to handle this somewhat differently since primary_runtime_from_config is still on AgentLoop
    let cfg = agent.config_snapshot();
    let mut rt = primary_rt_from_cfg(&cfg);
    let (provider, _) = crate::route_learning::extract_provider_and_model(agent, model_id);
    rt.model = model_id.trim().to_string();
    if !provider.is_empty() {
        rt.provider = Some(provider);
    }
    if let Some(p) = rt.provider.as_deref() {
        if let Some(rcfg) = cfg.runtime_providers.get(p) {
            if let Some(url) = rcfg
                .base_url
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
            {
                rt.base_url = Some(url);
            }
            if let Some(cmd) = rcfg
                .command
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                rt.command = Some(cmd.to_string());
            }
            if !rcfg.args.is_empty() {
                rt.args = rcfg
                    .args
                    .iter()
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect();
            }
        }
    }
    if let Some(url) = rt.base_url.as_deref() {
        if let Some(mode) = detect_api_mode_for_url(url) {
            rt.api_mode = mode;
        }
    }
    rt.credential_pool = agent.primary_credential_pool.clone();
    rt
}

pub(crate) fn active_model(agent: &AgentLoop) -> String {
    agent
        .state
        .lock()
        .map(|state| state.active_runtime.model.clone())
        .unwrap_or_else(|_| agent.config_snapshot().model.clone())
}
