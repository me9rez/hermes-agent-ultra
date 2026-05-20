//! Shared helpers for encoding inbound/tool images as OpenAI-style content parts.

use std::path::Path;

use base64::Engine;
use serde_json::{json, Value};
use tracing::warn;

/// Build an OpenAI-style `image_url` content part for a local path or HTTP URL.
pub async fn encode_image_url_part(image_ref: &str) -> Result<Value, String> {
    let trimmed = image_ref.trim();
    if trimmed.is_empty() {
        return Err("empty image reference".into());
    }
    let url = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.starts_with("file://") {
        let path = trimmed.trim_start_matches("file://");
        file_to_data_url(Path::new(path)).await?
    } else {
        file_to_data_url(Path::new(trimmed)).await?
    };
    Ok(json!({
        "type": "image_url",
        "image_url": { "url": url }
    }))
}

/// Encode a local image file as a `data:` URL at native size.
pub async fn file_to_data_url(path: &Path) -> Result<String, String> {
    let raw = tokio::fs::read(path).await.map_err(|e| {
        format!("failed to read image '{}': {e}", path.display())
    })?;
    let mime = guess_mime(path, Some(&raw));
    let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
    Ok(format!("data:{mime};base64,{encoded}"))
}

/// Detect image MIME from magic bytes; falls back to extension / `image/jpeg`.
pub fn guess_mime(path: &Path, raw: Option<&[u8]>) -> String {
    if let Some(bytes) = raw {
        if let Some(sniffed) = sniff_mime_from_bytes(bytes) {
            return sniffed;
        }
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png".into(),
        Some("gif") => "image/gif".into(),
        Some("webp") => "image/webp".into(),
        Some("bmp") => "image/bmp".into(),
        Some("heic" | "heif") => "image/heic".into(),
        _ => "image/jpeg".into(),
    }
}

fn sniff_mime_from_bytes(raw: &[u8]) -> Option<String> {
    if raw.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png".into());
    }
    if raw.starts_with(b"\xff\xd8\xff") {
        return Some("image/jpeg".into());
    }
    if raw.len() >= 6 && (&raw[..6] == b"GIF87a" || &raw[..6] == b"GIF89a") {
        return Some("image/gif".into());
    }
    if raw.len() >= 12 && &raw[..4] == b"RIFF" && &raw[8..12] == b"WEBP" {
        return Some("image/webp".into());
    }
    if raw.starts_with(b"BM") {
        return Some("image/bmp".into());
    }
    if raw.len() >= 12 && &raw[4..8] == b"ftyp" {
        let brand = &raw[8..12];
        if matches!(
            brand,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"heim" | b"heis"
        ) {
            return Some("image/heic".into());
        }
    }
    None
}

/// Sync variant for pure routing helpers (no runtime required).
pub fn file_to_data_url_sync(path: &Path) -> Option<String> {
    let raw = std::fs::read(path).ok()?;
    let mime = guess_mime(path, Some(&raw));
    let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
    Some(format!("data:{mime};base64,{encoded}"))
}

/// Read a local image into a data URL; logs and returns `None` on failure.
pub async fn file_to_data_url_opt(path: &Path) -> Option<String> {
    match file_to_data_url(path).await {
        Ok(url) => Some(url),
        Err(err) => {
            warn!(path = %path.display(), "{err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_png() {
        let png = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABpfZFQAAAAABJRU5ErkJggg==")
            .unwrap();
        assert_eq!(sniff_mime_from_bytes(&png), Some("image/png".into()));
    }

    #[tokio::test]
    async fn file_to_data_url_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.png");
        let png = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABpfZFQAAAAABJRU5ErkJggg==")
            .unwrap();
        tokio::fs::write(&path, &png).await.unwrap();
        let url = file_to_data_url(&path).await.unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }
}
