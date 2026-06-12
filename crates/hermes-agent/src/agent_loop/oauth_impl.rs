use super::*;

impl AgentLoop {
    pub(crate) fn resolve_runtime_api_key(
        &self,
        provider: &str,
        api_key_env_override: Option<&str>,
        explicit_api_key: Option<&str>,
    ) -> Option<String> {
        if provider == "copilot-acp" {
            return Some("copilot-acp".to_string());
        }
        if let Some(token) = self.resolve_oauth_store_api_key(provider) {
            return Some(token);
        }
        if let Some(key) = explicit_api_key.map(str::trim).filter(|s| !s.is_empty()) {
            return Some(key.to_string());
        }
        if let Some(env_name) = api_key_env_override
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Ok(v) = std::env::var(env_name) {
                if !v.trim().is_empty() {
                    return Some(v);
                }
            }
        }
        if let Some(cfg) = self.config().runtime_providers.get(provider) {
            if let Some(ref key) = cfg.api_key {
                let trimmed = key.trim();
                if let Some(env_ref) = trimmed.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
                {
                    if let Ok(v) = std::env::var(env_ref) {
                        if !v.trim().is_empty() {
                            return Some(v);
                        }
                    }
                } else if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            if let Some(env_name) = cfg
                .api_key_env
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Ok(v) = std::env::var(env_name) {
                    if !v.trim().is_empty() {
                        return Some(v);
                    }
                }
            }
        }
        if matches!(provider, "openai" | "codex" | "openai-codex") {
            return std::env::var("HERMES_OPENAI_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .filter(|v| !v.trim().is_empty());
        }
        if provider == "stepfun" {
            return std::env::var("HERMES_STEPFUN_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| std::env::var("STEPFUN_API_KEY").ok())
                .filter(|v| !v.trim().is_empty());
        }
        match provider {
            "anthropic" | "claude" | "claude-code" => std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| std::env::var("ANTHROPIC_TOKEN").ok())
                .filter(|v| !v.trim().is_empty())
                .or_else(|| std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok())
                .filter(|v| !v.trim().is_empty()),
            "google-gemini-cli" | "gemini-cli" | "gemini-oauth" => {
                std::env::var("HERMES_GEMINI_OAUTH_API_KEY")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
            }
            "openrouter" => std::env::var("OPENROUTER_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            "qwen" | "qwen-oauth" => std::env::var("DASHSCOPE_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            "kimi" | "moonshot" => std::env::var("MOONSHOT_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            "minimax" => std::env::var("MINIMAX_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            "nous" => std::env::var("NOUS_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            "copilot" | "copilot-acp" => std::env::var("GITHUB_COPILOT_TOKEN")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            _ => None,
        }
    }

    pub(crate) fn resolve_runtime_base_url(
        &self,
        provider: &str,
        route_base_url: Option<&str>,
    ) -> Option<String> {
        crate::runtime_provider::resolve_runtime_base_url(self, provider, route_base_url)
    }

    fn resolve_oauth_store_api_key(&self, provider: &str) -> Option<String> {
        let provider_key = match provider {
            "openai" => "openai",
            "openai-codex" | "codex" => "openai-codex",
            "nous" => "nous",
            "qwen-oauth" => "qwen-oauth",
            "anthropic" | "claude" | "claude-code" => "anthropic",
            "google-gemini-cli" | "gemini-cli" | "gemini-oauth" => "google-gemini-cli",
            _ => return None,
        };
        let path = self.auth_tokens_path();
        let raw = std::fs::read_to_string(path).ok()?;
        let entries: HashMap<String, OAuthStoreCredential> = serde_json::from_str(&raw).ok()?;
        let cred = entries.get(provider_key)?;
        if cred.access_token.trim().is_empty() {
            return None;
        }
        if cred
            .expires_at
            .map(|exp| exp <= Utc::now())
            .unwrap_or(false)
        {
            return None;
        }
        Some(cred.access_token.clone())
    }

    pub(crate) fn oauth_refresh_config(&self, provider_key: &str) -> Option<(String, String)> {
        // Preferred source: unified provider config centre (runtime_providers).
        let cfg_token_url = self
            .config()
            .runtime_providers
            .get(provider_key)
            .and_then(|c| c.oauth_token_url.as_deref())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let cfg_client_id = self
            .config()
            .runtime_providers
            .get(provider_key)
            .and_then(|c| c.oauth_client_id.as_deref())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // Env fallback - keeps previous behavior working when config centre is empty.
        let (token_url_env, client_id_env) = match provider_key {
            "openai" => (
                "HERMES_OPENAI_OAUTH_TOKEN_URL",
                "HERMES_OPENAI_OAUTH_CLIENT_ID",
            ),
            "openai-codex" => (
                "HERMES_OPENAI_CODEX_OAUTH_TOKEN_URL",
                "HERMES_OPENAI_CODEX_OAUTH_CLIENT_ID",
            ),
            "nous" => ("HERMES_NOUS_OAUTH_TOKEN_URL", "HERMES_NOUS_OAUTH_CLIENT_ID"),
            "qwen-oauth" => ("HERMES_QWEN_OAUTH_TOKEN_URL", "HERMES_QWEN_OAUTH_CLIENT_ID"),
            "anthropic" => (
                "HERMES_ANTHROPIC_OAUTH_TOKEN_URL",
                "HERMES_ANTHROPIC_OAUTH_CLIENT_ID",
            ),
            _ => return cfg_token_url.zip(cfg_client_id),
        };
        let token_url = cfg_token_url
            .or_else(|| {
                std::env::var(token_url_env)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| match provider_key {
                "openai" => std::env::var("HERMES_OPENAI_CODEX_OAUTH_TOKEN_URL")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| Some("https://auth.openai.com/oauth/token".to_string())),
                "nous" => std::env::var("NOUS_PORTAL_BASE_URL")
                    .ok()
                    .map(|s| s.trim().trim_end_matches('/').to_string())
                    .filter(|s| !s.is_empty())
                    .map(|base| format!("{base}/api/oauth/token"))
                    .or_else(|| {
                        Some("https://portal.nousresearch.com/api/oauth/token".to_string())
                    }),
                "anthropic" => Some("https://console.anthropic.com/v1/oauth/token".to_string()),
                _ => None,
            })?;
        let client_id = cfg_client_id
            .or_else(|| {
                std::env::var(client_id_env)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| match provider_key {
                "openai" => std::env::var("HERMES_OPENAI_CODEX_OAUTH_CLIENT_ID")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| Some("app_EMoamEEZ73f0CkXaXp7hrann".to_string())),
                "nous" => std::env::var("NOUS_CLIENT_ID")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| Some("hermes-cli".to_string())),
                "anthropic" => Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e".to_string()),
                _ => None,
            })?;
        Some((token_url, client_id))
    }

    fn auth_tokens_path(&self) -> PathBuf {
        let hermes_home = self
            .config()
            .hermes_home
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| std::env::var("HERMES_HOME").ok().map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|h| h.join(".hermes")))
            .unwrap_or_else(|| PathBuf::from(".hermes"));
        hermes_home.join("auth").join("tokens.json")
    }
}
