use super::*;

impl AgentLoop {
    fn interest_prefetch_block(&self, query: &str) -> String {
        if !self.config().interest.enabled {
            return String::new();
        }
        let Some(ref store) = self.interest_store else {
            return String::new();
        };
        let Ok(guard) = store.lock() else {
            return String::new();
        };
        guard.render_prefetch_block(query).unwrap_or_default()
    }

    pub(crate) fn reset_interest_sync_cursor(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.interest_synced_message_len = 0;
            state.interest_synced_user_hashes.clear();
            state.interest_session_buffer.clear();
        }
    }

    pub(crate) fn interest_sync_user_messages(&self, messages: &[Message]) {
        if !self.config().interest.enabled {
            return;
        }
        let interest_cfg = self.config().interest.clone();
        if !interest_cfg.per_turn_persist && !interest_cfg.per_turn_buffer {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let start = state.interest_synced_message_len;
        if start >= messages.len() {
            return;
        }
        for msg in messages.iter().skip(start) {
            if msg.role != MessageRole::User {
                continue;
            }
            let Some(text) = msg.content.as_deref() else {
                continue;
            };
            let trimmed = text.trim();
            if trimmed.is_empty() || is_poi_synthetic_user_text(trimmed) {
                continue;
            }
            let hash = {
                let mut hasher = Sha256::new();
                hasher.update(trimmed.as_bytes());
                let digest = hasher.finalize();
                u64::from_be_bytes(digest[..8].try_into().unwrap_or([0u8; 8]))
            };
            if !state.interest_synced_user_hashes.insert(hash) {
                continue;
            }
            if interest_cfg.per_turn_persist {
                let Some(ref store) = self.interest_store else {
                    continue;
                };
                if let Ok(guard) = store.lock() {
                    let _ = ingest_user_message(&guard, &interest_cfg, trimmed, 0.35);
                }
            } else if interest_cfg.per_turn_buffer {
                state
                    .interest_session_buffer
                    .absorb_turn(trimmed, &interest_cfg);
            }
        }
        state.interest_synced_message_len = messages.len();
    }

    pub(crate) fn interest_on_session_end(&self, messages: &[Message]) {
        let interest_enabled = self.config().interest.enabled;
        let insights_enabled = hermes_config::load_config(None)
            .unwrap_or_default()
            .insights
            .contribution
            .enabled;
        if !interest_enabled && !insights_enabled {
            return;
        }
        let buffered = if interest_enabled {
            self.state
                .lock()
                .map(|mut state| {
                    let buf = state.interest_session_buffer.drain();
                    state.interest_synced_user_hashes.clear();
                    state.interest_synced_message_len = 0;
                    buf
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let as_values: Vec<Value> = messages
            .iter()
            .filter_map(|m| serde_json::to_value(m).ok())
            .collect();
        let interest_cfg = self.config().interest.clone();
        let insights_cfg = hermes_config::load_config(None)
            .unwrap_or_default()
            .insights
            .contribution;
        let hermes_home = self
            .config()
            .hermes_home
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(hermes_config::hermes_home);
        let session_id = self
            .config()
            .session_id
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let auxiliary = if interest_cfg.session_end_llm_enabled() {
            tracing::warn!(
                "interest: session-end LLM extraction is enabled; user-only messages may be sent to the auxiliary LLM provider"
            );
            try_build_auxiliary_arc_for_config(&self.config())
        } else {
            None
        };
        spawn_session_end_pipeline(
            hermes_home,
            interest_cfg,
            insights_cfg,
            session_id,
            as_values,
            buffered,
            auxiliary,
        );
    }

    pub(crate) fn memory_prefetch(&self, query: &str, session_id: &str) -> String {
        let mut parts = Vec::new();
        let interest = self.interest_prefetch_block(query);
        if !interest.is_empty() {
            parts.push(interest);
        }
        if self.config().skip_memory {
            return parts.join("\n\n");
        }
        if let Some(ref mm) = self.memory_manager {
            if let Ok(mm) = mm.lock() {
                let block = mm.prefetch_all(query, session_id);
                if !block.is_empty() {
                    parts.push(block);
                }
            }
        }
        parts.join("\n\n")
    }

    fn recall_enabled_for_agent(&self) -> bool {
        if self.config().skip_memory {
            return false;
        }
        if !self.config().recall_enabled {
            return false;
        }
        std::env::var("HERMES_RECALL_ENABLED")
            .ok()
            .map(|v| {
                !matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            })
            .unwrap_or(true)
    }

    /// Proactive session recall block for continuation-style user messages.
    pub(crate) async fn recall_prefetch(&self, query: &str, session_id: &str) -> String {
        if !self.recall_enabled_for_agent() {
            return String::new();
        }
        let Some(backend) = self.recall_backend.as_ref() else {
            return String::new();
        };
        let Some(rq) = crate::recall_planner::classify(query) else {
            return String::new();
        };
        let options = hermes_tools::SessionSearchOptions {
            summarize: false,
        };
        let json = match backend
            .search(
                Some(&rq.keywords),
                None,
                5,
                Some(session_id),
                options,
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(error = %e, signal = ?rq.signal, "recall_prefetch search failed");
                return String::new();
            }
        };
        crate::recall_planner::format_recall_block(&json)
    }

    pub(crate) fn memory_sync(&self, user: &str, assistant: &str, session_id: &str) {
        if self.config().skip_memory {
            return;
        }
        if let Some(ref mm) = self.memory_manager {
            if let Ok(mm) = mm.lock() {
                mm.sync_all(user, assistant, session_id);
                if !user.trim().is_empty() {
                    mm.queue_prefetch_all(user, session_id);
                }
            }
        }
    }

    /// Python `_sync_external_memory_for_turn` — end-of-turn durable memory sync.
    pub(crate) fn sync_external_memory_for_turn(
        &self,
        original_user_message: &str,
        final_response: Option<&str>,
        interrupted: bool,
    ) {
        if interrupted || self.config().skip_memory {
            return;
        }
        let Some(response) = final_response.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        if original_user_message.trim().is_empty() {
            return;
        }
        let session_id = self
            .config()
            .session_id
            .as_deref()
            .unwrap_or("")
            .to_string();
        self.memory_sync(original_user_message, response, &session_id);
    }

    pub(crate) fn reset_vision_supported_for_turn(&self) {
        self.vision_supported
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn disable_vision_supported_and_strip_context(&self, ctx: &mut ContextManager) {
        self.vision_supported
            .store(false, std::sync::atomic::Ordering::Release);
        crate::vision_message_prepare::strip_images_for_non_vision_model_in_place(
            ctx.get_messages_mut(),
        );
        self.invalidate_turn_api_messages_cache();
    }

    pub(crate) async fn cleanup_dead_connections_at_turn_start(&self) {
        let rt = crate::route_learning::primary_runtime_snapshot(self);
        let provider = rt.provider.as_deref().unwrap_or("").trim();
        let Some(mut base) = crate::runtime_provider::resolve_runtime_base_url(
            self,
            provider,
            rt.base_url.as_deref(),
        ) else {
            return;
        };
        if base.is_empty() {
            return;
        }
        if !base.ends_with('/') {
            base.push('/');
        }
        let probe_url = format!("{base}models");
        crate::runtime_provider::effective_llm_provider(self)
            .turn_start_connection_hygiene(&probe_url)
            .await;
    }

    async fn compute_compression_feasibility_warning(&self) -> Option<String> {
        const AUX_FLOOR: u64 = 64_000;
        let threshold = self
            .context_compressor
            .inner
            .lock()
            .await
            .threshold_tokens();
        let aux_model = std::env::var("HERMES_COMPRESSION_MODEL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "google/gemini-3-flash-preview".to_string());
        let aux_ctx = get_model_context_length(&aux_model);
        if aux_ctx >= AUX_FLOOR && aux_ctx < threshold {
            return Some(format!(
                "Compression model '{aux_model}' context ({aux_ctx} tokens) is below the \
                 session compression threshold ({threshold} tokens). Auto-lowered threshold \
                 for this session; set a larger compression model in config.yaml if needed."
            ));
        }
        None
    }

    /// Replay stored compression feasibility warning once (Python `_replay_compression_warning`).
    pub(crate) async fn replay_compression_warning_at_turn_start(&self) {
        let should_compile = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if !state.compression_feasibility_checked {
                state.compression_feasibility_checked = true;
                true
            } else {
                false
            }
        };
        if should_compile {
            if let Some(msg) = self.compute_compression_feasibility_warning().await {
                if let Ok(mut state) = self.state.lock() {
                    state.compression_warning = Some(msg);
                }
            }
        }
        let msg = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.compression_warning.take());
        if let Some(msg) = msg {
            crate::hooks::emit_status(self, "lifecycle", &msg);
        }
    }

    pub(crate) fn log_turn_exit_diagnostic(
        &self,
        loop_result: &hermes_core::AgentResult,
        messages: &[Message],
    ) {
        let last_role = messages
            .last()
            .map(|m| format!("{:?}", m.role))
            .unwrap_or_else(|| "none".into());
        let pending_tool_assistant = messages
            .iter()
            .filter(|m| {
                m.role == MessageRole::Assistant
                    && m.tool_calls.as_ref().is_some_and(|t| !t.is_empty())
            })
            .count();
        let max_turns = effective_max_turns(self.config().max_turns)
            .map(|m| m.to_string())
            .unwrap_or_else(|| "unlimited".into());
        let last_assistant_tail = messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::Assistant)
            .and_then(|m| m.content.as_deref())
            .map(|text| {
                let trimmed = text.trim();
                let count = trimmed.chars().count();
                if count <= 120 {
                    trimmed.replace('\n', " ")
                } else {
                    let tail: String = trimmed.chars().skip(count.saturating_sub(120)).collect();
                    format!("…{}", tail.replace('\n', " "))
                }
            })
            .unwrap_or_default();
        tracing::info!(
            session_id = %crate::session_log::current_session_tag(),
            turn_exit_reason = %loop_result.turn_exit_reason,
            api_calls = loop_result.api_calls,
            total_turns = loop_result.total_turns,
            max_turns = %max_turns,
            interrupted = loop_result.interrupted,
            failed = loop_result.failed,
            partial = loop_result.partial,
            finished_naturally = loop_result.finished_naturally,
            last_msg_role = %last_role,
            pending_tool_assistant_msgs = pending_tool_assistant,
            last_assistant_tail = %last_assistant_tail,
            "conversation turn exit"
        );
    }

    pub(crate) fn memory_write_event_from_tool_call(
        tc: &ToolCall,
    ) -> Option<(String, String, String)> {
        if tc.function.name != "memory" {
            return None;
        }
        let args: Value = serde_json::from_str(&tc.function.arguments).ok()?;
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_lowercase();
        if action != "add" && action != "replace" && action != "remove" {
            return None;
        }
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("memory")
            .to_string();
        let content = if action == "remove" {
            args.get("old_text")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .to_string()
        } else {
            args.get("content")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .to_string()
        };
        Some((action, target, content))
    }

    pub(crate) fn notify_memory_writes(&self, tool_calls: &[ToolCall], results: &[ToolResult]) {
        crate::tool_executor::notify_memory_writes(self, tool_calls, results)
    }

    fn delegation_event_from_tool_result(
        tc: &ToolCall,
        result: &ToolResult,
    ) -> Option<(String, String)> {
        if tc.function.name != "delegate_task" || result.is_error {
            return None;
        }
        let args: Value = serde_json::from_str(&tc.function.arguments).ok()?;
        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();

        let sub_agent_id = serde_json::from_str::<Value>(&result.content)
            .ok()
            .and_then(|v| {
                v.get("sub_agent_id")
                    .and_then(|id| id.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
            })
            .unwrap_or_default();

        Some((task, sub_agent_id))
    }

    pub(crate) fn notify_delegations(&self, tool_calls: &[ToolCall], results: &[ToolResult]) {
        crate::tool_executor::notify_delegations(self, tool_calls, results)
    }

    pub(crate) fn memory_on_turn_start(&self, turn: u32, message: &str) {
        crate::tool_executor::memory_on_turn_start(self, turn, message)
    }

    pub(crate) fn memory_system_prompt(&self) -> String {
        crate::tool_executor::memory_system_prompt(self)
    }

    pub(crate) fn memory_pre_compress_note(&self, messages: &[Message]) -> Option<String> {
        if self.config().skip_memory {
            return None;
        }
        let Some(ref mm) = self.memory_manager else {
            return None;
        };
        let Ok(mm) = mm.lock() else {
            return None;
        };
        let as_values: Vec<Value> = messages
            .iter()
            .filter_map(|m| serde_json::to_value(m).ok())
            .collect();
        let note = mm.on_pre_compress(&as_values);
        if note.trim().is_empty() {
            None
        } else {
            Some(note)
        }
    }

    /// Notify memory providers that `session_id` rotated (compression, `/new`, resume, branch).
    pub fn memory_on_session_switch(
        &self,
        new_session_id: &str,
        parent_session_id: &str,
        reset: bool,
        reason: &str,
    ) {
        if self.config().skip_memory {
            return;
        }
        let Some(ref mm) = self.memory_manager else {
            return;
        };
        let Ok(mm) = mm.lock() else {
            return;
        };
        mm.on_session_switch(new_session_id, parent_session_id, reset, reason);
    }

    /// Update the active runtime session id (CLI `/new`, `/resume`, manual `/compress`).
    pub fn set_runtime_session_id(&self, session_id: &str) {
        let sid = session_id.trim();
        if sid.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.config_runtime.write() {
            let mut updated = (*guard).as_ref().clone();
            updated.session_id = Some(sid.to_string());
            *guard = Arc::new(updated);
        }
        let hermes_home = self
            .config()
            .hermes_home
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(hermes_config::hermes_home);
        touch_active_session(&hermes_home, sid);
    }

    /// Current runtime session id from agent config.
    pub fn runtime_session_id(&self) -> Option<String> {
        self.config().session_id.clone()
    }
}
