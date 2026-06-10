//! POI session-end commit: rules extraction + persist via `session_end_hooks`.

use std::sync::Arc;

use futures::StreamExt;
use hermes_agent::{AgentConfig, AgentLoop, InterestStore, agent_loop::ToolRegistry};
use hermes_config::InterestConfig;
use hermes_core::{AgentError, LlmProvider, Message, ToolSchema};

struct NoopProvider;

#[async_trait::async_trait]
impl LlmProvider for NoopProvider {
    async fn chat_completion(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _max_tokens: Option<u32>,
        _temperature: Option<f64>,
        _model: Option<&str>,
        _extra_body: Option<&serde_json::Value>,
    ) -> Result<hermes_core::LlmResponse, AgentError> {
        Ok(hermes_core::LlmResponse {
            message: Message::assistant("ok"),
            ..Default::default()
        })
    }

    fn chat_completion_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _max_tokens: Option<u32>,
        _temperature: Option<f64>,
        _model: Option<&str>,
        _extra_body: Option<&serde_json::Value>,
    ) -> futures::stream::BoxStream<'static, Result<hermes_core::StreamChunk, AgentError>> {
        futures::stream::empty().boxed()
    }
}

#[tokio::test]
async fn session_end_hooks_persists_rule_extracted_poi() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_string_lossy().into_owned();
    let mut interest = InterestConfig::default();
    interest.enabled = true;
    interest.extract_mode = "rules".to_string();
    interest.per_turn_buffer = true;
    interest.per_turn_persist = false;

    let db_path = tmp.path().join("interest.db");
    let store = InterestStore::open(&db_path, interest.clone()).expect("open store");
    let mut cfg = AgentConfig::default();
    cfg.interest = interest.clone();
    cfg.hermes_home = Some(home.clone());
    cfg.session_id = Some("poi-session-end-test".into());

    let agent = AgentLoop::new(cfg, Arc::new(ToolRegistry::new()), Arc::new(NoopProvider))
        .with_interest_store(Arc::new(std::sync::Mutex::new(store)));

    let sample = "Help me continue the Rust parity port in crates/hermes-parity-tests please";
    let messages = vec![Message::user(sample)];

    hermes_agent::hooks::session_end_hooks(&agent, &messages, false, false, 1, true);

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let db_path = tmp.path().join("interest.db");
    let store = hermes_agent::InterestStore::open(&db_path, interest).expect("reopen");
    let topics = store.top_topics(20).expect("top_topics");
    assert!(
        !topics.is_empty(),
        "expected session-end POI commit to persist at least one topic"
    );
}
