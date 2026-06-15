//! Email OTP login — stub until server API docs are provided.

use async_trait::async_trait;

use super::provider::{AuthContext, AuthProvider};
use super::types::{AuthPollResult, AuthUserInput, LoginMethod, PendingLogin};
use crate::error::ServerClientError;

pub struct EmailOtpAuthProvider;

#[async_trait]
impl AuthProvider for EmailOtpAuthProvider {
    fn method(&self) -> LoginMethod {
        LoginMethod::EmailOtp
    }

    async fn start(&self, _ctx: &AuthContext<'_>) -> Result<PendingLogin, ServerClientError> {
        Err(ServerClientError::not_configured("email OTP login"))
    }

    async fn poll_or_submit(
        &self,
        _ctx: &AuthContext<'_>,
        _pending: &PendingLogin,
        _input: AuthUserInput,
    ) -> Result<AuthPollResult, ServerClientError> {
        Err(ServerClientError::not_configured("email OTP login"))
    }
}
