//! Vision backend placeholder — real implementation is injected via `register_builtin_tools_with_vision`.

use async_trait::async_trait;

use crate::tools::vision::VisionBackend;
use hermes_core::ToolError;

/// Fallback when no auxiliary vision adapter was wired at startup.
pub struct UnconfiguredVisionBackend;

#[async_trait]
impl VisionBackend for UnconfiguredVisionBackend {
    async fn analyze(&self, _image_url: &str, _question: &str) -> Result<String, ToolError> {
        Err(ToolError::ExecutionFailed(
            "vision_analyze is not configured: start Hermes with an auxiliary LLM provider \
             (e.g. OPENROUTER_API_KEY or HERMES_OPENAI_API_KEY)"
                .into(),
        ))
    }
}
