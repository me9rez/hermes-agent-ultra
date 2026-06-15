//! Remote LLM server client — authentication and OpenAI-compatible inference gateway.
//!
//! Agent business logic (AgentLoop, tools, sessions) stays local; this crate only
//! talks to the server for login and LLM HTTP calls.

pub mod auth;
pub mod doctor;
pub mod error;
pub mod llm;
pub mod session;
pub mod transport;

pub use auth::{
    AuthManager, AuthPollResult, AuthUserInput, LoginMethod, PendingLogin, WhoamiStatus,
};
pub use doctor::{DoctorReport, run_doctor};
pub use error::ServerClientError;
pub use llm::ServerLlmProvider;
pub use session::{SERVER_TOKEN_PROVIDER, ServerSession, ServerTokens, TokenSource};
pub use transport::HttpTransport;

#[cfg(test)]
mod tests {
    use super::*;
    use hermes_config::ServerConfig;

    #[test]
    fn login_method_parse_aliases() {
        assert_eq!(LoginMethod::parse("wechat"), Some(LoginMethod::WechatQr));
        assert_eq!(LoginMethod::parse("email_otp"), Some(LoginMethod::EmailOtp));
        assert!(LoginMethod::parse("unknown").is_none());
    }

    #[tokio::test]
    async fn auth_manager_wechat_start_returns_not_configured() {
        let mut config = ServerConfig::default();
        config.enabled = true;
        config.base_url = "https://example.com".to_string();
        let manager = AuthManager::new(config, std::env::temp_dir()).expect("manager");
        let err = manager
            .start_login(LoginMethod::WechatQr)
            .await
            .expect_err("stub");
        assert!(matches!(err, ServerClientError::NotConfigured(_)));
    }

    #[tokio::test]
    async fn auth_manager_disabled_errors() {
        let config = ServerConfig::default();
        let result = AuthManager::new(config, std::env::temp_dir());
        assert!(matches!(result, Err(ServerClientError::Disabled)));
    }
}
