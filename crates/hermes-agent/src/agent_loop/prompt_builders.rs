use super::*;
use crate::system_prompt::BACKEND_PROBE_COMMAND;

impl AgentLoop {
    pub(crate) fn openrouter_provider_preferences(&self) -> Option<Value> {
        let cfg = self.config();
        let mut prefs = serde_json::Map::new();
        if !cfg.providers_allowed.is_empty() {
            prefs.insert(
                "only".into(),
                Value::Array(
                    cfg.providers_allowed
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );
        }
        if !cfg.providers_ignored.is_empty() {
            prefs.insert(
                "ignore".into(),
                Value::Array(
                    cfg.providers_ignored
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );
        }
        if !cfg.providers_order.is_empty() {
            prefs.insert(
                "order".into(),
                Value::Array(
                    cfg.providers_order
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(sort) = cfg.provider_sort.as_deref().filter(|s| !s.is_empty()) {
            prefs.insert("sort".into(), Value::String(sort.to_string()));
        }
        if let Some(req) = cfg.provider_require_parameters {
            prefs.insert("require_parameters".into(), Value::Bool(req));
        }
        if let Some(dc) = cfg
            .provider_data_collection
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            prefs.insert("data_collection".into(), Value::String(dc.to_string()));
        }
        if prefs.is_empty() && cfg.openrouter_min_coding_score.is_none() {
            return None;
        }
        let mut provider_obj = Value::Object(prefs);
        if let Some(score) = cfg.openrouter_min_coding_score {
            if let Some(obj) = provider_obj.as_object_mut() {
                obj.insert(
                    "plugins".into(),
                    serde_json::json!([{ "id": "pareto-router", "min_coding_score": score }]),
                );
            }
        }
        Some(provider_obj)
    }

    pub(crate) fn invoke_pre_api_request_hook(
        &self,
        api_call_count: u32,
        api_messages: &[Message],
        tool_count: usize,
        model: &str,
        provider: &str,
        base_url: Option<&str>,
        api_mode: &ApiMode,
        max_tokens: Option<u32>,
    ) {
        let request_messages: Vec<Value> = api_messages
            .iter()
            .filter_map(|m| serde_json::to_value(m).ok())
            .collect();
        let message_count = api_messages.len();
        let request_char_count: usize = api_messages
            .iter()
            .map(|m| {
                m.content.as_deref().map(str::len).unwrap_or(0)
                    + m.reasoning_content.as_deref().map(str::len).unwrap_or(0)
            })
            .sum();
        let approx_input_tokens = (request_char_count / 4).max(1);
        let user_message = api_messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.content.clone())
            .unwrap_or_default();
        let hook_ctx = serde_json::json!({
            "session_id": self.config().session_id.as_deref().unwrap_or(""),
            "user_message": user_message,
            "platform": self.config().platform.as_deref().unwrap_or(""),
            "model": model,
            "provider": provider,
            "base_url": base_url.unwrap_or(""),
            "api_mode": crate::hooks::api_mode_as_hook_str(api_mode),
            "api_call_count": api_call_count,
            "attempt": api_call_count,
            "stream": false,
            "request_messages": request_messages,
            "message_count": message_count,
            "tool_count": tool_count,
            "approx_input_tokens": approx_input_tokens,
            "request_char_count": request_char_count,
            "max_tokens": max_tokens,
        });
        let _ = crate::hooks::invoke_hook(self, HookType::PreApiRequest, &hook_ctx);
    }

    pub(crate) fn code_index_repo_map_block(&self) -> Option<String> {
        let Some(ref idx) = self.code_index else {
            return None;
        };
        let Ok(mut idx) = idx.lock() else {
            return None;
        };
        let rendered = idx.render_repo_map(
            Some(self.config().code_index_max_files),
            Some(self.config().code_index_max_symbols),
        );
        if rendered.trim().is_empty() {
            None
        } else {
            Some(rendered)
        }
    }

    pub(crate) fn lsp_context_note(
        &self,
        tool_calls: &[ToolCall],
        results: &[ToolResult],
    ) -> Option<String> {
        if !self.lsp_context.enabled {
            return None;
        }
        let Some(ref idx) = self.code_index else {
            return None;
        };
        let Ok(mut idx) = idx.lock() else {
            return None;
        };
        build_lsp_context_note(tool_calls, results, &mut idx, &self.lsp_context)
    }

    pub(crate) fn should_inject_tool_enforcement(&self, model: &str) -> bool {
        should_inject_tool_enforcement_for_model(model)
    }

    pub(crate) fn platform_hint_text(&self) -> Option<&'static str> {
        platform_hint_for(self.config().platform.as_deref())
    }

    pub(crate) fn probe_remote_backend_text(&self, env_type: &str) -> Option<String> {
        let cwd_hint = std::env::var("TERMINAL_CWD").unwrap_or_default();
        probe_remote_backend_cached(env_type, &cwd_hint, || {
            let terminal = self.tool_registry.get("terminal")?;
            let output =
                (terminal.handler)(serde_json::json!({ "command": BACKEND_PROBE_COMMAND })).ok()?;
            format_probe_output(output.trim())
        })
    }

    pub(crate) fn effective_provider_for_prompt(&self, model: &str) -> Option<String> {
        if let Some(ref p) = self.config().provider {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        model
            .split_once(':')
            .map(|(provider, _)| provider.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn runtime_skills_tier() -> &'static str {
        match std::env::var("HERMES_SKILLS_EXECUTION_TIER")
            .ok()
            .unwrap_or_else(|| "balanced".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "trusted" => "trusted",
            "open" | "permissive" => "open",
            _ => "balanced",
        }
    }

    fn runtime_skills_tier_bypass_enabled() -> bool {
        std::env::var("HERMES_SKILLS_TIER_BYPASS")
            .ok()
            .is_some_and(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
    }

    fn skill_trust_score(cmd: &str, name: &str, description: &str) -> i32 {
        let corpus = format!(
            "{} {} {}",
            cmd.to_ascii_lowercase(),
            name.to_ascii_lowercase(),
            description.to_ascii_lowercase()
        );
        let mut score = 70i32;
        let high_risk_terms = [
            "trade",
            "money",
            "wallet",
            "deploy",
            "delete",
            "shell",
            "execute",
            "terminal",
            "browser automation",
            "computer use",
            "send email",
            "gmail",
            "calendar",
        ];
        for term in high_risk_terms {
            if corpus.contains(term) {
                score -= 12;
            }
        }
        let medium_risk_terms = ["write", "modify", "edit", "publish", "install", "webhook"];
        for term in medium_risk_terms {
            if corpus.contains(term) {
                score -= 6;
            }
        }
        let trusted_terms = ["search", "read", "summarize", "analyze", "query", "list"];
        for term in trusted_terms {
            if corpus.contains(term) {
                score += 4;
            }
        }
        score.clamp(0, 100)
    }

    fn skill_allowed_for_tier(tier: &str, score: i32) -> bool {
        match tier {
            "trusted" => score >= 62,
            "balanced" => score >= 34,
            _ => true,
        }
    }

    pub(crate) fn skills_system_prompt(&self, tool_names: &HashSet<&str>) -> Option<String> {
        let has_skills_tools = ["skills_list", "skill_view", "skill_manage"]
            .iter()
            .any(|t| tool_names.contains(*t));
        if !has_skills_tools {
            return None;
        }
        let mut orch = SkillOrchestrator::default_dir();
        orch.set_enabled_disabled(
            &self.config().enabled_skills,
            &self.config().disabled_skills,
        );
        let commands = orch.scan_skill_commands();
        if commands.is_empty() {
            return Some(
                "## Skills (mandatory)\nSkills tools are enabled. Use `skills_list` to discover available skills and `skill_view` before applying one."
                    .to_string(),
            );
        }
        let tier = Self::runtime_skills_tier();
        let bypass = Self::runtime_skills_tier_bypass_enabled();
        let mut rows: Vec<_> = commands
            .iter()
            .filter(|(cmd, info)| {
                if bypass || tier == "open" {
                    return true;
                }
                let score = Self::skill_trust_score(cmd, &info.name, &info.description);
                Self::skill_allowed_for_tier(tier, score)
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        let filtered = commands.len().saturating_sub(rows.len());
        if rows.is_empty() {
            return Some(format!(
                "## Skills (mandatory)\nSkills tools are enabled but current skills tier '{}' filtered all candidates. Use `/ops skills-tier balanced` or `/ops skills-tier open` for broader access.",
                tier
            ));
        }
        let mut body = String::from(
            "## Skills (mandatory)\nBefore replying, check whether an existing skill applies. If yes, inspect it with `skill_view` and follow it.\n<available_skills>\n",
        );
        body.push_str(&format!(
            "<skills_tier mode=\"{}\" bypass=\"{}\" filtered=\"{}\" />\n",
            tier,
            if bypass { "on" } else { "off" },
            filtered
        ));
        for (cmd, info) in rows.into_iter().take(80) {
            body.push_str(&format!(
                "- {}: {} ({})\n",
                cmd,
                info.name,
                info.description.trim()
            ));
        }
        body.push_str("</available_skills>");
        Some(body)
    }

    pub(crate) fn context_files_prompt(&self) -> Option<String> {
        if self.config().skip_context_files {
            return None;
        }
        let cwd = std::env::var("TERMINAL_CWD")
            .ok()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            });

        let mut sections = Vec::new();
        if let Some(workspace) = load_workspace_context(&cwd) {
            sections.push(format!("## Workspace Context\n{}", workspace));
        }

        let hermes_home = self
            .config()
            .hermes_home
            .as_deref()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("HERMES_HOME")
                    .ok()
                    .map(std::path::PathBuf::from)
            })
            .or_else(|| dirs::home_dir().map(|h| h.join(".hermes")))
            .unwrap_or_else(|| std::path::PathBuf::from(".hermes"));

        let personal_ctx = load_hermes_context_files(&hermes_home);
        if !personal_ctx.trim().is_empty() {
            sections.push(format!("## Personal Context\n{}", personal_ctx));
        }

        if sections.is_empty() {
            None
        } else {
            Some(sections.join("\n\n"))
        }
    }

    pub(crate) fn extract_provider_and_model<'a>(&self, model: &'a str) -> (String, &'a str) {
        if let Some((p, m)) = model.split_once(':') {
            let p = p.trim();
            let m = m.trim();
            if !p.is_empty() && !m.is_empty() {
                return (p.to_string(), m);
            }
        }
        let fallback_provider = self
            .config()
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("openai")
            .to_string();
        (fallback_provider, model)
    }
}
