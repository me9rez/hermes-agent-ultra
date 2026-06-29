use serde_json::json;

use crate::db::{DbResult, parse_ulid_id};
use crate::repo::EventRepository;
use crate::runtime::TaskRuntime;
use crate::types::{Actor, EventId, EventKind, TaskEvent, TaskId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointState {
    pub last_event_id: EventId,
    pub agent_state_json: serde_json::Value,
    pub working_memory: serde_json::Value,
}

pub fn create_checkpoint_event(task_id: TaskId, state: &CheckpointState) -> TaskEvent {
    TaskEvent::new(
        task_id,
        EventKind::Checkpoint,
        Actor::System,
        json!({
            "last_event_id": state.last_event_id.to_string(),
            "agent_state": state.agent_state_json,
            "working_memory": state.working_memory,
        }),
        format!("checkpoint-{}", state.last_event_id),
    )
}

impl TaskRuntime {
    pub fn checkpoint(&self, task_id: TaskId, state: &CheckpointState) -> DbResult<TaskEvent> {
        let event = create_checkpoint_event(task_id, state);
        self.events().append(&event)?;
        Ok(event)
    }
}

pub fn latest_checkpoint(
    events: &EventRepository,
    task_id: TaskId,
) -> DbResult<Option<CheckpointState>> {
    let all = events.list_for_task(task_id)?;
    for event in all.into_iter().rev() {
        if event.kind == EventKind::Checkpoint {
            let last_event_id: EventId = event
                .payload
                .get("last_event_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| crate::db::DbError::Other("missing last_event_id".into()))
                .and_then(parse_ulid_id)?;
            return Ok(Some(CheckpointState {
                last_event_id,
                agent_state_json: event
                    .payload
                    .get("agent_state")
                    .cloned()
                    .unwrap_or(json!({})),
                working_memory: event
                    .payload
                    .get("working_memory")
                    .cloned()
                    .unwrap_or(json!({})),
            }));
        }
    }
    Ok(None)
}
