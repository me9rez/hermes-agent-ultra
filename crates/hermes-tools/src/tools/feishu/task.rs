//! Feishu Task tool — create, list, update, complete tasks.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{json, Value};

use hermes_core::{tool_schema, JsonSchema, ToolError, ToolHandler, ToolSchema};

use super::FeishuApiClient;

/// Handler for the `feishu_task` tool.
pub struct FeishuTaskHandler {
    client: Arc<FeishuApiClient>,
}

impl FeishuTaskHandler {
    pub fn new(client: Arc<FeishuApiClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ToolHandler for FeishuTaskHandler {
    async fn execute(&self, params: Value) -> Result<String, ToolError> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing required param: action".into()))?;

        let data = match action {
            "create" => {
                let summary = params
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParams("create requires 'summary'".into())
                    })?;

                let mut body = serde_json::Map::new();
                body.insert("summary".into(), json!(summary));

                if let Some(v) = params.get("due").and_then(|v| v.as_str()) {
                    body.insert("due".into(), json!({ "timestamp": v }));
                }
                if let Some(v) = params.get("description").and_then(|v| v.as_str()) {
                    body.insert("description".into(), json!(v));
                }
                if let Some(arr) = params.get("assignees").and_then(|v| v.as_array()) {
                    let assignees: Vec<Value> = arr
                        .iter()
                        .filter_map(|a| a.as_str().map(|s| json!({ "id": s })))
                        .collect();
                    body.insert("assignees".into(), json!(assignees));
                }

                self.client
                    .post("/task/v2/tasks", &Value::Object(body))
                    .await?
            }
            "list" => self.client.get("/task/v2/tasks", &[]).await?,
            "update" => {
                let task_id = params
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParams("update requires 'task_id'".into())
                    })?;

                let mut body = serde_json::Map::new();
                if let Some(v) = params.get("summary").and_then(|v| v.as_str()) {
                    body.insert("summary".into(), json!(v));
                }
                if let Some(v) = params.get("due").and_then(|v| v.as_str()) {
                    body.insert("due".into(), json!({ "timestamp": v }));
                }
                if let Some(v) = params.get("description").and_then(|v| v.as_str()) {
                    body.insert("description".into(), json!(v));
                }

                let path = format!("/task/v2/tasks/{task_id}");
                self.client.patch(&path, &Value::Object(body)).await?
            }
            "complete" => {
                let task_id = params
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParams("complete requires 'task_id'".into())
                    })?;

                let path = format!("/task/v2/tasks/{task_id}/complete");
                self.client.post(&path, &json!({})).await?
            }
            other => {
                return Err(ToolError::InvalidParams(format!(
                    "Unknown action '{other}'. Expected: create, list, update, complete"
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
                "enum": ["create", "list", "update", "complete"],
                "description": "Task operation to perform"
            }),
        );
        props.insert(
            "summary".into(),
            json!({
                "type": "string",
                "description": "Task title (required for create)"
            }),
        );
        props.insert(
            "due".into(),
            json!({
                "type": "string",
                "description": "Due time in ISO 8601 format"
            }),
        );
        props.insert(
            "description".into(),
            json!({
                "type": "string",
                "description": "Task description"
            }),
        );
        props.insert(
            "task_id".into(),
            json!({
                "type": "string",
                "description": "Task ID (required for update and complete)"
            }),
        );
        props.insert(
            "assignees".into(),
            json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "List of assignee user IDs"
            }),
        );

        tool_schema(
            "feishu_task",
            concat!(
                "Interact with Feishu/Lark Task. ",
                "Actions: create (create a task), ",
                "list (list tasks), ",
                "update (update task details), ",
                "complete (mark task as done)."
            ),
            JsonSchema::object(props, vec!["action".into()]),
        )
    }
}
