//! Markdown formatting and message chunking for WhatsApp.

use regex::Regex;
use std::sync::LazyLock;

use super::config::WhatsAppConfig;

static FENCE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"```[\s\S]*?```").expect("valid regex"));
static INLINE_CODE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`[^`\n]+`").expect("valid regex"));
static BOLD_STAR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").expect("valid regex"));
static BOLD_UNDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"__(.+?)__").expect("valid regex"));
static STRIKE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"~~(.+?)~~").expect("valid regex"));
static HEADER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#{1,6}\s+(.+)$").expect("valid regex"));
static LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("valid regex"));

/// Convert standard markdown to WhatsApp-compatible formatting.
pub fn format_message(content: &str) -> String {
    if content.is_empty() {
        return content.to_string();
    }

    let mut fences: Vec<String> = Vec::new();
    let mut result = FENCE_RE
        .replace_all(content, |caps: &regex::Captures| {
            fences.push(caps[0].to_string());
            format!("\x00FENCE{}\x00", fences.len() - 1)
        })
        .into_owned();

    let mut codes: Vec<String> = Vec::new();
    result = INLINE_CODE_RE
        .replace_all(&result, |caps: &regex::Captures| {
            codes.push(caps[0].to_string());
            format!("\x00CODE{}\x00", codes.len() - 1)
        })
        .into_owned();

    result = BOLD_STAR_RE.replace_all(&result, "*$1*").into_owned();
    result = BOLD_UNDER_RE.replace_all(&result, "*$1*").into_owned();
    result = STRIKE_RE.replace_all(&result, "~$1~").into_owned();
    result = HEADER_RE.replace_all(&result, "*$1*").into_owned();
    result = LINK_RE.replace_all(&result, "$1 ($2)").into_owned();

    for (i, fence) in fences.iter().enumerate() {
        result = result.replace(&format!("\x00FENCE{i}\x00"), fence);
    }
    for (i, code) in codes.iter().enumerate() {
        result = result.replace(&format!("\x00CODE{i}\x00"), code);
    }
    result
}

/// Split long text into chunks, preserving code fences when possible.
pub fn truncate_message(content: &str, limit: usize) -> Vec<String> {
    if content.is_empty() {
        return vec![];
    }
    if content.len() <= limit {
        return vec![content.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = content;
    while remaining.len() > limit {
        let slice = &remaining[..limit];
        let mut split_at = slice.rfind('\n').unwrap_or(0);
        if split_at < limit / 2 {
            split_at = slice.rfind(' ').unwrap_or(limit);
        }
        if split_at == 0 {
            split_at = limit;
        }
        chunks.push(remaining[..split_at].trim_end().to_string());
        remaining = remaining[split_at..].trim_start();
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}

pub fn outgoing_chunks(
    cfg: &WhatsAppConfig,
    content: &str,
    include_reply_prefix: bool,
) -> Vec<String> {
    let mut body = format_message(content);
    if include_reply_prefix {
        let prefix = cfg.effective_reply_prefix();
        if !prefix.is_empty() {
            body = format!("{prefix}{body}");
        }
    }
    truncate_message(&body, cfg.outgoing_chunk_limit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_double_asterisk() {
        assert_eq!(format_message("**hello**"), "*hello*");
    }

    #[test]
    fn strikethrough() {
        assert_eq!(format_message("~~gone~~"), "~gone~");
    }

    #[test]
    fn code_fence_preserved() {
        let input = "before\n```rust\nlet x = 1;\n```\nafter";
        assert!(format_message(input).contains("```rust"));
    }

    #[test]
    fn chunk_limit_reserves_prefix_space() {
        let reserved = MAX_MESSAGE_LENGTH
            .saturating_sub(DEFAULT_REPLY_PREFIX.len())
            .max(1024);
        assert!(reserved < MAX_MESSAGE_LENGTH);
    }

    #[test]
    fn multi_chunk() {
        let text = "a".repeat(5000);
        let chunks = truncate_message(&text, 4096);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn outgoing_self_chat_includes_separator_only() {
        let mut cfg = WhatsAppConfig::default();
        cfg.mode = Some("self-chat".into());
        let chunks = outgoing_chunks(&cfg, "hello", true);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].starts_with("────────────"));
        assert!(!chunks[0].contains("Hermes"));
        assert!(chunks[0].contains("hello"));
    }
}
