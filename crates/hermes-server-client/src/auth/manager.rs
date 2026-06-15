//! Auth flow orchestration across login methods.

use std::sync::Arc;

use hermes_config::ServerConfig;

use super::email_otp::EmailOtpAuthProvider;
use super::provider::{AuthContext, AuthProvider};
use super::types::{AuthPollResult, AuthUserInput, LoginMethod, PendingLogin};
use super::wechat_qr::WeChatQrAuthProvider;
use crate::error::ServerClientError;
use crate::session::{ServerSession, ServerTokens};
use crate::transport::HttpTransport;

/// Coordinates remote server login flows.
pub struct AuthManager {
    config: ServerConfig,
    transport: HttpTransport,
    session: ServerSession,
    providers: Vec<Arc<dyn AuthProvider>>,
}

impl AuthManager {
    pub fn new(
        config: ServerConfig,
        hermes_home: impl AsRef<std::path::Path>,
    ) -> Result<Self, ServerClientError> {
        if !config.enabled {
            return Err(ServerClientError::Disabled);
        }
        let transport = HttpTransport::new(&config)?;
        let session = ServerSession::from_config(&config, hermes_home);
        let providers: Vec<Arc<dyn AuthProvider>> = vec![
            Arc::new(WeChatQrAuthProvider),
            Arc::new(EmailOtpAuthProvider),
        ];
        Ok(Self {
            config,
            transport,
            session,
            providers,
        })
    }

    pub fn session(&self) -> &ServerSession {
        &self.session
    }

    pub fn transport(&self) -> &HttpTransport {
        &self.transport
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn resolve_method(&self, override_method: Option<LoginMethod>) -> LoginMethod {
        override_method.unwrap_or_else(|| self.config.auth.preferred_method.into())
    }

    fn provider_for(&self, method: LoginMethod) -> Option<&Arc<dyn AuthProvider>> {
        self.providers.iter().find(|p| p.method() == method)
    }

    pub async fn start_login(
        &self,
        method: LoginMethod,
    ) -> Result<PendingLogin, ServerClientError> {
        let provider = self.provider_for(method).ok_or_else(|| {
            ServerClientError::NotConfigured(format!("login method {}", method.as_str()))
        })?;
        let ctx = AuthContext {
            transport: &self.transport,
        };
        provider.start(&ctx).await
    }

    pub async fn continue_login(
        &self,
        pending: &PendingLogin,
        input: AuthUserInput,
    ) -> Result<AuthPollResult, ServerClientError> {
        let provider = self.provider_for(pending.method).ok_or_else(|| {
            ServerClientError::NotConfigured(format!("login method {}", pending.method.as_str()))
        })?;
        let ctx = AuthContext {
            transport: &self.transport,
        };
        let result = provider.poll_or_submit(&ctx, pending, input).await?;
        if let AuthPollResult::Success(tokens) = &result {
            self.session.save_tokens(tokens.clone()).await?;
        }
        Ok(result)
    }

    pub async fn logout(&self) -> Result<bool, ServerClientError> {
        self.session.logout().await
    }

    pub async fn whoami(&self) -> Result<WhoamiStatus, ServerClientError> {
        let source = self.session.token_source().await;
        let tokens = self.session.load_tokens().await?;
        Ok(WhoamiStatus {
            source,
            tokens,
            server_enabled: self.config.enabled,
            base_url: self.config.base_url.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct WhoamiStatus {
    pub source: crate::session::TokenSource,
    pub tokens: Option<ServerTokens>,
    pub server_enabled: bool,
    pub base_url: String,
}

impl WhoamiStatus {
    pub fn is_logged_in(&self) -> bool {
        self.tokens
            .as_ref()
            .map(|t| !t.access_token.is_empty())
            .unwrap_or(false)
    }

    pub fn token_expired(&self) -> bool {
        self.tokens
            .as_ref()
            .map(|t| t.is_expired(0))
            .unwrap_or(false)
    }
}
