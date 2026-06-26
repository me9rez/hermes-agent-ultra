//! Shared HTML utilities for institutional report sections.

#[must_use]
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
}
