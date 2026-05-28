//! P2-2 Discord `allowed_mentions` safe defaults + REST body.

#![cfg(feature = "discord")]

use hermes_config::PlatformConfig;
use hermes_gateway::platforms::discord::{
    parse_bool_like, DiscordAdapter, DiscordAllowedMentions, DiscordConfig,
};
use serde_json::Value;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn parse_bool_like_truthy_falsy() {
    for v in ["true", "1", "yes", "on", "TRUE"] {
        assert!(parse_bool_like(v), "{v}");
    }
    for v in ["false", "0", "no", "off", "", "maybe"] {
        assert!(!parse_bool_like(v), "{v}");
    }
}

#[test]
fn default_allowed_mentions_blocks_everyone_roles() {
    let am = DiscordAllowedMentions::default();
    let v = am.to_api_value();
    let parse = v["parse"].as_array().unwrap();
    assert_eq!(parse.len(), 1);
    assert_eq!(parse[0], "users");
    assert_eq!(v["replied_user"], true);
}

#[test]
fn env_everyone_override() {
    unsafe {
        std::env::set_var("DISCORD_ALLOW_MENTION_EVERYONE", "true");
    }
    let am = DiscordAllowedMentions::from_platform(&PlatformConfig::default());
    assert!(am.everyone);
    let v = am.to_api_value();
    let parse = v["parse"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(parse.contains(&"everyone"));
    unsafe {
        std::env::remove_var("DISCORD_ALLOW_MENTION_EVERYONE");
    }
}

#[tokio::test]
async fn post_includes_allowed_mentions() {
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
    config.allowed_mentions = DiscordAllowedMentions::default();

    let adapter = DiscordAdapter::new(config).unwrap();
    adapter
        .send_text("ch1", "ping @everyone")
        .await
        .expect("send ok");

    let requests = server.received_requests().await.expect("requests");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let am = &body["allowed_mentions"];
    assert_eq!(am["parse"], serde_json::json!(["users"]));
    assert_eq!(am["replied_user"], true);
}
