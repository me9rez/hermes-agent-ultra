use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ids::{DeviceId, EventId, TaskId, TurnId, UserId};
use super::turn::TokenUsage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Instruction,
    Plan,
    Thinking,
    ToolCall,
    ToolResult,
    SubagentSpawn,
    Message,
    Artifact,
    ApprovalRequest,
    ApprovalResponse,
    Checkpoint,
    Error,
    System,
}

impl EventKind {
    pub fn collapsed_by_default(&self) -> bool {
        matches!(
            self,
            EventKind::Thinking | EventKind::ToolCall | EventKind::ToolResult
        )
    }

    pub fn default_toc_icon(&self) -> TocIcon {
        match self {
            EventKind::Instruction => TocIcon::Message,
            EventKind::Plan => TocIcon::Plan,
            EventKind::Thinking => TocIcon::Thinking,
            EventKind::ToolCall | EventKind::ToolResult => TocIcon::Tool,
            EventKind::SubagentSpawn => TocIcon::Fork,
            EventKind::Message => TocIcon::Message,
            EventKind::Artifact => TocIcon::Artifact,
            EventKind::ApprovalRequest | EventKind::ApprovalResponse => TocIcon::Approval,
            EventKind::Checkpoint => TocIcon::Checkpoint,
            EventKind::Error => TocIcon::Error,
            EventKind::System => TocIcon::Message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Actor {
    User {
        user_id: UserId,
        device_id: DeviceId,
    },
    Agent {
        model_id: String,
        provider_id: String,
    },
    Tool {
        tool_name: String,
    },
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TocIcon {
    Message,
    Plan,
    Thinking,
    Tool,
    Artifact,
    Approval,
    Checkpoint,
    Error,
    Fork,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: EventId,
    pub task_id: TaskId,
    pub parent_event_id: Option<EventId>,
    pub kind: EventKind,
    pub actor: Actor,
    pub payload: Value,
    pub collapsed_by_default: bool,
    pub streaming: bool,
    pub created_at: DateTime<Utc>,
    pub duration_ms: Option<u64>,
    pub cost_tokens: Option<TokenUsage>,
    pub turn_id: Option<TurnId>,
    pub toc_label: Option<String>,
    pub toc_icon: Option<TocIcon>,
    pub anchor_slug: String,
}

impl TaskEvent {
    pub fn new(
        task_id: TaskId,
        kind: EventKind,
        actor: Actor,
        payload: Value,
        anchor_slug: impl Into<String>,
    ) -> Self {
        Self {
            id: EventId::new(),
            task_id,
            parent_event_id: None,
            kind,
            actor,
            payload,
            collapsed_by_default: kind.collapsed_by_default(),
            streaming: false,
            created_at: Utc::now(),
            duration_ms: None,
            cost_tokens: None,
            turn_id: None,
            toc_label: None,
            toc_icon: Some(kind.default_toc_icon()),
            anchor_slug: anchor_slug.into(),
        }
    }

    pub fn with_parent(mut self, parent_event_id: EventId) -> Self {
        self.parent_event_id = Some(parent_event_id);
        self
    }

    pub fn with_turn(mut self, turn_id: TurnId) -> Self {
        self.turn_id = Some(turn_id);
        self
    }

    pub fn with_toc_label(mut self, label: impl Into<String>) -> Self {
        self.toc_label = Some(label.into());
        self
    }
}
