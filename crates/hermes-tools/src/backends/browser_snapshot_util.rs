//! Snapshot truncation and optional LLM summarization (Python browser_tool parity).

use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};

pub const SNAPSHOT_SUMMARIZE_THRESHOLD: usize = 8000;

struct SnapshotSummaryClient {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
}

fn summary_client_from_env() -> Option<SnapshotSummaryClient> {
    let model = std::env::var("HERMES_BROWSER_SNAPSHOT_SUMMARY_MODEL")
        .ok()
        .or_else(|| std::env::var("HERMES_MODEL").ok())
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    let model = model
        .split(':')
        .next_back()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("gpt-4o-mini")
        .to_string();

    let base_url = std::env::var("HERMES_BROWSER_SNAPSHOT_SUMMARY_BASE_URL")
        .ok()
        .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string())
        .trim()
        .trim_end_matches('/')
        .to_string();

    let mut api_key = std::env::var("HERMES_BROWSER_SNAPSHOT_SUMMARY_API_KEY")
        .ok()
        .or_else(|| std::env::var("HERMES_OPENAI_API_KEY").ok())
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .unwrap_or_default();
    if api_key.trim().is_empty() && base_url.to_lowercase().contains("openrouter.ai") {
        api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
    }
    if api_key.trim().is_empty() {
        return None;
    }

    Some(SnapshotSummaryClient {
        client: Client::new(),
        base_url,
        api_key,
        model,
    })
}

pub fn truncate_snapshot(snapshot_text: &str, max_chars: usize) -> String {
    if snapshot_text.len() <= max_chars {
        return snapshot_text.to_string();
    }
    let reserve = 80usize;
    let mut result: Vec<String> = Vec::new();
    let mut chars = 0usize;
    let lines: Vec<&str> = snapshot_text.split('\n').collect();
    for line in &lines {
        if chars + line.len() + 1 > max_chars.saturating_sub(reserve) {
            break;
        }
        result.push((*line).to_string());
        chars += line.len() + 1;
    }
    let remaining = lines.len().saturating_sub(result.len());
    if remaining > 0 {
        result.push(format!(
            "\n[... {remaining} more lines truncated, use browser_snapshot with full=true for complete content]"
        ));
    }
    result.join("\n")
}

async fn summarize_via_llm(
    client: &SnapshotSummaryClient,
    snapshot_text: &str,
    user_task: Option<&str>,
) -> Option<String> {
    let prompt = if let Some(task) = user_task.filter(|t| !t.trim().is_empty()) {
        format!(
            "You are a content extractor for a browser automation agent.\n\n\
             The user's task is: {task}\n\n\
             Given the following page snapshot (accessibility tree), extract and summarize \
             the most relevant information. Keep ref IDs (like [ref=e5]) for interactive elements.\n\n\
             Page Snapshot:\n{snapshot_text}\n\n\
             Provide a concise summary preserving actionable information."
        )
    } else {
        format!(
            "Summarize this page snapshot, preserving interactive elements with ref IDs, \
             key headings, and important visible text.\n\nPage Snapshot:\n{snapshot_text}"
        )
    };

    let url = format!("{}/chat/completions", client.base_url);
    let body = json!({
        "model": client.model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.1,
        "max_tokens": 4000,
    });
    let resp = client
        .client
        .post(&url)
        .bearer_auth(client.api_key.trim())
        .timeout(Duration::from_secs(60))
        .json(&body)
        .send()
        .await
        .ok()?;
    let value: Value = resp.json().await.ok()?;
    value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Apply Python-parity snapshot post-processing.
pub async fn process_snapshot_text(snapshot_text: &str, user_task: Option<&str>) -> String {
    if snapshot_text.len() <= SNAPSHOT_SUMMARIZE_THRESHOLD {
        return snapshot_text.to_string();
    }
    if let Some(task) = user_task.filter(|t| !t.trim().is_empty()) {
        if let Some(client) = summary_client_from_env() {
            if let Some(summary) = summarize_via_llm(&client, snapshot_text, Some(task)).await {
                return summary;
            }
        }
    }
    truncate_snapshot(snapshot_text, SNAPSHOT_SUMMARIZE_THRESHOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_at_line_boundary() {
        let long = (0..200)
            .map(|i| format!("line {i} with some content"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = truncate_snapshot(&long, 500);
        assert!(out.len() <= 600);
        assert!(out.contains("truncated"));
    }

    #[test]
    fn short_snapshot_unchanged() {
        let s = "hello\nworld";
        assert_eq!(truncate_snapshot(s, 8000), s);
    }
}
