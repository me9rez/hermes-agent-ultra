use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::db::DbResult;
use crate::runtime::TaskRuntime;
use crate::types::{TaskId, TaskStatus};

#[derive(Clone, Default)]
pub struct TaskCancellationRegistry {
    tokens: Arc<Mutex<HashMap<TaskId, CancellationToken>>>,
}

impl TaskCancellationRegistry {
    pub fn register(&self, task_id: TaskId) -> CancellationToken {
        let token = CancellationToken::new();
        if let Ok(mut g) = self.tokens.try_lock() {
            g.insert(task_id, token.clone());
        }
        token
    }

    pub async fn cancel(&self, task_id: TaskId) -> bool {
        let token = {
            let g = self.tokens.lock().await;
            g.get(&task_id).cloned()
        };
        if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub async fn remove(&self, task_id: TaskId) {
        let mut g = self.tokens.lock().await;
        g.remove(&task_id);
    }
}

impl TaskRuntime {
    pub async fn cancel_task(
        &self,
        registry: &TaskCancellationRegistry,
        task_id: TaskId,
    ) -> DbResult<bool> {
        let cancelled = registry.cancel(task_id).await;
        if !cancelled {
            warn!(%task_id, "cancel requested but no active token");
        }
        if let Some(mut task) = self.tasks().get(task_id)? {
            task.status = TaskStatus::Cancelled;
            task.updated_at = chrono::Utc::now();
            self.tasks().update(&task)?;
        }
        registry.remove(task_id).await;
        Ok(cancelled)
    }
}
