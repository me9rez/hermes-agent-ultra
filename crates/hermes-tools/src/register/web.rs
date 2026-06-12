//! Web, content-framework, and capture tool registrations.
//!
//! Preconditions: no hard env requirements; web_search backend chosen from env
//! (Exa key preferred, DuckDuckGo as fallback).
//!
//! web_search and web_extract require the `web` Cargo feature (enabled by default via `full`).

use std::sync::Arc;

use super::{RegistryContext, reg};

pub fn register(ctx: &RegistryContext<'_>) {
    #[cfg(feature = "web")]
    reg(
        ctx,
        "web",
        Arc::new(crate::tools::web::WebSearchHandler::new(
            crate::backends::web::search_backend_from_env_or_fallback(),
        )),
        "🔍",
        vec![],
    );
    #[cfg(feature = "web")]
    reg(
        ctx,
        "web",
        Arc::new(crate::tools::web::WebExtractHandler::new(Box::new(
            crate::backends::web::SimpleExtractBackend::new(),
        ))),
        "📄",
        vec![],
    );

    reg(
        ctx,
        "content",
        Arc::new(crate::tools::content_framework::ContentPlanHandler),
        "🧭",
        vec![],
    );
    reg(
        ctx,
        "content",
        Arc::new(crate::tools::content_framework::ContentNormalizeHandler),
        "🧩",
        vec![],
    );
    reg(
        ctx,
        "content",
        Arc::new(crate::tools::content_framework::ContentExecuteHandler),
        "▶️",
        vec![],
    );

    reg(
        ctx,
        "capture",
        Arc::new(crate::tools::capture::CaptureHandler),
        "📥",
        vec![],
    );
}
