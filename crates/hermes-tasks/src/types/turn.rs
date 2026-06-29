use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use super::ids::{EventId, TaskId, TurnId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd_cents: u32,
}

impl TokenUsage {
    pub fn zero() -> Self {
        Self::default()
    }

    pub fn add_assign(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cost_usd_cents = self.cost_usd_cents.saturating_add(other.cost_usd_cents);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Running,
    Done,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTurn {
    pub id: TurnId,
    pub task_id: TaskId,
    pub instruction_event_id: EventId,
    pub label: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: TurnStatus,
    pub artifact_count: u32,
    pub approval_count: u32,
    pub error_count: u32,
    pub cost_tokens: TokenUsage,
    pub sub_task_ids: Vec<TaskId>,
}

impl TaskTurn {
    pub fn new(task_id: TaskId, instruction_event_id: EventId, label: impl Into<String>) -> Self {
        Self {
            id: TurnId::new(),
            task_id,
            instruction_event_id,
            label: label.into(),
            started_at: Utc::now(),
            ended_at: None,
            status: TurnStatus::Running,
            artifact_count: 0,
            approval_count: 0,
            error_count: 0,
            cost_tokens: TokenUsage::zero(),
            sub_task_ids: Vec::new(),
        }
    }

    pub fn finish(&mut self, status: TurnStatus) {
        self.status = status;
        self.ended_at = Some(Utc::now());
    }
}

static SLUG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("valid slug regex"));

pub fn truncate_label(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect()
}

pub fn anchor_slug_from_label(label: &str, turn_id: TurnId) -> String {
    let base = label.to_lowercase();
    let slug = SLUG_RE.replace_all(&base, "-");
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        format!("turn-{}", turn_id)
    } else {
        format!("{slug}-{}", turn_id)
    }
}

pub fn turn_id_from_event(event_id: EventId) -> TurnId {
    TurnId::from_ulid(event_id.inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_slug_sanitizes() {
        let turn = TurnId::new();
        let slug = anchor_slug_from_label("Hello World! 测试", turn);
        assert!(slug.starts_with("hello-world"));
        assert!(slug.contains(&turn.to_string()));
    }

    #[test]
    fn truncate_label_respects_chars() {
        assert_eq!(truncate_label("abc", 80), "abc");
        assert_eq!(truncate_label("一二三四五", 3), "一二三");
    }
}
