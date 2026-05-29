//! Rust-native FAL video generation backend.
//!
//! This ports the FAL video plugin surface into the built-in Rust tool
//! runtime. Direct mode uses FAL's queue HTTP API, matching
//! `fal_client.subscribe`; managed mode routes through the existing Nous
//! `fal-queue` gateway resolver.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Map, Value};
use std::path::Path;
use std::time::Duration;

use crate::tools::video::{VideoGenerateBackend, VideoGenerateRequest};
use hermes_config::managed_gateway::{
    resolve_managed_tool_gateway, ManagedToolGatewayConfig, ResolveOptions,
};
use hermes_core::ToolError;

const DEFAULT_FAL_VIDEO_MODEL: &str = "pixverse-v6";
const DEFAULT_TIMEOUT_SECONDS: u64 = 600;
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurationSpec {
    Range(u32, u32),
    Enum(&'static [u32]),
}

#[derive(Debug, Clone, Copy)]
struct FalVideoFamily {
    id: &'static str,
    text_endpoint: &'static str,
    image_endpoint: &'static str,
    image_param_key: &'static str,
    aspect_ratios: &'static [&'static str],
    resolutions: &'static [&'static str],
    durations: Option<DurationSpec>,
    audio: bool,
    negative: bool,
}

const VEO_DURATIONS: &[u32] = &[4, 6, 8];

const FAL_VIDEO_FAMILIES: &[FalVideoFamily] = &[
    FalVideoFamily {
        id: "ltx-2.3",
        text_endpoint: "fal-ai/ltx-2.3-22b/text-to-video",
        image_endpoint: "fal-ai/ltx-2.3-22b/image-to-video",
        image_param_key: "image_url",
        aspect_ratios: &[],
        resolutions: &[],
        durations: None,
        audio: true,
        negative: true,
    },
    FalVideoFamily {
        id: "pixverse-v6",
        text_endpoint: "fal-ai/pixverse/v6/text-to-video",
        image_endpoint: "fal-ai/pixverse/v6/image-to-video",
        image_param_key: "image_url",
        aspect_ratios: &[],
        resolutions: &["360p", "540p", "720p", "1080p"],
        durations: Some(DurationSpec::Range(1, 15)),
        audio: true,
        negative: true,
    },
    FalVideoFamily {
        id: "veo3.1",
        text_endpoint: "fal-ai/veo3.1",
        image_endpoint: "fal-ai/veo3.1/image-to-video",
        image_param_key: "image_url",
        aspect_ratios: &["16:9", "9:16"],
        resolutions: &["720p", "1080p"],
        durations: Some(DurationSpec::Enum(VEO_DURATIONS)),
        audio: true,
        negative: true,
    },
    FalVideoFamily {
        id: "seedance-2.0",
        text_endpoint: "bytedance/seedance-2.0/text-to-video",
        image_endpoint: "bytedance/seedance-2.0/image-to-video",
        image_param_key: "image_url",
        aspect_ratios: &["21:9", "16:9", "4:3", "1:1", "3:4", "9:16"],
        resolutions: &["480p", "720p", "1080p"],
        durations: Some(DurationSpec::Range(4, 15)),
        audio: true,
        negative: false,
    },
    FalVideoFamily {
        id: "kling-v3-4k",
        text_endpoint: "fal-ai/kling-video/v3/4k/text-to-video",
        image_endpoint: "fal-ai/kling-video/v3/4k/image-to-video",
        image_param_key: "start_image_url",
        aspect_ratios: &["16:9", "9:16", "1:1"],
        resolutions: &[],
        durations: Some(DurationSpec::Range(3, 15)),
        audio: true,
        negative: true,
    },
    FalVideoFamily {
        id: "happy-horse",
        text_endpoint: "fal-ai/happy-horse/text-to-video",
        image_endpoint: "fal-ai/happy-horse/image-to-video",
        image_param_key: "image_url",
        aspect_ratios: &[],
        resolutions: &[],
        durations: None,
        audio: false,
        negative: false,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum FalVideoTransport {
    Direct {
        api_key: String,
    },
    Managed {
        gateway_origin: String,
        nous_token: String,
    },
    Unconfigured,
}

impl FalVideoTransport {
    fn label(&self) -> &'static str {
        match self {
            Self::Direct { .. } => "direct",
            Self::Managed { .. } => "managed",
            Self::Unconfigured => "unconfigured",
        }
    }

    fn submit_url(&self, endpoint: &str) -> Result<String, ToolError> {
        match self {
            Self::Direct { .. } => Ok(format!("https://queue.fal.run/{endpoint}")),
            Self::Managed { gateway_origin, .. } => {
                let root = gateway_origin.trim_end_matches('/');
                Ok(format!("{root}/run/{endpoint}"))
            }
            Self::Unconfigured => Err(ToolError::ExecutionFailed(
                "FAL_KEY not set and Nous-managed fal-queue gateway is not configured.".into(),
            )),
        }
    }

    fn auth_header(&self) -> Result<(String, String), ToolError> {
        match self {
            Self::Direct { api_key } => Ok(("Authorization".into(), format!("Key {api_key}"))),
            Self::Managed { nous_token, .. } => {
                Ok(("Authorization".into(), format!("Bearer {nous_token}")))
            }
            Self::Unconfigured => Err(ToolError::ExecutionFailed(
                "FAL_KEY not set and Nous-managed fal-queue gateway is not configured.".into(),
            )),
        }
    }
}

/// FAL video generation backend using direct FAL queue API or the
/// Nous-managed fal-queue gateway.
#[derive(Debug)]
pub struct FalVideoGenBackend {
    client: Client,
    transport: FalVideoTransport,
}

impl FalVideoGenBackend {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            transport: FalVideoTransport::Direct { api_key },
        }
    }

    pub fn from_managed(cfg: &ManagedToolGatewayConfig) -> Self {
        Self {
            client: Client::new(),
            transport: FalVideoTransport::Managed {
                gateway_origin: cfg.gateway_origin.clone(),
                nous_token: cfg.nous_user_token.clone(),
            },
        }
    }

    pub fn unconfigured() -> Self {
        Self {
            client: Client::new(),
            transport: FalVideoTransport::Unconfigured,
        }
    }

    /// Priority: direct `FAL_KEY` -> Nous-managed `fal-queue` -> error.
    pub fn from_env_or_managed() -> Result<Self, ToolError> {
        if let Ok(key) = std::env::var("FAL_KEY") {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                return Ok(Self::new(trimmed.to_string()));
            }
        }
        if let Some(cfg) = resolve_managed_tool_gateway("fal-queue", ResolveOptions::default()) {
            return Ok(Self::from_managed(&cfg));
        }
        Err(ToolError::ExecutionFailed(
            "FAL_KEY not set and Nous-managed fal-queue gateway is not configured.".into(),
        ))
    }

    pub fn transport_label(&self) -> &'static str {
        self.transport.label()
    }

    async fn submit_managed(&self, endpoint: &str, payload: &Value) -> Result<Value, ToolError> {
        let url = self.transport.submit_url(endpoint)?;
        let (auth_name, auth_value) = self.transport.auth_header()?;
        let resp = self
            .client
            .post(url)
            .header(auth_name, auth_value)
            .header("Content-Type", "application/json")
            .json(payload)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("FAL video request failed: {e}")))?;
        read_json_response(resp, "FAL video generation").await
    }

    async fn submit_direct_queue(
        &self,
        endpoint: &str,
        payload: &Value,
    ) -> Result<Value, ToolError> {
        let url = self.transport.submit_url(endpoint)?;
        let (auth_name, auth_value) = self.transport.auth_header()?;
        let submit = self
            .client
            .post(url)
            .header(auth_name, auth_value.clone())
            .header("Content-Type", "application/json")
            .json(payload)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("FAL queue submit failed: {e}")))?;
        let submitted = read_json_response(submit, "FAL queue submit").await?;
        if extract_video(&submitted).is_some() {
            return Ok(submitted);
        }

        let status_url = submitted
            .get("status_url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let response_url = submitted
            .get("response_url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let request_id = submitted
            .get("request_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        let timeout = env_u64("FAL_VIDEO_TIMEOUT_SECONDS").unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        let poll_interval =
            env_u64("FAL_VIDEO_POLL_INTERVAL_SECONDS").unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);

        if let Some(status_url) = status_url.as_deref() {
            loop {
                if tokio::time::Instant::now() >= deadline {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Timed out waiting for FAL video generation after {timeout}s"
                    )));
                }
                let resp = self
                    .client
                    .get(status_url)
                    .header("Authorization", auth_value.clone())
                    .send()
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!("FAL queue status failed: {e}"))
                    })?;
                let status = read_json_response(resp, "FAL queue status").await?;
                match status
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                {
                    "COMPLETED" | "OK" => break,
                    "FAILED" | "ERROR" => {
                        return Err(ToolError::ExecutionFailed(format!(
                            "FAL video generation failed: {status}"
                        )));
                    }
                    _ => tokio::time::sleep(Duration::from_secs(poll_interval.max(1))).await,
                }
            }
        }

        let response_url = response_url
            .or_else(|| {
                request_id.map(|id| format!("https://queue.fal.run/{endpoint}/requests/{id}"))
            })
            .ok_or_else(|| {
                ToolError::ExecutionFailed("FAL queue response omitted request URL".into())
            })?;
        let resp = self
            .client
            .get(response_url)
            .header("Authorization", auth_value)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("FAL queue response failed: {e}")))?;
        read_json_response(resp, "FAL queue response").await
    }
}

#[async_trait]
impl VideoGenerateBackend for FalVideoGenBackend {
    async fn generate_video(&self, request: VideoGenerateRequest) -> Result<String, ToolError> {
        let prompt = request.prompt.trim();
        if prompt.is_empty() {
            return Err(ToolError::InvalidParams("prompt is required.".into()));
        }

        let family = resolve_family(request.model.as_deref());
        let image_url = request
            .image_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let (endpoint, modality) = if image_url.is_some() {
            (family.image_endpoint, "image")
        } else {
            (family.text_endpoint, "text")
        };
        let payload_meta = build_payload(family, &request);
        let payload = Value::Object(payload_meta.payload);
        let response = match self.transport {
            FalVideoTransport::Direct { .. } => {
                self.submit_direct_queue(endpoint, &payload).await?
            }
            FalVideoTransport::Managed { .. } => self.submit_managed(endpoint, &payload).await?,
            FalVideoTransport::Unconfigured => {
                return Err(ToolError::ExecutionFailed(
                    "FAL_KEY not set and Nous-managed fal-queue gateway is not configured.".into(),
                ));
            }
        };

        let video = extract_video(&response).ok_or_else(|| {
            ToolError::ExecutionFailed("FAL returned no video URL in response".into())
        })?;

        let mut out = Map::new();
        out.insert("success".into(), Value::Bool(true));
        out.insert("video".into(), Value::String(video.url));
        out.insert("model".into(), Value::String(family.id.to_string()));
        out.insert("prompt".into(), Value::String(prompt.to_string()));
        out.insert("modality".into(), Value::String(modality.to_string()));
        out.insert(
            "aspect_ratio".into(),
            Value::String(payload_meta.aspect_ratio.unwrap_or_default()),
        );
        out.insert(
            "duration".into(),
            payload_meta
                .duration
                .map(|d| Value::Number(d.into()))
                .unwrap_or_else(|| Value::Number(0.into())),
        );
        out.insert("provider".into(), Value::String("fal".into()));
        out.insert("endpoint".into(), Value::String(endpoint.to_string()));
        out.insert(
            "transport".into(),
            Value::String(self.transport.label().to_string()),
        );
        if let Some(file_size) = video.file_size {
            out.insert("file_size".into(), Value::Number(file_size.into()));
        }
        if let Some(content_type) = video.content_type {
            out.insert("content_type".into(), Value::String(content_type));
        }

        Ok(Value::Object(out).to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PayloadMeta {
    payload: Map<String, Value>,
    aspect_ratio: Option<String>,
    duration: Option<u32>,
}

fn build_payload(family: &FalVideoFamily, request: &VideoGenerateRequest) -> PayloadMeta {
    let mut payload = Map::new();
    payload.insert(
        "prompt".into(),
        Value::String(request.prompt.trim().to_string()),
    );
    if let Some(image_url) = request
        .image_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        payload.insert(
            family.image_param_key.into(),
            Value::String(image_url.to_string()),
        );
    }
    if let Some(seed) = request.seed {
        payload.insert("seed".into(), Value::Number(seed.into()));
    }

    let mut sent_aspect_ratio = None;
    if !family.aspect_ratios.is_empty()
        && family
            .aspect_ratios
            .contains(&request.aspect_ratio.as_str())
    {
        payload.insert(
            "aspect_ratio".into(),
            Value::String(request.aspect_ratio.clone()),
        );
        sent_aspect_ratio = Some(request.aspect_ratio.clone());
    }

    if !family.resolutions.is_empty() && family.resolutions.contains(&request.resolution.as_str()) {
        payload.insert(
            "resolution".into(),
            Value::String(request.resolution.clone()),
        );
    }

    let duration = clamp_duration(family.durations, request.duration);
    if let Some(duration) = duration {
        payload.insert("duration".into(), Value::String(duration.to_string()));
    }

    if family.audio {
        if let Some(audio) = request.audio {
            payload.insert("generate_audio".into(), Value::Bool(audio));
        }
    }

    if family.negative {
        if let Some(negative_prompt) = request.negative_prompt.as_deref() {
            payload.insert(
                "negative_prompt".into(),
                Value::String(negative_prompt.to_string()),
            );
        }
    }

    PayloadMeta {
        payload,
        aspect_ratio: sent_aspect_ratio,
        duration,
    }
}

fn clamp_duration(spec: Option<DurationSpec>, duration: Option<u32>) -> Option<u32> {
    match spec {
        None => None,
        Some(DurationSpec::Range(lo, hi)) => Some(duration.unwrap_or(lo).clamp(lo, hi)),
        Some(DurationSpec::Enum(values)) => {
            let requested = duration.unwrap_or_else(|| values[0]);
            values
                .iter()
                .copied()
                .min_by_key(|candidate| candidate.abs_diff(requested))
        }
    }
}

fn resolve_family(explicit: Option<&str>) -> &'static FalVideoFamily {
    let candidates = explicit
        .into_iter()
        .map(ToOwned::to_owned)
        .chain(std::env::var("FAL_VIDEO_MODEL").ok())
        .chain(configured_video_model_candidates());
    for candidate in candidates {
        if let Some(family) = family_by_id(candidate.trim()) {
            return family;
        }
    }
    family_by_id(DEFAULT_FAL_VIDEO_MODEL).expect("default FAL video family exists")
}

fn family_by_id(id: &str) -> Option<&'static FalVideoFamily> {
    FAL_VIDEO_FAMILIES.iter().find(|family| family.id == id)
}

fn configured_video_model_candidates() -> Vec<String> {
    let mut out = Vec::new();
    for path in [
        hermes_config::cli_config_path(),
        hermes_config::config_path(),
    ] {
        collect_video_model_candidates(&path, &mut out);
    }
    out
}

fn collect_video_model_candidates(path: &Path, out: &mut Vec<String>) {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else {
        return;
    };
    let Some(video_gen) = root.get("video_gen") else {
        return;
    };
    if let Some(model) = video_gen
        .get("fal")
        .and_then(|fal| fal.get("model"))
        .and_then(serde_yaml::Value::as_str)
    {
        out.push(model.to_string());
    }
    if let Some(model) = video_gen.get("model").and_then(serde_yaml::Value::as_str) {
        out.push(model.to_string());
    }
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.trim().parse::<u64>().ok()
}

async fn read_json_response(resp: reqwest::Response, label: &str) -> Result<Value, ToolError> {
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read {label} response: {e}")))?;
    if !status.is_success() {
        return Err(ToolError::ExecutionFailed(format!(
            "{label} error ({status}): {text}"
        )));
    }
    serde_json::from_str(&text)
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to parse {label} response: {e}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VideoArtifact {
    url: String,
    file_size: Option<u64>,
    content_type: Option<String>,
}

fn extract_video(value: &Value) -> Option<VideoArtifact> {
    if let Some(data) = value.get("data").filter(|data| data.is_object()) {
        if let Some(video) = extract_video(data) {
            return Some(video);
        }
    }
    let video = value.get("video")?;
    if let Some(url) = video.as_str().filter(|url| !url.trim().is_empty()) {
        return Some(VideoArtifact {
            url: url.to_string(),
            file_size: None,
            content_type: None,
        });
    }
    let obj = video.as_object()?;
    let url = obj.get("url")?.as_str()?.trim();
    if url.is_empty() {
        return None;
    }
    Some(VideoArtifact {
        url: url.to_string(),
        file_size: obj.get("file_size").and_then(Value::as_u64),
        content_type: obj
            .get("content_type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermes_config::managed_gateway::test_lock;
    use serde_json::json;

    struct EnvScope {
        _tmp: tempfile::TempDir,
        original: Vec<(&'static str, Option<String>)>,
        _g: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvScope {
        fn new() -> Self {
            let g = test_lock::lock();
            let tmp = tempfile::tempdir().unwrap();
            let keys = [
                "HERMES_HOME",
                "FAL_KEY",
                "FAL_VIDEO_MODEL",
                "FAL_VIDEO_TIMEOUT_SECONDS",
                "FAL_VIDEO_POLL_INTERVAL_SECONDS",
                "HERMES_ENABLE_NOUS_MANAGED_TOOLS",
                "TOOL_GATEWAY_USER_TOKEN",
                "TOOL_GATEWAY_DOMAIN",
                "TOOL_GATEWAY_SCHEME",
            ];
            let original = keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
            for k in keys {
                std::env::remove_var(k);
            }
            std::env::set_var("HERMES_HOME", tmp.path());
            Self {
                _tmp: tmp,
                original,
                _g: g,
            }
        }
    }

    impl Drop for EnvScope {
        fn drop(&mut self) {
            for (key, value) in &self.original {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn request(model: Option<&str>) -> VideoGenerateRequest {
        VideoGenerateRequest {
            prompt: "make a trailer".into(),
            model: model.map(ToOwned::to_owned),
            image_url: None,
            duration: None,
            aspect_ratio: "16:9".into(),
            resolution: "720p".into(),
            negative_prompt: None,
            audio: None,
            seed: None,
        }
    }

    #[test]
    fn resolve_family_prefers_explicit_then_env_then_default() {
        let _env = EnvScope::new();
        std::env::set_var("FAL_VIDEO_MODEL", "veo3.1");
        assert_eq!(resolve_family(Some("seedance-2.0")).id, "seedance-2.0");
        assert_eq!(resolve_family(None).id, "veo3.1");
        std::env::set_var("FAL_VIDEO_MODEL", "not-real");
        assert_eq!(resolve_family(None).id, DEFAULT_FAL_VIDEO_MODEL);
    }

    #[test]
    fn resolve_family_reads_config_candidates() {
        let _env = EnvScope::new();
        let config = hermes_config::config_path();
        std::fs::write(
            config,
            "video_gen:\n  fal:\n    model: kling-v3-4k\n  model: veo3.1\n",
        )
        .unwrap();
        assert_eq!(resolve_family(None).id, "kling-v3-4k");
    }

    #[test]
    fn payload_clamps_range_duration_and_uses_kling_start_image_key() {
        let family = family_by_id("kling-v3-4k").unwrap();
        let mut req = request(Some("kling-v3-4k"));
        req.image_url = Some("https://example.com/start.png".into());
        req.duration = Some(99);
        req.resolution = "1080p".into();
        req.audio = Some(true);
        let payload = build_payload(family, &req).payload;
        assert_eq!(
            payload.get("start_image_url"),
            Some(&json!("https://example.com/start.png"))
        );
        assert_eq!(payload.get("duration"), Some(&json!("15")));
        assert_eq!(payload.get("generate_audio"), Some(&json!(true)));
        assert!(payload.get("resolution").is_none());
    }

    #[test]
    fn payload_snaps_enum_duration_and_drops_unsupported_negative_prompt() {
        let family = family_by_id("seedance-2.0").unwrap();
        let mut req = request(Some("seedance-2.0"));
        req.duration = Some(2);
        req.negative_prompt = Some("low quality".into());
        req.aspect_ratio = "21:9".into();
        req.resolution = "480p".into();
        let meta = build_payload(family, &req);
        assert_eq!(meta.payload.get("duration"), Some(&json!("4")));
        assert_eq!(meta.payload.get("aspect_ratio"), Some(&json!("21:9")));
        assert_eq!(meta.payload.get("resolution"), Some(&json!("480p")));
        assert!(meta.payload.get("negative_prompt").is_none());
    }

    #[test]
    fn payload_uses_nearest_veo_duration() {
        let family = family_by_id("veo3.1").unwrap();
        let mut req = request(Some("veo3.1"));
        req.duration = Some(5);
        let meta = build_payload(family, &req);
        assert_eq!(meta.payload.get("duration"), Some(&json!("4")));
    }

    #[test]
    fn transport_urls_and_auth_match_direct_and_managed_modes() {
        let direct = FalVideoGenBackend::new("fal-key".into());
        assert_eq!(
            direct
                .transport
                .submit_url("fal-ai/pixverse/v6/text-to-video")
                .unwrap(),
            "https://queue.fal.run/fal-ai/pixverse/v6/text-to-video"
        );
        assert_eq!(direct.transport.auth_header().unwrap().1, "Key fal-key");

        let cfg = ManagedToolGatewayConfig {
            vendor: "fal-queue".into(),
            gateway_origin: "https://fal-queue.gw.example.com".into(),
            nous_user_token: "tok".into(),
            managed_mode: true,
        };
        let managed = FalVideoGenBackend::from_managed(&cfg);
        assert_eq!(
            managed
                .transport
                .submit_url("fal-ai/pixverse/v6/text-to-video")
                .unwrap(),
            "https://fal-queue.gw.example.com/run/fal-ai/pixverse/v6/text-to-video"
        );
        assert_eq!(managed.transport.auth_header().unwrap().1, "Bearer tok");
    }

    #[test]
    fn from_env_or_managed_prefers_direct_and_supports_managed() {
        let _env = EnvScope::new();
        std::env::set_var("FAL_KEY", "direct-key");
        assert_eq!(
            FalVideoGenBackend::from_env_or_managed()
                .unwrap()
                .transport_label(),
            "direct"
        );
        std::env::remove_var("FAL_KEY");
        std::env::set_var("HERMES_ENABLE_NOUS_MANAGED_TOOLS", "1");
        std::env::set_var("TOOL_GATEWAY_USER_TOKEN", "nous-token");
        assert_eq!(
            FalVideoGenBackend::from_env_or_managed()
                .unwrap()
                .transport_label(),
            "managed"
        );
    }

    #[tokio::test]
    async fn unconfigured_backend_errors_before_network() {
        let backend = FalVideoGenBackend::unconfigured();
        let err = backend.generate_video(request(None)).await.unwrap_err();
        assert!(err.to_string().contains("FAL_KEY"));
    }

    #[test]
    fn extract_video_handles_wrapped_and_string_shapes() {
        assert_eq!(
            extract_video(&json!({"data":{"video":{"url":"https://cdn.example/v.mp4","file_size":42,"content_type":"video/mp4"}}}))
                .unwrap(),
            VideoArtifact {
                url: "https://cdn.example/v.mp4".into(),
                file_size: Some(42),
                content_type: Some("video/mp4".into()),
            }
        );
        assert_eq!(
            extract_video(&json!({"video":"https://cdn.example/v.mp4"}))
                .unwrap()
                .url,
            "https://cdn.example/v.mp4"
        );
    }
}
