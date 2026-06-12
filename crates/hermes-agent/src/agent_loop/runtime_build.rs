use super::*;

impl AgentLoop {
    fn resolve_runtime_command_args(
        &self,
        provider: Option<&str>,
    ) -> (Option<String>, Vec<String>) {
        let mut command = self
            .config()
            .acp_command
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let mut args: Vec<String> = self
            .config()
            .acp_args
            .iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();

        if let Some(provider) = provider {
            if let Some(cfg) = self.config().runtime_providers.get(provider) {
                if let Some(cmd) = cfg
                    .command
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    command = Some(cmd.to_string());
                }
                if !cfg.args.is_empty() {
                    args = cfg
                        .args
                        .iter()
                        .map(|a| a.trim().to_string())
                        .filter(|a| !a.is_empty())
                        .collect();
                }
            }
            if provider == "copilot-acp" {
                if command.is_none() {
                    command = std::env::var("HERMES_COPILOT_ACP_COMMAND")
                        .ok()
                        .or_else(|| std::env::var("COPILOT_CLI_PATH").ok())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .or_else(|| Some("copilot".to_string()));
                }
                if args.is_empty() {
                    args = std::env::var("HERMES_COPILOT_ACP_ARGS")
                        .ok()
                        .and_then(|raw| shlex::split(raw.trim()))
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| vec!["--acp".to_string(), "--stdio".to_string()]);
                }
                if let Some(cmd) = command.as_deref() {
                    if let Ok(resolved) = which::which(cmd) {
                        command = Some(resolved.to_string_lossy().to_string());
                    }
                }
            }
        }
        (command, args)
    }

    fn resolve_runtime_request_timeout_seconds(&self, provider: &str) -> Option<f64> {
        self.config()
            .runtime_providers
            .get(provider)
            .and_then(|c| c.request_timeout_seconds)
            .or_else(|| {
                let alias = match provider {
                    "codex" => "openai-codex",
                    "openai-codex" => "codex",
                    "qwen" => "qwen-oauth",
                    "qwen-oauth" => "qwen",
                    "kimi" => "moonshot",
                    "moonshot" => "kimi",
                    _ => return None,
                };
                self.config()
                    .runtime_providers
                    .get(alias)
                    .and_then(|c| c.request_timeout_seconds)
            })
    }

    pub(crate) fn build_runtime_provider(
        &self,
        provider: &str,
        model_name: &str,
        route_base_url: Option<&str>,
        api_key_env_override: Option<&str>,
        explicit_api_key: Option<&str>,
        api_mode: Option<&ApiMode>,
        credential_pool: Option<&Arc<CredentialPool>>,
    ) -> Result<Arc<dyn LlmProvider>, AgentError> {
        crate::runtime_provider::build_runtime_provider(
            self,
            provider,
            model_name,
            route_base_url,
            api_key_env_override,
            explicit_api_key,
            api_mode,
            credential_pool,
        )
    }

    pub(crate) fn credential_pool_for_route<'a>(
        &'a self,
        rt: &'a TurnRuntimeRoute,
    ) -> Option<&'a Arc<CredentialPool>> {
        crate::runtime_provider::credentials_pool_for_route(self, rt)
    }

    /// Recompute prompt-cache policy from current route (Python `_anthropic_prompt_cache_policy`).
    pub fn refresh_prompt_cache_policy(&self, provider: &str, base_url: &str, api_mode: &str) {
        crate::runtime_provider::refresh_prompt_cache_policy(self, provider, base_url, api_mode)
    }
}
