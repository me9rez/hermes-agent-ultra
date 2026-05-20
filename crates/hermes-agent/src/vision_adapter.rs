//! [`VisionBackend`] adapter that routes `vision_analyze` through [`AuxiliaryClient`].

use std::sync::Arc;

use async_trait::async_trait;
use hermes_core::{Message, ToolError};
use hermes_intelligence::auxiliary::{
    AuxiliaryClient, AuxiliaryError, AuxiliaryRequest, AuxiliaryTask,
};
use hermes_intelligence::vision_media;
use hermes_tools::VisionBackend;
use serde_json::json;
use tracing::debug;

/// Routes vision tool calls through the auxiliary provider chain (not raw OpenAI env).
pub struct AuxiliaryVisionAdapter {
    client: Arc<AuxiliaryClient>,
    primary_provider: Option<String>,
    primary_model: Option<String>,
}

impl AuxiliaryVisionAdapter {
    pub fn new(client: Arc<AuxiliaryClient>) -> Self {
        Self {
            client,
            primary_provider: None,
            primary_model: None,
        }
    }

    pub fn with_primary_context(
        mut self,
        provider: Option<String>,
        model: Option<String>,
    ) -> Self {
        self.primary_provider = provider;
        self.primary_model = model;
        self
    }
}

fn map_auxiliary_err(err: AuxiliaryError) -> ToolError {
    ToolError::ExecutionFailed(err.to_string())
}

#[async_trait]
impl VisionBackend for AuxiliaryVisionAdapter {
    async fn analyze(&self, image_url: &str, question: &str) -> Result<String, ToolError> {
        let image_part = vision_media::encode_image_url_part(image_url)
            .await
            .map_err(ToolError::ExecutionFailed)?;
        let parts = vec![
            json!({"type": "text", "text": question}),
            image_part,
        ];
        let serialized =
            serde_json::to_string(&parts).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        const PREFIX: &str = "__hermes_acp_parts_json__:";
        let messages = vec![Message::user(format!("{PREFIX}{serialized}"))];
        let request = AuxiliaryRequest::new(AuxiliaryTask::Vision, messages);
        let response = self.client.call(request).await.map_err(map_auxiliary_err)?;
        let same_model = self
            .primary_provider
            .as_deref()
            .zip(self.primary_model.as_deref())
            .map(|(p, m)| response.provider_label == p && response.model == m)
            .unwrap_or(false);
        debug!(
            primary_provider = ?self.primary_provider,
            primary_model = ?self.primary_model,
            auxiliary_label = %response.provider_label,
            auxiliary_model = %response.model,
            same_model,
            "auxiliary vision attempt"
        );
        Ok(response
            .text()
            .unwrap_or("No analysis available")
            .to_string())
    }
}
