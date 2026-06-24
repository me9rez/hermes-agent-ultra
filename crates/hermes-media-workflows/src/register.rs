//! Register Flowy media backends and workflow tools into the tool registry.

use std::sync::Arc;

use hermes_config::{GatewayConfig, flowy_media_exposed};
use hermes_core::{ToolHandler, ToolSchema};
use hermes_tools::ToolRegistry;
use hermes_tools::{ImageGenerateHandler, VideoGenerateHandler};

use crate::backends::FlowyMediaServices;
use crate::backends::flowy_image::FlowyImageGenBackend;
use crate::backends::flowy_video::FlowyVideoGenBackend;
use crate::tool_schemas::{flowy_image_generate_schema, flowy_video_generate_schema};
use crate::tools::{MediaWorkflowPlanHandler, MediaWorkflowRunHandler, MediaWorkflowStatusHandler};
use crate::workflows::store::WorkflowRunStore;

fn flowy_media_check_fn() -> Arc<dyn Fn() -> bool + Send + Sync> {
    Arc::new(|| hermes_config::flowy_media_exposed_from_disk())
}

/// Wire Flowy image/video backends and workflow tools when server login is available.
pub fn wire_flowy_media(
    registry: &ToolRegistry,
    config: &GatewayConfig,
    hermes_home: &std::path::Path,
) {
    if !flowy_media_exposed(config) {
        tracing::debug!(
            provider = %config.media.provider,
            server_base_url = %config.server.base_url,
            "Flowy media wiring skipped (provider != flowy or server.base_url missing)"
        );
        return;
    }

    let Some(services) = FlowyMediaServices::try_new(config, hermes_home) else {
        tracing::warn!("Flowy media services could not be initialized");
        return;
    };

    let check = flowy_media_check_fn();
    register_overwrite(
        registry,
        "image_gen",
        Arc::new(ImageGenerateHandler::new(Arc::new(
            FlowyImageGenBackend::new(services.clone()),
        ))),
        flowy_image_generate_schema(),
        "🎨",
        Arc::clone(&check),
    );

    register_overwrite(
        registry,
        "video_gen",
        Arc::new(VideoGenerateHandler::new(Arc::new(
            FlowyVideoGenBackend::new(services.clone()),
        ))),
        flowy_video_generate_schema(),
        "🎞️",
        Arc::clone(&check),
    );

    if !config.media.workflows.enabled {
        tracing::info!("Flowy image/video backends registered (workflows disabled)");
        return;
    }

    let store = Arc::new(WorkflowRunStore::new());
    let plan_handler = Arc::new(MediaWorkflowPlanHandler::new(config.media.clone()));
    register_overwrite(
        registry,
        "media_workflow",
        plan_handler.clone(),
        plan_handler.schema(),
        "🎬",
        Arc::clone(&check),
    );
    let run_handler = Arc::new(MediaWorkflowRunHandler::new(services, store.clone()));
    register_overwrite(
        registry,
        "media_workflow",
        run_handler.clone(),
        run_handler.schema(),
        "🎬",
        Arc::clone(&check),
    );
    let status_handler = Arc::new(MediaWorkflowStatusHandler::new(store));
    register_overwrite(
        registry,
        "media_workflow",
        status_handler.clone(),
        status_handler.schema(),
        "🎬",
        Arc::clone(&check),
    );

    tracing::info!("Flowy media backends and workflow tools registered");
}

fn register_overwrite(
    registry: &ToolRegistry,
    toolset: &str,
    handler: Arc<dyn ToolHandler>,
    schema: ToolSchema,
    emoji: &str,
    check_fn: Arc<dyn Fn() -> bool + Send + Sync>,
) {
    let name = schema.name.clone();
    let desc = schema.description.clone();
    registry.register(
        name,
        toolset,
        schema,
        handler,
        check_fn,
        vec![],
        true,
        desc,
        emoji,
        None,
    );
}
