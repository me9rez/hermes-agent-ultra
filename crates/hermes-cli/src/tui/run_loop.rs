use std::collections::HashSet;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use hermes_auth::FileTokenStore;
use hermes_core::{AgentError, AgentResult, Message, StreamChunk};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::Tui;
use super::TuiLoopHost;
use super::event::{Event, StreamHandle};
use super::pipeline::{
    abort_agent_lanes, shutdown_crossterm_event_pipeline, shutdown_signal_bridge,
    spawn_crossterm_event_pipeline, spawn_signal_bridge,
};
use super::render::{
    draw_frame_now, parse_markdown_numbered_marker, render, should_redraw_stream_while_composing,
    should_render_completions_popup, should_route_prompt_via_managed_agent,
    stream_event_completes_background_task, stream_lane_budget, tool_complete_looks_failed,
};
use super::state::TuiState;
use super::text::{is_ctrl_c, is_submit_shortcut, truncate_chars};
use super::transcript_cache::TranscriptCache;
use super::types::{InputMode, ModalAction, PickerItem, PickerKind, PickerModal};
use crate::app::{
    AcpServerRuntime, AgentCoordinator, App, ModelRuntime, SessionRuntime, SessionSnapshotRuntime,
    SlashCommandHost, TranscriptRuntime, UiChromeRuntime,
    actors::{AgentLane, StandardAgentRunRequest},
};
use crate::commands;

pub(crate) fn parse_slash_parts(input: &str) -> Option<(String, Vec<String>)> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let mut iter = trimmed.split_whitespace();
    let cmd = iter.next()?.to_string();
    let args = iter.map(ToString::to_string).collect::<Vec<_>>();
    Some((cmd, args))
}

#[derive(Debug, Clone)]
pub(crate) struct InteractiveQuestionRequest {
    pub(crate) prompt: String,
    pub(crate) options: Vec<PickerItem>,
}

pub(crate) fn strip_question_option_marker(line: &str) -> String {
    let trimmed = line.trim();
    if let Some(body) = trimmed.strip_prefix("- ") {
        return body.trim().to_string();
    }
    if let Some(body) = trimmed.strip_prefix("* ") {
        return body.trim().to_string();
    }
    if let Some(body) = trimmed.strip_prefix("+ ") {
        return body.trim().to_string();
    }
    if let Some((_marker, body)) = parse_markdown_numbered_marker(trimmed) {
        return body.trim().to_string();
    }
    trimmed.to_string()
}

pub(crate) fn parse_question_option(value: &str) -> PickerItem {
    let raw = value.trim();
    if let Some((label, detail)) = raw.split_once("::") {
        return PickerItem {
            label: label.trim().to_string(),
            detail: detail.trim().to_string(),
            value: label.trim().to_string(),
        };
    }
    PickerItem {
        label: raw.to_string(),
        detail: String::new(),
        value: raw.to_string(),
    }
}

pub(crate) fn parse_interactive_question_request(
    input: &str,
) -> Result<InteractiveQuestionRequest, String> {
    let trimmed = input.trim();
    if !(trimmed.starts_with("/ask") || trimmed.starts_with("/question")) {
        return Err("not an interactive question command".to_string());
    }
    let cmd = trimmed
        .split_whitespace()
        .next()
        .ok_or_else(|| "missing command".to_string())?;
    let rest = trimmed.strip_prefix(cmd).unwrap_or("").trim();
    if rest.is_empty() || rest.eq_ignore_ascii_case("help") {
        return Err("Usage: `/ask <question> | <option 1> | <option 2> [| <option 3> ...]`\nAlternative multiline format:\n`/ask\\n<question>\\n- <option 1>\\n- <option 2>`".to_string());
    }

    if rest.eq_ignore_ascii_case("demo") {
        return Ok(InteractiveQuestionRequest {
            prompt: "How should we proceed?".to_string(),
            options: vec![
                PickerItem {
                    label: "Continue implementation (Recommended)".to_string(),
                    detail: "Keep shipping patches now.".to_string(),
                    value: "Continue implementation".to_string(),
                },
                PickerItem {
                    label: "Pause for diagnosis".to_string(),
                    detail: "Inspect logs and root-cause first.".to_string(),
                    value: "Pause for diagnosis".to_string(),
                },
                PickerItem {
                    label: "Switch model/provider".to_string(),
                    detail: "Try a different runtime profile.".to_string(),
                    value: "Switch model/provider".to_string(),
                },
            ],
        });
    }

    let mut prompt = String::new();
    let mut raw_options: Vec<String> = Vec::new();
    if rest.contains('|') {
        let pieces: Vec<String> = rest
            .split('|')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(ToString::to_string)
            .collect();
        if let Some(first) = pieces.first() {
            prompt = first.clone();
        }
        for piece in pieces.iter().skip(1) {
            raw_options.push(strip_question_option_marker(piece));
        }
    } else {
        let lines: Vec<String> = rest
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect();
        if let Some(first) = lines.first() {
            prompt = first.clone();
        }
        for line in lines.iter().skip(1) {
            raw_options.push(strip_question_option_marker(line));
        }
    }

    raw_options.retain(|o| !o.trim().is_empty());
    if prompt.trim().is_empty() {
        return Err("Question prompt is empty. Provide a question before the options.".to_string());
    }
    if raw_options.len() < 2 {
        return Err(
            "Provide at least 2 options. Example: `/ask Pick mode | safe | fast`".to_string(),
        );
    }
    if raw_options.len() > 12 {
        raw_options.truncate(12);
    }
    let options = raw_options
        .iter()
        .map(|value| parse_question_option(value))
        .collect();

    Ok(InteractiveQuestionRequest { prompt, options })
}

pub(crate) fn provider_env_key_hints(provider: &str) -> &'static [&'static str] {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openai" => &["HERMES_OPENAI_API_KEY", "OPENAI_API_KEY"],
        "openai-codex" | "codex" => &["HERMES_OPENAI_CODEX_API_KEY"],
        "anthropic" => &["ANTHROPIC_API_KEY"],
        "nous" => &["NOUS_API_KEY"],
        "openrouter" => &["OPENROUTER_API_KEY"],
        "gemini" | "google" => &["GOOGLE_API_KEY", "GEMINI_API_KEY"],
        "google-gemini-cli" => &["HERMES_GEMINI_OAUTH_API_KEY"],
        "qwen" => &["DASHSCOPE_API_KEY", "QWEN_API_KEY"],
        "qwen-oauth" => &["HERMES_QWEN_OAUTH_API_KEY", "DASHSCOPE_API_KEY"],
        "deepseek" => &["DEEPSEEK_API_KEY"],
        "kimi" | "moonshot" | "kimi-coding" | "kimi-coding-cn" => &["KIMI_API_KEY"],
        "ollama-local" => &["OLLAMA_LOCAL_API_KEY", "OLLAMA_API_KEY"],
        "llama-cpp" => &["LLAMA_CPP_API_KEY"],
        "vllm" => &["VLLM_API_KEY"],
        "mlx" => &["MLX_API_KEY"],
        "apple-ane" => &["APPLE_ANE_API_KEY"],
        "sglang" => &["SGLANG_API_KEY"],
        "tgi" => &["TGI_API_KEY"],
        "zai" => &["ZAI_API_KEY"],
        "minimax" | "minimax-cn" => &["MINIMAX_API_KEY"],
        "stepfun" => &["HERMES_STEPFUN_API_KEY", "STEPFUN_API_KEY"],
        _ => &[],
    }
}

pub(crate) async fn load_token_store_providers() -> HashSet<String> {
    let path = hermes_config::paths::hermes_home()
        .join("auth")
        .join("tokens.json");
    let Ok(store) = FileTokenStore::new(path).await else {
        return HashSet::new();
    };
    store
        .list_providers()
        .await
        .into_iter()
        .map(|provider| provider.to_ascii_lowercase())
        .collect()
}

pub(crate) fn provider_auth_detail(
    provider: &str,
    token_store_providers: &HashSet<String>,
) -> String {
    let normalized = provider.trim().to_ascii_lowercase();
    let mut sources: Vec<String> = Vec::new();
    for key in provider_env_key_hints(&normalized) {
        if std::env::var(key)
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            sources.push(format!("env:{key}"));
            break;
        }
    }
    if token_store_providers.contains(&normalized) {
        sources.push("vault".to_string());
    }
    if crate::auth::read_provider_auth_state(&normalized)
        .ok()
        .flatten()
        .is_some()
    {
        sources.push("oauth".to_string());
    }

    if sources.is_empty() {
        let setup_hint = provider_env_key_hints(&normalized)
            .first()
            .copied()
            .unwrap_or("API_KEY");
        format!("auth:missing (setup: /auth {normalized} or {setup_hint})")
    } else {
        format!("auth:{}", sources.join("+"))
    }
}

pub(crate) async fn disconnect_provider_credentials(
    provider: &str,
) -> Result<(bool, bool), AgentError> {
    let normalized = provider.trim().to_ascii_lowercase();
    let path = hermes_config::paths::hermes_home()
        .join("auth")
        .join("tokens.json");
    let mut removed_vault = false;
    if let Ok(store) = FileTokenStore::new(path).await {
        removed_vault = store
            .list_providers()
            .await
            .into_iter()
            .any(|p| p.eq_ignore_ascii_case(&normalized));
        let _ = store.remove(&normalized).await;
    }
    let removed_oauth = crate::auth::clear_provider_auth_state(&normalized).unwrap_or(false);
    Ok((removed_vault, removed_oauth))
}

pub(crate) async fn open_model_provider_modal(state: &mut TuiState, app: &impl ModelRuntime) {
    let providers = crate::model_switch::curated_provider_slugs();
    let entries = crate::model_switch::provider_catalog_entries(&providers, 4).await;
    let token_store_providers = load_token_store_providers().await;
    let mut items: Vec<PickerItem> = Vec::new();
    for provider in providers {
        let entry = entries
            .iter()
            .find(|entry| entry.provider.eq_ignore_ascii_case(provider));
        let auth_detail = provider_auth_detail(provider, &token_store_providers);
        let detail = if let Some(entry) = entry {
            if entry.models.is_empty() {
                format!("{} models • {}", entry.total_models, auth_detail)
            } else {
                format!(
                    "{} models • {} • {}",
                    entry.total_models,
                    entry.models.join(", "),
                    auth_detail
                )
            }
        } else {
            format!("catalog unavailable • {}", auth_detail)
        };
        items.push(PickerItem {
            label: provider.to_string(),
            detail,
            value: provider.to_string(),
        });
    }
    let mut modal = PickerModal::new(PickerKind::ModelProvider, "Select Provider", items);
    let (current_provider, _) = app
        .current_model()
        .split_once(':')
        .unwrap_or(("openai", app.current_model()));
    if let Some(idx) = modal.filtered_indices.iter().position(|item_idx| {
        modal.items[*item_idx]
            .value
            .eq_ignore_ascii_case(current_provider)
    }) {
        modal.selected_filtered = idx;
    }
    state.open_modal(modal);
}

pub(crate) async fn open_provider_model_modal(
    state: &mut TuiState,
    app: &impl ModelRuntime,
    provider: &str,
) {
    let models = crate::model_switch::provider_model_ids(provider).await;
    if models.is_empty() {
        state.status_message = format!("No models available for provider `{provider}`");
        return;
    }
    let mut items = Vec::with_capacity(models.len());
    for model in models {
        items.push(PickerItem {
            label: model.clone(),
            detail: format!("{provider}:{model}"),
            value: model,
        });
    }
    let mut modal = PickerModal::new(
        PickerKind::ModelForProvider {
            provider: provider.to_string(),
        },
        format!("Select {provider} model"),
        items,
    );
    let (_, current_model_id) = app
        .current_model()
        .split_once(':')
        .unwrap_or(("openai", app.current_model()));
    if let Some(idx) = modal.filtered_indices.iter().position(|item_idx| {
        modal.items[*item_idx]
            .value
            .eq_ignore_ascii_case(current_model_id)
    }) {
        modal.selected_filtered = idx;
    }
    state.open_modal(modal);
}

pub(crate) fn open_personality_modal(state: &mut TuiState, app: &impl ModelRuntime) {
    let descriptions = hermes_agent::builtin_personality_descriptions();
    let mut items = Vec::with_capacity(descriptions.len());
    for (name, detail) in descriptions {
        items.push(PickerItem {
            label: (*name).to_string(),
            detail: (*detail).to_string(),
            value: (*name).to_string(),
        });
    }
    let mut modal = PickerModal::new(PickerKind::Personality, "Select Personality", items);
    if let Some(current) = app.current_personality() {
        if let Some(idx) = modal
            .filtered_indices
            .iter()
            .position(|item_idx| modal.items[*item_idx].value.eq_ignore_ascii_case(current))
        {
            modal.selected_filtered = idx;
        }
    }
    state.open_modal(modal);
}

pub(crate) fn open_skin_modal(state: &mut TuiState) {
    let mut items = Vec::with_capacity(crate::skin_engine::BUILTIN_SKINS.len());
    for (name, detail) in crate::skin_engine::BUILTIN_SKINS {
        items.push(PickerItem {
            label: (*name).to_string(),
            detail: (*detail).to_string(),
            value: (*name).to_string(),
        });
    }
    let mut modal = PickerModal::new(PickerKind::Skin, "Select Skin", items);
    let active = std::env::var("HERMES_THEME").unwrap_or_else(|_| "ultra-sunburst".to_string());
    let active_canonical =
        crate::skin_engine::canonical_skin_name(&active).unwrap_or("ultra-sunburst");
    if let Some(idx) = modal.filtered_indices.iter().position(|item_idx| {
        modal.items[*item_idx]
            .value
            .eq_ignore_ascii_case(active_canonical)
    }) {
        modal.selected_filtered = idx;
    }
    state.open_modal(modal);
}

pub(crate) fn open_interactive_question_modal(
    state: &mut TuiState,
    request: InteractiveQuestionRequest,
) {
    let mut modal = PickerModal::new(
        PickerKind::InteractiveQuestion {
            prompt: request.prompt,
        },
        "Interactive Question",
        request.options,
    );
    modal.page_size = 8;
    modal.refresh_filter();
    state.open_modal(modal);
}

pub(crate) async fn process_modal_disconnect(
    state: &mut TuiState,
    app: &mut (impl SlashCommandHost + ModelRuntime),
) -> Result<(), AgentError> {
    let Some(modal) = state.phase.modal().clone() else {
        return Ok(());
    };
    let Some(item) = modal.selected_item().cloned() else {
        state.status_message = "No provider selected".to_string();
        return Ok(());
    };
    if !matches!(modal.kind, PickerKind::ModelProvider) {
        state.status_message = "Disconnect is only supported in provider picker".to_string();
        return Ok(());
    }
    let provider = item.value.trim().to_ascii_lowercase();
    match disconnect_provider_credentials(&provider).await {
        Ok((removed_vault, removed_oauth)) => {
            if removed_vault || removed_oauth {
                state.status_message = format!(
                    "Disconnected `{provider}` (vault={}, oauth={})",
                    removed_vault, removed_oauth
                );
                app.push_ui_assistant(format!(
                    "Disconnected provider `{}` (vault={}, oauth={}).",
                    provider, removed_vault, removed_oauth
                ));
            } else {
                state.status_message =
                    format!("No stored credential found for `{provider}` to disconnect");
            }
            open_model_provider_modal(state, app).await;
        }
        Err(err) => {
            state.status_message = format!("Disconnect failed for `{provider}`: {err}");
        }
    }
    Ok(())
}

pub(crate) async fn process_modal_confirm(
    state: &mut TuiState,
    app: &mut (impl SlashCommandHost + ModelRuntime + UiChromeRuntime),
) -> Result<(), AgentError> {
    let Some(modal) = state.phase.modal().clone() else {
        return Ok(());
    };
    let Some(item) = modal.selected_item().cloned() else {
        state.status_message = "Nothing selected".to_string();
        return Ok(());
    };
    match &modal.kind {
        PickerKind::ModelProvider => {
            open_provider_model_modal(state, app, &item.value).await;
            state.status_message = format!("Provider selected: {}", item.value);
        }
        PickerKind::ModelForProvider { provider } => {
            let provider_model = format!("{provider}:{}", item.value.trim());
            app.switch_model(&provider_model);
            app.push_ui_assistant(format!("Model switched to: {}", provider_model));
            state.close_modal();
            state.status_message = format!("Switched model to {}", provider_model);
        }
        PickerKind::Personality => {
            app.switch_personality(item.value.as_str());
            app.push_ui_assistant(format!("Switched personality to `{}`.", item.value));
            state.close_modal();
            state.status_message = format!("Personality: {}", item.value);
        }
        PickerKind::Skin => {
            let skin = crate::skin_engine::canonical_skin_name(item.value.as_str())
                .unwrap_or("ultra-sunburst")
                .to_string();
            crate::env_vars::set_var("HERMES_THEME", &skin);
            app.request_theme_change(&skin);
            app.push_ui_assistant(format!("Switched skin to `{}`.", skin));
            state.close_modal();
            state.status_message = format!("Skin: {}", skin);
        }
        PickerKind::InteractiveQuestion { prompt } => {
            let chosen = item.value.trim().to_string();
            state.phase.composer_mut().input = format!("{prompt}\nAnswer: {chosen}");
            state.phase.composer_mut().cursor_position = state.phase.composer_mut().input.len();
            state.close_modal();
            state.status_message = "Interactive answer inserted. Press Enter to send.".to_string();
            state.refresh_completions();
        }
    }
    Ok(())
}

pub(crate) fn handle_agent_run_complete(
    app: &mut (impl SessionSnapshotRuntime + TranscriptRuntime),
    state: &mut TuiState,
    result: Result<AgentResult, String>,
    elapsed_secs: f64,
) {
    match result {
        Ok(agent_result) => {
            let total_turns = agent_result.total_turns;
            let interrupted = agent_result.interrupted;
            let finished_naturally = agent_result.finished_naturally;
            if let Err(err) = app.apply_agent_result_and_persist(agent_result) {
                tracing::warn!("session autosave skipped: {}", err);
                state.push_activity(format!("⚠ autosave skipped: {}", err));
            }
            state.finish_processing_cycle("✔ completed in");
            state.status_message.clear();
            state.push_activity(format!(
                "run finished in {:.2}s (total_turns={})",
                elapsed_secs, total_turns
            ));
            if interrupted {
                app.push_ui_assistant("[Agent execution interrupted]".to_string());
            } else if !finished_naturally {
                state.push_activity("run stopped before natural finish".to_string());
            }
        }
        Err(err) => {
            state.finish_processing_cycle("✖ failed after");
            state.status_message = format!("Error: {}", err);
            state.push_activity(format!("✖ {}", err));
            app.push_ui_assistant(format!("Error: {}", err));
        }
    }
    // finish_processing_cycle drops ProcessingState; guard defensively
    if let Some(p) = state.phase.processing_mut() {
        p.stream_buffer.clear();
        p.stream_md_cache.clear();
        p.stream_muted = false;
        p.stream_needs_break = false;
        p.active_tools.clear();
        p.awaiting_run_complete = false;
    }
}

pub(crate) fn handle_managed_app_run_complete(
    app: &mut App,
    state: &mut TuiState,
    result: Result<Box<App>, String>,
    elapsed_secs: f64,
) {
    match result {
        Ok(completed_app) => {
            *app = *completed_app;
            state.finish_processing_cycle("✔ completed in");
            state.status_message.clear();
            state.push_activity(format!("managed run finished in {:.2}s", elapsed_secs));
        }
        Err(err) => {
            state.finish_processing_cycle("✖ failed after");
            state.status_message = format!("Error: {}", err);
            state.push_activity(format!("✖ {}", err));
            app.push_ui_assistant(format!("Error: {}", err));
        }
    }
    if let Some(p) = state.phase.processing_mut() {
        p.stream_buffer.clear();
        p.stream_md_cache.clear();
        p.stream_muted = false;
        p.stream_needs_break = false;
        p.active_tools.clear();
        p.awaiting_run_complete = false;
    }
}

pub(crate) fn extract_file_like_hints(text: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        if out.len() >= limit {
            break;
        }
        let cleaned = token
            .trim_matches(|c: char| {
                c == '"' || c == '\'' || c == ',' || c == ';' || c == ')' || c == '('
            })
            .to_string();
        if cleaned.len() < 4 {
            continue;
        }
        let looks_like_path = cleaned.contains('/')
            || cleaned.ends_with(".rs")
            || cleaned.ends_with(".py")
            || cleaned.ends_with(".toml")
            || cleaned.ends_with(".md")
            || cleaned.ends_with(".json")
            || cleaned.ends_with(".yaml")
            || cleaned.ends_with(".yml");
        if !looks_like_path {
            continue;
        }
        if !out.iter().any(|v| v == &cleaned) {
            out.push(cleaned);
        }
    }
    out
}

pub(crate) fn stream_chunk_has_progress(chunk: &StreamChunk) -> bool {
    if let Some(delta) = chunk.delta.as_ref() {
        let has_content = delta
            .content
            .as_ref()
            .is_some_and(|text| !text.trim().is_empty());
        let has_tool_calls = delta
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty());
        let has_extra_event = delta.extra.as_ref().is_some_and(|extra| match extra {
            serde_json::Value::Null => false,
            serde_json::Value::Object(map) => !map.is_empty(),
            _ => true,
        });
        if has_content || has_tool_calls || has_extra_event {
            return true;
        }
    }
    chunk
        .finish_reason
        .as_ref()
        .is_some_and(|reason| !reason.trim().is_empty())
        || chunk.usage.is_some()
}

pub(crate) enum StreamLaneOutcome {
    Handled(bool),
    AgentRunComplete {
        result: Result<AgentResult, String>,
        elapsed_secs: f64,
    },
    ManagedAppRunComplete {
        result: Result<Box<App>, String>,
        elapsed_secs: f64,
    },
}

fn apply_stream_lane_outcome(
    app: &mut App,
    state: &mut TuiState,
    outcome: StreamLaneOutcome,
) -> bool {
    match outcome {
        StreamLaneOutcome::Handled(mut redraw) => {
            if let Some(processing) = state.phase.processing_mut() {
                if let Some(prompt) = processing.pending_clarify_prompt.take() {
                    app.push_ui_assistant(prompt);
                    redraw = true;
                }
            }
            redraw
        }
        StreamLaneOutcome::AgentRunComplete {
            result,
            elapsed_secs,
        } => {
            handle_agent_run_complete(app, state, result, elapsed_secs);
            true
        }
        StreamLaneOutcome::ManagedAppRunComplete {
            result,
            elapsed_secs,
        } => {
            handle_managed_app_run_complete(app, state, result, elapsed_secs);
            true
        }
    }
}

pub(crate) fn process_stream_lane_event(state: &mut TuiState, event: Event) -> StreamLaneOutcome {
    match event {
        Event::StreamDelta(delta) => {
            if !delta.is_empty() {
                let processing = state.phase.processing_mut().expect("processing");
                processing.stream_chunk_count = processing.stream_chunk_count.saturating_add(1);
                processing.stream_char_count = processing
                    .stream_char_count
                    .saturating_add(delta.chars().count());
                if !processing.saw_first_token {
                    processing.saw_first_token = true;
                    let first_token_ms = processing
                        .started_at
                        .map(|t| t.elapsed().as_millis())
                        .unwrap_or_default();
                    state.push_activity(format!("↧ first token in {}ms", first_token_ms));
                }
            }
            state
                .phase
                .processing_mut()
                .expect("processing")
                .stream_buffer
                .push_str(&delta);
            StreamLaneOutcome::Handled(true)
        }
        Event::StreamChunk(chunk) => {
            if stream_chunk_has_progress(&chunk) {
                state
                    .phase
                    .processing_mut()
                    .expect("processing")
                    .stream_chunk_count = state
                    .phase
                    .processing_mut()
                    .expect("processing")
                    .stream_chunk_count
                    .saturating_add(1);
            }
            if let Some(delta) = chunk.delta {
                if let Some(content) = delta.content.as_ref().filter(|text| !text.is_empty()) {
                    state
                        .phase
                        .processing_mut()
                        .expect("processing")
                        .stream_char_count = state
                        .phase
                        .processing_mut()
                        .expect("processing")
                        .stream_char_count
                        .saturating_add(content.chars().count());
                }
                if let Some(extra) = delta.extra.as_ref() {
                    if let Some(control) = extra.get("control").and_then(|v| v.as_str()) {
                        if control == "mute_post_response" {
                            state
                                .phase
                                .processing_mut()
                                .expect("processing")
                                .stream_muted = extra
                                .get("enabled")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                        } else if control == "stream_break" {
                            state
                                .phase
                                .processing_mut()
                                .expect("processing")
                                .stream_needs_break = true;
                        }
                    }
                    if let Some(ui_event) = extra.get("ui_event").and_then(|v| v.as_str()) {
                        match ui_event {
                            "tool_start" => {
                                let tool = extra
                                    .get("tool")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("tool")
                                    .trim()
                                    .to_string();
                                if !tool.is_empty()
                                    && !state
                                        .phase
                                        .processing_mut()
                                        .expect("processing")
                                        .active_tools
                                        .iter()
                                        .any(|t| t == &tool)
                                {
                                    state
                                        .phase
                                        .processing_mut()
                                        .expect("processing")
                                        .active_tools
                                        .push(tool.clone());
                                }
                                if tool == "clarify" {
                                    let question = extra
                                        .get("clarify_question")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("(clarification needed)");
                                    let choices: Vec<String> = extra
                                        .get("clarify_choices")
                                        .and_then(|v| v.as_array())
                                        .map(|arr| {
                                            arr.iter()
                                                .filter_map(|v| v.as_str().map(str::to_string))
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    let prompt = crate::runtime_tool_wiring::format_clarify_prompt_for_ui(
                                        question,
                                        &choices,
                                    );
                                    let processing = state
                                        .phase
                                        .processing_mut()
                                        .expect("processing");
                                    processing.clarify_awaiting = true;
                                    processing.pending_clarify_prompt = Some(prompt);
                                    state.status_message =
                                        "Clarify: reply in the composer (number or text)"
                                            .to_string();
                                }
                                let args_preview = extra
                                    .get("args_preview")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .trim();
                                if args_preview.is_empty() {
                                    state.push_activity(format!("▶ {}", tool));
                                } else {
                                    state.push_activity(format!("▶ {} {}", tool, args_preview));
                                }
                                let active_count = state
                                    .phase
                                    .processing()
                                    .map(|processing| processing.active_tools.len())
                                    .unwrap_or(0);
                                state.push_activity(format!("Δtools active={active_count}"));
                            }
                            "tool_complete" => {
                                let tool = extra
                                    .get("tool")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("tool")
                                    .trim()
                                    .to_string();
                                if tool == "clarify" {
                                    state
                                        .phase
                                        .processing_mut()
                                        .expect("processing")
                                        .clarify_awaiting = false;
                                }
                                if let Some(idx) = state
                                    .phase
                                    .processing_mut()
                                    .expect("processing")
                                    .active_tools
                                    .iter()
                                    .position(|t| t == &tool)
                                {
                                    state
                                        .phase
                                        .processing_mut()
                                        .expect("processing")
                                        .active_tools
                                        .remove(idx);
                                }
                                let result_preview = extra
                                    .get("result_preview")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .trim();
                                let failed = tool_complete_looks_failed(extra, result_preview);
                                let mark = if failed { "✗" } else { "✓" };
                                if result_preview.is_empty() {
                                    state.push_activity(format!("{mark} {}", tool));
                                } else {
                                    state.push_activity(format!(
                                        "{mark} {} {}",
                                        tool, result_preview
                                    ));
                                    if !failed {
                                        let file_hints = extract_file_like_hints(result_preview, 3);
                                        if !file_hints.is_empty() {
                                            state.push_activity(format!(
                                                "Δfiles {}",
                                                file_hints.join(", ")
                                            ));
                                        }
                                    }
                                }
                            }
                            "status" => {
                                let event_type = extra
                                    .get("event_type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("status")
                                    .trim();
                                let message = extra
                                    .get("message")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .trim();
                                if !message.is_empty() {
                                    state.push_activity(format!("[{}] {}", event_type, message));
                                }
                            }
                            "phase" => {
                                let phase = extra
                                    .get("phase")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("phase");
                                let label =
                                    extra.get("label").and_then(|v| v.as_str()).unwrap_or("");
                                let progress_pct = extra
                                    .get("progress_pct")
                                    .and_then(|v| v.as_u64())
                                    .and_then(|v| u8::try_from(v).ok());
                                state.update_processing_phase(phase, label, progress_pct);
                            }
                            "lifecycle" => {
                                let message = extra
                                    .get("message")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .trim();
                                if !message.is_empty() {
                                    state.push_activity(format!("⟡ {}", message));
                                    let lower = message.to_ascii_lowercase();
                                    if lower.contains("mismatch")
                                        || lower.contains("remediation")
                                        || lower.contains("auto-refresh")
                                        || lower.contains("retrying")
                                        || lower.contains("fallback")
                                    {
                                        state
                                            .phase
                                            .processing_mut()
                                            .expect("processing")
                                            .processing_degraded = true;
                                        state
                                            .phase
                                            .processing_mut()
                                            .expect("processing")
                                            .degraded_notes
                                            .push(truncate_chars(message, 120));
                                        if state
                                            .phase
                                            .processing_mut()
                                            .expect("processing")
                                            .degraded_notes
                                            .len()
                                            > 4
                                        {
                                            let drop_count = state
                                                .phase
                                                .processing_mut()
                                                .expect("processing")
                                                .degraded_notes
                                                .len()
                                                - 4;
                                            state
                                                .phase
                                                .processing_mut()
                                                .expect("processing")
                                                .degraded_notes
                                                .drain(0..drop_count);
                                        }
                                    }
                                }
                            }
                            "thinking" => {
                                if let Some(text) = extra.get("text").and_then(|v| v.as_str()) {
                                    state.append_live_thinking(text);
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(thinking) = extra.get("thinking").and_then(|v| v.as_str()) {
                        state.append_live_thinking(thinking);
                    }
                }
                if let Some(content) = delta.content {
                    if !state
                        .phase
                        .processing_mut()
                        .expect("processing")
                        .stream_muted
                    {
                        if state
                            .phase
                            .processing_mut()
                            .expect("processing")
                            .stream_needs_break
                        {
                            state
                                .phase
                                .processing_mut()
                                .expect("processing")
                                .stream_buffer
                                .push_str("\n\n");
                            state
                                .phase
                                .processing_mut()
                                .expect("processing")
                                .stream_needs_break = false;
                        }
                        let processing = state.phase.processing_mut().expect("processing");
                        processing.stream_buffer.push_str(&content);
                        processing.stream_char_count = processing
                            .stream_char_count
                            .saturating_add(content.chars().count());
                        if !processing.saw_first_token {
                            processing.saw_first_token = true;
                            let first_token_ms = processing
                                .started_at
                                .map(|t| t.elapsed().as_millis())
                                .unwrap_or_default();
                            state.push_activity(format!("↧ first token in {}ms", first_token_ms));
                        }
                        if state.auto_follow_transcript {
                            state.scroll_offset = 0;
                        }
                    }
                }
            }
            if let Some(usage) = chunk.usage {
                state.last_usage = Some((
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens,
                ));
                let previous = state.last_usage_total_emitted.unwrap_or(0);
                if usage.total_tokens >= previous.saturating_add(64)
                    || state.last_usage_total_emitted.is_none()
                {
                    let delta_total = usage.total_tokens.saturating_sub(previous);
                    state.push_activity(format!(
                        "Δtokens p={} c={} t={} (+{})",
                        usage.prompt_tokens,
                        usage.completion_tokens,
                        usage.total_tokens,
                        delta_total
                    ));
                    state.last_usage_total_emitted = Some(usage.total_tokens);
                }
            }
            StreamLaneOutcome::Handled(true)
        }
        Event::AgentDone => {
            if state
                .phase
                .processing_mut()
                .expect("processing")
                .awaiting_run_complete
            {
                state.push_activity("finalizing transcript writeback…".to_string());
                state.status_message = "Finalizing response…".to_string();
            } else {
                state.finish_processing_cycle("✔ completed in");
                state
                    .phase
                    .processing_mut()
                    .expect("processing")
                    .stream_buffer
                    .clear();
                state
                    .phase
                    .processing_mut()
                    .expect("processing")
                    .stream_md_cache
                    .clear();
                state
                    .phase
                    .processing_mut()
                    .expect("processing")
                    .stream_muted = false;
                state
                    .phase
                    .processing_mut()
                    .expect("processing")
                    .stream_needs_break = false;
                state
                    .phase
                    .processing_mut()
                    .expect("processing")
                    .active_tools
                    .clear();
                state.status_message.clear();
            }
            StreamLaneOutcome::Handled(true)
        }
        Event::AgentRunComplete {
            result,
            elapsed_secs,
        } => StreamLaneOutcome::AgentRunComplete {
            result,
            elapsed_secs,
        },
        Event::ManagedAppRunComplete {
            result,
            elapsed_secs,
        } => StreamLaneOutcome::ManagedAppRunComplete {
            result,
            elapsed_secs,
        },
        _ => StreamLaneOutcome::Handled(false),
    }
}

// ---------------------------------------------------------------------------
// Main TUI run loop
// ---------------------------------------------------------------------------

/// Run the interactive TUI with the given App.
///
/// This is the main entry point for the interactive TUI mode.
/// It sets up the terminal, renders frames, and handles events.
pub async fn run(mut app: App) -> Result<(), AgentError> {
    let mut tui = Tui::new().map_err(|e| AgentError::Config(e.to_string()))?;
    let mut state = TuiState::default();
    let mut last_jobs_refresh = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or_else(Instant::now);
    let mut last_pet_tick = Instant::now();
    let mut last_compose_stream_redraw = Instant::now();
    app.set_stream_handle(Some(StreamHandle::from(tui.stream_sender())));

    // Crossterm blocking I/O runs on a dedicated OS thread (poll timeout 16ms).
    let event_pipeline = spawn_crossterm_event_pipeline(tui.event_sender())?;
    let signal_bridge = spawn_signal_bridge(tui.event_sender());

    let mut frame_tick = tokio::time::interval(Duration::from_millis(60));
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut needs_redraw = true;
    let agent_lane = AgentLane::spawn();
    let mut active_managed_task: Option<JoinHandle<()>> = None;

    // Main event loop
    while app.running() {
        tui.set_mouse_capture(app.mouse_enabled())
            .map_err(|e| AgentError::Config(e.to_string()))?;

        if let Some(theme_name) = app.take_pending_theme_change() {
            let applied = crate::skin_engine::resolve_theme(&theme_name);
            tui.set_theme(applied);
            needs_redraw = true;
        }

        if needs_redraw {
            state.refresh_sticky_prompt(&app);
            let active_theme = tui.theme().clone();
            tui.terminal
                .draw(|f| {
                    render(f, &app, &mut state, &active_theme);
                })
                .map_err(|e| AgentError::Config(e.to_string()))?;
            needs_redraw = false;
        }

        tokio::select! {
            biased;
            event = tui.events.recv() => {
                match event {
                    Some(Event::Paste(text)) => {
                        if state.modal_active() {
                            state.status_message = "Paste ignored while picker is open".to_string();
                        } else {
                            let line_count = text.lines().count().max(1);
                            state.insert_paste_at_cursor(&text);
                            state.status_message = format!("Pasted {} line(s)", line_count);
                        }
                        needs_redraw = true;
                        continue;
                    }
                    Some(Event::Key(key)) => {
                        // Ctrl+C always exits back to parent terminal. If work is in flight,
                        // emit interrupt first so in-progress tools can stop gracefully.
                        if is_ctrl_c(&key) {
                            if state.phase.is_processing() {
                                app.interrupt_controller_mut().interrupt(None);
                                abort_agent_lanes(&agent_lane, &mut active_managed_task).await;
                                tui.event_sender().send(Event::Interrupt).ok();
                            }
                            app.set_running(false);
                            break;
                        }

                        if state.modal_active() {
                            match state.handle_modal_key(key) {
                                ModalAction::Close => {
                                    state.close_modal();
                                    state.status_message = "Picker closed".to_string();
                                }
                                ModalAction::Confirm => {
                                    process_modal_confirm(&mut state, &mut app).await?;
                                }
                                ModalAction::DisconnectProvider => {
                                    process_modal_disconnect(&mut state, &mut app).await?;
                                }
                                ModalAction::None => {}
                            }
                            needs_redraw = true;
                            continue;
                        }

                        let should_quit = state.handle_key(key, &mut app);
                        if should_quit {
                            app.interrupt_controller_mut().interrupt(None);
                            abort_agent_lanes(&agent_lane, &mut active_managed_task).await;
                            app.set_running(false);
                            break;
                        }

                        let is_submit = is_submit_shortcut(&key, &state.phase.composer_mut().input);

                        if is_submit {
                            if state.try_accept_completion_on_enter() {
                                needs_redraw = true;
                                continue;
                            }
                            if state.phase.is_processing() {
                                let clarify_input =
                                    state.phase.composer_mut().input.trim().to_string();
                                let clarify_awaiting = state
                                    .phase
                                    .processing()
                                    .map(|p| p.clarify_awaiting)
                                    .unwrap_or(false);
                                if !clarify_input.is_empty()
                                    && active_managed_task.is_some()
                                    && (clarify_awaiting || app.clarify_awaiting_answer().await)
                                {
                                    state.phase.composer_mut().input.clear();
                                    state.phase.composer_mut().cursor_position = 0;
                                    state.phase.composer_mut().completions.clear();
                                    state.phase.composer_mut().completion_index = None;
                                    app.push_ui_user(clarify_input.clone());
                                    if app.try_fulfill_clarify(&clarify_input).await {
                                        if let Some(processing) = state.phase.processing_mut() {
                                            processing.clarify_awaiting = false;
                                        }
                                        state.status_message =
                                            "Clarify answer sent — agent continuing…".to_string();
                                        state.push_activity(format!(
                                            "clarify answer: {}",
                                            App::preview_for_status(&clarify_input, 80)
                                        ));
                                    } else {
                                        state.status_message =
                                            "Could not deliver clarify answer (timed out?)"
                                                .to_string();
                                    }
                                    needs_redraw = true;
                                    continue;
                                }
                                state.status_message =
                                    "Still processing previous request… wait for completion."
                                        .to_string();
                                needs_redraw = true;
                                continue;
                            }
                            let input = state.phase.composer_mut().input.clone();
                            state.phase.composer_mut().input.clear();
                            state.phase.composer_mut().cursor_position = 0;
                            state.phase.composer_mut().completions.clear();
                            state.phase.composer_mut().completion_index = None;
                            state.jump_to_latest();

                            if !input.is_empty() {
                                let mut handled_by_tui = false;
                                if let Some((cmd, args)) = parse_slash_parts(&input) {
                                    if cmd.eq_ignore_ascii_case("/ask")
                                        || cmd.eq_ignore_ascii_case("/question")
                                    {
                                        match parse_interactive_question_request(&input) {
                                            Ok(request) => {
                                                open_interactive_question_modal(
                                                    &mut state,
                                                    request,
                                                );
                                                state.status_message = "Interactive question ready. Choose an answer.".to_string();
                                            }
                                            Err(message) => {
                                                state.status_message = message.clone();
                                                app.push_ui_assistant(message);
                                            }
                                        }
                                        handled_by_tui = true;
                                    } else if cmd.eq_ignore_ascii_case("/model") {
                                        if args.is_empty() || (args.len() == 1 && args[0].eq_ignore_ascii_case("list")) {
                                            open_model_provider_modal(&mut state, &app).await;
                                            state.status_message = "Choose provider, then model".to_string();
                                            handled_by_tui = true;
                                        } else if args.len() == 1 {
                                            let providers = crate::model_switch::curated_provider_slugs();
                                            if providers.iter().any(|p| p.eq_ignore_ascii_case(&args[0])) {
                                                open_provider_model_modal(&mut state, &app, &args[0].to_ascii_lowercase()).await;
                                                state.status_message = format!("Choose {} model", args[0].to_ascii_lowercase());
                                                handled_by_tui = true;
                                            }
                                        }
                                    } else if cmd.eq_ignore_ascii_case("/personality")
                                        && (args.is_empty() || (args.len() == 1 && args[0].eq_ignore_ascii_case("list")))
                                    {
                                        open_personality_modal(&mut state, &app);
                                        state.status_message = "Choose personality".to_string();
                                        handled_by_tui = true;
                                    } else if (cmd.eq_ignore_ascii_case("/skin")
                                        || cmd.eq_ignore_ascii_case("/skins"))
                                        && (args.is_empty()
                                            || (args.len() == 1
                                                && (args[0].eq_ignore_ascii_case("list")
                                                    || args[0].eq_ignore_ascii_case("status")
                                                    || args[0].eq_ignore_ascii_case("show"))))
                                    {
                                        open_skin_modal(&mut state);
                                        state.status_message = "Choose skin".to_string();
                                        handled_by_tui = true;
                                    } else if cmd.eq_ignore_ascii_case("/toolcards")
                                        && args.first().is_some_and(|a| a.eq_ignore_ascii_case("export"))
                                    {
                                        let export_path = hermes_config::hermes_home().join("logs/toolcards-export.txt");
                                        let mut out = String::new();
                                        for msg in app.transcript_messages().iter().filter(|m| m.role == hermes_core::MessageRole::Tool) {
                                            if let Some(content) = msg.content.as_deref() {
                                                out.push_str(content);
                                                out.push_str("\n\n---\n\n");
                                            }
                                        }
                                        if let Err(err) = std::fs::write(&export_path, out) {
                                            state.status_message = format!("Export failed: {}", err);
                                        } else {
                                            state.status_message = format!("Exported tool cards to {}", export_path.display());
                                            app.push_ui_assistant(format!("Exported tool cards to `{}`.", export_path.display()));
                                        }
                                        handled_by_tui = true;
                                    }
                                }

                                if !handled_by_tui {
                                    let trimmed = input.trim().to_string();
                                    if trimmed.starts_with('/') {
                                        let command_name =
                                            trimmed.split_whitespace().next().unwrap_or("/");
                                        if command_name.eq_ignore_ascii_case("/quit")
                                            || command_name.eq_ignore_ascii_case("/exit")
                                        {
                                            app.push_ui_user(trimmed.clone());
                                            app.push_ui_assistant("Goodbye!");
                                            app.set_running(false);
                                            state.status_message.clear();
                                            state.phase.composer_mut().completions.clear();
                                            state.phase.composer_mut().completion_index = None;
                                            needs_redraw = true;
                                            continue;
                                        }
                                        if command_name.eq_ignore_ascii_case("/curator")
                                            && trimmed
                                                .split_whitespace()
                                                .nth(1)
                                                .is_some_and(|s| s.eq_ignore_ascii_case("run"))
                                        {
                                            // Curator run is long-running (LLM review can take
                                            // 30-120s). Spawn it as a background managed task so
                                            // the TUI event loop stays responsive.
                                            app.push_ui_user(trimmed.clone());
                                            state.begin_processing_cycle(app.current_model());
                                            state.mark_blocking_action("running curator");
                                            state.status_message =
                                                "Running curator (LLM review in background)…"
                                                    .to_string();
                                            draw_frame_now(&mut tui, &app, &mut state)?;

                                            let mut worker_app = app.clone();
                                            let stream_tx = tui.stream_sender();
                                            let input_for_task = trimmed.clone();
                                            let task = tokio::spawn(async move {
                                                let started = Instant::now();
                                                let result = worker_app
                                                    .handle_input(&input_for_task)
                                                    .await
                                                    .map(|_| Box::new(worker_app))
                                                    .map_err(|e| e.to_string());
                                                let _ = stream_tx.send(
                                                    Event::ManagedAppRunComplete {
                                                        result,
                                                        elapsed_secs: started.elapsed().as_secs_f64(),
                                                    },
                                                );
                                            });
                                            active_managed_task = Some(task);
                                            continue;
                                        }
                                        state.begin_processing_cycle(app.current_model());
                                        state.mark_blocking_action(format!(
                                            "running {command_name} command"
                                        ));
                                        state.status_message =
                                            format!("Running {command_name}…");
                                        draw_frame_now(&mut tui, &app, &mut state)?;
                                        match app.handle_input(&input).await {
                                            Ok(_) => {
                                                state.finish_processing_cycle("✔ completed in");
                                                if let Some(prefill) =
                                                    app.take_pending_input_prefill()
                                                {
                                                    state.phase.composer_mut().input = prefill;
                                                    state.phase.composer_mut().cursor_position =
                                                        state.phase.composer_mut().input.chars().count();
                                                    state.status_message =
                                                        "Prompt restored for editing. Press Enter to send."
                                                            .to_string();
                                                } else {
                                                    state.status_message.clear();
                                                }
                                            }
                                            Err(e) => {
                                                state.finish_processing_cycle("✖ failed after");
                                                state.status_message = format!("Error: {}", e);
                                                state.push_activity(format!("✖ {}", e));
                                                app.push_ui_assistant(format!("Error: {}", e));
                                            }
                                        }
                                    } else if !trimmed.is_empty() {
                                        let managed_turn_required =
                                            should_route_prompt_via_managed_agent(
                                                app.quorum_armed_once(),
                                                app.messages(),
                                            );
                                        if managed_turn_required {
                                            // Quorum/system-hint turns must run through App::run_agent
                                            // so fanout orchestration, artifact persistence, and arm/disarm
                                            // behavior remain correct. Run it on a cloned App so the
                                            // render loop can keep drawing live activity while the
                                            // worker mutates/persists the final session state.
                                            let mut worker_app = app.clone();
                                            app.push_ui_user(trimmed.clone());
                                            state.begin_processing_cycle(app.current_model());
                                            state.mark_blocking_action(
                                                "running managed quorum/system turn",
                                            );
                                            state.status_message =
                                                "Running managed agent turn…".to_string();
                                            draw_frame_now(&mut tui, &app, &mut state)?;
                                            let stream_tx = tui.stream_sender();
                                            let input_for_task = input.clone();
                                            let task = tokio::spawn(async move {
                                                let started = Instant::now();
                                                let result = worker_app
                                                    .handle_input(&input_for_task)
                                                    .await
                                                    .map(|_| Box::new(worker_app))
                                                    .map_err(|e| e.to_string());
                                                let _ = stream_tx.send(Event::ManagedAppRunComplete {
                                                    result,
                                                    elapsed_secs: started.elapsed().as_secs_f64(),
                                                });
                                            });
                                            active_managed_task = Some(task);
                                        } else {
                                            app.input_history_mut().push(trimmed.clone());
                                            *app.history_index_mut() = app.input_history().len();
                                            let user_message = app.prepare_user_message(&trimmed);
                                            app.messages_mut().push(Message::user(user_message));

                                            state.begin_processing_cycle(app.current_model());
                                            state.status_message = "Processing...".to_string();

                                            agent_lane.submit(StandardAgentRunRequest {
                                                agent: app.agent().clone(),
                                                messages: app.messages().to_vec(),
                                                stream_enabled: app.config().streaming.enabled,
                                                tool_schemas: app.tool_schemas().to_vec(),
                                                stream_handle: app.stream_handle().cloned(),
                                                session_id: app.session_id().to_string(),
                                                result_tx: tui.stream_sender(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        needs_redraw = true;
                    }
                    Some(Event::Resize(_, _)) => {
                        let _ = tui.terminal.autoresize();
                        state.transcript_cache = TranscriptCache::default();
                        state.reset_input_paint_cache();
                        if state.auto_follow_transcript {
                            state.scroll_offset = 0;
                        }
                        needs_redraw = true;
                    }
                    Some(Event::Mouse(mouse)) => {
                        if !app.mouse_enabled() {
                            continue;
                        }
                        use crossterm::event::MouseEventKind;
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                state.scroll_history_up(1);
                            }
                            MouseEventKind::ScrollDown => {
                                state.scroll_history_down(1);
                            }
                            _ => {}
                        }
                        needs_redraw = true;
                    }
                    Some(Event::Message(msg)) => {
                        state.status_message = msg;
                        needs_redraw = true;
                    }
                    Some(Event::AgentRunComplete {
                        result,
                        elapsed_secs,
                    }) => {
                        handle_agent_run_complete(
                            &mut app,
                            &mut state,
                            result,
                            elapsed_secs,
                        );
                        needs_redraw = true;
                    }
                    Some(Event::ManagedAppRunComplete {
                        result,
                        elapsed_secs,
                    }) => {
                        active_managed_task = None;
                        handle_managed_app_run_complete(
                            &mut app,
                            &mut state,
                            result,
                            elapsed_secs,
                        );
                        needs_redraw = true;
                    }
                    Some(Event::Interrupt) => {
                        abort_agent_lanes(&agent_lane, &mut active_managed_task).await;
                        state.finish_processing_cycle("⏹ interrupted after");
                        state.phase.processing_mut().expect("processing").stream_buffer.clear();
                        state.phase.processing_mut().expect("processing").stream_md_cache.clear();
                        state.phase.processing_mut().expect("processing").stream_muted = false;
                        state.phase.processing_mut().expect("processing").stream_needs_break = false;
                        state.phase.processing_mut().expect("processing").active_tools.clear();
                        app.set_running(false);
                        break;
                    }
                    Some(Event::Shutdown) => {
                        app.interrupt_controller_mut().interrupt(None);
                        abort_agent_lanes(&agent_lane, &mut active_managed_task).await;
                        state.finish_processing_cycle("⏹ interrupted after");
                        state.phase.processing_mut().expect("processing").stream_buffer.clear();
                        state.phase.processing_mut().expect("processing").stream_md_cache.clear();
                        state.phase.processing_mut().expect("processing").stream_muted = false;
                        state.phase.processing_mut().expect("processing").stream_needs_break = false;
                        state.phase.processing_mut().expect("processing").active_tools.clear();
                        app.set_running(false);
                        break;
                    }
                    Some(Event::StreamDelta(_)) | Some(Event::StreamChunk(_)) | Some(Event::AgentDone) => {
                        // Stream events are consumed on the dedicated stream lane.
                    }
                    None => {
                        // Channel closed
                        break;
                    }
                }
            }
            stream_event = tui.stream_events.recv() => {
                if let Some(first) = stream_event {
                    let mut task_completed =
                        stream_event_completes_background_task(&first);
                    let outcome = process_stream_lane_event(&mut state, first);
                    let mut redraw = apply_stream_lane_outcome(&mut app, &mut state, outcome);
                    let (drain_cap, drain_budget) = stream_lane_budget(
                        state.phase.is_processing(),
                        state.phase.processing_mut().map_or(0, |p| p.stream_chunk_count),
                    );
                    let drain_started = Instant::now();
                    for _ in 0..drain_cap {
                        match tui.stream_events.try_recv() {
                            Ok(next) => {
                                task_completed |= stream_event_completes_background_task(&next);
                                let outcome = process_stream_lane_event(&mut state, next);
                                redraw |= apply_stream_lane_outcome(&mut app, &mut state, outcome);
                                if drain_started.elapsed() >= drain_budget {
                                    break;
                                }
                            }
                            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                        }
                    }
                    if task_completed {
                        active_managed_task = None;
                    }
                    if redraw {
                        if should_redraw_stream_while_composing(
                            !state.phase.composer_mut().input.is_empty(),
                            &mut last_compose_stream_redraw,
                        ) {
                            needs_redraw = true;
                        }
                    }
                }
            }
            _ = frame_tick.tick() => {
                let previous_jobs = state.background_jobs_running;
                if last_jobs_refresh.elapsed() >= Duration::from_secs(1) {
                    state.background_jobs_running = app.running_background_job_count();
                    last_jobs_refresh = Instant::now();
                }
                if state.phase.is_processing() {
                    state.tick_spinner();
                    state.maybe_emit_progress_pulse();
                    // While the user is drafting in the composer, skip spinner-only
                    // redraws so stream/spinner ticks don't flash the input box.
                    if state.phase.composer_mut().input.is_empty() {
                        needs_redraw = true;
                    }
                }
                if app.pet_settings().enabled
                    && last_pet_tick.elapsed()
                        >= Duration::from_millis(app.pet_settings().tick_ms.clamp(120, 2000))
                {
                    state.tick_pet();
                    last_pet_tick = Instant::now();
                    needs_redraw = true;
                }
                if previous_jobs != state.background_jobs_running {
                    needs_redraw = true;
                }
            }
        }
    }

    app.interrupt_controller_mut().interrupt(None);
    abort_agent_lanes(&agent_lane, &mut active_managed_task).await;
    app.flush_session_teardown(false);
    shutdown_crossterm_event_pipeline(event_pipeline).await;
    shutdown_signal_bridge(signal_bridge).await;

    // Restore terminal
    tui.restore()
        .map_err(|e| AgentError::Config(e.to_string()))?;

    Ok(())
}
