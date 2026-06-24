//! Image/video generation settings (Flowy server + optional FAL fallback).

use serde::{Deserialize, Serialize};

/// Top-level media generation configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MediaGenConfig {
    /// Primary provider: `flowy` (server) or `fal` (direct FAL_KEY).
    #[serde(default = "default_media_provider")]
    pub provider: String,

    #[serde(default)]
    pub image: ImageGenSettings,

    #[serde(default)]
    pub video: VideoGenSettings,

    #[serde(default)]
    pub workflows: MediaWorkflowSettings,
}

fn default_media_provider() -> String {
    "flowy".to_string()
}

/// Image generation defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageGenSettings {
    /// List `id` from `GET /model/availableListClaw?category=6` (e.g. `AIPC-z-image-turbo`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,

    #[serde(default = "default_true")]
    pub save_locally: bool,
}

impl Default for ImageGenSettings {
    fn default() -> Self {
        Self {
            model: String::new(),
            save_locally: true,
        }
    }
}

/// Video generation defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoGenSettings {
    /// List `id` from `GET /model/availableListClaw?category=4` (e.g. `AIPC-doubao-seedance-...`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,

    #[serde(default = "default_video_duration")]
    pub default_duration: u32,

    #[serde(default = "default_aspect_ratio")]
    pub default_aspect_ratio: String,

    #[serde(default = "default_video_resolution")]
    pub default_resolution: String,

    #[serde(default = "default_video_poll_timeout")]
    pub poll_timeout_seconds: u64,

    #[serde(default = "default_true")]
    pub save_locally: bool,
}

impl Default for VideoGenSettings {
    fn default() -> Self {
        Self {
            model: String::new(),
            default_duration: default_video_duration(),
            default_aspect_ratio: default_aspect_ratio(),
            default_resolution: default_video_resolution(),
            poll_timeout_seconds: default_video_poll_timeout(),
            save_locally: true,
        }
    }
}

/// Workflow orchestration settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaWorkflowSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_workflow_max_retries")]
    pub max_retries: u32,

    #[serde(default)]
    pub default_templates: MediaWorkflowTemplateMap,
}

impl Default for MediaWorkflowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: default_workflow_max_retries(),
            default_templates: MediaWorkflowTemplateMap::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MediaWorkflowTemplateMap {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub txt2img: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub txt2video: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub img2video: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub storyboard: String,
}

fn default_true() -> bool {
    true
}

fn default_video_duration() -> u32 {
    5
}

fn default_aspect_ratio() -> String {
    "16:9".to_string()
}

fn default_video_resolution() -> String {
    "720p".to_string()
}

fn default_video_poll_timeout() -> u64 {
    600
}

fn default_workflow_max_retries() -> u32 {
    1
}

impl MediaGenConfig {
    pub fn uses_flowy(&self) -> bool {
        self.provider.trim().eq_ignore_ascii_case("flowy")
    }
}

/// True when Flowy-backed image/video tools should be exposed (provider + server URL).
pub fn flowy_media_exposed(config: &crate::GatewayConfig) -> bool {
    config.media.uses_flowy() && config.server.api_ready()
}

/// Load `config.yaml` and check whether Flowy media tools should be exposed.
pub fn flowy_media_exposed_from_disk() -> bool {
    crate::loader::load_config(None)
        .map(|c| flowy_media_exposed(&c))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_gen_defaults() {
        let cfg = MediaGenConfig::default();
        assert!(cfg.uses_flowy());
        assert!(cfg.image.save_locally);
        assert!(cfg.workflows.enabled);
    }

    #[test]
    fn flowy_media_exposed_requires_server_url() {
        let mut cfg = crate::GatewayConfig::default();
        assert!(!super::flowy_media_exposed(&cfg));
        cfg.media.provider = "flowy".into();
        assert!(!super::flowy_media_exposed(&cfg));
        cfg.server.base_url = "https://server.flowyaipc.cn/claw".into();
        assert!(super::flowy_media_exposed(&cfg));
    }

    #[test]
    fn media_gen_yaml_roundtrip() {
        let yaml = r#"
provider: flowy
image:
  model: AIPC-z-image-turbo
video:
  model: flowy/doubao-seedance-1-0-pro-250528
  default_duration: 8
workflows:
  enabled: true
  default_templates:
    txt2img: simple_txt2img
"#;
        let cfg: MediaGenConfig = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(cfg.image.model, "AIPC-z-image-turbo");
        assert_eq!(cfg.video.default_duration, 8);
        assert_eq!(cfg.workflows.default_templates.txt2img, "simple_txt2img");
    }
}
