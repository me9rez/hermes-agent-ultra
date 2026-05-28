//! P2-1 Discord `reply_to_mode` (config + REST `message_reference`).

#![cfg(feature = "discord")]

use hermes_config::PlatformConfig;
use hermes_gateway::platforms::discord::{DiscordAdapter, DiscordConfig, ReplyToMode};
use serde_json::Value;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn mock_config(server: &MockServer, reply_to_mode: ReplyToMode) -> DiscordConfig {
    let mut config = DiscordConfig::for_test("test-token");
    config.rest_api_base = format!("{}/api/v10", server.uri());
    config.reply_to_mode = reply_to_mode;
    config
}

async fn mount_message_post(server: &MockServer, times: u64) {
    Mock::given(method("POST"))
        .and(path_regex(r"/api/v10/channels/.*/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "id": "out-msg", "channel_id": "ch1" })),
        )
        .expect(times)
        .mount(server)
        .await;
}

fn body_has_reference(body: &[u8]) -> bool {
    let v: Value = serde_json::from_slice(body).expect("json body");
    v.get("message_reference").is_some()
}

#[test]
fn reply_to_mode_parse_defaults_and_invalid() {
    assert_eq!(ReplyToMode::parse(None), ReplyToMode::First);
    assert_eq!(ReplyToMode::parse(Some("")), ReplyToMode::First);
    assert_eq!(ReplyToMode::parse(Some("off")), ReplyToMode::Off);
    assert_eq!(ReplyToMode::parse(Some("first")), ReplyToMode::First);
    assert_eq!(ReplyToMode::parse(Some("all")), ReplyToMode::All);
    assert_eq!(ReplyToMode::parse(Some("invalid")), ReplyToMode::First);
}

#[test]
fn reply_to_mode_from_platform_extra_and_env() {
    let mut platform = PlatformConfig::default();
    platform
        .extra
        .insert("reply_to_mode".into(), serde_json::json!("all"));
    let cfg = DiscordConfig::from_platform(&platform, "tok".into());
    assert_eq!(cfg.reply_to_mode, ReplyToMode::All);

    // SAFETY: single-threaded test binary.
    unsafe {
        std::env::set_var("DISCORD_REPLY_TO_MODE", "off");
    }
    let cfg = DiscordConfig::from_platform(&PlatformConfig::default(), "tok".into());
    assert_eq!(cfg.reply_to_mode, ReplyToMode::Off);
    unsafe {
        std::env::remove_var("DISCORD_REPLY_TO_MODE");
    }
}

#[tokio::test]
async fn first_mode_only_first_chunk_references() {
    let server = MockServer::start().await;
    mount_message_post(&server, 2).await;

    let config = mock_config(&server, ReplyToMode::First);
    let adapter = DiscordAdapter::new(config).unwrap();
    let content = "x".repeat(2001);

    adapter
        .send_text_with_reply("ch1", &content, Some("orig123"))
        .await
        .expect("send ok");

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 2);
    assert!(body_has_reference(&requests[0].body));
    assert!(!body_has_reference(&requests[1].body));
}

#[tokio::test]
async fn all_mode_every_chunk_references() {
    let server = MockServer::start().await;
    mount_message_post(&server, 2).await;

    let config = mock_config(&server, ReplyToMode::All);
    let adapter = DiscordAdapter::new(config).unwrap();
    let content = "y".repeat(2001);

    adapter
        .send_text_with_reply("ch1", &content, Some("orig456"))
        .await
        .expect("send ok");

    let requests = server.received_requests().await.expect("requests");
    assert!(body_has_reference(&requests[0].body));
    assert!(body_has_reference(&requests[1].body));
}

#[tokio::test]
async fn off_mode_never_references() {
    let server = MockServer::start().await;
    mount_message_post(&server, 1).await;

    let config = mock_config(&server, ReplyToMode::Off);
    let adapter = DiscordAdapter::new(config).unwrap();

    adapter
        .send_text_with_reply("ch1", "hello", Some("orig789"))
        .await
        .expect("send ok");

    let requests = server.received_requests().await.expect("requests");
    assert!(!body_has_reference(&requests[0].body));
}

#[tokio::test]
async fn no_reply_to_never_references() {
    let server = MockServer::start().await;
    mount_message_post(&server, 1).await;

    let config = mock_config(&server, ReplyToMode::All);
    let adapter = DiscordAdapter::new(config).unwrap();

    adapter
        .send_text_with_reply("ch1", "hello", None)
        .await
        .expect("send ok");

    let requests = server.received_requests().await.expect("requests");
    assert!(!body_has_reference(&requests[0].body));
}
