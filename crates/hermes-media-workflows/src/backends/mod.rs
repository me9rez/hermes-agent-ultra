//! Shared Flowy media service handle.

use std::sync::Arc;

use hermes_config::{GatewayConfig, MediaGenConfig};
use hermes_server_client::{
    ClawModelEntry, FlowyApiClient, MODEL_CATEGORY_IMAGE, MODEL_CATEGORY_VIDEO, ServerSession,
    resolve_model_in_catalog,
};

use crate::flowy_params::is_flowy_model_id;

/// Runtime handle for Flowy image/video APIs (login token + config).
#[derive(Clone)]
pub struct FlowyMediaServices {
    pub api: Arc<FlowyApiClient>,
    pub session: ServerSession,
    pub media: MediaGenConfig,
}

impl FlowyMediaServices {
    pub fn try_new(config: &GatewayConfig, hermes_home: &std::path::Path) -> Option<Self> {
        if !config.media.uses_flowy() || !config.server.api_ready() {
            return None;
        }
        let api = FlowyApiClient::new(&config.server).ok()?;
        let session = ServerSession::from_config(&config.server, hermes_home);
        Some(Self {
            api: Arc::new(api),
            session,
            media: config.media.clone(),
        })
    }

    pub async fn is_authenticated(&self) -> bool {
        self.session
            .access_token()
            .await
            .ok()
            .flatten()
            .is_some_and(|t| !t.trim().is_empty())
    }

    pub async fn fetch_image_models(&self) -> Result<Vec<ClawModelEntry>, hermes_core::ToolError> {
        self.require_token().await?;
        let models = self
            .api
            .get_available_models_claw(&self.session, Some(MODEL_CATEGORY_IMAGE))
            .await
            .map_err(map_server_err)?;
        Ok(models.cloud)
    }

    pub async fn fetch_video_models(&self) -> Result<Vec<ClawModelEntry>, hermes_core::ToolError> {
        self.require_token().await?;
        let models = self
            .api
            .get_available_models_claw(&self.session, Some(MODEL_CATEGORY_VIDEO))
            .await
            .map_err(map_server_err)?;
        Ok(models.cloud)
    }

    /// Resolve image `model` for API requests (list `id` from `availableListClaw`).
    pub async fn resolve_image_model(
        &self,
        agent_model: Option<&str>,
    ) -> Result<String, hermes_core::ToolError> {
        let catalog = self.fetch_image_models().await?;
        self.resolve_model_in_catalog(
            agent_model,
            self.media.image.model.as_str(),
            &catalog,
            "image",
        )
    }

    /// Resolve video `model` for API requests (list `id` from `availableListClaw`).
    pub async fn resolve_video_model(
        &self,
        agent_model: Option<&str>,
    ) -> Result<String, hermes_core::ToolError> {
        let catalog = self.fetch_video_models().await?;
        self.resolve_model_in_catalog(
            agent_model,
            self.media.video.model.as_str(),
            &catalog,
            "video",
        )
    }

    fn resolve_model_in_catalog(
        &self,
        agent_model: Option<&str>,
        configured: &str,
        catalog: &[ClawModelEntry],
        kind: &str,
    ) -> Result<String, hermes_core::ToolError> {
        if let Some(m) = agent_model.map(str::trim).filter(|s| !s.is_empty()) {
            if is_flowy_model_id(m) || resolve_model_in_catalog(m, catalog).is_some() {
                if let Some(resolved) = resolve_model_in_catalog(m, catalog) {
                    return Ok(resolved);
                }
            } else {
                tracing::warn!(
                    agent_model = m,
                    "ignoring non-Flowy model id from tool call; using configured default"
                );
            }
        }

        let configured = configured.trim();
        if !configured.is_empty() {
            if let Some(resolved) = resolve_model_in_catalog(configured, catalog) {
                return Ok(resolved);
            }
            return Err(hermes_core::ToolError::ExecutionFailed(format!(
                "configured {kind} model '{configured}' not found in server catalog — run `hermes media models pick {kind}`"
            )));
        }

        catalog.first().map(|m| m.api_model_id()).ok_or_else(|| {
            hermes_core::ToolError::ExecutionFailed(format!(
                "no {kind} models available — check login and credits"
            ))
        })
    }

    pub async fn default_image_model(&self) -> Result<String, hermes_core::ToolError> {
        self.resolve_image_model(None).await
    }

    pub async fn default_video_model(&self) -> Result<String, hermes_core::ToolError> {
        self.resolve_video_model(None).await
    }

    pub async fn require_token(&self) -> Result<String, hermes_core::ToolError> {
        self.session
            .access_token()
            .await
            .map_err(|e| hermes_core::ToolError::ExecutionFailed(e.to_string()))?
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                hermes_core::ToolError::ExecutionFailed(
                    "not logged in — run `hermes server login` first".into(),
                )
            })
    }
}

pub fn map_server_err(err: hermes_server_client::ServerClientError) -> hermes_core::ToolError {
    hermes_core::ToolError::ExecutionFailed(err.to_string())
}

pub mod flowy_image;
pub mod flowy_video;
