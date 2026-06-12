use super::*;

impl AgentLoop {
    // Collect one streaming completion into [`LlmResponse`] (first attempt in `run_stream` D-step).

    /// Expand `@file:` / `@diff` / … tokens in user messages before the LLM sees them.
    ///
    /// Mirrors Python `agent.context_references.preprocess_context_references_async`
    /// (also invoked from gateway/CLI before `run_conversation` on some paths). Both
    /// `run` and `run_stream` call this so streaming callers get the same expansion.
    fn context_reference_workspace_root() -> PathBuf {
        std::env::var("TERMINAL_CWD")
            .ok()
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    pub(crate) async fn preprocess_user_message_context_references(
        &self,
        messages: &mut [Message],
    ) {
        let cwd = Self::context_reference_workspace_root();
        let context_length = get_model_context_length(&crate::runtime_provider::active_model(self));
        for msg in messages.iter_mut() {
            if msg.role != MessageRole::User {
                continue;
            }
            let Some(content) = msg.content.clone() else {
                continue;
            };
            let result =
                preprocess_context_references_async(&content, &cwd, context_length, Some(&cwd))
                    .await;
            if result.expanded && result.message != content {
                msg.content = Some(result.message);
            }
        }
    }

    /// Per-turn message prelude (sanitize, budget strip, @file expansion, restore primary).
    pub(crate) async fn apply_turn_message_prelude(&self, messages: &mut Vec<Message>) {
        for msg in messages.iter_mut() {
            if let Some(ref mut c) = msg.content {
                *c = sanitize_surrogates(c).into_owned();
            }
        }
        strip_budget_warnings_from_messages(messages);
        self.preprocess_user_message_context_references(messages)
            .await;
        self.restore_primary_runtime_at_turn_start();
    }

    /// Ask the LLM for a final summary when the turn budget is exhausted.
    pub(crate) async fn handle_max_iterations(
        &self,
        ctx: &mut ContextManager,
    ) -> Result<Option<Message>, AgentError> {
        if hermes_tools::kanban_task_from_env().is_some() {
            let block = hermes_tools::kanban_block_reason(Some("iteration_budget_exhausted"));
            ctx.add_message(Message::tool_result(
                "kanban_block",
                serde_json::to_string(&block).unwrap_or_else(|_| block.to_string()),
            ));
            return Ok(None);
        }
        ctx.add_message(Message::system(
            "[SYSTEM] Maximum conversation turns reached. Please provide a brief summary of \
             what was accomplished and any remaining tasks.",
        ));
        let runtime = crate::route_learning::primary_runtime_snapshot(self);
        let (_, model_name) =
            crate::route_learning::extract_provider_and_model(self, runtime.model.as_str());
        let response = self
            .llm_provider
            .chat_completion(
                ctx.get_messages(),
                &[],
                self.config().max_tokens,
                self.config().temperature,
                Some(model_name),
                crate::llm_caller::extra_body_for_api_mode(self, &runtime.api_mode).as_ref(),
            )
            .await
            .map_err(|e| AgentError::LlmApi(e.to_string()))?;
        Ok(Some(response.message))
    }

    pub(crate) async fn handle_tool_loop_guard_summary(
        &self,
        ctx: &mut ContextManager,
        consecutive_error_turns: u32,
        failed_calls: u32,
        total_calls: usize,
    ) -> Result<Option<Message>, AgentError> {
        ctx.add_message(Message::system(format!(
            "[SYSTEM] Tool-loop guard triggered after {} consecutive error turn(s). Latest turn failed {}/{} tool call(s). Stop calling tools and provide a concise final response with what succeeded, what failed, and precise next manual step(s).",
            consecutive_error_turns, failed_calls, total_calls
        )));
        let runtime = crate::route_learning::primary_runtime_snapshot(self);
        let (_, model_name) =
            crate::route_learning::extract_provider_and_model(self, runtime.model.as_str());
        let response = self
            .llm_provider
            .chat_completion(
                ctx.get_messages(),
                &[],
                self.config().max_tokens,
                self.config().temperature,
                Some(model_name),
                crate::llm_caller::extra_body_for_api_mode(self, &runtime.api_mode).as_ref(),
            )
            .await
            .map_err(|e| AgentError::LlmApi(e.to_string()))?;
        Ok(Some(response.message))
    }

    pub(crate) fn emit_background_review_metrics(&self, turn: u32, ctx: &ContextManager) {
        if !self.config().background_review_metrics_enabled {
            return;
        }
        let snapshot = ctx.get_messages().to_vec();
        tokio::spawn(async move {
            let tool_msg_count = snapshot
                .iter()
                .filter(|m| matches!(m.role, hermes_core::MessageRole::Tool))
                .count();
            tracing::debug!(
                turn,
                tool_messages = tool_msg_count,
                total_messages = snapshot.len(),
                "Background review snapshot captured"
            );
        });
    }

    /// Metrics (always) + optional Python-style memory/skill review LLM pass on session end.
    pub(crate) fn spawn_background_review(
        &self,
        turn: u32,
        ctx: &ContextManager,
        review_memory_at_end: bool,
        session_key: Option<&str>,
    ) {
        self.emit_background_review_metrics(turn, ctx);
        if !self.config().background_review_enabled {
            return;
        }
        let mut review_skills = false;
        if self.config().skill_creation_nudge_interval > 0
            && self
                .tool_registry
                .names()
                .iter()
                .any(|n| n == "skill_manage")
        {
            if let Ok(mut state) = self.state.lock() {
                if state.evolution_counters.iters_since_skill
                    >= self.config().skill_creation_nudge_interval
                {
                    review_skills = true;
                    state.evolution_counters.iters_since_skill = 0;
                }
            }
        }
        let review_memory = review_memory_at_end;
        if !review_memory && !review_skills {
            return;
        }
        let trigger = match crate::evolution_ledger::review_trigger(review_memory, review_skills) {
            Some(t) => t,
            None => return,
        };
        let prompt: &'static str = match (review_memory, review_skills) {
            (true, true) => COMBINED_REVIEW_PROMPT,
            (true, false) => MEMORY_REVIEW_PROMPT,
            (false, true) => SKILL_REVIEW_PROMPT,
            _ => return,
        };
        let ledger_enabled = crate::evolution_ledger::evolution_ledger_enabled(self.config().as_ref());
        let hermes_home = crate::evolution_ledger::resolve_hermes_home(self.config().as_ref());
        let ledger_max = self.config().evolution_ledger_max_entries;
        let review_id = crate::evolution_ledger::new_review_id();
        let session_key_owned = session_key.map(str::to_string);
        if ledger_enabled {
            let started = crate::evolution_ledger::started_event(
                review_id.clone(),
                session_key_owned.clone(),
                trigger,
            );
            if let Err(e) = crate::evolution_ledger::append_event(&hermes_home, &started, ledger_max)
            {
                tracing::debug!(error = %e, "evolution ledger append (started) failed");
            }
        }
        let mut hist = ctx.get_messages().to_vec();
        hist.push(Message::user(prompt));
        let mut cfg = (*self.config()).clone();
        cfg.background_review_enabled = false;
        cfg.background_review_metrics_enabled = false;
        cfg.memory_nudge_interval = 0;
        cfg.skill_creation_nudge_interval = 0;
        cfg.max_concurrent_delegates = 0;
        cfg.quiet_mode = true;
        cfg.skip_memory = true;
        cfg.use_prompt_caching = self.config().use_prompt_caching;
        cfg.use_native_cache_layout = self.config().use_native_cache_layout;
        cfg.cache_ttl = self.config().cache_ttl.clone();
        if let Some(sys) = ctx
            .get_messages()
            .iter()
            .find(|m| m.role == MessageRole::System)
            .and_then(|m| m.content.clone())
            .filter(|s| !s.trim().is_empty())
        {
            cfg.stored_system_prompt = Some(sys);
        } else if let Some(sys) = self.config().stored_system_prompt.clone() {
            cfg.stored_system_prompt = Some(sys);
        }
        cfg.max_turns = if cfg.max_turns == 0 {
            16
        } else {
            cfg.max_turns.min(16)
        };
        let tools = self.tool_registry.clone();
        let provider = self.llm_provider.clone();
        let async_tool_dispatch = self.async_tool_dispatch();
        let review_cb = self.callbacks.background_review_callback.clone();
        tokio::spawn(async move {
            let timer = crate::evolution_ledger::ReviewTimer::start();
            let agent = AgentLoop::new(cfg, tools, provider)
                .maybe_with_async_tool_dispatch(async_tool_dispatch);
            match agent.run(hist, None).await {
                Ok(result) => {
                    let tools = crate::evolution_ledger::extract_review_tools(&result.messages);
                    let summary = crate::evolution_ledger::summarize_review_for_chat(&result.messages);
                    if ledger_enabled {
                        let completed = crate::evolution_ledger::completed_event(
                            review_id.clone(),
                            session_key_owned.clone(),
                            trigger,
                            timer.elapsed_ms(),
                            tools,
                            summary.clone(),
                        );
                        if let Err(e) =
                            crate::evolution_ledger::append_event(&hermes_home, &completed, ledger_max)
                        {
                            tracing::debug!(error = %e, "evolution ledger append (completed) failed");
                        }
                    }
                    if let Some(cb) = review_cb.as_ref() {
                        if let Some(summary) = summary {
                            cb(&summary);
                        }
                    }
                }
                Err(e) => {
                    if ledger_enabled {
                        let failed = crate::evolution_ledger::failed_event(
                            review_id,
                            session_key_owned,
                            trigger,
                            timer.elapsed_ms(),
                            e.to_string(),
                        );
                        if let Err(err) =
                            crate::evolution_ledger::append_event(&hermes_home, &failed, ledger_max)
                        {
                            tracing::debug!(error = %err, "evolution ledger append (failed) failed");
                        }
                    }
                    tracing::debug!(error = %e, "background memory/skill review failed");
                }
            }
        });
    }

    /// Recover todo-state hints from historical messages at loop start.
    pub(crate) fn hydrate_todo_store(&self, ctx: &ContextManager) {
        let todo_markers = ctx
            .get_messages()
            .iter()
            .filter_map(|m| m.content.as_deref())
            .filter(|c| c.contains("TODO") || c.contains("[ ]") || c.contains("[x]"))
            .count();
        if todo_markers > 0 {
            tracing::debug!(todo_markers, "Hydrated todo markers from prior context");
        }
    }
}
