use chrono::Utc;
use serde_json::json;

use crate::db::DbResult;
use crate::runtime::TaskRuntime;
use crate::types::{
    AgentPersona, DeviceId, EventKind, Task, TaskEvent, TaskId, TaskStatus, TaskTurn, UserId,
    VerticalId,
};

#[derive(Debug, Clone)]
pub struct ForkRequest {
    pub parent_task_id: TaskId,
    pub parent_turn_id: crate::types::TurnId,
    pub owner_user_id: UserId,
    pub device_id: DeviceId,
    pub vertical: VerticalId,
    pub title: String,
    pub instruction: String,
    pub persona: AgentPersona,
}

impl TaskRuntime {
    pub fn fork_subtask(&self, req: ForkRequest) -> DbResult<(Task, TaskEvent, TaskTurn)> {
        let Some(mut parent) = self.tasks().get(req.parent_task_id)? else {
            return Err(crate::db::DbError::Other("parent task not found".into()));
        };
        let Some(mut parent_turn) = self.turns().get(req.parent_turn_id)? else {
            return Err(crate::db::DbError::Other("parent turn not found".into()));
        };

        let mut subtask = Task::new(
            req.owner_user_id,
            req.device_id,
            req.title,
            Some(req.vertical),
        )
        .with_parent(req.parent_task_id)
        .with_persona(req.persona);
        subtask.status = TaskStatus::Running;
        self.tasks().create(&subtask)?;

        let mut event = TaskEvent::new(
            subtask.id,
            EventKind::Instruction,
            crate::types::Actor::User {
                user_id: req.owner_user_id,
                device_id: req.device_id,
            },
            json!({ "text": req.instruction, "forked_from": req.parent_task_id.to_string() }),
            "fork-instruction",
        );
        let turn =
            self.turns()
                .bind_instruction_event(self.events(), &mut event, &req.instruction)?;

        parent_turn.sub_task_ids.push(subtask.id);
        parent.updated_at = Utc::now();
        self.turns().update(&parent_turn)?;
        self.tasks().update(&parent)?;

        let fork_event = TaskEvent::new(
            req.parent_task_id,
            EventKind::SubagentSpawn,
            crate::types::Actor::System,
            json!({
                "sub_task_id": subtask.id.to_string(),
                "vertical": subtask.vertical.as_ref().map(|v| v.as_str()),
            }),
            format!("fork-{}", subtask.id),
        );
        self.events().append(&fork_event)?;

        Ok((subtask, event, turn))
    }
}
