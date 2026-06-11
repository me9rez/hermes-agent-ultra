//! Shared HTTP client for meta-search HTML fetches.

use rand::RngExt;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, COOKIE, REFERER};
use reqwest::Client;
use std::time::Duration;

pub const BROWSER_ACCEPT_HTML: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.4 Safari/605.1.15",
];

/// Default max HTML body bytes read per CN engine response (raised from 512KB; overridable).
pub fn max_cn_html_bytes() -> usize {
    std::env::var("HERMES_CN_SEARCH_MAX_HTML_BYTES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(2_000_000)
        .min(8_000_000)
}

/// Pick a browser-like user agent per request (websurfx-style rotation).
pub fn meta_search_user_agent() -> &'static str {
    let mut rng = rand::rng();
    let idx = rng.random_range(0..USER_AGENTS.len());
    USER_AGENTS[idx]
}

pub fn build_meta_search_client(default_timeout_secs: u64) -> Client {
    Client::builder()
        .timeout(Duration::from_secs(default_timeout_secs.max(1)))
        .user_agent(meta_search_user_agent())
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Per-engine request headers to reduce bot detection (Referer/Cookie patterns from websurfx).
pub fn cn_request_headers(engine_id: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(BROWSER_ACCEPT_HTML),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
    );
    match engine_id {
        "bing_cn" => {
            headers.insert(REFERER, HeaderValue::from_static("https://www.bing.com/"));
            if let Ok(cookie) = HeaderValue::from_str(
                "_EDGE_V=1; SRCHHPGUSR=SRCHLANG=zh-Hans; _UR=QS=0&TQS=0",
            ) {
                headers.insert(COOKIE, cookie);
            }
        }
        "sogou" => {
            headers.insert(REFERER, HeaderValue::from_static("https://www.sogou.com/"));
        }
        _ => {
            headers.insert(REFERER, HeaderValue::from_static("https://www.google.com/"));
        }
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_pool_is_non_empty() {
        assert!(meta_search_user_agent().contains("Mozilla/"));
    }

    #[test]
    fn max_html_bytes_default_is_two_megabytes() {
        hermes_core::test_env::remove_var("HERMES_CN_SEARCH_MAX_HTML_BYTES");
        assert_eq!(max_cn_html_bytes(), 2_000_000);
    }

    #[test]
    fn bing_cn_headers_include_cookie() {
        let headers = cn_request_headers("bing_cn");
        assert!(headers.contains_key(COOKIE));
        assert!(headers.contains_key(REFERER));
    }

    #[test]
    fn sogou_headers_use_sogou_referer() {
        let headers = cn_request_headers("sogou");
        assert_eq!(
            headers.get(REFERER).and_then(|v| v.to_str().ok()),
            Some("https://www.sogou.com/")
        );
    }
}
