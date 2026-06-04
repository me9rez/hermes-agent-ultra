//! Message content decode helpers (Python `_decode_content` preview parity).

const CONTENT_JSON_PREFIX: &str = "\x00json:";

/// Decode persisted message content to a plain-text preview string.
pub fn decode_content_preview(content: Option<&str>) -> String {
    let Some(raw) = content else {
        return String::new();
    };
    if let Some(json_part) = raw.strip_prefix(CONTENT_JSON_PREFIX) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_part) {
            if let Some(arr) = v.as_array() {
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|p| {
                        (p.get("type")?.as_str()? == "text")
                            .then(|| p.get("text")?.as_str())
                            .flatten()
                            .map(str::to_string)
                    })
                    .collect();
                let text = parts.join(" ").trim().to_string();
                return if text.is_empty() {
                    "[multimodal content]".into()
                } else {
                    text
                };
            }
        }
    }
    raw.to_string()
}
