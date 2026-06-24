//! Flowy media request normalization (resolution, duration).

/// True when `model` looks like a Flowy server model id (`AIPC-...` or `flowy/...`).
pub fn is_flowy_model_id(model: &str) -> bool {
    let m = model.trim();
    if m.is_empty() {
        return false;
    }
    let lower = m.to_ascii_lowercase();
    lower.starts_with("aipc-") || lower.starts_with("flowy/")
}

/// Seedance fast tiers typically reject 1080p; clamp to 720p.
pub fn normalize_video_resolution(model: &str, resolution: &str) -> Option<String> {
    let r = resolution.trim().to_ascii_lowercase();
    if r.is_empty() {
        return None;
    }
    let model_lower = model.to_ascii_lowercase();
    if model_lower.contains("seedance") && r == "1080p" {
        tracing::warn!(
            model,
            "clamping video resolution 1080p -> 720p for Seedance model"
        );
        return Some("720p".to_string());
    }
    Some(r)
}

/// Cap duration for fast Seedance variants (upstream often max 5–10s).
pub fn normalize_video_duration(model: &str, duration: u32) -> u32 {
    let model_lower = model.to_ascii_lowercase();
    if model_lower.contains("seedance") && model_lower.contains("fast") && duration > 10 {
        tracing::warn!(
            model,
            duration,
            "clamping video duration to 10s for Seedance fast model"
        );
        10
    } else {
        duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flowy_model_id_detection() {
        assert!(is_flowy_model_id("AIPC-Z-Image-Turbo"));
        assert!(is_flowy_model_id("flowy/doubao-seedance-1-0-pro"));
        assert!(!is_flowy_model_id("seedance-2.0"));
        assert!(!is_flowy_model_id("pixverse-v6"));
    }

    #[test]
    fn resolution_clamp_for_seedance() {
        assert_eq!(
            normalize_video_resolution("flowy/doubao-seedance-fast", "1080p").as_deref(),
            Some("720p")
        );
        assert_eq!(
            normalize_video_resolution("other-model", "1080p").as_deref(),
            Some("1080p")
        );
    }
}
