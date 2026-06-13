//! Model-related slash command handlers.
//!
//! Extracted from `commands.rs` into a sub-module as part of modularization.
//! Handles `/model` and all its sub-commands (`explain`, `why-not`, `failover`,
//! `backend`, `harness`).

use std::path::PathBuf;

use hermes_config::GatewayConfig;
use hermes_core::AgentError;
use hermes_intelligence::model_metadata::{get_model_context_length, get_model_info};
pub(crate) use hermes_intelligence::models_dev::default_client;

use crate::app::provider_api_key_from_env;
use crate::commands::{CommandResult, emit_command_output};
use crate::env_vars;
use crate::model_switch::{
    cached_provider_catalog_status, curated_provider_slugs, normalize_provider_model,
    provider_model_ids,
};
use crate::{SelectResult, curses_select, curses_select_embedded};

// ---------------------------------------------------------------------------
// ModelSwitchRequest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModelSwitchRequest {
    PickProviderThenModel,
    PickModelFromProvider(String),
    SetDirect(String),
}

// ---------------------------------------------------------------------------
// ModelCapabilityRequirements
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ModelCapabilityRequirements {
    pub(crate) require_tools: bool,
    pub(crate) require_vision: bool,
    pub(crate) require_reasoning: bool,
    pub(crate) require_long_context: bool,
    pub(crate) min_context_window: Option<u64>,
}

impl ModelCapabilityRequirements {
    const LONG_CONTEXT_DEFAULT: u64 = 128_000;

    pub(crate) fn is_empty(self) -> bool {
        !self.require_tools
            && !self.require_vision
            && !self.require_reasoning
            && !self.require_long_context
            && self.min_context_window.is_none()
    }

    fn effective_min_context(self) -> Option<u64> {
        match (self.require_long_context, self.min_context_window) {
            (true, Some(value)) => Some(value.max(Self::LONG_CONTEXT_DEFAULT)),
            (true, None) => Some(Self::LONG_CONTEXT_DEFAULT),
            (false, value) => value,
        }
    }

    pub(crate) fn summary(self) -> String {
        let mut parts = Vec::new();
        if self.require_tools {
            parts.push("tools".to_string());
        }
        if self.require_vision {
            parts.push("vision".to_string());
        }
        if self.require_reasoning {
            parts.push("reasoning".to_string());
        }
        if let Some(min_ctx) = self.effective_min_context() {
            parts.push(format!("context>={min_ctx}"));
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(", ")
        }
    }
}

// ---------------------------------------------------------------------------
// ResolvedModelCapabilities
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResolvedModelCapabilities {
    pub(crate) supports_tools: bool,
    pub(crate) supports_vision: bool,
    pub(crate) supports_reasoning: bool,
    pub(crate) context_window: u64,
}

// ---------------------------------------------------------------------------
// Capability helpers
// ---------------------------------------------------------------------------

fn normalize_model_capability_name(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "tools" | "tool" | "function-calling" | "function_calling" => Some("tools"),
        "vision" | "image" | "images" => Some("vision"),
        "reasoning" | "reason" => Some("reasoning"),
        "long-context" | "long_context" | "longcontext" | "context" => Some("long-context"),
        _ => None,
    }
}

fn apply_model_capability_token(
    requirements: &mut ModelCapabilityRequirements,
    token: &str,
) -> Result<(), AgentError> {
    let Some(normalized) = normalize_model_capability_name(token) else {
        return Err(AgentError::Config(format!(
            "Unknown model capability '{}' (expected one of: tools, vision, reasoning, long-context).",
            token
        )));
    };
    match normalized {
        "tools" => requirements.require_tools = true,
        "vision" => requirements.require_vision = true,
        "reasoning" => requirements.require_reasoning = true,
        "long-context" => requirements.require_long_context = true,
        _ => {}
    }
    Ok(())
}

fn parse_model_command_args(
    args: &[&str],
) -> Result<(Vec<String>, ModelCapabilityRequirements, Option<String>), AgentError> {
    let mut requirements = ModelCapabilityRequirements::default();
    let mut positional = Vec::new();
    let mut provider_override: Option<String> = None;
    let mut idx = 0usize;

    while idx < args.len() {
        let token = args[idx].trim();
        if token.is_empty() {
            idx += 1;
            continue;
        }

        if matches!(
            token.to_ascii_lowercase().as_str(),
            "--vision" | "--tools" | "--reasoning" | "--long-context" | "--long_context"
        ) {
            apply_model_capability_token(&mut requirements, token.trim_start_matches('-'))?;
            idx += 1;
            continue;
        }

        if matches!(
            token.to_ascii_lowercase().as_str(),
            "--cap" | "--caps" | "--require" | "--requires"
        ) {
            let value = args
                .get(idx + 1)
                .ok_or_else(|| AgentError::Config(format!("{} requires a value.", token)))?;
            for raw in value.split(',') {
                let candidate = raw.trim();
                if candidate.is_empty() {
                    continue;
                }
                apply_model_capability_token(&mut requirements, candidate)?;
            }
            idx += 2;
            continue;
        }

        if token.eq_ignore_ascii_case("--provider") || token.eq_ignore_ascii_case("-p") {
            let provider = args
                .get(idx + 1)
                .ok_or_else(|| AgentError::Config(format!("{} requires a provider slug.", token)))?
                .trim();
            if provider.is_empty() {
                return Err(AgentError::Config(
                    "provider override cannot be empty.".to_string(),
                ));
            }
            provider_override = Some(provider.to_ascii_lowercase());
            idx += 2;
            continue;
        }

        if token.eq_ignore_ascii_case("--min-context")
            || token.eq_ignore_ascii_case("--min_context")
        {
            let value = args
                .get(idx + 1)
                .ok_or_else(|| {
                    AgentError::Config("--min-context requires a numeric value.".into())
                })?
                .trim();
            let parsed = value.parse::<u64>().map_err(|_| {
                AgentError::Config(format!(
                    "Invalid --min-context value '{}'; expected integer token count.",
                    value
                ))
            })?;
            requirements.min_context_window = Some(parsed);
            idx += 2;
            continue;
        }

        positional.push(token.to_string());
        idx += 1;
    }

    Ok((positional, requirements, provider_override))
}

pub(crate) fn resolve_model_capabilities(
    provider: &str,
    model_id: &str,
    client: &hermes_intelligence::models_dev::ModelsDevClient,
) -> ResolvedModelCapabilities {
    if let Some(caps) = client.capabilities(provider, model_id) {
        return ResolvedModelCapabilities {
            supports_tools: caps.supports_tools,
            supports_vision: caps.supports_vision,
            supports_reasoning: caps.supports_reasoning,
            context_window: caps.context_window.max(1),
        };
    }

    let provider_model = format!("{}:{}", provider.trim(), model_id.trim());
    let info = get_model_info(&provider_model).or_else(|| get_model_info(model_id));
    ResolvedModelCapabilities {
        supports_tools: info
            .as_ref()
            .map(|entry| entry.supports_tools)
            .unwrap_or(true),
        supports_vision: info
            .as_ref()
            .map(|entry| entry.supports_vision)
            .unwrap_or(false),
        supports_reasoning: info
            .as_ref()
            .map(|entry| entry.supports_reasoning)
            .unwrap_or(false),
        context_window: get_model_context_length(&provider_model),
    }
}

fn model_meets_requirements(
    capabilities: ResolvedModelCapabilities,
    requirements: ModelCapabilityRequirements,
) -> bool {
    if requirements.require_tools && !capabilities.supports_tools {
        return false;
    }
    if requirements.require_vision && !capabilities.supports_vision {
        return false;
    }
    if requirements.require_reasoning && !capabilities.supports_reasoning {
        return false;
    }
    if let Some(min_context) = requirements.effective_min_context() {
        if capabilities.context_window < min_context {
            return false;
        }
    }
    true
}

pub(crate) fn unmet_model_requirements(
    capabilities: ResolvedModelCapabilities,
    requirements: ModelCapabilityRequirements,
) -> Vec<String> {
    let mut missing = Vec::new();
    if requirements.require_tools && !capabilities.supports_tools {
        missing.push("tools".to_string());
    }
    if requirements.require_vision && !capabilities.supports_vision {
        missing.push("vision".to_string());
    }
    if requirements.require_reasoning && !capabilities.supports_reasoning {
        missing.push("reasoning".to_string());
    }
    if let Some(min_context) = requirements.effective_min_context() {
        if capabilities.context_window < min_context {
            missing.push(format!(
                "context>={} (actual={})",
                min_context, capabilities.context_window
            ));
        }
    }
    missing
}

// ---------------------------------------------------------------------------
// handle_model_explain_command
// ---------------------------------------------------------------------------

async fn handle_model_explain_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
    strict_why_not: bool,
) -> Result<CommandResult, AgentError> {
    let (mut positional, requirements, provider_override) = parse_model_command_args(args)?;
    if let Some(provider) = provider_override {
        if positional.is_empty() {
            positional.push(provider);
        } else if let Some(first) = positional.first().cloned() {
            let model_id = first
                .split_once(':')
                .map(|(_, rhs)| rhs.to_string())
                .unwrap_or(first);
            positional[0] = format!("{}:{}", provider, model_id.trim());
        }
    }
    let target = if positional.is_empty() {
        host.current_model().to_string()
    } else {
        normalize_model_target(host.current_model(), &positional[0])?
    };
    let (guarded, remap_note) = guard_provider_model_selection(&target).await?;
    let (provider, model_id) = split_provider_model(&guarded);
    let client = default_client();
    client.fetch(false).await;
    let capabilities = resolve_model_capabilities(provider, model_id, client);

    let mut out = String::new();
    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("Model capability report\n"));
    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("-----------------------\n"));
    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("target: {}\n", guarded));
    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("provider: {}\n", provider.trim()));
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!("tools: {}\n", capabilities.supports_tools),
    );
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!("vision: {}\n", capabilities.supports_vision),
    );
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!("reasoning: {}\n", capabilities.supports_reasoning),
    );
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!("context_window: {}\n", capabilities.context_window),
    );
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!(
            "acp_multimodal_parts: {}\n",
            if capabilities.supports_vision {
                "supported"
            } else {
                "text-only fallback"
            }
        ),
    );
    if let Some(note) = remap_note.as_deref() {
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("catalog_guard: {}\n", note));
    }

    if !requirements.is_empty() {
        let unmet = unmet_model_requirements(capabilities, requirements);
        if unmet.is_empty() {
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!("requirements: satisfied ({})\n", requirements.summary()),
            );
        } else {
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!("requirements: FAILED ({})\n", requirements.summary()),
            );
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!("missing: {}\n", unmet.join(", ")),
            );
            let catalog = provider_model_ids(provider).await;
            let alternatives: Vec<String> = catalog
                .into_iter()
                .filter(|candidate| {
                    model_meets_requirements(
                        resolve_model_capabilities(provider, candidate, client),
                        requirements,
                    )
                })
                .take(8)
                .collect();
            if alternatives.is_empty() {
                let _ = std::fmt::Write::write_fmt(
                    &mut out,
                    format_args!("alternatives: none in provider catalog\n"),
                );
            } else {
                let _ = std::fmt::Write::write_fmt(
                    &mut out,
                    format_args!(
                        "alternatives: {}\n",
                        alternatives
                            .iter()
                            .map(|m| format!("{}:{}", provider, m))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
            if strict_why_not {
                return Err(AgentError::Config(out.trim_end().to_string()));
            }
        }
    } else if strict_why_not {
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "why-not mode requires constraints. Example: `/model why-not --cap tools,reasoning --min-context 200000`\n"
            ),
        );
    }

    emit_command_output(host, out.trim_end());
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Model switch helpers
// ---------------------------------------------------------------------------

fn parse_model_switch_request(args: &[&str], known_providers: &[&str]) -> ModelSwitchRequest {
    if args.is_empty() {
        return ModelSwitchRequest::PickProviderThenModel;
    }
    let raw = args.join(" ");
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ModelSwitchRequest::PickProviderThenModel;
    }
    if trimmed.contains(':') {
        return ModelSwitchRequest::SetDirect(trimmed.to_string());
    }
    if known_providers
        .iter()
        .any(|p| p.eq_ignore_ascii_case(trimmed))
    {
        return ModelSwitchRequest::PickModelFromProvider(trimmed.to_ascii_lowercase());
    }
    ModelSwitchRequest::SetDirect(trimmed.to_string())
}

pub(crate) fn split_provider_model(provider_model: &str) -> (&str, &str) {
    provider_model
        .split_once(':')
        .unwrap_or(("openai", provider_model))
}

fn model_catalog_guard_enabled() -> bool {
    !matches!(
        std::env::var("HERMES_MODEL_CATALOG_GUARD")
            .ok()
            .as_deref()
            .map(|v| v.trim().to_ascii_lowercase()),
        Some(v) if matches!(v.as_str(), "0" | "false" | "off" | "no")
    )
}

pub(crate) fn resolve_catalog_model_candidate(
    requested_model: &str,
    catalog: &[String],
) -> Option<String> {
    if catalog.is_empty() {
        return None;
    }
    let requested_trimmed = requested_model.trim();
    if requested_trimmed.is_empty() {
        return catalog.first().cloned();
    }
    if let Some(hit) = catalog
        .iter()
        .find(|m| m.trim().eq_ignore_ascii_case(requested_trimmed))
    {
        return Some(hit.clone());
    }
    let requested_lc = requested_trimmed.to_ascii_lowercase();
    let slash_suffix = format!("/{requested_lc}");
    if let Some(hit) = catalog.iter().find(|m| {
        let lower = m.trim().to_ascii_lowercase();
        lower.ends_with(&slash_suffix) || lower == requested_lc
    }) {
        return Some(hit.clone());
    }
    rank_catalog_model_candidates(requested_trimmed, catalog, 1)
        .into_iter()
        .next()
}

pub(crate) fn rank_catalog_model_candidates(
    requested_model: &str,
    catalog: &[String],
    limit: usize,
) -> Vec<String> {
    if catalog.is_empty() || limit == 0 {
        return Vec::new();
    }
    let requested = requested_model.trim().to_ascii_lowercase();
    if requested.is_empty() {
        return catalog.iter().take(limit).cloned().collect();
    }
    let requested_tail = requested.rsplit('/').next().unwrap_or(requested.as_str());
    let requested_norm: String = requested
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();

    let mut scored: Vec<(usize, usize, String)> = catalog
        .iter()
        .enumerate()
        .filter_map(|(idx, candidate)| {
            let cand_trimmed = candidate.trim();
            if cand_trimmed.is_empty() {
                return None;
            }
            let cand = cand_trimmed.to_ascii_lowercase();
            let cand_tail = cand.rsplit('/').next().unwrap_or(cand.as_str());
            let cand_norm: String = cand.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            let mut score = 0usize;

            if cand == requested {
                score += 10_000;
            }
            if cand_tail == requested_tail {
                score += 8_000;
            }
            if cand.ends_with(&format!("/{}", requested_tail)) {
                score += 6_000;
            }
            if cand.contains(requested_tail) || requested_tail.contains(cand_tail) {
                score += 2_000;
            }

            let shared_prefix = requested_norm
                .chars()
                .zip(cand_norm.chars())
                .take_while(|(a, b)| a == b)
                .count();
            score += shared_prefix.saturating_mul(40);

            let shared_chars = requested_norm
                .chars()
                .filter(|ch| cand_norm.contains(*ch))
                .count();
            score += shared_chars.saturating_mul(12);

            let len_delta = requested_norm.len().abs_diff(cand_norm.len());
            score = score.saturating_sub(len_delta.saturating_mul(4));
            if score == 0 {
                return None;
            }
            Some((score, idx, cand_trimmed.to_string()))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, candidate)| candidate)
        .collect()
}

/// Guard provider/model selection against the curated catalog.
async fn guard_provider_model_selection(
    provider_model: &str,
) -> Result<(String, Option<String>), AgentError> {
    if !model_catalog_guard_enabled() {
        return Ok((provider_model.to_string(), None));
    }

    let (provider, model_id) = split_provider_model(provider_model);
    let provider = provider.trim().to_ascii_lowercase();
    if provider.is_empty() {
        return Ok((provider_model.to_string(), None));
    }
    if matches!(provider.as_str(), "openai-codex" | "codex")
        || (provider == "openai" && model_id.to_ascii_lowercase().contains("codex"))
    {
        return Ok((
            provider_model.to_string(),
            Some(format!(
                "Catalog guard soft-accepted unlisted Codex model `{}`.",
                model_id.trim()
            )),
        ));
    }
    if !curated_provider_slugs()
        .iter()
        .any(|slug| slug.eq_ignore_ascii_case(&provider))
    {
        return Ok((provider_model.to_string(), None));
    }

    let catalog = provider_model_ids(&provider).await;
    if catalog.is_empty() {
        return Ok((provider_model.to_string(), None));
    }
    let Some(candidate) = resolve_catalog_model_candidate(model_id, &catalog) else {
        let suggestions = rank_catalog_model_candidates(model_id, &catalog, 5);
        return Err(AgentError::Config(format!(
            "Model '{}' is not available for provider '{}'. Close matches: {}. Use `/model {}` to pick a valid catalog entry.",
            model_id.trim(),
            provider,
            if suggestions.is_empty() {
                "(none)".to_string()
            } else {
                suggestions.join(", ")
            },
            provider,
        )));
    };
    let guarded = format!("{}:{}", provider, candidate);
    if guarded.eq_ignore_ascii_case(provider_model) {
        return Ok((provider_model.to_string(), None));
    }
    Ok((
        guarded.clone(),
        Some(format!(
            "Model catalog guard remapped `{}` -> `{}` based on provider catalog.",
            provider_model, guarded
        )),
    ))
}

fn normalize_model_target(current_model: &str, raw: &str) -> Result<String, AgentError> {
    let trimmed = raw.trim();
    if trimmed.contains(':') {
        return normalize_provider_model(trimmed);
    }
    let (provider, _) = split_provider_model(current_model);
    normalize_provider_model(&format!("{}:{}", provider.trim(), trimmed))
}

/// Run `curses_select` safely from both plain CLI and active TUI sessions.
fn run_model_picker_select(
    host: &impl crate::app::TranscriptRuntime,
    title: &str,
    items: &[String],
    initial_index: usize,
) -> SelectResult {
    if host.stream_attached() {
        curses_select_embedded(title, items, initial_index)
    } else {
        curses_select(title, items, initial_index)
    }
}

fn persist_current_model_selection(
    host: &(impl crate::app::SessionRuntime + crate::app::ModelRuntime),
) -> Result<PathBuf, AgentError> {
    let cfg_path = host.state_root().join("config.yaml");
    let mut disk = hermes_config::load_user_config_file(&cfg_path)
        .map_err(|e| AgentError::Config(e.to_string()))?;
    disk.model = Some(host.current_model().to_string());
    hermes_config::save_config_yaml(&cfg_path, &disk)
        .map_err(|e| AgentError::Config(e.to_string()))?;
    Ok(cfg_path)
}

fn format_model_persistence_note(
    host: &(impl crate::app::SessionRuntime + crate::app::ModelRuntime),
) -> String {
    match persist_current_model_selection(host) {
        Ok(path) => format!("Persisted default model in {}.", path.display()),
        Err(err) => format!(
            "Warning: switched for this session, but failed to persist default model: {}",
            err
        ),
    }
}

/// Interactive model picker for a specific provider.
async fn pick_model_for_provider(
    host: &mut impl crate::app::SlashCommandHost,
    provider: &str,
    current_model: &str,
    requirements: ModelCapabilityRequirements,
) -> Result<bool, AgentError> {
    let models = provider_model_ids(provider).await;
    if models.is_empty() {
        emit_command_output(
            host,
            format!("No models available for provider '{}'.", provider),
        );
        return Ok(false);
    }

    let normalized_provider = provider.trim().to_ascii_lowercase();
    let mut filtered_models = models.clone();
    if !requirements.is_empty() {
        let client = default_client();
        client.fetch(false).await;
        filtered_models = models
            .iter()
            .filter(|model_id| {
                model_meets_requirements(
                    resolve_model_capabilities(&normalized_provider, model_id, client),
                    requirements,
                )
            })
            .cloned()
            .collect();
    }

    if filtered_models.is_empty() {
        emit_command_output(
            host,
            format!(
                "No models for provider '{}' satisfy required capabilities: {}.",
                provider,
                requirements.summary()
            ),
        );
        return Ok(false);
    }

    let (_, current_model_id) = split_provider_model(current_model);
    let default_index = filtered_models
        .iter()
        .position(|m| m.eq_ignore_ascii_case(current_model_id))
        .unwrap_or(0);
    let labels: Vec<String> = filtered_models.clone();
    let title = format!("Select {} model ({} available)", provider, labels.len());
    let pick = run_model_picker_select(host, &title, &labels, default_index);
    if !pick.confirmed || pick.index >= filtered_models.len() {
        emit_command_output(host, "Model switch cancelled.");
        return Ok(false);
    }
    let provider_model = format!("{}:{}", provider, filtered_models[pick.index].trim());
    let (guarded, note) = guard_provider_model_selection(&provider_model).await?;
    host.switch_model(&guarded);
    let mut msg = format!("Model switched to: {}", guarded);
    if let Some(n) = note {
        msg.push_str("\n");
        msg.push_str(&n);
    }
    msg.push_str("\n");
    msg.push_str(&format_model_persistence_note(host));
    emit_command_output(host, msg);
    Ok(true)
}

// ---------------------------------------------------------------------------
// Failover helpers
// ---------------------------------------------------------------------------

fn parse_failover_chain(raw: &str) -> Result<Vec<String>, AgentError> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for token in raw.split(',') {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = normalize_provider_model(trimmed)?;
        let key = normalized.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(normalized);
        }
    }
    Ok(out)
}

fn read_failover_chain_from_env() -> Vec<String> {
    if let Ok(raw) = std::env::var("HERMES_FALLBACK_MODELS") {
        let parsed: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .collect();
        if !parsed.is_empty() {
            return parsed;
        }
    }
    if let Ok(raw) = std::env::var("HERMES_FALLBACK_MODEL") {
        let value = raw.trim();
        if !value.is_empty() {
            return vec![value.to_string()];
        }
    }
    Vec::new()
}

fn handle_model_failover_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    match action.as_str() {
        "status" | "show" => {
            let chain_items = read_failover_chain_from_env();
            let fallback = chain_items.first().map(|s| s.as_str()).unwrap_or("(none)");
            let chain = if chain_items.is_empty() {
                "(none)".to_string()
            } else {
                chain_items.join(", ")
            };
            emit_command_output(
                host,
                format!(
                    "Failover fabric\nprimary_fallback: {}\nchain: {}\nusage: `/model failover set provider:model[,provider:model...]` or `/model failover clear`",
                    fallback, chain
                ),
            );
        }
        "clear" | "reset" => {
            env_vars::remove_var("HERMES_FALLBACK_MODEL");
            env_vars::remove_var("HERMES_FALLBACK_MODELS");
            let current = host.current_model().to_string();
            host.switch_model(&current);
            emit_command_output(host, "Cleared retry failover chain.");
        }
        "set" => {
            let raw = args
                .get(1)
                .ok_or_else(|| {
                    AgentError::Config(
                        "Usage: /model failover set provider:model[,provider:model...]".to_string(),
                    )
                })?
                .trim();
            let chain = parse_failover_chain(raw)?;
            if chain.is_empty() {
                return Err(AgentError::Config(
                    "Failover chain cannot be empty.".to_string(),
                ));
            }
            env_vars::set_var("HERMES_FALLBACK_MODELS", chain.join(","));
            if let Some(first) = chain.first() {
                env_vars::set_var("HERMES_FALLBACK_MODEL", first);
            }
            let current = host.current_model().to_string();
            host.switch_model(&current);
            emit_command_output(host, format!("Failover chain set: {}", chain.join(", ")));
        }
        _ => {
            emit_command_output(
                host,
                "Usage: /model failover [status|set provider:model[,provider:model...]|clear]",
            );
        }
    }
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Backend best-practice profiles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct BackendBestPracticeProfile {
    provider: &'static str,
    profile: &'static str,
    summary: &'static str,
    launch_hint: &'static str,
    env_overrides: &'static [(&'static str, &'static str)],
}

const VLLM_PROFILE_BALANCED_ENV: &[(&str, &str)] = &[
    ("VLLM_GPU_MEMORY_UTILIZATION", "0.88"),
    ("VLLM_ENABLE_PREFIX_CACHING", "1"),
    ("VLLM_ENABLE_CHUNKED_PREFILL", "1"),
];
const VLLM_PROFILE_THROUGHPUT_ENV: &[(&str, &str)] = &[
    ("VLLM_GPU_MEMORY_UTILIZATION", "0.92"),
    ("VLLM_MAX_NUM_SEQS", "256"),
    ("VLLM_ENABLE_PREFIX_CACHING", "1"),
];
const VLLM_PROFILE_RELIABILITY_ENV: &[(&str, &str)] = &[
    ("VLLM_GPU_MEMORY_UTILIZATION", "0.80"),
    ("VLLM_MAX_NUM_SEQS", "64"),
    ("VLLM_ENABLE_CHUNKED_PREFILL", "0"),
];
const LLAMA_CPP_PROFILE_BALANCED_ENV: &[(&str, &str)] = &[
    ("LLAMA_CPP_THREADS", "8"),
    ("LLAMA_CPP_CTX_SIZE", "8192"),
    ("LLAMA_CPP_BATCH", "512"),
];
const MLX_PROFILE_BALANCED_ENV: &[(&str, &str)] = &[
    ("MLX_QUANT", "4bit"),
    ("MLX_MAX_BATCH_SIZE", "16"),
    ("MLX_ENABLE_PROMPT_CACHE", "1"),
];
const SGLANG_PROFILE_BALANCED_ENV: &[(&str, &str)] = &[
    ("SGLANG_ENABLE_RADIX_CACHE", "1"),
    ("SGLANG_MAX_RUNNING_REQUESTS", "256"),
];
const TGI_PROFILE_BALANCED_ENV: &[(&str, &str)] = &[
    ("TGI_MAX_BATCH_TOTAL_TOKENS", "32768"),
    ("TGI_WAITING_SERVED_RATIO", "0.30"),
];
const APPLE_ANE_PROFILE_BALANCED_ENV: &[(&str, &str)] = &[
    ("APPLE_ANE_ENABLE_LOW_LATENCY", "1"),
    ("APPLE_ANE_PREFILL_TOKENS", "1024"),
];
const MISTRAL_RS_PROFILE_BALANCED_ENV: &[(&str, &str)] = &[
    ("MISTRAL_RS_PAGED_ATTENTION", "1"),
    ("MISTRAL_RS_KV_CACHE_DTYPE", "fp16"),
    ("MISTRAL_RS_SPECULATIVE_DECODING", "0"),
];

const BACKEND_BEST_PRACTICE_PROFILES: &[BackendBestPracticeProfile] = &[
    BackendBestPracticeProfile {
        provider: "vllm",
        profile: "balanced",
        summary: "Default SOTA profile for stable throughput and latency.",
        launch_hint: "vllm serve MODEL --enable-prefix-caching --enable-chunked-prefill --gpu-memory-utilization 0.88",
        env_overrides: VLLM_PROFILE_BALANCED_ENV,
    },
    BackendBestPracticeProfile {
        provider: "vllm",
        profile: "throughput",
        summary: "Higher concurrency profile for heavy parallel workloads.",
        launch_hint: "vllm serve MODEL --enable-prefix-caching --max-num-seqs 256 --gpu-memory-utilization 0.92",
        env_overrides: VLLM_PROFILE_THROUGHPUT_ENV,
    },
    BackendBestPracticeProfile {
        provider: "vllm",
        profile: "reliability",
        summary: "Lower-pressure profile tuned for long sessions and fewer OOM events.",
        launch_hint: "vllm serve MODEL --max-num-seqs 64 --gpu-memory-utilization 0.80 --disable-chunked-prefill",
        env_overrides: VLLM_PROFILE_RELIABILITY_ENV,
    },
    BackendBestPracticeProfile {
        provider: "llama-cpp",
        profile: "balanced",
        summary: "General local GGUF serving profile with predictable latency.",
        launch_hint: "llama-server -m MODEL.gguf -c 8192 -t 8 -b 512 --host 127.0.0.1 --port 8080",
        env_overrides: LLAMA_CPP_PROFILE_BALANCED_ENV,
    },
    BackendBestPracticeProfile {
        provider: "mlx",
        profile: "balanced",
        summary: "Apple Silicon profile prioritizing cache reuse and compact memory.",
        launch_hint: "python -m mlx_lm.server --model mlx-community/Qwen3-8B-4bit --host 127.0.0.1 --port 8080",
        env_overrides: MLX_PROFILE_BALANCED_ENV,
    },
    BackendBestPracticeProfile {
        provider: "apple-ane",
        profile: "balanced",
        summary: "ANE-optimized low-latency settings for on-device endpoints.",
        launch_hint: "Use your ANE OpenAI-compatible server with low-latency prefill settings.",
        env_overrides: APPLE_ANE_PROFILE_BALANCED_ENV,
    },
    BackendBestPracticeProfile {
        provider: "sglang",
        profile: "balanced",
        summary: "SGLang cache-first profile for sustained request loads.",
        launch_hint: "python -m sglang.launch_server --model-path MODEL --host 127.0.0.1 --port 30000",
        env_overrides: SGLANG_PROFILE_BALANCED_ENV,
    },
    BackendBestPracticeProfile {
        provider: "tgi",
        profile: "balanced",
        summary: "Text-Generation-Inference profile balancing batch depth and tail latency.",
        launch_hint: "text-generation-launcher --model-id MODEL --port 8082 --max-batch-total-tokens 32768",
        env_overrides: TGI_PROFILE_BALANCED_ENV,
    },
    BackendBestPracticeProfile {
        provider: "mistral-rs",
        profile: "balanced",
        summary: "mistral.rs runtime baseline for robust local serving.",
        launch_hint: "mistralrs-server --model MODEL --port 8083 --paged-attention",
        env_overrides: MISTRAL_RS_PROFILE_BALANCED_ENV,
    },
];

fn normalize_backend_provider(value: &str) -> String {
    let raw = value.trim().to_ascii_lowercase();
    match raw.as_str() {
        "llvm" | "ollvm" => "vllm".to_string(),
        "llama.cpp" | "llamacpp" => "llama-cpp".to_string(),
        "ane" => "apple-ane".to_string(),
        other => other.to_string(),
    }
}

fn backend_profile_lookup(
    provider: &str,
    profile: Option<&str>,
) -> Option<&'static BackendBestPracticeProfile> {
    let normalized = normalize_backend_provider(provider);
    let profile = profile.unwrap_or("balanced").trim().to_ascii_lowercase();
    BACKEND_BEST_PRACTICE_PROFILES.iter().find(|row| {
        row.provider.eq_ignore_ascii_case(&normalized) && row.profile.eq_ignore_ascii_case(&profile)
    })
}

fn render_backend_profiles(provider: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("Backend best-practice profiles\n");
    out.push_str("-------------------------------\n");
    let filtered: Vec<&BackendBestPracticeProfile> = if let Some(provider) = provider {
        let normalized = normalize_backend_provider(provider);
        BACKEND_BEST_PRACTICE_PROFILES
            .iter()
            .filter(|row| row.provider.eq_ignore_ascii_case(&normalized))
            .collect()
    } else {
        BACKEND_BEST_PRACTICE_PROFILES.iter().collect()
    };
    if filtered.is_empty() {
        let selected = provider.unwrap_or("(none)");
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!("No backend profile presets found for '{}'.", selected),
        );
        return out.trim_end().to_string();
    }
    for row in filtered {
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "- {}:{}\n  {}\n  launch: {}\n  env: {}\n",
                row.provider,
                row.profile,
                row.summary,
                row.launch_hint,
                row.env_overrides
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
        );
    }
    out.push_str("\nUse `/model backend apply <provider> [profile]` to load env overrides for current runtime.");
    out.trim_end().to_string()
}

fn persist_backend_profile_env(
    provider: &str,
    profile: &str,
    env_pairs: &[(&str, &str)],
) -> Result<PathBuf, AgentError> {
    let dir = hermes_config::hermes_home()
        .join("runtime")
        .join("backend_profiles");
    std::fs::create_dir_all(&dir).map_err(|e| {
        AgentError::Io(format!(
            "Failed to create backend profile directory {}: {}",
            dir.display(),
            e
        ))
    })?;
    let path = dir.join(format!(
        "{}-{}.env",
        normalize_backend_provider(provider),
        profile.trim().to_ascii_lowercase()
    ));
    let mut body = String::new();
    for (key, value) in env_pairs {
        let _ = std::fmt::Write::write_fmt(&mut body, format_args!("{}={}\n", key, value));
    }
    std::fs::write(&path, body).map_err(|e| {
        AgentError::Io(format!(
            "Failed to write backend profile file {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(path)
}

fn model_current_provider_and_id(model: &str) -> (String, String) {
    if let Some((provider, model_id)) = model.split_once(':') {
        (
            provider.trim().to_ascii_lowercase(),
            model_id.trim().to_string(),
        )
    } else {
        ("openai".to_string(), model.trim().to_string())
    }
}

// ---------------------------------------------------------------------------
// handle_model_harness_command
// ---------------------------------------------------------------------------

async fn handle_model_harness_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let (current_provider, current_model_id) = model_current_provider_and_id(host.current_model());
    let target = args.first().copied().unwrap_or_default().trim();
    let (provider, requested_model) = if target.is_empty() {
        (current_provider.clone(), current_model_id.clone())
    } else if target.contains(':') {
        let normalized = normalize_provider_model(target)?;
        let (prov, model_id) = model_current_provider_and_id(&normalized);
        (prov, model_id)
    } else {
        (normalize_backend_provider(target), current_model_id.clone())
    };

    let catalog = provider_model_ids(&provider).await;
    let catalog_total = catalog.len();
    let selected_model = requested_model.trim().to_string();
    let selected_lc = selected_model.to_ascii_lowercase();
    let selected_ok = catalog.iter().any(|candidate| {
        let lower = candidate.trim().to_ascii_lowercase();
        lower == selected_lc || lower.ends_with(&format!("/{selected_lc}"))
    });
    let credential_present = provider_api_key_from_env(&provider).is_some();
    let auth_state_present = crate::auth::read_provider_auth_state(&provider)
        .ok()
        .flatten()
        .is_some();
    let cache_status = cached_provider_catalog_status(&provider);
    let mut out = String::new();
    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("Model/provider harness\n"));
    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("provider: {}\n", provider));
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!("selected_model: {}\n", selected_model),
    );
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!(
            "credentials: api_key={} oauth_state={}\n",
            super::yes_no(credential_present),
            super::yes_no(auth_state_present)
        ),
    );
    let _ =
        std::fmt::Write::write_fmt(&mut out, format_args!("catalog_total: {}\n", catalog_total));
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!("selected_in_catalog: {}\n", super::yes_no(selected_ok)),
    );
    if let Some(status) = cache_status {
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "catalog_cache: verified={} age_secs={}\n",
                super::yes_no(status.verified),
                status
                    .age_secs
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".to_string())
            ),
        );
    } else {
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("catalog_cache: unavailable\n"));
    }
    if !selected_ok {
        let sample = catalog
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<String>>()
            .join(", ");
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "remediation: switch via `/model {} --provider {}` (or run `/model {}`)\n",
                selected_model, provider, provider
            ),
        );
        if !sample.is_empty() {
            let _ =
                std::fmt::Write::write_fmt(&mut out, format_args!("catalog_sample: {}\n", sample));
        }
    }
    if provider == "openrouter" && !credential_present && !auth_state_present {
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "openrouter_hint: set OPENROUTER_API_KEY or use a provider with OAuth (`/auth refresh`).\n"
            ),
        );
    }
    if provider == "huggingface" {
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "huggingface_hint: prefer HF_TOKEN + HF_BASE_URL for full catalog enumeration.\n"
            ),
        );
    }
    emit_command_output(host, out.trim_end());
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// handle_model_backend_command
// ---------------------------------------------------------------------------

fn handle_model_backend_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args.first().copied().unwrap_or("list").to_ascii_lowercase();
    match action.as_str() {
        "list" | "status" => {
            let provider = args.get(1).copied();
            emit_command_output(host, render_backend_profiles(provider));
        }
        "show" => {
            let Some(provider) = args.get(1).copied() else {
                emit_command_output(host, "Usage: /model backend show <provider> [profile]");
                return Ok(CommandResult::Handled);
            };
            let profile = args.get(2).copied();
            let Some(row) = backend_profile_lookup(provider, profile) else {
                emit_command_output(
                    host,
                    format!(
                        "No backend profile found for {}:{}.",
                        provider,
                        profile.unwrap_or("balanced")
                    ),
                );
                return Ok(CommandResult::Handled);
            };
            emit_command_output(
                host,
                format!(
                    "{}:{}\n{}\nlaunch: {}\nenv: {}",
                    row.provider,
                    row.profile,
                    row.summary,
                    row.launch_hint,
                    row.env_overrides
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<String>>()
                        .join(", ")
                ),
            );
        }
        "apply" => {
            let Some(provider) = args.get(1).copied() else {
                emit_command_output(host, "Usage: /model backend apply <provider> [profile]");
                return Ok(CommandResult::Handled);
            };
            let profile = args.get(2).copied().unwrap_or("balanced");
            let Some(row) = backend_profile_lookup(provider, Some(profile)) else {
                emit_command_output(
                    host,
                    format!("No backend profile found for {}:{}.", provider, profile),
                );
                return Ok(CommandResult::Handled);
            };
            for (key, value) in row.env_overrides {
                env_vars::set_var(key, value);
            }
            env_vars::set_var("HERMES_LOCAL_BACKEND_PROFILE", row.profile);
            env_vars::set_var("HERMES_LOCAL_BACKEND_PROVIDER", row.provider);
            let persisted =
                persist_backend_profile_env(row.provider, row.profile, row.env_overrides)?;
            let (current_provider, _) = model_current_provider_and_id(host.current_model());
            if current_provider == row.provider {
                let current = host.current_model().to_string();
                host.switch_model(&current);
            }
            emit_command_output(
                host,
                format!(
                    "Applied backend profile {}:{}.\nlaunch: {}\npersisted_env_file: {}\nUse `set -a && source {}` before launching external backend processes.",
                    row.provider,
                    row.profile,
                    row.launch_hint,
                    persisted.display(),
                    persisted.display()
                ),
            );
        }
        _ => emit_command_output(
            host,
            "Usage: /model backend [list|status [provider]|show <provider> [profile]|apply <provider> [profile]]",
        ),
    }
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// handle_model_command (main entry point)
// ---------------------------------------------------------------------------

pub(crate) async fn handle_model_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if let Some(sub) = args.first().map(|v| v.trim()) {
        if sub.eq_ignore_ascii_case("failover") {
            return handle_model_failover_command(host, &args[1..]);
        }
        if sub.eq_ignore_ascii_case("backend") {
            return handle_model_backend_command(host, &args[1..]);
        }
        if sub.eq_ignore_ascii_case("harness") {
            return handle_model_harness_command(host, &args[1..]).await;
        }
        if sub.eq_ignore_ascii_case("explain") {
            return handle_model_explain_command(host, &args[1..], false).await;
        }
        if sub.eq_ignore_ascii_case("why-not")
            || sub.eq_ignore_ascii_case("whynot")
            || sub.eq_ignore_ascii_case("diagnose")
        {
            return handle_model_explain_command(host, &args[1..], true).await;
        }
    }

    let (mut positional, requirements, provider_override) = parse_model_command_args(args)?;
    if let Some(provider) = provider_override {
        if positional.is_empty() {
            positional.push(provider);
        } else if let Some(first) = positional.first().cloned() {
            let model_id = first
                .split_once(':')
                .map(|(_, rhs)| rhs.to_string())
                .unwrap_or(first);
            positional[0] = format!("{}:{}", provider, model_id.trim());
        }
    }
    let positional_refs: Vec<&str> = positional.iter().map(String::as_str).collect();
    let known_providers = curated_provider_slugs();
    match parse_model_switch_request(&positional_refs, &known_providers) {
        ModelSwitchRequest::SetDirect(raw) => {
            let provider_model = normalize_model_target(host.current_model(), &raw)?;
            let (guarded, note) = guard_provider_model_selection(&provider_model).await?;
            if !requirements.is_empty() {
                let (provider, model_id) = split_provider_model(&guarded);
                let client = default_client();
                client.fetch(false).await;
                let caps = resolve_model_capabilities(provider, model_id, client);
                if !model_meets_requirements(caps, requirements) {
                    return Err(AgentError::Config(format!(
                        "Requested model '{}' does not satisfy required capabilities: {}.",
                        guarded,
                        requirements.summary()
                    )));
                }
            }
            host.switch_model(&guarded);
            let mut msg = format!("Model switched to: {}", guarded);
            if let Some(n) = note {
                msg.push_str("\n");
                msg.push_str(&n);
            }
            if !requirements.is_empty() {
                msg.push_str("\n");
                msg.push_str(&format!(
                    "Capability constraints satisfied: {}.",
                    requirements.summary()
                ));
            }
            msg.push_str("\n");
            msg.push_str(&format_model_persistence_note(host));
            emit_command_output(host, msg);
        }
        ModelSwitchRequest::PickModelFromProvider(provider) => {
            let current_model = host.current_model().to_string();
            pick_model_for_provider(host, &provider, &current_model, requirements).await?;
        }
        ModelSwitchRequest::PickProviderThenModel => {
            emit_command_output(host, format!("Current model: {}", host.current_model()));
            let providers: Vec<String> = known_providers.iter().map(|p| (*p).to_string()).collect();
            if providers.is_empty() {
                emit_command_output(host, "No providers are registered for selection.");
                return Ok(CommandResult::Handled);
            }
            let (current_provider, _) = split_provider_model(host.current_model());
            let default_provider_index = providers
                .iter()
                .position(|p| p.eq_ignore_ascii_case(current_provider))
                .unwrap_or(0);
            let provider_pick = run_model_picker_select(
                host,
                "Select provider",
                &providers,
                default_provider_index,
            );
            if !provider_pick.confirmed || provider_pick.index >= providers.len() {
                emit_command_output(host, "Model switch cancelled.");
                return Ok(CommandResult::Handled);
            }
            let provider = providers[provider_pick.index].as_str();
            let current_model = host.current_model().to_string();
            pick_model_for_provider(host, provider, &current_model, requirements).await?;
        }
    }
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Reasoning effort helpers (needed by model tests)
// ---------------------------------------------------------------------------

pub(crate) fn resolve_provider_key<'a>(cfg: &'a GatewayConfig, provider: &str) -> String {
    cfg.llm_providers
        .keys()
        .find(|key| key.eq_ignore_ascii_case(provider))
        .cloned()
        .unwrap_or_else(|| provider.trim().to_ascii_lowercase())
}

pub(crate) fn gemini_thinking_level_for_effort(effort: &str) -> &'static str {
    match effort {
        "minimal" | "low" => "low",
        "medium" => "medium",
        "high" | "xhigh" => "high",
        _ => "medium",
    }
}

pub(crate) fn openai_reasoning_effort_for_level(effort: &str) -> &'static str {
    match effort {
        "minimal" => "low",
        "xhigh" => "high",
        "low" => "low",
        "medium" => "medium",
        "high" => "high",
        _ => "medium",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use hermes_config::LlmProviderConfig;

    fn set_provider_reasoning_effort(
        cfg: &mut GatewayConfig,
        provider: &str,
        effort: Option<&str>,
    ) {
        let provider_key = resolve_provider_key(cfg, provider);
        let provider_cfg = cfg
            .llm_providers
            .entry(provider_key.clone())
            .or_insert_with(LlmProviderConfig::default);

        let mut body_map = provider_cfg
            .extra_body
            .take()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();

        match effort {
            Some(level) => {
                body_map.remove("reasoning_effort");
                let mut reasoning_obj = body_map
                    .get("reasoning")
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                let mapped_reasoning = openai_reasoning_effort_for_level(level);
                reasoning_obj.insert(
                    "effort".to_string(),
                    serde_json::Value::String(mapped_reasoning.to_string()),
                );
                body_map.insert(
                    "reasoning".to_string(),
                    serde_json::Value::Object(reasoning_obj),
                );

                if provider_key.contains("gemini") || provider_key == "google" {
                    let level_mapped = gemini_thinking_level_for_effort(level);
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
                        serde_json::Value::String(level_mapped.to_string()),
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
            None => {
                body_map.remove("reasoning_effort");
                if let Some(reasoning_obj) = body_map
                    .get_mut("reasoning")
                    .and_then(|value| value.as_object_mut())
                {
                    reasoning_obj.remove("effort");
                    if reasoning_obj.is_empty() {
                        body_map.remove("reasoning");
                    }
                }
                body_map.remove("thinking_config");
                if let Some(google_obj) = body_map
                    .get_mut("google")
                    .and_then(|value| value.as_object_mut())
                {
                    google_obj.remove("thinking_config");
                    if google_obj.is_empty() {
                        body_map.remove("google");
                    }
                }
            }
        }

        provider_cfg.extra_body = if body_map.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(body_map))
        };
    }
    use crate::test_env_lock;

    fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        test_env_lock::lock()
    }

    #[tokio::test]
    async fn guard_provider_model_selection_soft_accepts_unlisted_codex_models() {
        let _guard = env_test_lock();
        crate::env_vars::set_var("HERMES_MODEL_CATALOG_GUARD", "1");
        let (guarded, note) = guard_provider_model_selection("openai-codex:gpt-9-codex-preview")
            .await
            .expect("codex soft-accept");
        assert_eq!(guarded, "openai-codex:gpt-9-codex-preview");
        assert!(
            note.as_deref()
                .unwrap_or_default()
                .contains("soft-accepted")
        );
        crate::env_vars::remove_var("HERMES_MODEL_CATALOG_GUARD");
    }

    #[test]
    fn parse_model_switch_request_picks_provider_when_empty() {
        let providers = vec!["openai", "nous", "anthropic"];
        let req = parse_model_switch_request(&[], &providers);
        assert_eq!(req, ModelSwitchRequest::PickProviderThenModel);
    }

    #[test]
    fn backend_profile_lookup_resolves_aliases() {
        let row = backend_profile_lookup("llvm", Some("throughput")).expect("profile");
        assert_eq!(row.provider, "vllm");
        assert_eq!(row.profile, "throughput");
    }

    #[test]
    fn parse_model_command_args_extracts_capability_flags() {
        let (positional, requirements, provider_override) = parse_model_command_args(&[
            "nous",
            "--cap",
            "vision,reasoning",
            "--min-context",
            "200000",
        ])
        .expect("parse");
        assert_eq!(positional, vec!["nous".to_string()]);
        assert!(requirements.require_vision);
        assert!(requirements.require_reasoning);
        assert!(!requirements.require_tools);
        assert_eq!(requirements.min_context_window, Some(200_000));
        assert!(provider_override.is_none());
    }

    #[test]
    fn parse_model_command_args_supports_boolean_capability_switches() {
        let (positional, requirements, provider_override) =
            parse_model_command_args(&["openai:gpt-4o", "--tools", "--long-context"])
                .expect("parse");
        assert_eq!(positional, vec!["openai:gpt-4o".to_string()]);
        assert!(requirements.require_tools);
        assert!(requirements.require_long_context);
        assert_eq!(
            requirements.effective_min_context(),
            Some(ModelCapabilityRequirements::LONG_CONTEXT_DEFAULT)
        );
        assert!(provider_override.is_none());
    }

    #[test]
    fn parse_model_command_args_extracts_provider_override() {
        let (positional, _requirements, provider_override) =
            parse_model_command_args(&["gpt-4o", "--provider", "openai"]).expect("parse");
        assert_eq!(positional, vec!["gpt-4o".to_string()]);
        assert_eq!(provider_override.as_deref(), Some("openai"));
    }

    #[test]
    fn model_meets_requirements_checks_tools_vision_reasoning_and_context() {
        let requirements = ModelCapabilityRequirements {
            require_tools: true,
            require_vision: true,
            require_reasoning: true,
            require_long_context: false,
            min_context_window: Some(128_000),
        };
        let caps = ResolvedModelCapabilities {
            supports_tools: true,
            supports_vision: true,
            supports_reasoning: true,
            context_window: 200_000,
        };
        assert!(model_meets_requirements(caps, requirements));
        let weak_caps = ResolvedModelCapabilities {
            supports_tools: true,
            supports_vision: false,
            supports_reasoning: true,
            context_window: 200_000,
        };
        assert!(!model_meets_requirements(weak_caps, requirements));
    }

    #[test]
    fn unmet_model_requirements_lists_missing_constraints() {
        let requirements = ModelCapabilityRequirements {
            require_tools: true,
            require_vision: true,
            require_reasoning: true,
            require_long_context: false,
            min_context_window: Some(256_000),
        };
        let caps = ResolvedModelCapabilities {
            supports_tools: true,
            supports_vision: false,
            supports_reasoning: false,
            context_window: 128_000,
        };
        let missing = unmet_model_requirements(caps, requirements);
        assert!(missing.iter().any(|m| m == "vision"));
        assert!(missing.iter().any(|m| m == "reasoning"));
        assert!(
            missing
                .iter()
                .any(|m| m.contains("context>=256000 (actual=128000)"))
        );
    }

    #[test]
    fn parse_model_command_args_rejects_unknown_capability() {
        let err = parse_model_command_args(&["--cap", "telepathy"]).expect_err("expected error");
        let message = err.to_string().to_ascii_lowercase();
        assert!(message.contains("unknown model capability"));
    }

    #[test]
    fn parse_model_switch_request_uses_provider_picker_for_provider_arg() {
        let providers = vec!["openai", "nous", "anthropic"];
        let req = parse_model_switch_request(&["NOUS"], &providers);
        assert_eq!(
            req,
            ModelSwitchRequest::PickModelFromProvider("nous".to_string())
        );
    }

    #[test]
    fn parse_model_switch_request_accepts_direct_provider_model() {
        let providers = vec!["openai", "nous", "anthropic"];
        let req = parse_model_switch_request(&["openai:gpt-4o"], &providers);
        assert_eq!(
            req,
            ModelSwitchRequest::SetDirect("openai:gpt-4o".to_string())
        );
    }

    #[test]
    fn parse_model_switch_request_keeps_bare_model_as_direct() {
        let providers = vec!["openai", "nous", "anthropic"];
        let req = parse_model_switch_request(&["gpt-4o"], &providers);
        assert_eq!(req, ModelSwitchRequest::SetDirect("gpt-4o".to_string()));
    }

    #[test]
    fn normalize_model_target_uses_current_provider_for_bare_model() {
        let normalized = normalize_model_target("nous:moonshotai/kimi-k2.6", "openai/gpt-5.5")
            .expect("normalize");
        assert_eq!(normalized, "nous:openai/gpt-5.5");
    }

    #[test]
    fn normalize_model_target_keeps_explicit_provider_model() {
        let normalized = normalize_model_target("nous:moonshotai/kimi-k2.6", "openai:gpt-5.4")
            .expect("normalize");
        assert_eq!(normalized, "openai:gpt-5.4");
    }

    #[test]
    fn set_provider_reasoning_effort_updates_and_clears_extra_body() {
        let mut cfg = GatewayConfig::default();
        set_provider_reasoning_effort(&mut cfg, "nous", Some("high"));
        let extra = cfg
            .llm_providers
            .get("nous")
            .and_then(|entry| entry.extra_body.as_ref())
            .expect("extra body");
        assert!(extra.get("reasoning_effort").is_none());
        assert_eq!(
            extra
                .get("reasoning")
                .and_then(|value| value.get("effort"))
                .and_then(|value| value.as_str())
                .expect("reasoning.effort"),
            "high"
        );

        set_provider_reasoning_effort(&mut cfg, "nous", None);
        let extra_after_clear = cfg
            .llm_providers
            .get("nous")
            .and_then(|entry| entry.extra_body.as_ref());
        assert!(extra_after_clear.is_none());
    }

    #[test]
    fn set_provider_reasoning_effort_normalizes_openai_effort_levels() {
        let mut cfg = GatewayConfig::default();
        set_provider_reasoning_effort(&mut cfg, "nous", Some("xhigh"));
        let extra = cfg
            .llm_providers
            .get("nous")
            .and_then(|entry| entry.extra_body.as_ref())
            .expect("extra body");
        assert_eq!(
            extra
                .get("reasoning")
                .and_then(|value| value.get("effort"))
                .and_then(|value| value.as_str()),
            Some("high")
        );
        set_provider_reasoning_effort(&mut cfg, "nous", Some("minimal"));
        let extra = cfg
            .llm_providers
            .get("nous")
            .and_then(|entry| entry.extra_body.as_ref())
            .expect("extra body");
        assert_eq!(
            extra
                .get("reasoning")
                .and_then(|value| value.get("effort"))
                .and_then(|value| value.as_str()),
            Some("low")
        );
    }

    #[test]
    fn set_provider_reasoning_effort_sets_gemini_thinking_level() {
        let mut cfg = GatewayConfig::default();
        set_provider_reasoning_effort(&mut cfg, "gemini", Some("xhigh"));
        let extra = cfg
            .llm_providers
            .get("gemini")
            .and_then(|entry| entry.extra_body.as_ref())
            .expect("extra body");
        assert_eq!(
            extra
                .get("google")
                .and_then(|value| value.get("thinking_config"))
                .and_then(|value| value.get("thinking_level"))
                .and_then(|value| value.as_str()),
            Some("high")
        );
        assert_eq!(
            extra
                .get("thinking_config")
                .and_then(|value| value.get("thinking_level"))
                .and_then(|value| value.as_str()),
            Some("high")
        );
    }

    #[test]
    fn resolve_catalog_model_candidate_prefers_suffix_match() {
        let catalog = vec![
            "nousresearch/hermes-4-405b".to_string(),
            "moonshotai/kimi-k2.6".to_string(),
        ];
        let chosen = resolve_catalog_model_candidate("kimi-k2.6", &catalog).expect("candidate");
        assert_eq!(chosen, "moonshotai/kimi-k2.6");
    }

    #[test]
    fn resolve_catalog_model_candidate_uses_relative_match_for_near_miss() {
        let catalog = vec![
            "qwen/qwen3.6-plus".to_string(),
            "qwen/qwen3.6-max-preview".to_string(),
            "moonshotai/kimi-k2.6".to_string(),
        ];
        let chosen = resolve_catalog_model_candidate("qwen3.6-max", &catalog).expect("candidate");
        assert_eq!(chosen, "qwen/qwen3.6-max-preview");
    }

    #[test]
    fn rank_catalog_model_candidates_returns_best_first() {
        let catalog = vec![
            "qwen/qwen3.6-plus".to_string(),
            "qwen/qwen3.6-max-preview".to_string(),
            "moonshotai/kimi-k2.6".to_string(),
        ];
        let ranked = rank_catalog_model_candidates("qwen3.6-max", &catalog, 2);
        assert_eq!(
            ranked,
            vec![
                "qwen/qwen3.6-max-preview".to_string(),
                "qwen/qwen3.6-plus".to_string()
            ]
        );
    }
}
