//! Session rotation and transcript reset across core/session/stream shards.

use uuid::Uuid;

use super::App;
use super::state::SessionState;

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
    app.notify_memory_session_switch(
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
}
