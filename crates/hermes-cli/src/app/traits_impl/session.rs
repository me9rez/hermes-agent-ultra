use std::path::Path;

use async_trait::async_trait;
use hermes_core::AgentError;

use super::super::traits::{SessionRuntime, SessionRuntimeAsync};
use super::super::{App, UiTranscriptMessage};

impl SessionRuntime for App {
    fn state_root(&self) -> &Path {
        &self.state_root
    }

    fn session_id(&self) -> &str {
        &self.session.session_id
    }

    fn session_id_mut(&mut self) -> &mut String {
        &mut self.session.session_id
    }

    fn messages(&self) -> &[hermes_core::Message] {
        &self.session.messages
    }

    fn messages_mut(&mut self) -> &mut Vec<hermes_core::Message> {
        &mut self.session.messages
    }

    fn ui_messages(&self) -> &[UiTranscriptMessage] {
        &self.session.ui_messages
    }

    fn ui_messages_mut(&mut self) -> &mut Vec<UiTranscriptMessage> {
        &mut self.session.ui_messages
    }

    fn session_objective(&self) -> Option<&str> {
        self.session.session_objective.as_deref()
    }

    fn set_session_objective(&mut self, objective: Option<String>) {
        self.session.set_session_objective(objective);
    }

    fn input_history(&self) -> &[String] {
        &self.session.input_history
    }

    fn input_history_mut(&mut self) -> &mut Vec<String> {
        &mut self.session.input_history
    }

    fn history_index_mut(&mut self) -> &mut usize {
        &mut self.session.history_index
    }

    fn notify_memory_session_switch(
        &self,
        new_session_id: &str,
        parent_session_id: &str,
        reset: bool,
        reason: &str,
    ) {
        self.core
            .notify_memory_session_switch(new_session_id, parent_session_id, reset, reason);
    }

    fn new_session(&mut self) {
        super::super::session_lifecycle::new_session(self);
    }

    fn reset_session(&mut self) {
        super::super::session_lifecycle::new_session(self);
    }

    fn undo_last(&mut self) -> Option<String> {
        App::undo_last(self)
    }

    fn undo_last_n(&mut self, user_turns: usize) -> Option<String> {
        App::undo_last_n(self, user_turns)
    }

    fn sync_agent_runtime_session_id(&self, session_id: &str) {
        self.core.agent.set_runtime_session_id(session_id);
    }

    fn history_prev(&mut self) -> Option<&str> {
        self.session.history_prev()
    }

    fn history_next(&mut self) -> Option<&str> {
        self.session.history_next()
    }
}

#[async_trait]
impl SessionRuntimeAsync for App {
    async fn retry_last(&mut self) -> Result<(), AgentError> {
        App::retry_last(self).await
    }

    async fn compress_conversation_context(&mut self) -> Result<(usize, usize, bool), AgentError> {
        App::compress_conversation_context(self).await
    }
}
