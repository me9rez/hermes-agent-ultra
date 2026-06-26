//! Shared HTML utilities for institutional report sections.

#[must_use]
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[must_use]
pub fn truncate_for_display(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n... (truncated)", &s[..end])
}

#[must_use]
pub fn render_bullet_list(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let lis: String = items
        .iter()
        .map(|item| format!("<li>{}</li>", escape_html(item)))
        .collect();
    format!("<ul class=\"bullets\">{lis}</ul>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_strips_tags() {
        assert!(escape_html("<script>").contains("&lt;"));
    }

    #[test]
    fn truncate_respects_utf8_boundary() {
        let s = "中".repeat(800);
        let out = truncate_for_display(&s, 10);
        assert!(out.ends_with("(truncated)"));
        assert!(out.len() <= 10 + 20);
    }
}
