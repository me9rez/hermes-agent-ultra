use std::time::{Duration, Instant};

use chrono::Local;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::super::state::TuiState;
use super::super::text::{hard_wrap_segments, transcript_divider, truncate_chars};
use super::super::transcript_cache::{TranscriptCache, expanded_tool_cards_signature};
use super::super::types::{StreamMarkdownCache, ViewDensity};
use crate::tool_preview::{build_tool_preview_from_value, tool_emoji};
const TRANSCRIPT_HARD_WRAP_COLS: u16 = 80;
const TRANSCRIPT_CONTENT_WRAP_COLS: usize = 76;
const OFFSET_ANCHOR_SEARCH_RADIUS: usize = 1200;
const DEFAULT_MAX_ASSISTANT_RENDER_LINES: usize = 260;
const MAX_STREAM_RENDER_LINES: usize = 140;
const DEFAULT_TOOL_OUTPUT_MAX_LINES: usize = 180;
const DEFAULT_TOOL_OUTPUT_MAX_LINE_CHARS: usize = 600;
const DEFAULT_TOOL_OUTPUT_MAX_TOTAL_CHARS: usize = 48_000;

pub(crate) fn env_usize_with_bounds(key: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

pub(crate) fn max_assistant_render_lines() -> usize {
    env_usize_with_bounds(
        "HERMES_TUI_MAX_ASSISTANT_RENDER_LINES",
        DEFAULT_MAX_ASSISTANT_RENDER_LINES,
        40,
        4000,
    )
}

pub(crate) fn max_tool_output_lines() -> usize {
    env_usize_with_bounds(
        "HERMES_TUI_MAX_TOOL_OUTPUT_LINES",
        DEFAULT_TOOL_OUTPUT_MAX_LINES,
        20,
        5000,
    )
}

pub(crate) fn max_tool_output_line_chars() -> usize {
    env_usize_with_bounds(
        "HERMES_TUI_MAX_TOOL_OUTPUT_LINE_CHARS",
        DEFAULT_TOOL_OUTPUT_MAX_LINE_CHARS,
        120,
        4000,
    )
}

pub(crate) fn max_tool_output_total_chars() -> usize {
    env_usize_with_bounds(
        "HERMES_TUI_MAX_TOOL_OUTPUT_TOTAL_CHARS",
        DEFAULT_TOOL_OUTPUT_MAX_TOTAL_CHARS,
        2000,
        500_000,
    )
}

pub(crate) fn transcript_wrap_width(viewport_width: u16) -> u16 {
    viewport_width.min(TRANSCRIPT_HARD_WRAP_COLS).max(1)
}

/// Throttle stream-driven full-frame redraws while the user is drafting.
pub(crate) fn should_redraw_stream_while_composing(
    composing: bool,
    last_compose_stream_redraw: &mut Instant,
) -> bool {
    const COMPOSE_STREAM_REDRAW_INTERVAL: Duration = Duration::from_millis(120);

    if !composing {
        return true;
    }
    if last_compose_stream_redraw.elapsed() >= COMPOSE_STREAM_REDRAW_INTERVAL {
        *last_compose_stream_redraw = Instant::now();
        true
    } else {
        false
    }
}

pub(crate) fn stream_lane_budget(processing: bool, chunk_count: usize) -> (usize, Duration) {
    let profile = std::env::var("HERMES_PERF_AUTOPILOT_PROFILE")
        .ok()
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| "balanced".to_string());
    let mode = std::env::var("HERMES_PERF_AUTOPILOT_MODE")
        .ok()
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| "advisory".to_string());

    stream_lane_budget_from(mode.as_str(), profile.as_str(), processing, chunk_count)
}

pub(crate) fn stream_lane_budget_from(
    mode: &str,
    profile: &str,
    processing: bool,
    chunk_count: usize,
) -> (usize, Duration) {
    if mode == "off" {
        return (96, Duration::from_millis(6));
    }

    let mut cap = 96usize;
    let mut budget_ms = 6u64;

    match profile {
        "throughput" => {
            cap = 320;
            budget_ms = 16;
        }
        "quality" => {
            cap = 120;
            budget_ms = 8;
        }
        "reliability" => {
            cap = 192;
            budget_ms = 12;
        }
        "safety" => {
            cap = 96;
            budget_ms = 8;
        }
        _ => {}
    }

    if processing && chunk_count > 40 {
        cap = cap.max(224);
        budget_ms = budget_ms.max(12);
    }

    (cap, Duration::from_millis(budget_ms))
}

pub(crate) fn find_anchor_line_index(
    lines: &[Line<'static>],
    anchor_text: &str,
    expected_index: usize,
) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let len = lines.len();
    let center = expected_index.min(len.saturating_sub(1));
    let radius = OFFSET_ANCHOR_SEARCH_RADIUS.min(len.saturating_sub(1));
    let start = center.saturating_sub(radius);
    let end = (center + radius).min(len.saturating_sub(1));

    for (idx, line) in lines.iter().enumerate().take(end + 1).skip(start) {
        if line.to_string() == anchor_text {
            return Some(idx);
        }
    }
    lines
        .iter()
        .position(|line| line.to_string() == anchor_text)
}

pub(crate) fn role_visuals(
    role: hermes_core::MessageRole,
    styles: &crate::theme::ResolvedStyles,
    colors: &crate::theme::RatatuiColors,
) -> (&'static str, &'static str, Style, Style) {
    let role_bg = colors.background;
    match role {
        hermes_core::MessageRole::User => (
            "◆",
            "USER",
            styles.user_input.bg(role_bg),
            styles
                .user_input
                .remove_modifier(Modifier::BOLD)
                .bg(role_bg),
        ),
        hermes_core::MessageRole::Assistant => (
            "●",
            "HERMES",
            styles.assistant_response.bg(role_bg),
            styles.assistant_response.bg(role_bg),
        ),
        hermes_core::MessageRole::System => (
            "◇",
            "SYSTEM",
            styles.system_message.bg(role_bg),
            styles.system_message.bg(role_bg),
        ),
        hermes_core::MessageRole::Tool => (
            "◈",
            "TOOL",
            styles.tool_call.bg(role_bg),
            Style::default().fg(colors.status_bar_text).bg(role_bg),
        ),
    }
}

pub(crate) fn render_inline_with_code(
    prefix: &str,
    text: &str,
    base_style: Style,
    code_style: Style,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix.to_string(), base_style));
    }

    let mut in_code = false;
    let mut current = String::new();
    for ch in text.chars() {
        if ch == '`' {
            if !current.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut current),
                    if in_code { code_style } else { base_style },
                ));
            }
            in_code = !in_code;
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        spans.push(Span::styled(
            current,
            if in_code { code_style } else { base_style },
        ));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base_style));
    }
    Line::from(spans)
}

pub(crate) fn parse_markdown_numbered_marker(line: &str) -> Option<(&str, &str)> {
    let digits = line
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    if let Some(tail) = rest.strip_prefix(". ") {
        return Some((&line[..digits + 1], tail));
    }
    if let Some(tail) = rest.strip_prefix(") ") {
        return Some((&line[..digits + 1], tail));
    }
    None
}

pub(crate) fn keyword_set_for_lang(lang: &str) -> &'static [&'static str] {
    match lang.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" => &[
            "fn", "let", "mut", "pub", "impl", "struct", "enum", "match", "if", "else", "for",
            "while", "loop", "return", "async", "await", "use", "mod", "trait", "where",
        ],
        "python" | "py" => &[
            "def", "class", "if", "elif", "else", "for", "while", "return", "import", "from",
            "with", "as", "try", "except", "finally", "lambda", "yield", "async", "await",
        ],
        "javascript" | "js" | "typescript" | "ts" => &[
            "function", "const", "let", "var", "if", "else", "for", "while", "return", "class",
            "import", "export", "await", "async", "switch", "case", "break", "new",
        ],
        "json" => &[],
        "bash" | "sh" | "zsh" => &[
            "if", "then", "else", "fi", "for", "do", "done", "case", "esac", "function", "echo",
            "export",
        ],
        _ => &[],
    }
}

pub(crate) fn render_highlighted_code_line(
    line: &str,
    lang: &str,
    colors: &crate::theme::RatatuiColors,
) -> Line<'static> {
    let default_style = Style::default()
        .fg(colors.status_bar_text)
        .bg(colors.background);
    let keyword_style = Style::default()
        .fg(colors.accent)
        .bg(colors.background)
        .add_modifier(Modifier::BOLD);
    let string_style = Style::default()
        .fg(colors.status_bar_warn)
        .bg(colors.background);
    let number_style = Style::default()
        .fg(colors.status_bar_good)
        .bg(colors.background);
    let punctuation_style = Style::default()
        .fg(colors.status_bar_dim)
        .bg(colors.background);
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        "    │ ",
        Style::default()
            .fg(colors.status_bar_dim)
            .bg(colors.background),
    )];
    let keywords = keyword_set_for_lang(lang);
    let mut token = String::new();
    let mut in_string = false;
    let mut quote_char = '\0';
    let flush_token =
        |spans: &mut Vec<Span<'static>>, token: &mut String, style: Style, keywords: &[&str]| {
            if token.is_empty() {
                return;
            }
            let tok = std::mem::take(token);
            let tok_style = if keywords.iter().any(|kw| kw.eq_ignore_ascii_case(&tok)) {
                style
            } else if tok.chars().all(|ch| ch.is_ascii_digit()) {
                Style::default()
                    .fg(Color::Cyan)
                    .bg(style.bg.unwrap_or(Color::Reset))
            } else {
                default_style
            };
            spans.push(Span::styled(tok, tok_style));
        };

    for ch in line.chars() {
        if in_string {
            token.push(ch);
            if ch == quote_char {
                spans.push(Span::styled(std::mem::take(&mut token), string_style));
                in_string = false;
                quote_char = '\0';
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            flush_token(&mut spans, &mut token, keyword_style, keywords);
            in_string = true;
            quote_char = ch;
            token.push(ch);
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            token.push(ch);
            continue;
        }
        flush_token(&mut spans, &mut token, keyword_style, keywords);
        if ch.is_ascii_digit() {
            spans.push(Span::styled(ch.to_string(), number_style));
        } else if ch.is_whitespace() {
            spans.push(Span::styled(ch.to_string(), default_style));
        } else {
            spans.push(Span::styled(ch.to_string(), punctuation_style));
        }
    }
    flush_token(&mut spans, &mut token, keyword_style, keywords);
    if in_string && !token.is_empty() {
        spans.push(Span::styled(token, string_style));
    }
    Line::from(spans)
}

pub(crate) fn parse_table_cells(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let cells: Vec<String> = trimmed
        .split('|')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .map(ToString::to_string)
        .collect();
    if cells.len() < 2 {
        return None;
    }
    Some(cells)
}

pub(crate) fn content_width_for_table_row(cells: usize, min_per_cell: usize) -> usize {
    cells.saturating_mul(min_per_cell).max(8)
}

pub(crate) fn message_fingerprint(msg: &hermes_core::Message) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    let role_tag = match msg.role {
        hermes_core::MessageRole::System => "system",
        hermes_core::MessageRole::User => "user",
        hermes_core::MessageRole::Assistant => "assistant",
        hermes_core::MessageRole::Tool => "tool",
    };
    role_tag.hash(&mut hasher);
    msg.content.hash(&mut hasher);
    msg.tool_call_id.hash(&mut hasher);
    msg.reasoning_content.hash(&mut hasher);
    if let Some(calls) = msg.tool_calls.as_ref() {
        for tc in calls {
            tc.id.hash(&mut hasher);
            tc.function.name.hash(&mut hasher);
            tc.function.arguments.hash(&mut hasher);
        }
    }
    hasher.finish()
}

pub(crate) fn transcript_fingerprint(
    messages: &[hermes_core::Message],
    state: &TuiState,
    width: u16,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    width.hash(&mut hasher);
    let stream_buffer = state
        .phase
        .processing()
        .map(|processing| processing.stream_buffer.as_str())
        .unwrap_or("");
    stream_buffer.hash(&mut hasher);
    state.show_timestamps.hash(&mut hasher);
    state.view_density.hash(&mut hasher);
    let mut expanded = state.expanded_tool_cards.iter().collect::<Vec<_>>();
    expanded.sort();
    for key in expanded {
        key.hash(&mut hasher);
    }
    for msg in messages {
        message_fingerprint(msg).hash(&mut hasher);
    }
    hasher.finish()
}

pub(crate) fn transcript_message_fingerprints(messages: &[hermes_core::Message]) -> Vec<u64> {
    messages.iter().map(message_fingerprint).collect()
}

#[cfg(test)]
pub(crate) fn count_renderable_messages(messages: &[hermes_core::Message]) -> usize {
    messages
        .iter()
        .filter(|msg| !matches!(msg.role, hermes_core::MessageRole::System))
        .count()
}

pub(crate) fn count_renderable_messages_before(
    messages: &[hermes_core::Message],
    end_index: usize,
) -> usize {
    messages
        .iter()
        .take(end_index)
        .filter(|msg| !matches!(msg.role, hermes_core::MessageRole::System))
        .count()
}

pub(crate) fn streaming_transcript_active(state: &TuiState) -> bool {
    state
        .phase
        .processing()
        .is_some_and(|processing| !processing.stream_buffer.is_empty())
}

pub(crate) fn finalize_transcript_cache(
    fingerprint: u64,
    wrap_width: u16,
    lines: Vec<Line<'static>>,
    message_line_ends: Vec<usize>,
    messages_only_len: usize,
    rendered_messages: usize,
    message_fingerprints: Vec<u64>,
    transcript_len: usize,
    state: &TuiState,
    streaming_active: bool,
) -> TranscriptCache {
    TranscriptCache {
        fingerprint,
        width: wrap_width,
        visual_rows: approximate_visual_rows(&lines, wrap_width),
        total_messages: transcript_len,
        rendered_messages,
        message_fingerprints,
        message_line_ends,
        messages_only_len,
        show_timestamps: state.show_timestamps,
        view_density: state.view_density,
        had_streaming: streaming_active,
        expanded_tool_cards_sig: expanded_tool_cards_signature(&state.expanded_tool_cards),
        lines,
    }
}

pub(crate) fn looks_like_internal_scaffold_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let lowered = trimmed.to_ascii_lowercase();
    trimmed.starts_with("to=functions.")
        || trimmed.starts_with("to=tools.")
        || trimmed.starts_with("to=memory.")
        || trimmed.starts_with("->functions.")
        || trimmed.contains(" to=functions.")
        || trimmed.starts_with("<tool_call")
        || trimmed.starts_with("</tool_call")
        || trimmed.starts_with("<tool_use")
        || trimmed.starts_with("</tool_use")
        || trimmed.starts_with("<name>")
        || trimmed.starts_with("</name>")
        || trimmed.starts_with("<arguments>")
        || trimmed.starts_with("</arguments>")
        || trimmed.starts_with("<assistant(")
        || trimmed.starts_with("</assistant(")
        || trimmed.contains("(INVOKN_RESULT")
        || lowered.contains("<tool_use>")
        || lowered.contains("</tool_use>")
        || lowered.contains("<tool_call")
        || lowered.contains("</tool_call")
        || lowered.contains("<arguments>")
        || lowered.contains("</arguments>")
        || lowered.contains("<name>")
        || lowered.contains("</name>")
        || lowered.contains("<argument name=")
        || lowered.contains("</argument>")
        || lowered.contains("&lt;tool_use")
        || lowered.contains("&lt;/tool_use")
        || lowered.contains("&lt;tool_call")
        || lowered.contains("&lt;/tool_call")
        || lowered.contains("\\u003ctool_use")
        || lowered.contains("\\u003c/tool_use")
        || lowered.contains("\\u003ctool_call")
        || lowered.contains("\\u003c/tool_call")
        || lowered.contains("invoke_result")
        || lowered.contains("invokn_result")
        || lowered.contains("to=functions.")
        || lowered.contains("to=tools.")
        || lowered.contains("to=memory.")
}

/// Strip C0 controls except tab, LF, CR (Python `CONTROL_RE` parity).
pub(crate) fn strip_control_chars(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect()
}

pub(crate) fn line_has_ansi_escape(line: &str) -> bool {
    line.as_bytes().windows(2).any(|w| w == b"\x1b[")
}

const REASONING_TAGS: &[&str] = &[
    "think",
    "reasoning",
    "thinking",
    "thought",
    "REASONING_SCRATCHPAD",
    "redacted_thinking",
    "reflection",
];

pub(crate) fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let needle_bytes = needle.as_bytes();
    let hay_bytes = haystack.as_bytes();
    if hay_bytes.len() < needle_bytes.len() {
        return None;
    }
    for i in 0..=hay_bytes.len() - needle_bytes.len() {
        if hay_bytes[i..i + needle_bytes.len()]
            .iter()
            .zip(needle_bytes.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return Some(i);
        }
    }
    None
}

/// Split reasoning blocks out of assistant markdown (Python `splitReasoning` parity).
pub(crate) fn split_reasoning_from_content(input: &str) -> (String, String) {
    let mut text = input.to_string();
    let mut reasoning: Vec<String> = Vec::new();

    for tag in REASONING_TAGS {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");

        loop {
            let Some(start) = find_ascii_case_insensitive(&text, &open) else {
                break;
            };
            let body_start = start + open.len();
            if let Some(close_rel) = find_ascii_case_insensitive(&text[body_start..], &close) {
                let body_end = body_start + close_rel;
                let inner = text[body_start..body_end].trim();
                if !inner.is_empty() {
                    reasoning.push(inner.to_string());
                }
                let after = body_end + close.len();
                text = format!("{}{}", &text[..start], &text[after..]);
                continue;
            }
            let inner = text[body_start..].trim();
            if !inner.is_empty() {
                reasoning.push(inner.to_string());
            }
            text = text[..start].to_string();
            break;
        }
    }

    (text.trim().to_string(), reasoning.join("\n\n"))
}

pub(crate) fn line_toggles_code_fence(line: &str) -> bool {
    let trimmed = line.trim();
    let bytes = trimmed.as_bytes();
    (bytes.len() >= 3 && bytes.iter().take(3).all(|b| *b == b'`'))
        || (bytes.len() >= 3 && bytes.iter().take(3).all(|b| *b == b'~'))
}

/// True when `end` falls inside an open fenced code or display-math block (Python `fenceOpenAt`).
pub(crate) fn fence_open_at(text: &str, end: usize) -> bool {
    let end = end.min(text.len());
    let mut code_open = false;
    let mut math_open = false;
    let mut math_opener: Option<char> = None;
    let mut i = 0usize;

    while i < end {
        let line_end = text[i..end].find('\n').map(|off| i + off).unwrap_or(end);
        let line = text[i..line_end].trim();

        if line_toggles_code_fence(line) {
            code_open = !code_open;
        } else if !code_open {
            if !math_open && line.starts_with("$$") {
                let is_single_line = line.len() >= 4 && line.ends_with("$$");
                if !is_single_line {
                    math_open = true;
                    math_opener = Some('$');
                }
            } else if !math_open && line.starts_with("\\[") {
                let is_single_line = line.ends_with("\\]");
                if !is_single_line {
                    math_open = true;
                    math_opener = Some('[');
                }
            } else if math_open && math_opener == Some('$') && line.ends_with("$$") {
                math_open = false;
                math_opener = None;
            } else if math_open && math_opener == Some('[') && line.ends_with("\\]") {
                math_open = false;
                math_opener = None;
            }
        }

        if line_end >= end {
            break;
        }
        i = line_end + 1;
    }

    code_open || math_open
}

/// Last safe `\n\n` split index (start of next block), outside open fences (Python `findStableBoundary`).
pub(crate) fn find_stable_boundary(text: &str) -> Option<usize> {
    let mut idx = text.len();
    while idx > 0 {
        let boundary = text[..idx].rfind("\n\n")?;
        let split_at = boundary + 2;
        if !fence_open_at(text, split_at) {
            return Some(split_at);
        }
        if boundary == 0 {
            break;
        }
        idx = boundary;
    }
    None
}

/// Render streaming assistant markdown with monotonic stable-prefix cache (Python `StreamingMd`).
pub(crate) fn render_streaming_assistant_markdown_lines(
    cache: &mut StreamMarkdownCache,
    text: &str,
    styles: &crate::theme::ResolvedStyles,
    colors: &crate::theme::RatatuiColors,
    width: u16,
) -> Vec<Line<'static>> {
    if !text.starts_with(&cache.stable_prefix) {
        cache.clear();
    }

    if cache.cached_width != width {
        cache.cached_width = width;
        if cache.stable_prefix.is_empty() {
            cache.stable_lines.clear();
        } else {
            cache.stable_lines =
                render_assistant_markdown_lines(&cache.stable_prefix, styles, colors);
        }
    }

    if let Some(boundary) = find_stable_boundary(text) {
        if boundary > cache.stable_prefix.len() {
            cache.stable_prefix = text[..boundary].to_string();
            cache.stable_lines =
                render_assistant_markdown_lines(&cache.stable_prefix, styles, colors);
        }
    }

    let suffix = &text[cache.stable_prefix.len()..];
    if suffix.is_empty() {
        return cache.stable_lines.clone();
    }

    let mut out = cache.stable_lines.clone();
    out.extend(render_assistant_markdown_lines(suffix, styles, colors));
    out
}

pub(crate) fn tool_complete_looks_failed(extra: &serde_json::Value, result_preview: &str) -> bool {
    if extra
        .get("error")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
    {
        return true;
    }
    if extra.get("failed").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    if extra.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    let preview = result_preview.trim();
    preview.starts_with("Error")
        || preview.contains("Tool execution failed")
        || preview.contains("timed out after")
}

pub(crate) fn render_assistant_markdown_lines(
    content: &str,
    styles: &crate::theme::ResolvedStyles,
    colors: &crate::theme::RatatuiColors,
) -> Vec<Line<'static>> {
    let mut rendered: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut hidden_scaffold_lines = 0usize;
    let code_frame_style = Style::default()
        .fg(colors.status_bar_dim)
        .bg(colors.background);
    let heading_style = Style::default()
        .fg(colors.status_bar_strong)
        .bg(colors.background)
        .add_modifier(Modifier::BOLD);
    let bullet_style = Style::default()
        .fg(colors.accent)
        .bg(colors.background)
        .add_modifier(Modifier::BOLD);
    let quote_style = Style::default()
        .fg(colors.status_bar_dim)
        .bg(colors.background)
        .add_modifier(Modifier::ITALIC);
    let inline_code_style = Style::default()
        .fg(colors.accent)
        .bg(colors.background)
        .add_modifier(Modifier::BOLD);
    let reasoning_style = Style::default()
        .fg(colors.status_bar_dim)
        .bg(colors.background)
        .add_modifier(Modifier::ITALIC);

    let (main_content, reasoning_text) = split_reasoning_from_content(content);
    if !reasoning_text.is_empty() {
        rendered.push(Line::from(vec![Span::styled(
            "    🤔 reasoning",
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.background)
                .add_modifier(Modifier::BOLD),
        )]));
        for line in reasoning_text.lines() {
            let cleaned = strip_control_chars(line);
            if cleaned.trim().is_empty() {
                continue;
            }
            rendered.push(Line::from(vec![Span::styled(
                format!("      {}", cleaned.trim_end()),
                reasoning_style,
            )]));
        }
        rendered.push(Line::from(String::new()));
    }

    for raw in main_content.lines() {
        if looks_like_internal_scaffold_line(raw) {
            hidden_scaffold_lines = hidden_scaffold_lines.saturating_add(1);
            continue;
        }
        let raw = strip_control_chars(raw);
        if raw.trim().is_empty() {
            continue;
        }
        if line_has_ansi_escape(&raw) {
            rendered.push(Line::from(vec![Span::styled(
                format!("    {raw}"),
                styles.assistant_response.bg(colors.background),
            )]));
            continue;
        }
        let trimmed = raw.trim_start();
        let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if is_fence {
            if in_code_block {
                rendered.push(Line::from(vec![Span::styled(
                    "    └─ end code",
                    code_frame_style,
                )]));
                in_code_block = false;
                code_lang.clear();
            } else {
                in_code_block = true;
                code_lang = trimmed
                    .trim_start_matches('`')
                    .trim_start_matches('~')
                    .trim()
                    .to_string();
                let label = if code_lang.is_empty() {
                    "    ┌─ code".to_string()
                } else {
                    format!("    ┌─ code ({})", code_lang)
                };
                rendered.push(Line::from(vec![Span::styled(label, code_frame_style)]));
            }
            continue;
        }

        if in_code_block {
            rendered.push(render_highlighted_code_line(&raw, &code_lang, colors));
            continue;
        }

        if trimmed.is_empty() {
            rendered.push(Line::from(String::new()));
            continue;
        }

        let heading_level = trimmed.chars().take_while(|ch| *ch == '#').count();
        if (1..=6).contains(&heading_level) {
            // Avoid byte-index slicing with a char-count offset on multibyte text.
            let rest = trimmed.trim_start_matches('#').trim_start();
            if !rest.is_empty() {
                rendered.push(Line::from(vec![
                    Span::styled(
                        format!("    {} ", "#".repeat(heading_level)),
                        Style::default()
                            .fg(colors.status_bar_dim)
                            .bg(colors.background),
                    ),
                    Span::styled(rest.to_string(), heading_style),
                ]));
                continue;
            }
        }

        if let Some(quote) = trimmed.strip_prefix('>').map(str::trim_start) {
            rendered.push(render_inline_with_code(
                "    ▎ ",
                quote,
                quote_style,
                inline_code_style,
            ));
            continue;
        }

        if let Some(body) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            rendered.push(Line::from(vec![
                Span::styled("    • ", bullet_style),
                Span::styled(
                    body.to_string(),
                    styles.assistant_response.bg(colors.background),
                ),
            ]));
            continue;
        }

        if let Some(cells) = parse_table_cells(trimmed) {
            let separator = cells
                .iter()
                .all(|cell| cell.chars().all(|ch| ch == '-' || ch == ':'));
            if separator {
                rendered.push(Line::from(vec![Span::styled(
                    format!(
                        "    ├{}┤",
                        "─".repeat(content_width_for_table_row(cells.len(), 16))
                    ),
                    Style::default()
                        .fg(colors.status_bar_dim)
                        .bg(colors.background),
                )]));
                continue;
            }
            let mut row_spans: Vec<Span<'static>> = vec![Span::styled(
                "    │ ",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            )];
            for (idx, cell) in cells.iter().enumerate() {
                if idx > 0 {
                    row_spans.push(Span::styled(
                        " │ ",
                        Style::default()
                            .fg(colors.status_bar_dim)
                            .bg(colors.background),
                    ));
                }
                row_spans.push(Span::styled(
                    truncate_chars(cell, 24),
                    Style::default()
                        .fg(colors.status_bar_text)
                        .bg(colors.background),
                ));
            }
            row_spans.push(Span::styled(
                " │",
                Style::default()
                    .fg(colors.status_bar_dim)
                    .bg(colors.background),
            ));
            rendered.push(Line::from(row_spans));
            continue;
        }

        if let Some((marker, body)) = parse_markdown_numbered_marker(trimmed) {
            rendered.push(Line::from(vec![
                Span::styled(format!("    {marker} "), bullet_style),
                Span::styled(
                    body.to_string(),
                    styles.assistant_response.bg(colors.background),
                ),
            ]));
            continue;
        }

        for segment in hard_wrap_segments(trimmed, TRANSCRIPT_CONTENT_WRAP_COLS) {
            rendered.push(render_inline_with_code(
                "    ",
                &segment,
                styles.assistant_response,
                inline_code_style,
            ));
        }
    }

    if in_code_block {
        rendered.push(Line::from(vec![Span::styled(
            "    └─ end code",
            code_frame_style,
        )]));
    }
    if hidden_scaffold_lines > 0 {
        rendered.push(Line::from(vec![Span::styled(
            format!(
                "    [internal orchestration scaffold hidden: {} lines]",
                hidden_scaffold_lines
            ),
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.background)
                .add_modifier(Modifier::ITALIC),
        )]));
    }
    rendered
}

pub(crate) fn collapse_render_lines_with_notice(
    lines: Vec<Line<'static>>,
    max_lines: usize,
    colors: &crate::theme::RatatuiColors,
) -> Vec<Line<'static>> {
    if lines.len() <= max_lines.max(1) {
        return lines;
    }
    let cap = max_lines.max(8);
    let mut out: Vec<Line<'static>> = Vec::with_capacity(cap + 2);
    let head = (cap * 2) / 3;
    let tail = cap.saturating_sub(head).saturating_sub(1);
    let total = lines.len();
    out.extend(lines.iter().take(head).cloned());
    out.push(Line::from(vec![Span::styled(
        format!(
            "    … transcript compressed for readability ({} lines hidden)",
            total.saturating_sub(head + tail)
        ),
        Style::default()
            .fg(colors.status_bar_dim)
            .bg(colors.background)
            .add_modifier(Modifier::ITALIC),
    )]));
    if tail > 0 {
        out.extend(lines.into_iter().skip(total.saturating_sub(tail)));
    }
    out
}

pub(crate) fn tail_render_lines_with_notice(
    lines: Vec<Line<'static>>,
    max_lines: usize,
    colors: &crate::theme::RatatuiColors,
) -> Vec<Line<'static>> {
    if lines.len() <= max_lines.max(1) {
        return lines;
    }
    let keep = max_lines.max(4);
    let total = lines.len();
    let mut out = Vec::with_capacity(keep + 1);
    out.push(Line::from(vec![Span::styled(
        format!(
            "    … live stream trimmed (showing last {} of {} lines)",
            keep, total
        ),
        Style::default()
            .fg(colors.status_bar_dim)
            .bg(colors.background)
            .add_modifier(Modifier::ITALIC),
    )]));
    out.extend(lines.into_iter().skip(total.saturating_sub(keep)));
    out
}

pub(crate) fn value_to_display_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    return serde_json::to_string_pretty(&parsed)
                        .unwrap_or_else(|_| raw.to_string());
                }
            }
            raw.to_string()
        }
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

pub(crate) fn push_block(lines: &mut Vec<String>, header: &str, value: &serde_json::Value) {
    let rendered = value_to_display_text(value);
    if rendered.trim().is_empty() {
        return;
    }
    lines.push(format!("[{header}]"));
    for line in rendered.lines() {
        lines.push(line.to_string());
    }
}

pub(crate) fn sanitize_tool_line(raw: &str) -> String {
    let sanitized = strip_control_chars(raw);
    truncate_chars(&sanitized, max_tool_output_line_chars())
}

pub(crate) fn finalize_tool_message_lines(raw_lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut total_chars = 0usize;
    let mut omitted = 0usize;
    let max_lines = max_tool_output_lines();
    let max_total_chars = max_tool_output_total_chars();
    for line in raw_lines {
        let sanitized = sanitize_tool_line(&line);
        let line_chars = sanitized.chars().count();
        let next_total = total_chars.saturating_add(line_chars);
        if out.len() < max_lines && next_total <= max_total_chars {
            total_chars = next_total;
            out.push(sanitized);
        } else {
            omitted = omitted.saturating_add(1);
        }
    }
    if omitted > 0 {
        out.push(format!(
            "… tool output truncated ({} lines omitted)",
            omitted
        ));
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

pub(crate) fn format_tool_message_lines(content: &str) -> Vec<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return vec![String::new()];
    }

    let parsed = match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(v) => v,
        Err(_) => {
            return finalize_tool_message_lines(
                content
                    .lines()
                    .map(std::string::ToString::to_string)
                    .collect(),
            );
        }
    };

    if let Some(obj) = parsed.as_object() {
        let mut lines: Vec<String> = Vec::new();

        if let Some(w) = obj.get("_budget_warning").and_then(|v| v.as_str()) {
            lines.push(format!("⚠ {}", w.trim()));
        }

        for key in ["result", "error", "stdout", "stderr", "message"] {
            if let Some(value) = obj.get(key) {
                push_block(&mut lines, key, value);
            }
        }
        if let Some(remediation) = tool_policy_remediation_from_payload(obj) {
            lines.push("[remediation]".to_string());
            for row in remediation {
                lines.push(format!("- {}", row));
            }
        }

        let mut extras = serde_json::Map::new();
        for (k, v) in obj.iter() {
            if k == "_budget_warning"
                || k == "result"
                || k == "error"
                || k == "stdout"
                || k == "stderr"
                || k == "message"
            {
                continue;
            }
            extras.insert(k.clone(), v.clone());
        }
        if !extras.is_empty() {
            push_block(&mut lines, "meta", &serde_json::Value::Object(extras));
        }
        if !lines.is_empty() {
            return finalize_tool_message_lines(lines);
        }
    }

    finalize_tool_message_lines(
        serde_json::to_string_pretty(&parsed)
            .map(|s| s.lines().map(std::string::ToString::to_string).collect())
            .unwrap_or_else(|_| {
                content
                    .lines()
                    .map(std::string::ToString::to_string)
                    .collect()
            }),
    )
}

pub(crate) fn tool_policy_remediation_from_payload(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<Vec<String>> {
    let code = obj
        .get("policy")
        .and_then(|p| p.get("code"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let error_text = obj
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let blocked = error_text.contains("blocked by tool policy")
        || error_text.contains("denied by security policy")
        || !code.is_empty();
    if !blocked {
        return None;
    }

    let mut rows = Vec::new();
    match code.as_str() {
        "params_pattern_denied" => {
            rows.push(
                "Remove secret-like parameter names from tool args; pass secrets via local env/vault.".to_string(),
            );
            rows.push(
                "Retry with sanitized args that reference variable names, not credential material."
                    .to_string(),
            );
        }
        "params_too_large" => {
            rows.push(
                "Reduce payload size and pass only minimal fields required by the tool."
                    .to_string(),
            );
        }
        "tool_denylisted" | "tool_not_allowlisted" => {
            rows.push(
                "Switch to an approved tool surface (`/tools`) for this operation.".to_string(),
            );
        }
        "sandbox_profile_violation" => {
            rows.push(
                "Command matched sandbox denial pattern; use a safer equivalent command path."
                    .to_string(),
            );
            rows.push(
                "If necessary, change runtime sandbox policy explicitly before retrying."
                    .to_string(),
            );
        }
        _ => {
            rows.push(
                "Review policy decision details in `/ops status` and retry with safer parameters."
                    .to_string(),
            );
        }
    }
    Some(rows)
}

pub(crate) fn append_transcript_message_lines(
    lines: &mut Vec<Line<'static>>,
    msg: &hermes_core::Message,
    msg_idx: usize,
    rendered_messages: &mut usize,
    state: &mut TuiState,
    styles: &crate::theme::ResolvedStyles,
    colors: &crate::theme::RatatuiColors,
    divider: &str,
) {
    // Hide internal orchestration/system payloads from the chat transcript.
    if matches!(msg.role, hermes_core::MessageRole::System) {
        return;
    }
    if *rendered_messages > 0 && matches!(state.view_density, ViewDensity::Detailed) {
        lines.push(Line::from(String::new()));
    }
    *rendered_messages += 1;
    let (glyph, label, label_style, body_style) = role_visuals(msg.role, styles, colors);
    let stamp = if state.show_timestamps {
        let fp = message_fingerprint(msg);
        state
            .message_time_labels
            .entry(fp)
            .or_insert_with(|| Local::now().format("%H:%M:%S").to_string())
            .clone()
    } else {
        String::new()
    };
    let label_text = if stamp.is_empty() {
        label.to_string()
    } else {
        format!("{label}  {stamp}")
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!(" ╭ {} ", glyph),
            label_style.add_modifier(Modifier::BOLD),
        ),
        Span::styled(label_text, label_style.add_modifier(Modifier::BOLD)),
    ]));

    if let Some(content) = msg.content.as_deref() {
        match msg.role {
            hermes_core::MessageRole::Assistant => {
                let assistant_lines = render_assistant_markdown_lines(content, styles, colors);
                lines.extend(collapse_render_lines_with_notice(
                    assistant_lines,
                    max_assistant_render_lines(),
                    colors,
                ));
            }
            hermes_core::MessageRole::Tool => {
                let card_key = format!("tool:{msg_idx}");
                let expanded = state.expanded_tool_cards.contains(&card_key)
                    || state.expanded_tool_cards.contains("__all__");
                let all_lines = format_tool_message_lines(content);
                let shown = if expanded { 20 } else { 4 };
                lines.push(Line::from(vec![Span::styled(
                    format!(
                        "    [tool card: {} | {} lines | Ctrl+E toggles]",
                        if expanded { "expanded" } else { "collapsed" },
                        all_lines.len()
                    ),
                    Style::default()
                        .fg(colors.status_bar_dim)
                        .bg(colors.background),
                )]));
                for line in all_lines.iter().take(shown) {
                    for segment in hard_wrap_segments(line, TRANSCRIPT_CONTENT_WRAP_COLS) {
                        lines.push(render_inline_with_code(
                            "    ",
                            &segment,
                            styles.tool_result,
                            Style::default()
                                .fg(colors.accent)
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                }
                if all_lines.len() > shown {
                    lines.push(Line::from(vec![Span::styled(
                        format!("    … {} more lines", all_lines.len() - shown),
                        Style::default()
                            .fg(colors.status_bar_dim)
                            .bg(colors.background),
                    )]));
                }
            }
            _ => {
                for line in content.lines() {
                    for segment in hard_wrap_segments(line, TRANSCRIPT_CONTENT_WRAP_COLS) {
                        lines.push(render_inline_with_code(
                            "    ",
                            &segment,
                            body_style,
                            Style::default()
                                .fg(colors.accent)
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                }
            }
        }
    }

    if msg.role == hermes_core::MessageRole::Assistant {
        if matches!(state.view_density, ViewDensity::Detailed) {
            if let Some(reasoning) = msg
                .reasoning_content
                .as_ref()
                .filter(|s| !s.trim().is_empty())
            {
                lines.push(Line::from(vec![Span::styled(
                    "    🤔 reasoning",
                    Style::default()
                        .fg(colors.status_bar_dim)
                        .bg(colors.background),
                )]));
                for line in reasoning.lines() {
                    lines.push(Line::from(vec![Span::styled(
                        format!("      {}", line.trim_end()),
                        Style::default()
                            .fg(colors.status_bar_dim)
                            .bg(colors.background)
                            .add_modifier(Modifier::ITALIC),
                    )]));
                }
            }
        }
        if let Some(tool_calls) = msg.tool_calls.as_ref() {
            for tc in tool_calls {
                let args = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
                    .unwrap_or_else(|_| serde_json::Value::Null);
                let preview =
                    build_tool_preview_from_value(&tc.function.name, &args, 44).unwrap_or_default();
                let emoji = tool_emoji(&tc.function.name);
                let summary = if preview.is_empty() {
                    format!("{emoji} {}", tc.function.name)
                } else {
                    format!("{emoji} {} {}", tc.function.name, preview)
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        "    ↳ ",
                        Style::default()
                            .fg(colors.status_bar_dim)
                            .bg(colors.background),
                    ),
                    Span::styled(summary, styles.tool_call),
                ]));
            }
        }
    }
    if matches!(state.view_density, ViewDensity::Detailed) {
        lines.push(Line::from(vec![Span::styled(
            divider.to_string(),
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.background),
        )]));
    }
}

pub(crate) fn append_streaming_transcript_tail(
    lines: &mut Vec<Line<'static>>,
    state: &mut TuiState,
    styles: &crate::theme::ResolvedStyles,
    colors: &crate::theme::RatatuiColors,
    content_width: u16,
    divider: &str,
) {
    let Some(processing) = state.phase.processing_mut() else {
        return;
    };
    if processing.stream_buffer.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(Line::from(String::new()));
    }
    lines.push(Line::from(vec![
        Span::styled(
            " ╭ ● ",
            styles.assistant_response.add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "HERMES (streaming)",
            styles.assistant_response.add_modifier(Modifier::BOLD),
        ),
    ]));
    let stream_lines = render_streaming_assistant_markdown_lines(
        &mut processing.stream_md_cache,
        &processing.stream_buffer,
        styles,
        colors,
        content_width,
    );
    lines.extend(tail_render_lines_with_notice(
        stream_lines,
        MAX_STREAM_RENDER_LINES,
        colors,
    ));
    lines.push(Line::from(vec![Span::styled(
        "    ▌",
        Style::default()
            .fg(colors.accent)
            .bg(colors.background)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(vec![Span::styled(
        divider.to_string(),
        Style::default()
            .fg(colors.status_bar_dim)
            .bg(colors.background),
    )]));
}

pub(crate) struct TranscriptBuildOutput {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) message_line_ends: Vec<usize>,
    pub(crate) messages_only_len: usize,
    pub(crate) rendered_messages: usize,
}

pub(crate) fn build_transcript_lines(
    messages: &[hermes_core::Message],
    state: &mut TuiState,
    styles: &crate::theme::ResolvedStyles,
    colors: &crate::theme::RatatuiColors,
    content_width: u16,
) -> TranscriptBuildOutput {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut message_line_ends: Vec<usize> = Vec::with_capacity(messages.len());
    let mut rendered_messages = 0usize;
    let divider = transcript_divider(content_width);

    for (msg_idx, msg) in messages.iter().enumerate() {
        append_transcript_message_lines(
            &mut lines,
            msg,
            msg_idx,
            &mut rendered_messages,
            state,
            styles,
            colors,
            &divider,
        );
        message_line_ends.push(lines.len());
    }

    let messages_only_len = lines.len();

    append_streaming_transcript_tail(&mut lines, state, styles, colors, content_width, &divider);

    if lines.is_empty() {
        let neon = Style::default()
            .fg(colors.status_bar_strong)
            .bg(colors.background)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default()
            .fg(colors.status_bar_dim)
            .bg(colors.background);
        let accent = Style::default().fg(colors.accent).bg(colors.background);
        let hero = [
            " ╔══════════════════════════════════════════════════════════════════╗",
            " ║  ██╗  ██╗███████╗██████╗ ███╗   ███╗███████╗███████╗          ║",
            " ║  ██║  ██║██╔════╝██╔══██╗████╗ ████║██╔════╝██╔════╝          ║",
            " ║  ███████║█████╗  ██████╔╝██╔████╔██║█████╗  ███████╗          ║",
            " ║  ██╔══██║██╔══╝  ██╔══██╗██║╚██╔╝██║██╔══╝  ╚════██║          ║",
            " ║  ██║  ██║███████╗██║  ██║██║ ╚═╝ ██║███████╗███████║          ║",
            " ║  ╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝╚═╝     ╚═╝╚══════╝╚══════╝          ║",
            " ║                                                                  ║",
            " ║       AGENT ULTRA  //  SUNBURST OPS  //  LIVE EXECUTION         ║",
            " ║       YELLOW SIGNAL • REDLINE DRIVE • RUST-NATIVE CONTROL       ║",
            " ╚══════════════════════════════════════════════════════════════════╝",
        ];
        for (idx, row) in hero.iter().enumerate() {
            let style = if idx == 0 || idx == hero.len() - 1 {
                accent
            } else if row.contains("AGENT ULTRA") || row.contains("YELLOW SIGNAL") {
                neon
            } else {
                dim
            };
            lines.push(Line::from(vec![Span::styled((*row).to_string(), style)]));
        }
        lines.push(Line::from(String::new()));
        lines.push(Line::from(vec![Span::styled(
            " Start chatting — your messages and Hermes replies will appear here.",
            Style::default()
                .fg(colors.status_bar_dim)
                .bg(colors.background)
                .add_modifier(Modifier::ITALIC),
        )]));
    }
    TranscriptBuildOutput {
        lines,
        message_line_ends,
        messages_only_len,
        rendered_messages,
    }
}

pub(crate) fn approximate_visual_rows(lines: &[Line<'static>], wrap_width: u16) -> usize {
    let width = usize::from(wrap_width.max(1));
    lines
        .iter()
        .map(|line| line_visual_rows(line, width))
        .sum::<usize>()
        .max(1)
}

pub(crate) fn line_visual_rows(line: &Line<'static>, width: usize) -> usize {
    let display_width = UnicodeWidthStr::width(line.to_string().as_str()).max(1);
    ((display_width - 1) / width.max(1)) + 1
}

pub(crate) fn project_transcript_window(
    lines: &[Line<'static>],
    wrap_width: u16,
    top_visual_row: usize,
    viewport_rows: usize,
) -> (Vec<Line<'static>>, usize) {
    if lines.is_empty() {
        return (Vec::new(), 0);
    }

    let width = usize::from(wrap_width.max(1));
    let mut cumulative = 0usize;
    let mut start_idx = 0usize;
    let mut intra_line_offset = 0usize;

    for (idx, line) in lines.iter().enumerate() {
        let line_rows = line_visual_rows(line, width);
        if cumulative + line_rows > top_visual_row {
            start_idx = idx;
            intra_line_offset = top_visual_row.saturating_sub(cumulative);
            break;
        }
        cumulative = cumulative.saturating_add(line_rows);
        start_idx = idx.saturating_add(1);
    }

    if start_idx >= lines.len() {
        start_idx = lines.len().saturating_sub(1);
        intra_line_offset = 0;
    }

    while intra_line_offset > u16::MAX as usize && start_idx + 1 < lines.len() {
        let consume = line_visual_rows(&lines[start_idx], width);
        if consume == 0 {
            break;
        }
        intra_line_offset = intra_line_offset.saturating_sub(consume);
        start_idx += 1;
    }

    let needed_rows = intra_line_offset.saturating_add(viewport_rows.max(1));
    let mut collected_rows = 0usize;
    let mut window: Vec<Line<'static>> = Vec::new();
    for line in lines.iter().skip(start_idx) {
        collected_rows = collected_rows.saturating_add(line_visual_rows(line, width));
        window.push(line.clone());
        if collected_rows >= needed_rows {
            break;
        }
    }
    if window.is_empty() {
        window.push(lines[lines.len() - 1].clone());
    }

    (window, intra_line_offset)
}
