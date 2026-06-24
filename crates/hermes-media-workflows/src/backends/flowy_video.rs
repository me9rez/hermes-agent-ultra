//! Flowy Seedance video generation backend.

use async_trait::async_trait;
use serde_json::json;

use hermes_core::ToolError;
use hermes_server_client::flowy::video_task_failure_message;
use hermes_tools::VideoGenerateBackend;
use hermes_tools::tools::video::VideoGenerateRequest;

use super::{FlowyMediaServices, map_server_err};
use crate::assets::persist_from_url;
use crate::flowy_params::{normalize_video_duration, normalize_video_resolution};

pub struct FlowyVideoGenBackend {
    services: FlowyMediaServices,
}

impl FlowyVideoGenBackend {
    pub fn new(services: FlowyMediaServices) -> Self {
        Self { services }
    }

    pub async fn is_configured(services: &FlowyMediaServices) -> bool {
        services.is_authenticated().await
    }
}

#[async_trait]
impl VideoGenerateBackend for FlowyVideoGenBackend {
    async fn generate_video(&self, request: VideoGenerateRequest) -> Result<String, ToolError> {
        self.services.require_token().await?;

        let model = self
            .services
            .resolve_video_model(request.model.as_deref())
            .await?;

        let image_url = request
            .image_url
            .or_else(|| request.reference_image_urls.first().cloned());

        let raw_duration = request
            .duration
            .or(Some(self.services.media.video.default_duration));
        let duration = raw_duration.map(|d| normalize_video_duration(&model, d));

        let aspect_ratio = if request.aspect_ratio.trim().is_empty() {
            self.services.media.video.default_aspect_ratio.clone()
        } else {
            request.aspect_ratio.clone()
        };

        let resolution_input = if request.resolution.trim().is_empty() {
            self.services.media.video.default_resolution.as_str()
        } else {
            request.resolution.as_str()
        };
        let resolution = normalize_video_resolution(&model, resolution_input);

        let body = hermes_server_client::FlowyApiClient::build_video_create_body(
            &model,
            &request.prompt,
            image_url.as_deref(),
            duration,
            aspect_ratio.as_str(),
            resolution.as_deref(),
            request.negative_prompt.as_deref(),
            request.seed,
            false,
        );

        let poll_timeout = self.services.media.video.poll_timeout_seconds.max(30);
        let record = self
            .services
            .api
            .generate_video_with_timeout(&self.services.session, body, poll_timeout)
            .await
            .map_err(map_server_err)?;

        if !record.is_success() {
            return Err(ToolError::ExecutionFailed(video_task_failure_message(
                &record,
            )));
        }

        let video_url = record.video_url().ok_or_else(|| {
            ToolError::ExecutionFailed("video task succeeded but no video_url in result".into())
        })?;

        let mut local_path = String::new();
        let mut persist_warning: Option<String> = None;
        if self.services.media.video.save_locally {
            match persist_from_url(&video_url, "flowy", &model).await {
                Ok(artifact) => {
                    local_path = artifact.local_path.to_string_lossy().to_string();
                }
                Err(err) => {
                    persist_warning = Some(err.to_string());
                    tracing::warn!(
                        error = %err,
                        video_url = %video_url,
                        "video generated but local persist failed; returning remote URL"
                    );
                }
            }
        }

        Ok(json!({
            "success": true,
            "video": video_url,
            "local_path": if local_path.is_empty() { serde_json::Value::Null } else { json!(local_path) },
            "provider": "flowy",
            "model": model,
            "task_id": record.id,
            "upstream_task_id": record.task_id,
            "status": record.status,
            "persist_warning": persist_warning,
            "media_hint": if local_path.is_empty() { serde_json::Value::Null } else { json!(format!("MEDIA:{local_path}")) },
            "delivery_note": if local_path.is_empty() {
                json!("Video URL is available; share the link or retry download if MEDIA: path is needed.")
            } else {
                serde_json::Value::Null
            },
        })
        .to_string())
    }
}
