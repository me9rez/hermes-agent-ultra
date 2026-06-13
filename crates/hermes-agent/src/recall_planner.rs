//! Rule-based recall planner — proactive session_search injection at turn start.
//!
//! When the user message looks like a continuation ("继续上次…", "that PR", …),
//! extract keywords and let the agent loop prefetch matching past sessions
//! without waiting for the model to call `session_search`.

use serde_json::Value;

use crate::text_terms::query_terms;

/// Why recall was triggered (for logging / future tuning).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallSignal {
    Continuation,
    TimeAnchor,
    ContinuationWithPointer,
}

/// Parsed recall intent with FTS keywords.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallQuery {
    pub keywords: String,
    pub signal: RecallSignal,
}

const CONTINUATION_SIGNALS: &[&str] = &[
    "继续",
    "上次",
    "之前",
    "咱们说到哪",
    "说到哪了",
    "接着",
    "continue",
    "last time",
    "where were we",
    "like before",
    "pick up",
    "resume",
];

const TIME_ANCHORS: &[&str] = &[
    "上周",
    "昨天",
    "之前那次",
    "刚才那个",
    "yesterday",
    "last week",
    "earlier",
];

const POINTER_SIGNALS: &[&str] = &["它", "那个", "这事", "别像上次"];

const TOPIC_HINTS: &[&str] = &["pr", "issue", "部署", "deploy", "任务", "task"];

/// Strip recall signal phrases and stopwords; return joined FTS query or empty.
pub fn extract_keywords(msg: &str) -> String {
    let mut working = msg.to_string();
    for signal in CONTINUATION_SIGNALS
        .iter()
        .chain(TIME_ANCHORS.iter())
        .chain(POINTER_SIGNALS.iter())
    {
        working = working.replace(signal, " ");
    }
    let terms = query_terms(&working);
    terms.join(" ")
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    let lower = haystack.to_ascii_lowercase();
    needles
        .iter()
        .any(|n| haystack.contains(n) || lower.contains(&n.to_ascii_lowercase()))
}

fn has_topic_hint(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    TOPIC_HINTS.iter().any(|h| lower.contains(h))
}

/// Classify user message; return `None` when proactive recall should not run.
pub fn classify(msg: &str) -> Option<RecallQuery> {
    let trimmed = msg.trim();
    if trimmed.is_empty() {
        return None;
    }

    let has_continuation = contains_any(trimmed, CONTINUATION_SIGNALS);
    let has_pointer = contains_any(trimmed, POINTER_SIGNALS);
    let has_time = contains_any(trimmed, TIME_ANCHORS);
    let has_topic = has_topic_hint(trimmed);

    if has_pointer && !has_continuation && !has_time && !has_topic {
        return None;
    }

    if !(has_continuation || has_time || (has_pointer && has_topic)) {
        return None;
    }

    let keywords = extract_keywords(trimmed);
    if keywords.is_empty() {
        return None;
    }

    let signal = if has_continuation && has_pointer {
        RecallSignal::ContinuationWithPointer
    } else if has_time {
        RecallSignal::TimeAnchor
    } else if has_pointer && has_topic {
        RecallSignal::ContinuationWithPointer
    } else {
        RecallSignal::Continuation
    };

    Some(RecallQuery { keywords, signal })
}

fn snippet_from_summary(raw: &str) -> String {
    let stripped = raw
        .trim()
        .strip_prefix("[Raw preview — summarization unavailable]")
        .or_else(|| {
            raw.trim()
                .strip_prefix("[Raw preview — summarization disabled]")
        })
        .unwrap_or(raw)
        .trim();
    let one_line: String = stripped
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(" / ");
    if one_line.chars().count() > 240 {
        format!("{}…", one_line.chars().take(240).collect::<String>())
    } else if one_line.is_empty() {
        "(no preview)".to_string()
    } else {
        one_line
    }
}

fn format_result_line(session_id: &str, when: &str, snippet: &str) -> String {
    format!("[recall:{session_id} | {when} | {snippet}]")
}

/// Parse session_search JSON into human-readable recall injection lines.
pub fn format_recall_block(json_str: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(json_str) else {
        return String::new();
    };
    if v.get("success").and_then(|x| x.as_bool()) != Some(true) {
        return String::new();
    }
    let Some(results) = v.get("results").and_then(|r| r.as_array()) else {
        return String::new();
    };
    if results.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    let mode = v.get("mode").and_then(|m| m.as_str()).unwrap_or("search");

    for row in results {
        let session_id = row
            .get("session_id")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown");
        let when = if mode == "recent" {
            row.get("started_at")
                .or_else(|| row.get("last_active"))
                .and_then(|x| x.as_f64())
                .map(|ts| {
                    chrono::DateTime::from_timestamp(ts.trunc() as i64, 0)
                        .map(|dt| hermes_core::format_wall_datetime(dt.with_timezone(&chrono::Utc)))
                        .unwrap_or_else(|| "unknown".to_string())
                })
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            row.get("when")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string()
        };
        let snippet = if mode == "recent" {
            row.get("preview")
                .and_then(|x| x.as_str())
                .unwrap_or("(no preview)")
                .to_string()
        } else {
            row.get("summary")
                .and_then(|x| x.as_str())
                .map(snippet_from_summary)
                .unwrap_or_else(|| "(no preview)".to_string())
        };
        lines.push(format_result_line(session_id, &when, &snippet));
    }

    if lines.is_empty() {
        return String::new();
    }

    format!("<recall-context>\n{}\n</recall-context>", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_continuation_with_topic() {
        let q = classify("继续上次 K8s 部署").expect("should classify");
        assert_eq!(q.signal, RecallSignal::Continuation);
        assert!(q.keywords.contains("k8s"));
    }

    #[test]
    fn classify_that_pr() {
        let q = classify("那个 PR 怎么样了").expect("should classify");
        assert!(q.keywords.contains("pr") || !q.keywords.is_empty());
    }

    #[test]
    fn classify_pointer_only_skipped() {
        assert!(classify("它是什么").is_none());
        assert!(classify("那个功能怎么用").is_none());
    }

    #[test]
    fn classify_fresh_task_skipped() {
        assert!(classify("写一个 Rust hello world").is_none());
    }

    #[test]
    fn extract_keywords_strips_signals() {
        let kw = extract_keywords("继续上次 deploy nginx 配置");
        assert!(kw.contains("deploy"));
        assert!(kw.contains("nginx"));
        assert!(!kw.contains("继续"));
    }

    #[test]
    fn format_recall_block_search_mode() {
        let json = r#"{
            "success": true,
            "query": "k8s",
            "results": [{
                "session_id": "sess-abc",
                "when": "2026-05-01",
                "summary": "[Raw preview — summarization disabled]\nUser: fix ingress\nAssistant: patched yaml"
            }],
            "count": 1
        }"#;
        let block = format_recall_block(json);
        assert!(block.contains("<recall-context>"));
        assert!(block.contains("[recall:sess-abc | 2026-05-01 |"));
        assert!(block.contains("ingress"));
    }

    #[test]
    fn format_recall_block_recent_mode() {
        let json = r#"{
            "success": true,
            "mode": "recent",
            "results": [{
                "session_id": "sess-recent",
                "started_at": 1700000000.0,
                "preview": "Discussed cron jobs"
            }],
            "count": 1
        }"#;
        let block = format_recall_block(json);
        assert!(block.contains("[recall:sess-recent |"));
        assert!(block.contains("cron"));
    }
}
