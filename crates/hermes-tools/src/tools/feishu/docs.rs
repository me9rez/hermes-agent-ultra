//! Feishu Docs tool — search, read, create, append to documents.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{json, Value};

use hermes_core::{tool_schema, JsonSchema, ToolError, ToolHandler, ToolSchema};

use super::FeishuApiClient;

/// Handler for the `feishu_docs` tool.
pub struct FeishuDocsHandler {
    client: Arc<FeishuApiClient>,
}

impl FeishuDocsHandler {
    pub fn new(client: Arc<FeishuApiClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ToolHandler for FeishuDocsHandler {
    async fn execute(&self, params: Value) -> Result<String, ToolError> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing required param: action".into()))?;

        let data = match action {
            "search" => {
                let query = params
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParams("search requires 'query'".into())
                    })?;

                let mut body = serde_json::Map::new();
                body.insert("query".into(), json!(query));

                // Optionally filter by doc type.
                if let Some(dt) = params.get("doc_type").and_then(|v| v.as_str()) {
                    body.insert("docs_types".into(), json!([dt]));
                }

                self.client
                    .post("/suite/docs-api/search/object", &Value::Object(body))
                    .await?
            }
            "read" => {
                let document_id = params
                    .get("document_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParams("read requires 'document_id'".into())
                    })?;

                let path = format!("/docx/v1/documents/{document_id}/blocks");
                self.client.get(&path, &[]).await?
            }
            "create" => {
                let title = params
                    .get("title")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParams("create requires 'title'".into())
                    })?;

                let mut body = serde_json::Map::new();
                body.insert("title".into(), json!(title));

                if let Some(v) = params.get("folder_token").and_then(|v| v.as_str()) {
                    body.insert("folder_token".into(), json!(v));
                }

                self.client
                    .post("/docx/v1/documents", &Value::Object(body))
                    .await?
            }
            "append" => {
                let document_id = params
                    .get("document_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParams("append requires 'document_id'".into())
                    })?;

                let content = params
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParams("append requires 'content'".into())
                    })?;

                // Default block_id to document_id (root block).
                let block_id = params
                    .get("block_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(document_id);

                let children = json!([{
                    "block_type": 2,
                    "text": {
                        "elements": [{
                            "text_run": {
                                "content": content
                            }
                        }],
                        "style": {}
                    }
                }]);

                let body = json!({ "children": children });
                let path = format!("/docx/v1/documents/{document_id}/blocks/{block_id}/children");
                self.client.post(&path, &body).await?
            }
            other => {
                return Err(ToolError::InvalidParams(format!(
                    "Unknown action '{other}'. Expected: search, read, create, append"
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
                "enum": ["search", "read", "create", "append"],
                "description": "Document operation to perform"
            }),
        );
        props.insert(
            "query".into(),
            json!({
                "type": "string",
                "description": "Search keyword (required for search)"
            }),
        );
        props.insert(
            "document_id".into(),
            json!({
                "type": "string",
                "description": "Document token/ID (required for read and append)"
            }),
        );
        props.insert(
            "title".into(),
            json!({
                "type": "string",
                "description": "New document title (required for create)"
            }),
        );
        props.insert(
            "content".into(),
            json!({
                "type": "string",
                "description": "Text content to write (required for append)"
            }),
        );
        props.insert(
            "folder_token".into(),
            json!({
                "type": "string",
                "description": "Target folder token for create"
            }),
        );
        props.insert(
            "doc_type".into(),
            json!({
                "type": "string",
                "enum": ["doc", "sheet", "bitable"],
                "description": "Filter by document type (optional, for search)"
            }),
        );

        tool_schema(
            "feishu_docs",
            concat!(
                "Interact with Feishu/Lark Documents. ",
                "Actions: search (find documents), ",
                "read (read document content), ",
                "create (create a new document), ",
                "append (append text to an existing document)."
            ),
            JsonSchema::object(props, vec!["action".into()]),
        )
    }
}
