use super::*;

impl AgentLoop {
    /// Run Python-parity compression on a standalone message list (CLI `/compress`).
    pub async fn compress_messages(
        &self,
        messages: Vec<Message>,
        session_id: &str,
        model: &str,
    ) -> (Vec<Message>, bool) {
        self.set_runtime_session_id(session_id);
        if let Ok(mut guard) = self.config_runtime.write() {
            let m = model.trim();
            if !m.is_empty() {
                let mut updated = (*guard).as_ref().clone();
                updated.model = m.to_string();
                *guard = Arc::new(updated);
            }
        }
        let mut ctx = ContextManager::for_model(model);
        ctx.replace_messages(messages);
        let compressed = self.compress_context(&mut ctx).await;
        (ctx.get_messages().to_vec(), compressed)
    }

    pub(crate) fn memory_on_session_end(&self, messages: &[Message]) {
        self.interest_on_session_end(messages);
        if self.config().skip_memory {
            return;
        }
        let Some(ref mm) = self.memory_manager else {
            return;
        };
        let Ok(mm) = mm.lock() else {
            return;
        };
        let as_values: Vec<Value> = messages
            .iter()
            .filter_map(|m| serde_json::to_value(m).ok())
            .collect();
        mm.on_session_end(&as_values);
    }

    /// Full session teardown: memory providers + plugin `on_session_end`.
    pub fn session_end_hooks(
        &self,
        messages: &[Message],
        completed: bool,
        interrupted: bool,
        total_turns: u32,
        session_started_hooks_fired: bool,
    ) {
        crate::hooks::session_end_hooks(
            self,
            messages,
            completed,
            interrupted,
            total_turns,
            session_started_hooks_fired,
        );
    }
}
