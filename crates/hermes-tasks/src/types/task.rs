use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::{DeviceId, TaskId, UserId, VerticalId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    NeedsApproval,
    Done,
    Failed,
    Cancelled,
    Scheduled,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPersona {
    pub vertical_id: Option<VerticalId>,
    pub system_prompt: String,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSchedule {
    pub expr: String,
    pub timezone: String,
    pub next_run: Option<DateTime<Utc>>,
    pub last_run: Option<DateTime<Utc>>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub owner_user_id: UserId,
    pub primary_device_id: DeviceId,
    pub title: String,
    pub vertical: Option<VerticalId>,
    pub status: TaskStatus,
    pub parent_task_id: Option<TaskId>,
    pub persona_stack: Vec<AgentPersona>,
    pub schedule: Option<CronSchedule>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(
        owner_user_id: UserId,
        primary_device_id: DeviceId,
        title: impl Into<String>,
        vertical: Option<VerticalId>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: TaskId::new(),
            owner_user_id,
            primary_device_id,
            title: title.into(),
            vertical,
            status: TaskStatus::Pending,
            parent_task_id: None,
            persona_stack: Vec::new(),
            schedule: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_parent(mut self, parent_task_id: TaskId) -> Self {
        self.parent_task_id = Some(parent_task_id);
        self
    }

    pub fn with_persona(mut self, persona: AgentPersona) -> Self {
        self.persona_stack.push(persona);
        self
    }

    pub fn with_schedule(mut self, schedule: CronSchedule) -> Self {
        self.schedule = Some(schedule);
        self.status = TaskStatus::Scheduled;
        self
    }
}
