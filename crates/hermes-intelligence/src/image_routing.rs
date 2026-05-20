//! Per-turn routing for inbound user-attached images (native pixels vs text summary).

use std::path::Path;

use serde_json::{json, Value};

use crate::model_metadata::supports_vision;
use crate::vision_media::file_to_data_url_sync;

fn coerce_mode(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "native" => "native",
        "text" => "text",
        _ => "auto",
    }
}

/// True when the user configured a specific auxiliary vision backend (not auto/empty).
pub fn explicit_aux_vision_override(
    aux_vision_provider: Option<&str>,
    aux_vision_model: Option<&str>,
    aux_vision_base_url: Option<&str>,
) -> bool {
    let provider = aux_vision_provider.unwrap_or("").trim().to_lowercase();
    let model = aux_vision_model.unwrap_or("").trim();
    let base_url = aux_vision_base_url.unwrap_or("").trim();
    if provider.is_empty() || provider == "auto" {
        return !model.is_empty() || !base_url.is_empty();
    }
    true
}

/// Return `"native"` or `"text"` for how to present user-attached images this turn.
pub fn decide_image_input_mode(
    _provider: &str,
    model: &str,
    image_input_mode_cfg: &str,
    aux_vision_provider: Option<&str>,
    aux_vision_model: Option<&str>,
    aux_vision_base_url: Option<&str>,
) -> &'static str {
    let mode_cfg = coerce_mode(image_input_mode_cfg);
    if mode_cfg == "native" {
        return "native";
    }
    if mode_cfg == "text" {
        return "text";
    }
    if explicit_aux_vision_override(aux_vision_provider, aux_vision_model, aux_vision_base_url) {
        return "text";
    }
    if supports_vision(model) {
        return "native";
    }
    "text"
}

/// Build OpenAI-style multimodal `content` parts for a user turn.
pub fn build_native_content_parts(
    user_text: &str,
    image_paths: &[String],
) -> (Vec<Value>, Vec<String>) {
    let mut skipped = Vec::new();
    let mut image_parts = Vec::new();
    let mut attached_paths = Vec::new();

    for raw_path in image_paths {
        let p = Path::new(raw_path);
        if !p.exists() || !p.is_file() {
            skipped.push(raw_path.clone());
            continue;
        }
        let Some(data_url) = file_to_data_url_sync(p) else {
            skipped.push(raw_path.clone());
            continue;
        };
        image_parts.push(json!({
            "type": "image_url",
            "image_url": {"url": data_url}
        }));
        attached_paths.push(raw_path.clone());
    }

    let text = user_text.trim();
    if !attached_paths.is_empty() {
        let base_text = if text.is_empty() {
            "What do you see in this image?"
        } else {
            text
        };
        let path_hints: String = attached_paths
            .iter()
            .map(|p| format!("[Image attached at: {p}]"))
            .collect::<Vec<_>>()
            .join("\n");
        let combined_text = format!("{base_text}\n\n{path_hints}");
        let mut parts = vec![json!({"type": "text", "text": combined_text})];
        parts.extend(image_parts);
        return (parts, skipped);
    }

    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(json!({"type": "text", "text": text}));
    }
    (parts, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn explicit_aux_override_detects_provider() {
        assert!(explicit_aux_vision_override(Some("openrouter"), None, None));
        assert!(!explicit_aux_vision_override(Some("auto"), None, None));
        assert!(!explicit_aux_vision_override(None, None, None));
    }

    #[test]
    fn decide_native_for_vision_model_in_auto() {
        assert_eq!(
            decide_image_input_mode("openai", "gpt-4o", "auto", None, None, None),
            "native"
        );
    }

    #[test]
    fn decide_text_for_non_vision_model() {
        assert_eq!(
            decide_image_input_mode("openai", "deepseek-chat", "auto", None, None, None),
            "text"
        );
    }

    #[test]
    fn build_native_parts_includes_image_and_path_hint() {
        let mut f = NamedTempFile::new().unwrap();
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\r";
        f.write_all(png).unwrap();
        let path = f.path().to_string_lossy().to_string();
        let (parts, skipped) = build_native_content_parts("", &[path.clone()]);
        assert!(skipped.is_empty());
        assert!(parts.len() >= 2);
        assert_eq!(parts[0]["type"], "text");
        let text = parts[0]["text"].as_str().unwrap_or("");
        assert!(text.contains(&path));
    }
}
