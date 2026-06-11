//! Session rotation and transcript reset across core/session/stream shards.

use uuid::Uuid;

use super::App;
use super::state::{AgentCore, SessionState};

const SESSION_OBJECTIVE_PREFIX: &str = "[SESSION_OBJECTIVE] ";

impl SessionState {
    pub(super) fn rotate_session_id(&mut self) -> String {
        let old_session_id = self.session_id.clone();
        self.session_id = Uuid::new_v4().to_string();
        old_session_id
    }

    pub(super) fn clear_for_new_session(&mut self) {
        self.messages.clear();
        self.ui_messages.clear();
        self.session_objective = None;
        self.clear_input_history();
    }

    pub(super) fn prune_ui_after_current_messages(&mut self) {
        let cap = self.messages.len();
        self.ui_messages.retain(|m| m.insert_at <= cap);
    }

    pub(super) fn set_session_objective(&mut self, objective: Option<String>) {
        self.messages.retain(|m| {
            if m.role != hermes_core::MessageRole::System {
                return true;
            }
            !m.content
                .as_deref()
                .unwrap_or_default()
                .starts_with(SESSION_OBJECTIVE_PREFIX)
        });

        self.session_objective = objective
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(obj) = &self.session_objective {
            let system = hermes_core::Message::system(format!("{SESSION_OBJECTIVE_PREFIX}{obj}"));
            self.messages.insert(0, system);
        }
        self.prune_ui_after_current_messages();
    }
}

impl AgentCore {
    pub fn notify_memory_session_switch(
        &self,
        new_session_id: &str,
        parent_session_id: &str,
        reset: bool,
        reason: &str,
    ) {
        self.agent.set_runtime_session_id(new_session_id);
        self.agent
            .memory_on_session_switch(new_session_id, parent_session_id, reset, reason);
    }
}

pub(super) fn new_session(app: &mut App) {
    app.flush_session_teardown(false);
    let old_session_id = app.session.rotate_session_id();
    app.core
        .agent
        .set_runtime_session_id(&app.session.session_id);
    app.core.agent.reset_session_state(None, None, false);
    app.core.agent.reset_session_db_flush_cursor();
    app.core.agent.invalidate_cached_system_prompt();
    app.core.notify_memory_session_switch(
        &app.session.session_id,
        &old_session_id,
        true,
        "new_session",
    );
    app.session.clear_for_new_session();
    app.stream.pending_image_hint = None;
    app.ensure_session_stub_snapshot();
}

impl App {
    pub fn new_session(&mut self) {
        new_session(self);
    }

    pub fn reset_session(&mut self) {
        new_session(self);
    }

    pub fn set_session_objective(&mut self, objective: Option<String>) {
        self.session.set_session_objective(objective);
    }

    pub fn notify_memory_session_switch(
        &self,
        new_session_id: &str,
        parent_session_id: &str,
        reset: bool,
        reason: &str,
    ) {
        self.core
            .notify_memory_session_switch(new_session_id, parent_session_id, reset, reason);
    }
}
