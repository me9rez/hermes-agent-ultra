//! Flowy image generation (sync) and Seedance video task (async) APIs.

use std::time::Duration;

use serde_json::{Map, Value, json};
use tracing::debug;

use crate::error::ServerClientError;
use crate::flowy::media_types::{
    CreateVideoTaskResponse, ImageGenerationRequest, VIDEO_TASK_STATUS_CANCELLED,
    VIDEO_TASK_STATUS_EXPIRED, VIDEO_TASK_STATUS_FAILED, VideoTaskRecord,
};
use crate::flowy::response::FlowyEnvelope;
use crate::session::ServerSession;
use crate::transport::HttpTransport;

use super::FlowyApiClient;

const DEFAULT_VIDEO_POLL_INTERVAL_SECS: u64 = 5;
const DEFAULT_VIDEO_POLL_TIMEOUT_SECS: u64 = 600;

impl FlowyApiClient {
    /// `POST {LLM根}/images/generations` — upstream JSON passthrough on success.
    pub async fn images_generations(
        &self,
        session: &ServerSession,
        body: Value,
    ) -> Result<Value, ServerClientError> {
        self.post_upstream_json(&self.llm_transport, "/images/generations", session, body)
            .await
    }

    /// `POST {LLM根}/images/edits` — image-to-image / edit proxy.
    pub async fn images_edits(
        &self,
        session: &ServerSession,
        body: Value,
    ) -> Result<Value, ServerClientError> {
        self.post_upstream_json(&self.llm_transport, "/images/edits", session, body)
            .await
    }

    /// Convenience wrapper for minimal text-to-image requests.
    pub async fn generate_image(
        &self,
        session: &ServerSession,
        req: &ImageGenerationRequest,
    ) -> Result<Value, ServerClientError> {
        let mut body = Map::new();
        body.insert("model".into(), json!(req.model));
        body.insert("prompt".into(), json!(req.prompt));
        if let Some(url) = &req.image_url {
            body.insert("image_url".into(), json!(url));
        }
        if let Value::Object(extra) = &req.extra {
            for (k, v) in extra {
                body.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        let path = if req.image_url.is_some() {
            "/images/edits"
        } else {
            "/images/generations"
        };
        self.post_upstream_json(&self.llm_transport, path, session, Value::Object(body))
            .await
    }

    /// `POST {业务根}/video/generations/tasks`
    pub async fn create_video_task(
        &self,
        session: &ServerSession,
        body: Value,
    ) -> Result<CreateVideoTaskResponse, ServerClientError> {
        self.post_data("/video/generations/tasks", Some(session), &body)
            .await
    }

    /// `GET {业务根}/video/generations/tasks/:id`
    pub async fn get_video_task(
        &self,
        session: &ServerSession,
        local_id: i64,
    ) -> Result<VideoTaskRecord, ServerClientError> {
        let path = format!("/video/generations/tasks/{local_id}");
        self.get_data(&path, Some(session)).await
    }

    /// Poll until the video task reaches a terminal status.
    pub async fn poll_video_task(
        &self,
        session: &ServerSession,
        local_id: i64,
        poll_interval_secs: u64,
        timeout_secs: u64,
    ) -> Result<VideoTaskRecord, ServerClientError> {
        let interval = Duration::from_secs(poll_interval_secs.max(1));
        let timeout = Duration::from_secs(timeout_secs.max(30));
        let started = std::time::Instant::now();

        loop {
            let record = self.get_video_task(session, local_id).await?;
            if record.is_terminal() {
                return Ok(record);
            }
            if started.elapsed() >= timeout {
                return Err(ServerClientError::InvalidResponse(format!(
                    "video task {local_id} timed out after {}s (status={})",
                    timeout.as_secs(),
                    record.status
                )));
            }
            debug!(local_id, status = record.status, "polling video task");
            tokio::time::sleep(interval).await;
        }
    }

    /// Create a Seedance video task and poll until completion.
    pub async fn generate_video(
        &self,
        session: &ServerSession,
        body: Value,
    ) -> Result<VideoTaskRecord, ServerClientError> {
        self.generate_video_with_timeout(session, body, DEFAULT_VIDEO_POLL_TIMEOUT_SECS)
            .await
    }

    /// Create a Seedance video task and poll until completion with a custom timeout.
    pub async fn generate_video_with_timeout(
        &self,
        session: &ServerSession,
        body: Value,
        timeout_secs: u64,
    ) -> Result<VideoTaskRecord, ServerClientError> {
        let created: CreateVideoTaskResponse = self.create_video_task(session, body).await?;
        self.poll_video_task(
            session,
            created.id,
            DEFAULT_VIDEO_POLL_INTERVAL_SECS,
            timeout_secs.max(30),
        )
        .await
    }

    /// Build an Ark-compatible video create body from high-level parameters.
    pub fn build_video_create_body(
        model: &str,
        prompt: &str,
        image_url: Option<&str>,
        duration: Option<u32>,
        aspect_ratio: &str,
        resolution: Option<&str>,
        negative_prompt: Option<&str>,
        seed: Option<i64>,
        watermark: bool,
    ) -> Value {
        let mut content = vec![json!({"type": "text", "text": prompt})];
        if let Some(url) = image_url.filter(|u| !u.trim().is_empty()) {
            content.push(json!({
                "type": "image_url",
                "image_url": {"url": url},
                "role": "first_frame"
            }));
        }

        let mut body = Map::new();
        body.insert("model".into(), json!(model));
        body.insert("content".into(), Value::Array(content));
        body.insert("ratio".into(), json!(aspect_ratio));
        body.insert("watermark".into(), json!(watermark));
        if let Some(d) = duration {
            body.insert("duration".into(), json!(d));
        }
        if let Some(r) = resolution.filter(|s| !s.is_empty()) {
            body.insert("resolution".into(), json!(r));
        }
        if let Some(neg) = negative_prompt.filter(|s| !s.is_empty()) {
            body.insert("negative_prompt".into(), json!(neg));
        }
        if let Some(s) = seed {
            body.insert("seed".into(), json!(s));
        }
        Value::Object(body)
    }

    async fn post_upstream_json(
        &self,
        transport: &HttpTransport,
        path: &str,
        session: &ServerSession,
        body: Value,
    ) -> Result<Value, ServerClientError> {
        let resp = transport.post_json(path, Some(session), body).await?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ServerClientError::Http(e.to_string()))?;

        if status == 200 {
            return serde_json::from_str(&text).map_err(|e| {
                ServerClientError::InvalidResponse(format!("upstream image JSON: {e}"))
            });
        }

        if let Ok(env) = FlowyEnvelope::parse_body(&text) {
            return Err(ServerClientError::Api {
                code: env.code,
                msg: env.msg,
            });
        }

        Err(ServerClientError::Http(format!("HTTP {status}: {text}")))
    }
}

/// Map local video task status to a human-readable label.
pub fn video_task_status_label(status: i32) -> &'static str {
    match status {
        1 => "queued",
        2 => "running",
        3 => "cancelled",
        4 => "succeeded",
        5 => "failed",
        6 => "expired",
        _ => "unknown",
    }
}

/// Error message for terminal non-success video statuses.
pub fn video_task_failure_message(record: &VideoTaskRecord) -> String {
    let detail = record
        .failure_detail()
        .map(|d| format!(": {d}"))
        .unwrap_or_default();
    match record.status {
        VIDEO_TASK_STATUS_FAILED => format!("video generation failed{detail}"),
        VIDEO_TASK_STATUS_EXPIRED => format!("video task expired{detail}"),
        VIDEO_TASK_STATUS_CANCELLED => format!("video task cancelled{detail}"),
        _ => format!(
            "video task ended with status {} ({}){detail}",
            record.status,
            video_task_status_label(record.status)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flowy::media_types::VideoTaskRecord;
    use hermes_config::ServerConfig;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config(base_url: &str) -> ServerConfig {
        ServerConfig {
            base_url: base_url.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn build_video_create_body_text_only() {
        let body = FlowyApiClient::build_video_create_body(
            "flowy/doubao-seedance-1-0-pro",
            "a cat in the sun",
            None,
            Some(5),
            "16:9",
            Some("720p"),
            None,
            None,
            false,
        );
        assert_eq!(body["model"], "flowy/doubao-seedance-1-0-pro");
        assert_eq!(body["duration"], 5);
        assert_eq!(body["ratio"], "16:9");
        assert!(body["content"].is_array());
    }

    #[test]
    fn build_video_create_body_with_first_frame() {
        let body = FlowyApiClient::build_video_create_body(
            "flowy/doubao-seedance-1-0-pro",
            "animate this",
            Some("https://example.com/frame.png"),
            Some(8),
            "9:16",
            None,
            None,
            None,
            false,
        );
        let content = body["content"].as_array().expect("content array");
        assert_eq!(content.len(), 2);
        assert_eq!(content[1]["role"], "first_frame");
    }

    #[tokio::test]
    async fn create_and_poll_video_task_success() {
        let business = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/video/generations/tasks"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"code":200,"msg":"ok","data":{"id":42}}"#),
            )
            .mount(&business)
            .await;

        let poll_count = std::sync::atomic::AtomicU32::new(0);
        Mock::given(method("GET"))
            .and(path("/video/generations/tasks/42"))
            .respond_with(move |_: &wiremock::Request| {
                let n = poll_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                let status = if n >= 2 { 4 } else { 2 };
                ResponseTemplate::new(200).set_body_string(format!(
                    r#"{{"code":200,"msg":"ok","data":{{"id":42,"status":{status},"result":{{"content":{{"video_url":"https://cdn.example/v.mp4"}}}}}}}}"#
                ))
            })
            .mount(&business)
            .await;

        let config = test_config(&business.uri());
        let api = FlowyApiClient::new(&config).expect("client");
        let tmp = tempfile::tempdir().expect("tmpdir");
        hermes_core::test_env::set_var("HERMES_SERVER_TOKEN", "jwt-test");
        let session = ServerSession::from_config(&config, tmp.path());
        let body = FlowyApiClient::build_video_create_body(
            "flowy/doubao-seedance-1-0-pro",
            "test",
            None,
            Some(5),
            "16:9",
            None,
            None,
            None,
            false,
        );
        let record = api.generate_video(&session, body).await.expect("video");
        assert!(record.is_success());
        assert_eq!(
            record.video_url().as_deref(),
            Some("https://cdn.example/v.mp4")
        );
    }

    #[test]
    fn video_task_record_terminal_detection() {
        let mut rec = VideoTaskRecord {
            id: 1,
            task_id: None,
            status: 2,
            result: None,
            created_at: None,
            updated_at: None,
        };
        assert!(!rec.is_terminal());
        rec.status = 4;
        assert!(rec.is_terminal());
        assert!(rec.is_success());
    }
}
