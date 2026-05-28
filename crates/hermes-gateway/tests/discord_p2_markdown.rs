//! P2-3 Discord outbound Markdown via `to_discord_markdown`.

#![cfg(feature = "discord")]

use hermes_core::traits::{ParseMode, PlatformAdapter};
use hermes_gateway::platforms::discord::{DiscordAdapter, DiscordConfig};
use serde_json::Value;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn markdown_header_converted_in_post_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"/api/v10/channels/.*/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "id": "m1", "channel_id": "ch1" })),
        )
        .mount(&server)
        .await;

    let mut config = DiscordConfig::for_test("test-token");
    config.rest_api_base = format!("{}/api/v10", server.uri());
    let adapter = DiscordAdapter::new(config).unwrap();

    adapter
        .send_message_with_id("ch1", "# Title\nbody", Some(ParseMode::Markdown))
        .await
        .expect("send ok");

    let requests = server.received_requests().await.expect("requests");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let content = body["content"].as_str().unwrap();
    assert!(content.contains("**Title**"));
    assert!(!content.contains("# Title"));
}

#[tokio::test]
async fn plain_mode_skips_markdown_conversion() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"/api/v10/channels/.*/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "id": "m2", "channel_id": "ch1" })),
        )
        .mount(&server)
        .await;

    let mut config = DiscordConfig::for_test("test-token");
    config.rest_api_base = format!("{}/api/v10", server.uri());
    let adapter = DiscordAdapter::new(config).unwrap();

    adapter
        .send_message_with_id("ch1", "# Title", Some(ParseMode::Plain))
        .await
        .expect("send ok");

    let requests = server.received_requests().await.expect("requests");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["content"], "# Title");
}
