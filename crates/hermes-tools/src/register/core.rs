//! Core / infrastructure tool registrations.
//!
//! These tools are always available regardless of environment configuration.
//! Session-search and todo use filesystem state under hermes_home().

use std::sync::Arc;

use super::{RegistryContext, hermes_data_dir, reg};

pub fn register(ctx: &RegistryContext<'_>) {
    register_skills(ctx);
    register_memory(ctx);
    register_session_search(ctx);
    register_todo(ctx);
    register_clarify(ctx);
    register_code_execution(ctx);
    register_delegation(ctx);
    register_cronjob(ctx);
    register_dashboard(ctx);
    register_security(ctx);
    register_system(ctx);
}

fn register_skills(ctx: &RegistryContext<'_>) {
    reg(
        ctx,
        "skills",
        Arc::new(crate::tools::skills::SkillsListHandler::new(
            ctx.skill_provider.clone(),
        )),
        "📚",
        vec![],
    );
    reg(
        ctx,
        "skills",
        Arc::new(crate::tools::skills::SkillViewHandler::new(
            ctx.skill_provider.clone(),
        )),
        "📖",
        vec![],
    );
    reg(
        ctx,
        "skills",
        Arc::new(crate::tools::skills::SkillManageHandler::new(
            ctx.skill_provider.clone(),
        )),
        "⚙️",
        vec![],
    );
}

fn register_memory(ctx: &RegistryContext<'_>) {
    reg(
        ctx,
        "memory",
        Arc::new(crate::tools::memory::MemoryHandler::new(Arc::new(
            crate::backends::memory::FileMemoryBackend::new(),
        ))),
        "🧠",
        vec![],
    );
}

fn register_session_search(ctx: &RegistryContext<'_>) {
    let db_path = hermes_config::state_db_path();
    match crate::backends::session_search::SqliteSessionSearchBackend::new(
        &db_path.to_string_lossy(),
    )
    .or_else(|_| crate::backends::session_search::SqliteSessionSearchBackend::default_path())
    {
        Ok(backend) => {
            reg(
                ctx,
                "session_search",
                Arc::new(crate::tools::session_search::SessionSearchHandler::new(
                    Arc::new(backend),
                )),
                "🔍",
                vec![],
            );
        }
        Err(_) => {
            tracing::warn!("Failed to initialise session search DB; skipping session_search tool");
        }
    }
}

fn register_todo(ctx: &RegistryContext<'_>) {
    let todo_path = hermes_data_dir().join("todos.json");
    reg(
        ctx,
        "todo",
        Arc::new(crate::tools::todo::TodoHandler::new(Arc::new(
            crate::backends::todo::FileTodoBackend::new(todo_path),
        ))),
        "📋",
        vec![],
    );
}

fn register_clarify(ctx: &RegistryContext<'_>) {
    reg(
        ctx,
        "clarify",
        Arc::new(crate::tools::clarify::ClarifyHandler::new(Arc::new(
            crate::backends::clarify::SignalClarifyBackend::new(),
        ))),
        "❓",
        vec![],
    );
}

fn register_code_execution(ctx: &RegistryContext<'_>) {
    reg(
        ctx,
        "code_execution",
        Arc::new(crate::tools::code_execution::ExecuteCodeHandler::new(
            Arc::new(
                crate::backends::code_execution::LocalCodeExecutionBackend::with_tool_registry(
                    Arc::new(ctx.registry.clone()),
                ),
            ),
        )),
        "🖥️",
        vec![],
    );
}

fn register_delegation(ctx: &RegistryContext<'_>) {
    reg(
        ctx,
        "delegation",
        Arc::new(crate::tools::delegation::DelegateTaskHandler::new(
            Arc::new(crate::backends::delegation::SignalDelegationBackend::new()),
        )),
        "🤝",
        vec![],
    );
}

fn register_cronjob(ctx: &RegistryContext<'_>) {
    reg(
        ctx,
        "cronjob",
        Arc::new(crate::tools::cronjob::CronjobHandler::new(Arc::new(
            crate::backends::cronjob::SignalCronjobBackend::new(),
        ))),
        "⏰",
        vec![],
    );
}

fn register_dashboard(ctx: &RegistryContext<'_>) {
    reg(
        ctx,
        "dashboard",
        Arc::new(crate::tools::dashboard_control::DashboardControlHandler),
        "🖥️",
        vec![],
    );
}

fn register_security(ctx: &RegistryContext<'_>) {
    reg(
        ctx,
        "security",
        Arc::new(crate::tools::osv_check::OsvCheckHandler),
        "🛡️",
        vec![],
    );
    reg(
        ctx,
        "security",
        Arc::new(crate::tools::url_safety::UrlSafetyHandler::default()),
        "🔒",
        vec![],
    );
}

fn register_system(ctx: &RegistryContext<'_>) {
    reg(
        ctx,
        "system",
        Arc::new(crate::tools::env_passthrough::EnvPassthroughHandler),
        "🔧",
        vec![],
    );
    reg(
        ctx,
        "system",
        Arc::new(crate::tools::credential_files::CredentialFilesHandler),
        "🔑",
        vec![],
    );
    reg(
        ctx,
        "system",
        Arc::new(crate::tools::tool_result_storage::ToolResultStorageHandler::default()),
        "💾",
        vec![],
    );
}
