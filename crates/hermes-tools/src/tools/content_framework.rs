use async_trait::async_trait;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use hermes_core::{tool_schema, JsonSchema, ToolError, ToolHandler, ToolSchema};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContentItem {
    source: String,
    channel: String,
    item_id: Option<String>,
    title: String,
    url: Option<String>,
    summary: Option<String>,
    tags: Vec<String>,
    published_at: Option<String>,
    confidence: Option<f64>,
    fingerprint: Option<String>,
}

impl ContentItem {
    fn from_value(value: &Value) -> Result<Self, ToolError> {
        let source = value
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .trim()
            .to_string();
        let channel = value
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .trim()
            .to_string();
        let title = value
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("content item missing 'title'".into()))?
            .trim()
            .to_string();
        if title.is_empty() {
            return Err(ToolError::InvalidParams(
                "content item 'title' cannot be empty".into(),
            ));
        }

        let tags = value
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(Self {
            source,
            channel,
            item_id: value
                .get("item_id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            title,
            url: value.get("url").and_then(|v| v.as_str()).map(str::to_string),
            summary: value
                .get("summary")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            tags,
            published_at: value
                .get("published_at")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            confidence: value.get("confidence").and_then(|v| v.as_f64()),
            fingerprint: value
                .get("fingerprint")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    }

    fn ensure_fingerprint(&mut self) {
        if self
            .fingerprint
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return;
        }
        let title = self.title.trim().to_ascii_lowercase();
        let url = self.url.as_deref().unwrap_or("").trim().to_ascii_lowercase();
        let summary = self
            .summary
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let raw = format!("{}|{}|{}|{}|{}", self.source, self.channel, title, url, summary);
        let digest = Sha256::digest(raw.as_bytes());
        let fingerprint = digest
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();
        self.fingerprint = Some(fingerprint);
    }
}

pub struct ContentPlanHandler;

#[async_trait]
impl ToolHandler for ContentPlanHandler {
    async fn execute(&self, params: Value) -> Result<String, ToolError> {
        let objective = params
            .get("objective")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'objective' parameter".into()))?
            .trim()
            .to_string();
        if objective.is_empty() {
            return Err(ToolError::InvalidParams(
                "'objective' cannot be empty".into(),
            ));
        }
        let channels = params
            .get("channels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let keywords = params
            .get("keywords")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let max_candidates = params
            .get("max_candidates")
            .and_then(|v| v.as_u64())
            .unwrap_or(20);

        let steps = json!([
            {"step":"open_channel","description":"Navigate to a target channel and confirm login state"},
            {"step":"locate_candidates","description":"Collect candidate posts/threads matching objective","max_candidates":max_candidates},
            {"step":"apply_filters","description":"Filter by keywords/tags/recency","keywords":keywords},
            {"step":"extract_items","description":"Extract structured fields into normalized schema"},
            {"step":"dedupe_and_rank","description":"Generate fingerprint, remove duplicates, rank by confidence"},
            {"step":"deliver_digest","description":"Send concise digest to configured destination"}
        ]);

        Ok(json!({
            "objective": objective,
            "channels": channels,
            "playbook_version": "v1",
            "steps": steps
        })
        .to_string())
    }

    fn schema(&self) -> ToolSchema {
        let mut props = IndexMap::new();
        props.insert(
            "objective".into(),
            json!({"type":"string","description":"High-level retrieval objective"}),
        );
        props.insert(
            "channels".into(),
            json!({"type":"array","items":{"type":"string"},"description":"Target channels or sources"}),
        );
        props.insert(
            "keywords".into(),
            json!({"type":"array","items":{"type":"string"},"description":"Optional keyword filters"}),
        );
        props.insert(
            "max_candidates".into(),
            json!({"type":"integer","description":"Upper bound for candidate records","default":20}),
        );
        tool_schema(
            "content_plan",
            "Generate a reusable retrieval playbook for monitored channels.",
            JsonSchema::object(props, vec!["objective".into()]),
        )
    }
}

pub struct ContentNormalizeHandler;

#[async_trait]
impl ToolHandler for ContentNormalizeHandler {
    async fn execute(&self, params: Value) -> Result<String, ToolError> {
        let raw_items = params
            .get("items")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'items' array parameter".into()))?;
        let dedupe = params
            .get("dedupe")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut normalized = Vec::with_capacity(raw_items.len());
        let mut seen = std::collections::HashSet::new();
        let mut duplicates_removed = 0usize;

        for item in raw_items {
            let mut parsed = ContentItem::from_value(item)?;
            parsed.ensure_fingerprint();
            if dedupe {
                let key = parsed.fingerprint.clone().unwrap_or_default();
                if !seen.insert(key) {
                    duplicates_removed = duplicates_removed.saturating_add(1);
                    continue;
                }
            }
            normalized.push(parsed);
        }

        Ok(json!({
            "items": normalized,
            "stats": {
                "input_count": raw_items.len(),
                "output_count": normalized.len(),
                "duplicates_removed": duplicates_removed
            }
        })
        .to_string())
    }

    fn schema(&self) -> ToolSchema {
        let mut props = IndexMap::new();
        props.insert(
            "items".into(),
            json!({"type":"array","items":{"type":"object"},"description":"Raw extracted content items"}),
        );
        props.insert(
            "dedupe".into(),
            json!({"type":"boolean","description":"Deduplicate by deterministic fingerprint","default":true}),
        );
        tool_schema(
            "content_normalize",
            "Normalize extracted records into a reusable schema and deduplicate by fingerprint.",
            JsonSchema::object(props, vec!["items".into()]),
        )
    }
}

fn step_tool_hints(step: &str) -> &'static [&'static str] {
    match step {
        "open_channel" => &["browser_navigate", "browser_snapshot"],
        "locate_candidates" => &["browser_snapshot", "browser_scroll"],
        "apply_filters" => &["browser_snapshot"],
        "extract_items" => &["browser_snapshot", "content_normalize"],
        "dedupe_and_rank" => &["content_normalize"],
        "deliver_digest" => &["send_message", "memory"],
        _ => &["browser_snapshot"],
    }
}

pub struct ContentExecuteHandler;

#[async_trait]
impl ToolHandler for ContentExecuteHandler {
    async fn execute(&self, params: Value) -> Result<String, ToolError> {
        let plan = params.get("plan").cloned().ok_or_else(|| {
            ToolError::InvalidParams("Missing 'plan' object from content_plan".into())
        })?;
        let steps = plan
            .get("steps")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::InvalidParams("plan.steps must be an array".into()))?;
        if steps.is_empty() {
            return Err(ToolError::InvalidParams("plan.steps is empty".into()));
        }

        let step_index = params
            .get("step_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        if step_index >= steps.len() {
            return Ok(json!({
                "done": true,
                "step_index": step_index,
                "message": "All playbook steps completed"
            })
            .to_string());
        }

        let current = &steps[step_index];
        let step_name = current
            .get("step")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let suggested_tools = step_tool_hints(step_name);

        Ok(json!({
            "done": false,
            "step_index": step_index,
            "total_steps": steps.len(),
            "current_step": current,
            "suggested_tools": suggested_tools,
            "next_step_index": step_index + 1,
            "objective": plan.get("objective"),
        })
        .to_string())
    }

    fn schema(&self) -> ToolSchema {
        let mut props = IndexMap::new();
        props.insert(
            "plan".into(),
            json!({"type":"object","description":"Playbook object returned by content_plan"}),
        );
        props.insert(
            "step_index".into(),
            json!({"type":"integer","description":"Zero-based step to execute (default 0)","default":0}),
        );
        tool_schema(
            "content_execute",
            "Return the current playbook step and suggested tools (LLM-driven executor; does not run browser itself).",
            JsonSchema::object(props, vec!["plan".into()]),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn normalize_deduplicates_by_fingerprint() {
        let handler = ContentNormalizeHandler;
        let out = handler
            .execute(json!({
                "dedupe": true,
                "items": [
                    {"source":"site-a","channel":"ai","title":"A","url":"https://x/a"},
                    {"source":"site-a","channel":"ai","title":"A","url":"https://x/a"}
                ]
            }))
            .await
            .expect("normalize");
        let value: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(value["stats"]["output_count"], 1);
        assert_eq!(value["stats"]["duplicates_removed"], 1);
    }

    #[tokio::test]
    async fn execute_returns_first_step_hints() {
        let handler = ContentExecuteHandler;
        let plan = ContentPlanHandler
            .execute(json!({"objective":"test"}))
            .await
            .expect("plan");
        let plan_value: Value = serde_json::from_str(&plan).expect("json");
        let out = handler
            .execute(json!({"plan": plan_value, "step_index": 0}))
            .await
            .expect("execute");
        let value: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(value["done"], false);
        assert!(value["suggested_tools"].as_array().unwrap().len() >= 1);
    }

    #[tokio::test]
    async fn plan_contains_standard_steps() {
        let handler = ContentPlanHandler;
        let out = handler
            .execute(json!({"objective":"monitor agent browser articles"}))
            .await
            .expect("plan");
        let value: Value = serde_json::from_str(&out).expect("json");
        let steps = value["steps"].as_array().expect("steps");
        assert!(steps.iter().any(|s| s["step"] == "extract_items"));
        assert!(steps.iter().any(|s| s["step"] == "deliver_digest"));
    }
}
