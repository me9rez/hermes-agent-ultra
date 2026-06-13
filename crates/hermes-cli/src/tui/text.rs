use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;
pub(crate) fn transcript_divider(content_width: u16) -> String {
    let width = usize::from(content_width.max(12));
    let rule = "─".repeat(width.saturating_sub(3).max(8));
    format!(" ╰{}", rule)
}

pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars <= 1 {
        return "…".to_string();
    }
    let take = max_chars.saturating_sub(1);
    let mut out: String = text.chars().take(take).collect();
    out.push('…');
    out
}

pub(crate) fn fit_status_line(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if cw == 0 {
            continue;
        }
        if used + cw > width {
            break;
        }
        out.push(ch);
        used += cw;
    }
    while used < width {
        out.push(' ');
        used += 1;
    }
    out
}

pub(crate) fn hard_wrap_segments(text: &str, max_chars: usize) -> Vec<String> {
    let width = max_chars.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return vec![String::new()];
    }
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for token in trimmed.split_whitespace() {
        let token_len = token.chars().count();
        if token_len > width {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
                current_len = 0;
            }
            let mut chunk = String::new();
            let mut chunk_len = 0usize;
            for ch in token.chars() {
                chunk.push(ch);
                chunk_len += 1;
                if chunk_len >= width {
                    segments.push(std::mem::take(&mut chunk));
                    chunk_len = 0;
                }
            }
            if !chunk.is_empty() {
                current = chunk;
                current_len = chunk_len;
            }
            continue;
        }

        let needed = if current.is_empty() {
            token_len
        } else {
            current_len + 1 + token_len
        };
        if needed <= width {
            if !current.is_empty() {
                current.push(' ');
                current_len += 1;
            }
            current.push_str(token);
            current_len += token_len;
        } else {
            segments.push(std::mem::take(&mut current));
            current.push_str(token);
            current_len = token_len;
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    if segments.is_empty() {
        segments.push(String::new());
    }
    segments
}

pub(crate) fn is_ctrl_c(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('\u{3}'))
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C')))
}

pub(crate) fn is_submit_shortcut(key: &KeyEvent, _input: &str) -> bool {
    let mods = key.modifiers;

    if key.code == KeyCode::Enter {
        if mods.contains(KeyModifiers::SHIFT) {
            return false;
        }
        if mods.is_empty()
            || mods.contains(KeyModifiers::CONTROL)
            || mods.contains(KeyModifiers::ALT)
        {
            return true;
        }
    }

    key.code == KeyCode::Char('m') && mods.contains(KeyModifiers::CONTROL)
}
