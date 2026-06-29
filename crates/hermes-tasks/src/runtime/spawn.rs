use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;
use tracing::info;

use crate::db::{DbResult, TaskDb};
use crate::repo::{EventRepository, TaskRepository, TurnRepository};
use crate::types::{
    Actor, DeviceId, EventKind, Task, TaskEvent, TaskId, TaskStatus, UserId, VerticalId,
};

#[derive(Clone)]
pub struct TaskRuntime {
    db: TaskDb,
    tasks: TaskRepository,
    events: EventRepository,
    turns: TurnRepository,
    active: Arc<Mutex<Vec<TaskId>>>,
}

impl TaskRuntime {
    pub fn new(db: TaskDb) -> Self {
        Self {
            tasks: TaskRepository::new(db.clone()),
            events: EventRepository::new(db.clone()),
            turns: TurnRepository::new(db.clone()),
            db,
            active: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn db(&self) -> &TaskDb {
        &self.db
    }

    pub fn tasks(&self) -> &TaskRepository {
        &self.tasks
    }

    pub fn events(&self) -> &EventRepository {
        &self.events
    }

    pub fn turns(&self) -> &TurnRepository {
        &self.turns
    }

    pub async fn create_and_run(
        &self,
        owner_user_id: UserId,
        device_id: DeviceId,
        title: impl Into<String>,
        vertical: Option<VerticalId>,
        instruction: &str,
    ) -> DbResult<(Task, TaskEvent)> {
        let mut task = Task::new(owner_user_id, device_id, title, vertical);
        task.status = TaskStatus::Running;
        self.tasks.create(&task)?;

        let mut event = TaskEvent::new(
            task.id,
            EventKind::Instruction,
            Actor::User {
                user_id: owner_user_id,
                device_id,
            },
            json!({ "text": instruction }),
            "instruction",
        );
        self.turns
            .bind_instruction_event(&self.events, &mut event, instruction)?;

        self.active.lock().await.push(task.id);
        info!(task_id = %task.id, "task created and running");
        Ok((task, event))
    }
}
