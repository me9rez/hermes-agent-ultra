//! Auth provider trait — one implementation per server login method.

use async_trait::async_trait;

use crate::auth::types::{AuthPollResult, AuthUserInput, LoginMethod, PendingLogin};
use crate::error::ServerClientError;
use crate::transport::HttpTransport;

/// Shared context for auth provider HTTP calls.
pub struct AuthContext<'a> {
    pub transport: &'a HttpTransport,
}

/// Pluggable login method (WeChat QR, email OTP, …).
#[async_trait]
pub trait AuthProvider: Send + Sync {
    fn method(&self) -> LoginMethod;

    async fn start(&self, ctx: &AuthContext<'_>) -> Result<PendingLogin, ServerClientError>;

    async fn poll_or_submit(
        &self,
        ctx: &AuthContext<'_>,
        pending: &PendingLogin,
        input: AuthUserInput,
    ) -> Result<AuthPollResult, ServerClientError>;
}
