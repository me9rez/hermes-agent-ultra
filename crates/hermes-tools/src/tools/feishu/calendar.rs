//! Feishu Calendar tool — list events, create events, query free/busy.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{json, Value};

use hermes_core::{tool_schema, JsonSchema, ToolError, ToolHandler, ToolSchema};

use super::FeishuApiClient;

/// Handler for the `feishu_calendar` tool.
pub struct FeishuCalendarHandler {
    client: Arc<FeishuApiClient>,
}

impl FeishuCalendarHandler {
    pub fn new(client: Arc<FeishuApiClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ToolHandler for FeishuCalendarHandler {
    async fn execute(&self, params: Value) -> Result<String, ToolError> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing required param: action".into()))?;

        let calendar_id = params
            .get("calendar_id")
            .and_then(|v| v.as_str())
            .unwrap_or("primary");

        let data = match action {
            "list_events" => {
                let mut query: Vec<(&str, &str)> = Vec::new();
                let start_time;
                let end_time;
                if let Some(v) = params.get("start_time").and_then(|v| v.as_str()) {
                    start_time = v.to_string();
                    query.push(("start_time", &start_time));
                }
                if let Some(v) = params.get("end_time").and_then(|v| v.as_str()) {
                    end_time = v.to_string();
                    query.push(("end_time", &end_time));
                }
                let path = format!("/calendar/v4/calendars/{calendar_id}/events");
                self.client.get(&path, &query).await?
            }
            "create_event" => {
                let summary = params
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParams("create_event requires 'summary'".into())
                    })?;

                let mut body = serde_json::Map::new();
                body.insert("summary".into(), json!(summary));

                if let Some(v) = params.get("start_time").and_then(|v| v.as_str()) {
                    body.insert(
                        "start_time".into(),
                        json!({ "timestamp": v }),
                    );
                }
                if let Some(v) = params.get("end_time").and_then(|v| v.as_str()) {
                    body.insert(
                        "end_time".into(),
                        json!({ "timestamp": v }),
                    );
                }
                if let Some(v) = params.get("description").and_then(|v| v.as_str()) {
                    body.insert("description".into(), json!(v));
                }
                if let Some(arr) = params.get("attendees").and_then(|v| v.as_array()) {
                    let attendees: Vec<Value> = arr
                        .iter()
                        .map(|a| json!({ "type": "user", "user_id": a.as_str().unwrap_or("") }))
                        .collect();
                    body.insert("attendees".into(), json!(attendees));
                }

                let path = format!("/calendar/v4/calendars/{calendar_id}/events");
                self.client.post(&path, &Value::Object(body)).await?
            }
            "free_busy" => {
                let mut body = serde_json::Map::new();
                if let Some(v) = params.get("start_time").and_then(|v| v.as_str()) {
                    body.insert("time_min".into(), json!(v));
                }
                if let Some(v) = params.get("end_time").and_then(|v| v.as_str()) {
                    body.insert("time_max".into(), json!(v));
                }
                if let Some(v) = params.get("user_id").and_then(|v| v.as_str()) {
                    body.insert("user_id".into(), json!(v));
                }
                self.client
                    .post("/calendar/v4/freebusy/list", &Value::Object(body))
                    .await?
            }
            other => {
                return Err(ToolError::InvalidParams(format!(
                    "Unknown action '{other}'. Expected: list_events, create_event, free_busy"
                )));
            }
        };

        serde_json::to_string_pretty(&data)
            .map_err(|e| ToolError::ExecutionFailed(format!("JSON serialize error: {e}")))
    }

    fn schema(&self) -> ToolSchema {
        let mut props = IndexMap::new();
        props.insert(
            "action".into(),
            json!({
                "type": "string",
                "enum": ["list_events", "create_event", "free_busy"],
                "description": "Calendar operation to perform"
            }),
        );
        props.insert(
            "start_time".into(),
            json!({
                "type": "string",
                "description": "Start time in ISO 8601 format"
            }),
        );
        props.insert(
            "end_time".into(),
            json!({
                "type": "string",
                "description": "End time in ISO 8601 format"
            }),
        );
        props.insert(
            "summary".into(),
            json!({
                "type": "string",
                "description": "Event title (required for create_event)"
            }),
        );
        props.insert(
            "description".into(),
            json!({
                "type": "string",
                "description": "Event description"
            }),
        );
        props.insert(
            "attendees".into(),
            json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "List of attendee email addresses or open_ids"
            }),
        );
        props.insert(
            "user_id".into(),
            json!({
                "type": "string",
                "description": "User ID for free/busy query"
            }),
        );
        props.insert(
            "calendar_id".into(),
            json!({
                "type": "string",
                "description": "Calendar ID (default: primary)",
                "default": "primary"
            }),
        );

        tool_schema(
            "feishu_calendar",
            concat!(
                "Interact with Feishu/Lark Calendar. ",
                "Actions: list_events (list calendar events), ",
                "create_event (create a new event), ",
                "free_busy (query free/busy status)."
            ),
            JsonSchema::object(props, vec!["action".into()]),
        )
    }
}
