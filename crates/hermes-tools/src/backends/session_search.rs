//! Real session search backend using shared `state_db` search APIs.

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use crate::state_db::{StateDb, decode_content_preview};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;
use tokio::task::JoinSet;

use crate::tools::session_search::{SessionSearchBackend, SessionSearchOptions};
use hermes_core::ToolError;

const MAX_SESSION_CHARS: usize = 100_000;
const MAX_SUMMARY_TOKENS: usize = 10_000;
const HIDDEN_SESSION_SOURCES: &[&str] = &["tool", "internal"];

/// Real session search backend backed by `SessionPersistence` search parity.
pub struct SqliteSessionSearchBackend {
    db: StateDb,
}

#[derive(Clone)]
struct SessionSummaryClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

#[derive(Clone)]
struct SummaryTask {
    session_id: String,
    source: String,
    when: Option<String>,
    model: Option<String>,
    conversation_text: String,
}

impl SqliteSessionSearchBackend {
    fn resolve_lineage_root(&self, session_id: &str) -> String {
        self.db
            .get_compression_tip(session_id)
            .unwrap_or_else(|_| session_id.to_string())
    }

    fn format_conversation(messages: &[(String, String, Option<String>)]) -> String {
        let mut parts = Vec::new();
        for (role_raw, content_raw, tool_calls_raw) in messages {
            let role_upper = role_raw.to_uppercase();
            let mut content = decode_content_preview(Some(content_raw.as_str()));

            if role_upper == "TOOL" {
                if content.chars().count() > 500 {
                    let head: String = content.chars().take(250).collect();
                    let tail: String = content
                        .chars()
                        .rev()
                        .take(250)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    content = format!("{head}\n...[truncated]...\n{tail}");
                }
                parts.push(format!("[TOOL]: {content}"));
                continue;
            }

            if role_upper == "ASSISTANT" {
                if let Some(raw) = tool_calls_raw {
                    let mut names = Vec::new();
                    if let Ok(v) = serde_json::from_str::<Value>(raw) {
                        if let Some(arr) = v.as_array() {
                            for tc in arr {
                                let name = tc
                                    .get("name")
                                    .and_then(|x| x.as_str())
                                    .or_else(|| {
                                        tc.get("function")
                                            .and_then(|f| f.get("name"))
                                            .and_then(|x| x.as_str())
                                    })
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty());
                                if let Some(n) = name {
                                    names.push(n.to_string());
                                }
                            }
                        }
                    }
                    if !names.is_empty() {
                        parts.push(format!("[ASSISTANT]: [Called: {}]", names.join(", ")));
                    }
                }
                if !content.trim().is_empty() {
                    parts.push(format!("[ASSISTANT]: {content}"));
                }
                continue;
            }

            parts.push(format!("[{role_upper}]: {content}"));
        }
        parts.join("\n\n")
    }

    fn truncate_around_matches(full_text: &str, query: &str, max_chars: usize) -> String {
        if full_text.chars().count() <= max_chars {
            return full_text.to_string();
        }
        let text_lower = full_text.to_lowercase();
        let mut first_match = text_lower.len();
        for term in query.to_lowercase().split_whitespace() {
            let t = term.trim();
            if t.is_empty() {
                continue;
            }
            if let Some(pos) = text_lower.find(t) {
                first_match = first_match.min(pos);
            }
        }
        if first_match == text_lower.len() {
            first_match = 0;
        }

        let half = max_chars / 2;
        let mut start = first_match.saturating_sub(half);
        let end = (start + max_chars).min(full_text.len());
        if end.saturating_sub(start) < max_chars {
            start = end.saturating_sub(max_chars);
        }
        let body = &full_text[start..end];
        let prefix = if start > 0 {
            "...[earlier conversation truncated]...\n\n"
        } else {
            ""
        };
        let suffix = if end < full_text.len() {
            "\n\n...[later conversation truncated]..."
        } else {
            ""
        };
        format!("{prefix}{body}{suffix}")
    }

    fn format_timestamp(ts: Option<f64>) -> String {
        let Some(seconds) = ts else {
            return "unknown".to_string();
        };
        let sec = seconds.trunc() as i64;
        let nanos = ((seconds.fract().abs()) * 1_000_000_000_f64).round() as u32;
        if let Some(dt_utc) = Utc.timestamp_opt(sec, nanos).single() {
            return hermes_core::format_wall_datetime(dt_utc);
        }
        "unknown".to_string()
    }

    fn raw_preview_text(conversation_text: &str) -> String {
        if conversation_text.chars().count() > 500 {
            format!(
                "{}\n…[truncated]",
                conversation_text.chars().take(500).collect::<String>()
            )
        } else if conversation_text.trim().is_empty() {
            "No preview available.".to_string()
        } else {
            conversation_text.to_string()
        }
    }

    fn summary_client_from_env() -> Option<SessionSummaryClient> {
        let model = std::env::var("HERMES_SESSION_SEARCH_SUMMARY_MODEL")
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

        let base_url = std::env::var("HERMES_SESSION_SEARCH_SUMMARY_BASE_URL")
            .ok()
            .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string())
            .trim()
            .trim_end_matches('/')
            .to_string();

        let mut api_key = std::env::var("HERMES_SESSION_SEARCH_SUMMARY_API_KEY")
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

        Some(SessionSummaryClient {
            client: reqwest::Client::new(),
            base_url,
            api_key,
            model,
        })
    }

    async fn summarize_one(
        summary_client: &SessionSummaryClient,
        query: &str,
        task: &SummaryTask,
    ) -> Option<String> {
        let system_prompt = "You are reviewing a past conversation transcript to help recall what happened. Summarize the conversation with a focus on the search topic. Include: 1) user goal, 2) actions and outcomes, 3) key decisions/solutions, 4) specific commands/files/URLs/errors, 5) unresolved items. Be thorough but concise and factual in past tense.";
        let user_prompt = format!(
            "Search topic: {query}\nSession source: {}\nSession date: {}\n\nCONVERSATION TRANSCRIPT:\n{}\n\nSummarize this conversation with focus on: {query}",
            task.source,
            task.when.clone().unwrap_or_else(|| "unknown".to_string()),
            task.conversation_text,
        );

        let url = format!("{}/chat/completions", summary_client.base_url);
        for attempt in 0..3 {
            let request_body = json!({
                "model": summary_client.model,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": user_prompt},
                ],
                "temperature": 0.1,
                "max_tokens": MAX_SUMMARY_TOKENS,
            });
            let mut req = summary_client
                .client
                .post(&url)
                .bearer_auth(summary_client.api_key.trim())
                .timeout(Duration::from_secs(60))
                .json(&request_body);
            if summary_client
                .base_url
                .to_lowercase()
                .contains("openrouter.ai")
            {
                req = req
                    .header("HTTP-Referer", "https://hermes-agent.nousresearch.com")
                    .header("X-OpenRouter-Title", "Hermes Agent");
            }
            if let Ok(ok_resp) = req.send().await {
                if let Ok(v) = ok_resp.json::<Value>().await {
                    let text = v
                        .get("choices")
                        .and_then(|c| c.get(0))
                        .and_then(|x| x.get("message"))
                        .and_then(|m| m.get("content"))
                        .and_then(|c| c.as_str())
                        .map(str::trim)
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_secs((attempt + 1) as u64)).await;
            }
        }
        None
    }

    pub fn new(db_path: &str) -> Result<Self, ToolError> {
        StateDb::open(db_path).map(|db| Self { db }).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to open state DB: {}", e))
        })
    }

    pub fn default_path() -> Result<Self, ToolError> {
        StateDb::open_default().map(|db| Self { db }).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to open state DB: {}", e))
        })
    }
}

#[async_trait]
impl SessionSearchBackend for SqliteSessionSearchBackend {
    async fn search(
        &self,
        query: Option<&str>,
        role_filter: Option<&str>,
        limit: usize,
        current_session_id: Option<&str>,
        options: SessionSearchOptions,
    ) -> Result<String, ToolError> {
        let query = query.map(str::trim).unwrap_or("");
        let limit = limit.min(5).max(1);

        let role_values: Vec<String> = role_filter
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let role_refs: Vec<&str> = role_values.iter().map(String::as_str).collect();
        let role_arg = if role_refs.is_empty() {
            None
        } else {
            Some(role_refs.as_slice())
        };

        let current_lineage_root = current_session_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|sid| self.resolve_lineage_root(sid));

        if query.is_empty() {
            let rows = self
                .db
                .list_sessions_rich(
                    None,
                    HIDDEN_SESSION_SOURCES,
                    limit,
                    0,
                    0,
                    true,
                )
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            let mut results = Vec::new();
            for row in rows {
                if let Some(ref root) = current_lineage_root {
                    if root == &row.id {
                        continue;
                    }
                }
                results.push(json!({
                    "session_id": row.id,
                    "title": row.title,
                    "source": row.source,
                    "started_at": row.started_at,
                    "last_active": row.last_active,
                    "message_count": row.message_count,
                    "preview": row.preview.unwrap_or_default(),
                }));
            }
            return Ok(json!({
                "success": true,
                "mode": "recent",
                "results": results,
                "count": results.len(),
                "message": format!(
                    "Showing {} most recent sessions. Use a keyword query to search specific topics.",
                    results.len()
                ),
            })
            .to_string());
        }

        let hits = self
            .db
            .search_messages(
                query,
                None,
                Some(HIDDEN_SESSION_SOURCES),
                role_arg,
                50,
                0,
                None,
            )
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let mut seen = HashSet::new();
        let mut tasks = Vec::new();
        for hit in hits {
            let resolved = self.resolve_lineage_root(&hit.session_id);
            if let Some(ref root) = current_lineage_root {
                if root == &resolved {
                    continue;
                }
            }
            if !seen.insert(resolved.clone()) {
                continue;
            }
            let messages = self
                .db
                .load_session_messages(&resolved)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            if messages.is_empty() {
                continue;
            }
            let transcript = Self::format_conversation(&messages);
            let transcript = Self::truncate_around_matches(&transcript, query, MAX_SESSION_CHARS);
            tasks.push(SummaryTask {
                session_id: resolved,
                source: hit.source,
                when: Some(Self::format_timestamp(Some(hit.session_started))),
                model: hit.model,
                conversation_text: transcript,
            });
            if tasks.len() >= limit {
                break;
            }
        }

        let sessions_searched = seen.len();
        let mut summaries = Vec::new();
        if !options.summarize {
            for task in tasks {
                let preview = Self::raw_preview_text(&task.conversation_text);
                summaries.push(json!({
                    "session_id": task.session_id,
                    "when": task.when,
                    "source": task.source,
                    "model": task.model,
                    "summary": format!("[Raw preview — summarization disabled]\n{preview}"),
                }));
            }
        } else if let Some(summary_client) = Self::summary_client_from_env() {
            let mut join_set = JoinSet::new();
            for (idx, task) in tasks.iter().cloned().enumerate() {
                let summary_client = summary_client.clone();
                let q = query.to_string();
                join_set.spawn(async move {
                    let summary = Self::summarize_one(&summary_client, &q, &task).await;
                    (idx, task, summary)
                });
            }
            let mut ordered: Vec<Option<(SummaryTask, Option<String>)>> = vec![None; tasks.len()];
            while let Some(joined) = join_set.join_next().await {
                if let Ok((idx, task, summary)) = joined {
                    if idx < ordered.len() {
                        ordered[idx] = Some((task, summary));
                    }
                }
            }
            for item in ordered.into_iter().flatten() {
                let (task, maybe_summary) = item;
                let summary = maybe_summary.unwrap_or_else(|| {
                    format!(
                        "[Raw preview — summarization unavailable]\n{}",
                        Self::raw_preview_text(&task.conversation_text)
                    )
                });
                summaries.push(json!({
                    "session_id": task.session_id,
                    "when": task.when,
                    "source": task.source,
                    "model": task.model,
                    "summary": summary,
                }));
            }
        } else {
            for task in tasks {
                let preview = Self::raw_preview_text(&task.conversation_text);
                summaries.push(json!({
                    "session_id": task.session_id,
                    "when": task.when,
                    "source": task.source,
                    "model": task.model,
                    "summary": format!("[Raw preview — summarization unavailable]\n{preview}"),
                }));
            }
        }

        Ok(json!({
            "success": true,
            "query": query,
            "results": summaries,
            "count": summaries.len(),
            "sessions_searched": sessions_searched,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteSessionSearchBackend;

    #[test]
    fn format_timestamp_handles_none() {
        assert_eq!(SqliteSessionSearchBackend::format_timestamp(None), "unknown");
    }

    #[test]
    fn format_timestamp_handles_unix_number() {
        let out = SqliteSessionSearchBackend::format_timestamp(Some(1_700_000_000.0));
        assert!(out.contains("2023") || out.contains("2024"));
    }
}
