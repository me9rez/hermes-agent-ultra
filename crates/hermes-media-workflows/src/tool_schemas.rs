//! Tool schemas for Flowy-backed media tools (no FAL/xAI model enums).

use indexmap::IndexMap;
use serde_json::json;

use hermes_core::{JsonSchema, ToolSchema, tool_schema};

/// Schema for Flowy `video_generate` — model is optional Flowy id, not FAL families.
pub fn flowy_video_generate_schema() -> ToolSchema {
    let mut props = IndexMap::new();
    props.insert(
        "prompt".into(),
        json!({
            "type": "string",
            "description": "Text prompt for text-to-video or image-to-video generation."
        }),
    );
    props.insert(
        "model".into(),
        json!({
            "type": "string",
            "description": "Optional Flowy video model id (AIPC-... or flowy/... from `hermes media models`). Omit to use media.video.model from config."
        }),
    );
    props.insert(
        "image_url".into(),
        json!({
            "type": "string",
            "description": "Optional starting image URL for image-to-video."
        }),
    );
    props.insert(
        "reference_image_urls".into(),
        json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "Optional reference image URLs for first-frame / reference-guided video."
        }),
    );
    props.insert(
        "duration".into(),
        json!({
            "type": "integer",
            "minimum": 1,
            "maximum": 15,
            "description": "Video length in seconds (config default used when omitted)."
        }),
    );
    props.insert(
        "aspect_ratio".into(),
        json!({
            "type": "string",
            "description": "Aspect ratio such as 16:9 or 9:16.",
            "default": "16:9"
        }),
    );
    props.insert(
        "resolution".into(),
        json!({
            "type": "string",
            "description": "Output resolution when supported (e.g. 720p). Config default used when omitted.",
            "enum": ["360p", "480p", "540p", "720p", "1080p"]
        }),
    );
    props.insert(
        "negative_prompt".into(),
        json!({
            "type": "string",
            "description": "Optional negative prompt."
        }),
    );

    tool_schema(
        "video_generate",
        "Generate a video via the Flowy cloud API (Seedance). Returns a video URL and MEDIA: local path when save_locally is enabled.",
        JsonSchema::object(props, vec!["prompt".into()]),
    )
}

/// Schema for Flowy `image_generate`.
pub fn flowy_image_generate_schema() -> ToolSchema {
    let mut props = IndexMap::new();
    props.insert(
        "prompt".into(),
        json!({
            "type": "string",
            "description": "Text description of the image to generate."
        }),
    );
    props.insert(
        "model".into(),
        json!({
            "type": "string",
            "description": "Optional Flowy image model id (AIPC-... or flowy/...). Omit to use media.image.model from config."
        }),
    );
    props.insert(
        "image_url".into(),
        json!({
            "type": "string",
            "description": "Optional reference image URL for image-to-image / edit."
        }),
    );

    tool_schema(
        "image_generate",
        "Generate an image via the Flowy cloud API. Returns image URL(s) and MEDIA: local path when save_locally is enabled.",
        JsonSchema::object(props, vec!["prompt".into()]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flowy_video_schema_has_no_fal_model_enum() {
        let schema = flowy_video_generate_schema();
        let props = schema.parameters.properties.as_ref().expect("properties");
        let model = props.get("model").expect("model property");
        assert!(
            model.get("enum").is_none(),
            "Flowy video schema must not list FAL model families"
        );
    }
}
