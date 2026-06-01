//! Feishu (Lark) OpenAPI native tools.
//!
//! Provides calendar, docs, task and chat-history tools that talk to
//! the Feishu Open Platform REST API with automatic tenant-token management.

use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use hermes_core::ToolError;

pub mod calendar;
pub mod docs;
pub mod task;

pub use calendar::FeishuCalendarHandler;
pub use docs::FeishuDocsHandler;
pub use task::FeishuTaskHandler;

/// Token valid for 2 hours.
const TOKEN_EXPIRY_SECS: u64 = 2 * 60 * 60;
/// Refresh 5 minutes before real expiry.
const TOKEN_REFRESH_MARGIN_SECS: u64 = 5 * 60;

struct CachedToken {
    value: String,
    obtained_at: Instant,
}

impl CachedToken {
    fn is_expired(&self) -> bool {
        self.obtained_at.elapsed()
            > Duration::from_secs(TOKEN_EXPIRY_SECS - TOKEN_REFRESH_MARGIN_SECS)
    }
}

/// Thin HTTP client that owns a tenant-access-token cache.
pub struct FeishuApiClient {
    client: Client,
    app_id: String,
    app_secret: String,
    base_url: String,
    tenant_token: RwLock<Option<CachedToken>>,
}

impl FeishuApiClient {
    /// Build a client from environment variables.
    ///
    /// Returns `None` when `FEISHU_APP_ID` or `FEISHU_APP_SECRET` is missing,
    /// so callers can do conditional registration with `if let Some(…)`.
    pub fn from_env() -> Option<Self> {
        let app_id = std::env::var("FEISHU_APP_ID").ok()?;
        let app_secret = std::env::var("FEISHU_APP_SECRET").ok()?;
        let domain = std::env::var("FEISHU_DOMAIN").unwrap_or_else(|_| "feishu".into());
        let base_url = if domain == "lark" {
            "https://open.larksuite.com/open-apis".into()
        } else {
            "https://open.feishu.cn/open-apis".into()
        };
        Some(Self {
            client: Client::new(),
            app_id,
            app_secret,
            base_url,
            tenant_token: RwLock::new(None),
        })
    }

    // -- token management ---------------------------------------------------

    /// Return a valid tenant-access-token, refreshing when necessary.
    pub async fn get_token(&self) -> Result<String, ToolError> {
        // Fast path: cached and still valid.
        {
            let guard = self.tenant_token.read().await;
            if let Some(cached) = guard.as_ref() {
                if !cached.is_expired() {
                    return Ok(cached.value.clone());
                }
            }
        }

        // Slow path: fetch a new token.
        let mut guard = self.tenant_token.write().await;

        // Double-check after acquiring write lock.
        if let Some(cached) = guard.as_ref() {
            if !cached.is_expired() {
                return Ok(cached.value.clone());
            }
        }

        let url = format!(
            "{}/auth/v3/tenant_access_token/internal",
            self.base_url
        );
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret,
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Feishu token request failed: {e}")))?;

        let json: Value = resp
            .json()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Feishu token response parse error: {e}")))?;

        let code = json
            .get("code")
            .and_then(|c| c.as_i64())
            .unwrap_or(-1);
        if code != 0 {
            let msg = json
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(ToolError::ExecutionFailed(format!(
                "Feishu token error: code={code}, msg={msg}"
            )));
        }

        let token = json
            .get("tenant_access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| {
                ToolError::ExecutionFailed("Feishu token response missing tenant_access_token".into())
            })?
            .to_string();

        debug!("Feishu tenant token refreshed");
        *guard = Some(CachedToken {
            value: token.clone(),
            obtained_at: Instant::now(),
        });

        Ok(token)
    }

    // -- HTTP helpers -------------------------------------------------------

    /// Authenticated GET, returns `data` field from Feishu response.
    pub async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, ToolError> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.base_url, path);

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .query(query)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Feishu GET {path} failed: {e}")))?;

        self.parse_response(path, resp).await
    }

    /// Authenticated POST, returns `data` field from Feishu response.
    pub async fn post(&self, path: &str, body: &Value) -> Result<Value, ToolError> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.base_url, path);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Feishu POST {path} failed: {e}")))?;

        self.parse_response(path, resp).await
    }

    /// Authenticated PATCH, returns `data` field from Feishu response.
    pub async fn patch(&self, path: &str, body: &Value) -> Result<Value, ToolError> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.base_url, path);

        let resp = self
            .client
            .patch(&url)
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Feishu PATCH {path} failed: {e}")))?;

        self.parse_response(path, resp).await
    }

    /// Parse a Feishu API response: check `code == 0` and return `data`.
    async fn parse_response(
        &self,
        path: &str,
        resp: reqwest::Response,
    ) -> Result<Value, ToolError> {
        let json: Value = resp.json().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Feishu {path} response parse error: {e}"))
        })?;

        let code = json.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = json
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            warn!(path, code, msg, "Feishu API error");
            return Err(ToolError::ExecutionFailed(format!(
                "Feishu API error on {path}: code={code}, msg={msg}"
            )));
        }

        Ok(json.get("data").cloned().unwrap_or(Value::Null))
    }
}
