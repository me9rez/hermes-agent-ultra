//! Shared HTTP client and retry logic for market data providers.

use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use tracing::warn;

use crate::error::TradingError;

const MAX_ATTEMPTS: u32 = 3;
const BACKOFF_MS: [u64; 3] = [200, 400, 800];

/// HTTP client with connect timeout 10s and total request timeout 30s.
#[must_use]
pub fn default_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build reqwest client")
}

/// Send an HTTP request with exponential backoff retry (max 3 attempts).
///
/// Retries on transport errors, 5xx, and 429 (honors `Retry-After` when present).
pub async fn send_with_retry(
    build: impl Fn() -> reqwest::RequestBuilder,
) -> Result<Response, TradingError> {
    let mut last_err: Option<TradingError> = None;

    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            let delay_ms = BACKOFF_MS[(attempt - 1) as usize];
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }

        let response = match build().send().await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(attempt, error = %e, "HTTP request failed");
                last_err = Some(TradingError::Http(e));
                continue;
            }
        };

        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after_secs = parse_retry_after(response.headers());
            warn!(attempt, retry_after_secs, "Rate limited (429), backing off");
            if attempt + 1 < MAX_ATTEMPTS {
                tokio::time::sleep(Duration::from_secs(retry_after_secs)).await;
                continue;
            }
            let body = response.text().await.unwrap_or_default();
            return Err(TradingError::InvalidResponse(format!(
                "HTTP 429 Too Many Requests after {MAX_ATTEMPTS} attempts: {body}"
            )));
        }

        if status.is_server_error() {
            let body = response.text().await.unwrap_or_default();
            warn!(%status, attempt, body = %body, "Server error, retrying");
            last_err = Some(TradingError::InvalidResponse(format!(
                "HTTP {status}: {body}"
            )));
            continue;
        }

        return Ok(response);
    }

    Err(last_err.unwrap_or_else(|| {
        TradingError::InvalidResponse("HTTP request failed after retries".into())
    }))
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> u64 {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn send_with_retry_succeeds_after_429() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/klines"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/klines"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let url = format!("{}/klines", server.uri());
        let client = default_client();
        let resp = send_with_retry(|| client.get(&url)).await.unwrap();
        assert!(resp.status().is_success());
    }
}
