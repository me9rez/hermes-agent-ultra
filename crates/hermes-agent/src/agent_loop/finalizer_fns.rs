use super::*;

/// Extract the last user and assistant content from a message slice for memory sync.
pub(crate) fn extract_last_user_assistant(messages: &[Message]) -> (String, String) {
    let user = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, hermes_core::MessageRole::User))
        .and_then(|m| m.content.clone())
        .unwrap_or_default();
    let assistant = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, hermes_core::MessageRole::Assistant))
        .and_then(|m| m.content.clone())
        .unwrap_or_default();
    (user, assistant)
}

pub(crate) fn latest_user_content(messages: &[Message]) -> Option<&str> {
    messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, hermes_core::MessageRole::User))
        .and_then(|m| m.content.as_deref())
}

pub(crate) fn session_search_has_query(tc: &ToolCall) -> bool {
    serde_json::from_str::<Value>(&tc.function.arguments)
        .ok()
        .and_then(|v| {
            v.get("query")
                .and_then(|q| q.as_str())
                .map(str::trim)
                .map(str::to_string)
        })
        .is_some_and(|q| !q.is_empty())
}

pub(crate) fn inject_runtime_tool_params(
    tool_name: &str,
    params: &mut Value,
    task_id: Option<&str>,
    user_task: Option<&str>,
) {
    if !params.is_object() {
        *params = serde_json::json!({});
    }
    let Some(obj) = params.as_object_mut() else {
        return;
    };

    if let Some(task_id) = task_id.filter(|v| !v.trim().is_empty()) {
        obj.entry("task_id".to_string())
            .or_insert_with(|| Value::String(task_id.to_string()));
    }
    if tool_name.starts_with("browser_") {
        if let Some(user_task) = user_task.filter(|v| !v.trim().is_empty()) {
            obj.entry("user_task".to_string())
                .or_insert_with(|| Value::String(user_task.to_string()));
        }
    }
}

pub(crate) fn objective_eval_score(state: &str) -> f64 {
    match state.trim().to_ascii_lowercase().as_str() {
        "advancing" => 1.0,
        "flat" => 0.5,
        "regressing" => 0.0,
        "unproven" => 0.25,
        _ => 0.4,
    }
}

fn claim_verifier_enabled_runtime() -> bool {
    if let Ok(raw) = std::env::var("HERMES_CLAIM_VERIFIER_ENABLED") {
        return !matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        );
    }
    let hermes_home = std::env::var("HERMES_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".hermes")))
        .unwrap_or_else(|| PathBuf::from(".hermes"));
    let path = hermes_home.join("alpha").join("claim_verifier_policy.json");
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return true,
    };
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return true,
    };
    parsed
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

pub(crate) fn finalizer_claim_requires_evidence_retry(
    messages: &[Message],
    assistant_text: &str,
    retry_count: u32,
) -> bool {
    if !claim_verifier_enabled_runtime() {
        return false;
    }
    if retry_count >= FINALIZER_EVIDENCE_MAX_RETRIES || !detect_repo_review_intent(messages) {
        return false;
    }
    let lower = assistant_text.to_ascii_lowercase();
    let claims_completion = [
        "completed",
        "implemented",
        "fixed",
        "done",
        "resolved",
        "ready",
        "finished",
        "shipped",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if !claims_completion {
        return false;
    }
    let has_evidence = lower.contains("file=")
        || lower.contains("path=")
        || lower.contains("cmd=")
        || lower.contains("exists_now=")
        || lower.contains("`/users/")
        || lower.contains("cargo test");
    let has_confidence = lower.contains("confidence=high")
        || lower.contains("confidence=medium")
        || lower.contains("confidence=low")
        || lower.contains("confidence:");
    !(has_evidence && has_confidence)
}

fn strip_list_prefix(line: &str) -> &str {
    let trimmed = line.trim();
    let without_bullet = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
        .unwrap_or(trimmed);
    let mut chars = without_bullet.char_indices();
    let mut end_idx = 0usize;
    while let Some((idx, ch)) = chars.next() {
        if ch.is_ascii_digit() {
            end_idx = idx + ch.len_utf8();
            continue;
        }
        if (ch == '.' || ch == ')') && end_idx > 0 {
            let tail = &without_bullet[idx + ch.len_utf8()..];
            return tail.trim_start();
        }
        break;
    }
    without_bullet
}

pub(crate) fn finalizer_output_quality_requires_retry(
    assistant_text: &str,
    retry_count: u32,
) -> bool {
    if retry_count >= FINALIZER_OUTPUT_QUALITY_MAX_RETRIES {
        return false;
    }
    let lower = assistant_text.to_ascii_lowercase();
    let placeholder_markers = [
        "[url](url)",
        "(url)",
        "[paper details](url)",
        "pack of authors",
        "lorem ipsum",
        "<insert",
        "<todo",
    ];
    if placeholder_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return true;
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut in_code_block = false;
    for raw_line in assistant_text.lines() {
        let line = raw_line.trim();
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }
        let normalized = strip_list_prefix(line).trim().to_ascii_lowercase();
        if normalized.len() < 24 {
            continue;
        }
        let entry = counts.entry(normalized).or_insert(0);
        *entry += 1;
        if *entry >= 3 {
            return true;
        }
    }
    false
}

fn assistant_response_has_execution_evidence(lower: &str) -> bool {
    [
        "file=",
        "path=",
        "cmd=",
        "exists_now=",
        "objective_state=",
        "error:",
        "blocked:",
        "blocker:",
        "run finished",
        "tested",
        "verified",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn detect_execution_required_intent(messages: &[Message]) -> bool {
    let user = latest_user_content(messages)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if user.trim().is_empty() {
        return false;
    }
    let objective = extract_session_objective(messages)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let combined = format!("{} {}", user, objective);
    let action_terms = [
        "proceed",
        "implement",
        "fix",
        "debug",
        "diagnose",
        "run",
        "test",
        "patch",
        "sync",
        "rebuild",
        "verify",
        "connect",
        "integrat",
        "investigate",
        "analyze",
        "review",
    ];
    let has_action = action_terms.iter().any(|needle| combined.contains(needle));
    let has_surface = combined.contains("repo")
        || combined.contains("repository")
        || combined.contains("codebase")
        || combined.contains("contextlattice")
        || combined.contains('/')
        || combined.contains(".rs")
        || combined.contains(".py")
        || combined.contains("session");
    has_action && has_surface
}

pub(crate) fn finalizer_action_execution_requires_retry(
    messages: &[Message],
    assistant_text: &str,
    retry_count: u32,
) -> bool {
    if retry_count >= FINALIZER_ACTION_EXECUTION_MAX_RETRIES {
        return false;
    }
    if !detect_execution_required_intent(messages) {
        return false;
    }
    let lower = assistant_text.to_ascii_lowercase();
    if assistant_response_has_execution_evidence(&lower) {
        return false;
    }
    let deferral_markers = [
        "i will",
        "i'll",
        "let me",
        "i can",
        "i'm going to",
        "proceeding",
        "next i",
        "i should",
        "i would",
    ];
    deferral_markers.iter().any(|needle| lower.contains(needle))
}

pub(crate) fn detect_contextlattice_connect_intent(messages: &[Message]) -> bool {
    let Some(last_user) = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, hermes_core::MessageRole::User))
        .and_then(|m| m.content.as_deref())
    else {
        return false;
    };
    let lower = last_user.to_ascii_lowercase();
    if !lower.contains("contextlattice") {
        return false;
    }
    [
        "connect",
        "connection",
        "configure",
        "setup",
        "set up",
        "verify",
        "harden",
        "probe",
        "integrat",
        "health",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(crate) fn contextlattice_connect_system_hint(messages: &[Message]) -> Option<String> {
    if !detect_contextlattice_connect_intent(messages) {
        return None;
    }
    Some(
        "[SYSTEM] ContextLattice integration intent detected. Execute this order: \
         (1) If available, inspect local instructions file from `HERMES_CONTEXTLATTICE_INSTRUCTIONS_PATH` \
         or workspace `scripts/agent_orchestration.py` (preferred path: `/Users/sheawinkler/Documents/Projects/scripts/agent_orchestration.py`); \
         (2) call `contextlattice_search` for a direct connectivity probe; \
         (3) if needed call `contextlattice_context_pack` for broader grounding; \
         (4) call `contextlattice_write` to checkpoint what was verified. \
         Never use terminal command `contextlattice` for this workflow."
            .to_string(),
    )
}

pub(crate) fn contextlattice_intelligence_system_hint(
    messages: &[Message],
    tool_schemas: &[ToolSchema],
) -> Option<String> {
    let has_context_tools = tool_schemas.iter().any(|t| {
        matches!(
            t.name.as_str(),
            "contextlattice_search"
                | "contextlattice_context_pack"
                | "contextlattice_write"
                | "memory"
        )
    });
    if !has_context_tools {
        return None;
    }

    let objective_active = objective_guard_policy(messages).0;
    let repo_intent = detect_repo_review_intent(messages);
    let connect_intent = detect_contextlattice_connect_intent(messages);
    if !(objective_active || repo_intent || connect_intent) {
        return None;
    }

    Some(
        "[SYSTEM] ContextLattice-first intelligence policy active.\n\
         1) Start with scoped retrieval (`contextlattice_search`) using project + topic path.\n\
         2) If scoped retrieval is empty/degraded, run one broader retrieval in the same project and compare.\n\
         3) For broad or multi-file tasks, run `contextlattice_context_pack` before deep tool loops.\n\
         4) During long execution, checkpoint durable progress with `contextlattice_write`.\n\
         5) Before final answer, run one scoped readback and report contradictions as `unproven` rather than guessing.\n\
         6) Copy numeric facts verbatim; do not normalize or round unless explicitly requested."
            .to_string(),
    )
}

pub(crate) fn is_contextlattice_shell_invocation(raw_args: &str) -> bool {
    let Ok(args) = serde_json::from_str::<Value>(raw_args) else {
        return false;
    };
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or_default();
    let lower = command.to_ascii_lowercase();
    lower == "contextlattice" || lower.starts_with("contextlattice ")
}

pub(crate) fn summarize_background_review_result(messages: &[Message]) -> Option<String> {
    crate::evolution_ledger::summarize_review_for_chat(messages)
}
