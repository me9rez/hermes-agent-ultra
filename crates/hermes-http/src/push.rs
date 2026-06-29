use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::HttpServerState;

#[derive(Debug, Deserialize)]
pub struct PushRegisterRequest {
    pub device_id: String,
    pub token: String,
    pub platform: String,
    pub manufacturer: Option<String>,
}

pub async fn register_push(
    State(_state): State<HttpServerState>,
    Json(req): Json<PushRegisterRequest>,
) -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "device_id": req.device_id,
        "platform": req.platform,
    }))
}

pub async fn send_push() -> impl IntoResponse {
    Json(json!({ "ok": true, "queued": 0 }))
}

pub fn push_routes() -> axum::Router<HttpServerState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/api/push/register", post(register_push))
        .route("/api/push/send", post(send_push))
}
