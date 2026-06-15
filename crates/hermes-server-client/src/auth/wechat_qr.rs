//! WeChat QR scan login — stub until server API docs are provided.

use async_trait::async_trait;

use super::provider::{AuthContext, AuthProvider};
use super::types::{AuthPollResult, AuthUserInput, LoginMethod, PendingLogin};
use crate::error::ServerClientError;

pub struct WeChatQrAuthProvider;

#[async_trait]
impl AuthProvider for WeChatQrAuthProvider {
    fn method(&self) -> LoginMethod {
        LoginMethod::WechatQr
    }

    async fn start(&self, _ctx: &AuthContext<'_>) -> Result<PendingLogin, ServerClientError> {
        Err(ServerClientError::not_configured("WeChat QR login"))
    }

    async fn poll_or_submit(
        &self,
        _ctx: &AuthContext<'_>,
        _pending: &PendingLogin,
        _input: AuthUserInput,
    ) -> Result<AuthPollResult, ServerClientError> {
        Err(ServerClientError::not_configured("WeChat QR login"))
    }
}
