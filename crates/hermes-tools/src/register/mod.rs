//! Literate registration of built-in tools by tool-family modules.
//!
//! Each sub-module is responsible for one cohesive family of tools and
//! documents its own preconditions (API keys, environment flags, etc.).
//! The public entry point is [`register_all`].

pub mod browser;
pub mod core;
pub mod file;
pub mod integrations;
pub mod media;
pub mod terminal;
pub mod vibe;
pub mod web;

use std::path::PathBuf;
use std::sync::Arc;

use hermes_config::voice::{SttConfig, TtsConfig};
use hermes_core::{SkillProvider, TerminalBackend, ToolHandler};

use crate::ToolRegistry;

/// All shared dependencies required to register built-in tools.
pub struct RegistryContext<'a> {
    pub registry: &'a ToolRegistry,
    pub terminal_backend: Arc<dyn TerminalBackend>,
    pub skill_provider: Arc<dyn SkillProvider>,
    pub vision_backend: Option<Arc<dyn crate::tools::vision::VisionBackend>>,
    pub tts_cfg: Option<TtsConfig>,
    pub stt_cfg: Option<SttConfig>,
    pub terminal_check: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// Register all built-in tool families into the registry.
pub fn register_all(ctx: &RegistryContext<'_>) {
    web::register(ctx);
    file::register(ctx);
    terminal::register(ctx);
    media::register(ctx);
    browser::register(ctx);
    integrations::register(ctx);
    core::register(ctx);
    vibe::register(ctx);

    tracing::info!(
        tool_count = ctx.registry.list_tools().len(),
        "Registered built-in tools"
    );
}

pub(super) fn hermes_data_dir() -> PathBuf {
    let dir = hermes_config::hermes_home();
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub(super) fn reg(
    ctx: &RegistryContext<'_>,
    toolset: &str,
    handler: Arc<dyn ToolHandler>,
    emoji: &str,
    env_deps: Vec<String>,
) {
    let schema = handler.schema();
    let name = schema.name.clone();
    let desc = schema.description.clone();
    ctx.registry.register(
        name,
        toolset,
        schema,
        handler,
        Arc::new(|| true),
        env_deps,
        true,
        desc,
        emoji,
        None,
    );
}

pub(super) fn reg_with_check(
    ctx: &RegistryContext<'_>,
    toolset: &str,
    handler: Arc<dyn ToolHandler>,
    emoji: &str,
    env_deps: Vec<String>,
    check_fn: Arc<dyn Fn() -> bool + Send + Sync>,
) {
    let schema = handler.schema();
    let name = schema.name.clone();
    let desc = schema.description.clone();
    ctx.registry.register(
        name, toolset, schema, handler, check_fn, env_deps, true, desc, emoji, None,
    );
}
