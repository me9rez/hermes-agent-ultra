use crate::db::DbResult;
use crate::runtime::TaskRuntime;
use crate::runtime::checkpoint::{CheckpointState, latest_checkpoint};
use crate::types::{Task, TaskId, TaskStatus};

#[derive(Debug, Clone)]
pub struct ResumeContext {
    pub task: Task,
    pub checkpoint: Option<CheckpointState>,
}

impl TaskRuntime {
    pub fn resume_context(&self, task_id: TaskId) -> DbResult<Option<ResumeContext>> {
        let Some(mut task) = self.tasks().get(task_id)? else {
            return Ok(None);
        };
        let checkpoint = latest_checkpoint(self.events(), task_id)?;
        task.status = TaskStatus::Running;
        task.updated_at = chrono::Utc::now();
        self.tasks().update(&task)?;
        Ok(Some(ResumeContext { task, checkpoint }))
    }
}
