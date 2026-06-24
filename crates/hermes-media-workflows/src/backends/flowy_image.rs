//! Flowy server image generation backend.

use async_trait::async_trait;
use serde_json::{Value, json};

use hermes_core::ToolError;
use hermes_server_client::flowy::ImageGenerationRequest;
use hermes_tools::ImageGenBackend;
use hermes_tools::tools::image_gen::ImageGenRequest;

use super::{FlowyMediaServices, map_server_err};
use crate::assets::{extract_image_urls, persist_data_url, persist_from_url};

pub struct FlowyImageGenBackend {
    services: FlowyMediaServices,
}

impl FlowyImageGenBackend {
    pub fn new(services: FlowyMediaServices) -> Self {
        Self { services }
    }

    pub async fn is_configured(services: &FlowyMediaServices) -> bool {
        services.is_authenticated().await
    }
}

#[async_trait]
impl ImageGenBackend for FlowyImageGenBackend {
    async fn generate(&self, request: ImageGenRequest) -> Result<String, ToolError> {
        self.services.require_token().await?;

        let model = self
            .services
            .resolve_image_model(request.model.as_deref())
            .await?;

        let flowy_req = ImageGenerationRequest {
            model: model.clone(),
            prompt: request.prompt.clone(),
            image_url: request.image_url.clone(),
            extra: request.extra.unwrap_or(Value::Null),
        };

        let upstream = self
            .services
            .api
            .generate_image(&self.services.session, &flowy_req)
            .await
            .map_err(map_server_err)?;

        let mut artifacts = Vec::new();
        let urls = extract_image_urls(&upstream);
        if urls.is_empty() {
            if let Some(data_url) = find_data_url(&upstream) {
                let artifact = persist_data_url(&data_url, "flowy", &model).await?;
                artifacts.push(artifact);
            }
        } else {
            for url in urls {
                if self.services.media.image.save_locally {
                    match persist_from_url(&url, "flowy", &model).await {
                        Ok(artifact) => artifacts.push(artifact),
                        Err(err) => {
                            tracing::warn!(
                                error = %err,
                                url = %url,
                                "image generated but local persist failed; keeping remote URL"
                            );
                            artifacts.push(crate::assets::MediaArtifact {
                                local_path: std::path::PathBuf::new(),
                                remote_url: Some(url),
                                mime: "image/png".into(),
                                width: None,
                                height: None,
                                duration_secs: None,
                                provider: "flowy".into(),
                                model: model.clone(),
                                job_id: uuid::Uuid::new_v4().to_string(),
                            });
                        }
                    }
                } else {
                    artifacts.push(crate::assets::MediaArtifact {
                        local_path: std::path::PathBuf::new(),
                        remote_url: Some(url),
                        mime: "image/png".into(),
                        width: None,
                        height: None,
                        duration_secs: None,
                        provider: "flowy".into(),
                        model: model.clone(),
                        job_id: uuid::Uuid::new_v4().to_string(),
                    });
                }
            }
        }

        if artifacts.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "image API returned no downloadable URLs".into(),
            ));
        }

        let images: Vec<Value> = artifacts
            .iter()
            .map(|a| {
                let mut obj = json!({
                    "url": a.remote_url,
                    "provider": a.provider,
                    "model": a.model,
                    "job_id": a.job_id,
                });
                if !a.local_path.as_os_str().is_empty() {
                    obj["local_path"] = json!(a.local_path.to_string_lossy());
                }
                obj
            })
            .collect();

        Ok(json!({
            "success": true,
            "images": images,
            "transport": "flowy",
            "model": model,
            "upstream": upstream,
            "media_hint": artifacts.first().map(|a| a.media_tag()),
        })
        .to_string())
    }
}

fn find_data_url(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if s.starts_with("data:image/") => Some(s.clone()),
        Value::Array(arr) => arr.iter().find_map(find_data_url),
        Value::Object(map) => map.values().find_map(find_data_url),
        _ => None,
    }
}
