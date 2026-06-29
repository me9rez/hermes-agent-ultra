use chrono::{DateTime, Utc};
use rusqlite::params;
use serde_json;

use crate::db::{DbError, DbResult, TaskDb, parse_ulid_id};
use crate::types::{CronSchedule, Task, TaskId, TaskStatus, UserId, VerticalId};

#[derive(Debug, Clone, Default)]
pub struct TaskListQuery {
    pub owner_user_id: Option<UserId>,
    pub status: Option<TaskStatus>,
    pub vertical: Option<VerticalId>,
    pub cursor: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Clone)]
pub struct TaskListPage {
    pub tasks: Vec<Task>,
    pub next_cursor: Option<String>,
}

#[derive(Clone)]
pub struct TaskRepository {
    db: TaskDb,
}

impl TaskRepository {
    pub fn new(db: TaskDb) -> Self {
        Self { db }
    }

    pub fn create(&self, task: &Task) -> DbResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (
                    id, owner_user_id, primary_device_id, title, vertical_id, status,
                    parent_task_id, persona_stack_json, schedule_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    task.id.to_string(),
                    task.owner_user_id.to_string(),
                    task.primary_device_id.to_string(),
                    task.title,
                    task.vertical.as_ref().map(|v| v.as_str()),
                    serde_json::to_string(&task.status)
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    task.parent_task_id.map(|id| id.to_string()),
                    serde_json::to_string(&task.persona_stack)
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    task.schedule
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    task.created_at.to_rfc3339(),
                    task.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get(&self, id: TaskId) -> DbResult<Option<Task>> {
        self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, owner_user_id, primary_device_id, title, vertical_id, status,
                        parent_task_id, persona_stack_json, schedule_json, created_at, updated_at
                 FROM tasks WHERE id = ?1",
            )?;
            let mut rows = stmt.query(params![id.to_string()])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row_to_task(row)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn update(&self, task: &Task) -> DbResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE tasks SET
                    title = ?2, vertical_id = ?3, status = ?4, parent_task_id = ?5,
                    persona_stack_json = ?6, schedule_json = ?7, updated_at = ?8
                 WHERE id = ?1",
                params![
                    task.id.to_string(),
                    task.title,
                    task.vertical.as_ref().map(|v| v.as_str()),
                    serde_json::to_string(&task.status)
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    task.parent_task_id.map(|id| id.to_string()),
                    serde_json::to_string(&task.persona_stack)
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    task.schedule
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    task.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete(&self, id: TaskId) -> DbResult<bool> {
        self.db.with_conn(|conn| {
            let n = conn.execute("DELETE FROM tasks WHERE id = ?1", params![id.to_string()])?;
            Ok(n > 0)
        })
    }

    pub fn list(&self, query: &TaskListQuery) -> DbResult<TaskListPage> {
        let limit = query.limit.clamp(1, 100);
        self.db.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT id, owner_user_id, primary_device_id, title, vertical_id, status,
                        parent_task_id, persona_stack_json, schedule_json, created_at, updated_at
                 FROM tasks WHERE 1=1",
            );
            let mut bind: Vec<String> = Vec::new();

            if let Some(owner) = &query.owner_user_id {
                sql.push_str(&format!(" AND owner_user_id = ?{}", bind.len() + 1));
                bind.push(owner.to_string());
            }
            if let Some(status) = &query.status {
                sql.push_str(&format!(" AND status = ?{}", bind.len() + 1));
                bind.push(
                    serde_json::to_string(status).map_err(|e| DbError::Other(e.to_string()))?,
                );
            }
            if let Some(vertical) = &query.vertical {
                sql.push_str(&format!(" AND vertical_id = ?{}", bind.len() + 1));
                bind.push(vertical.as_str().to_string());
            }
            if let Some(cursor) = &query.cursor {
                sql.push_str(&format!(" AND updated_at < ?{}", bind.len() + 1));
                bind.push(cursor.clone());
            }
            sql.push_str(&format!(
                " ORDER BY updated_at DESC LIMIT ?{}",
                bind.len() + 1
            ));
            bind.push(limit.to_string());

            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<&dyn rusqlite::ToSql> =
                bind.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
            let mut rows = stmt.query(params.as_slice())?;
            let mut tasks = Vec::new();
            while let Some(row) = rows.next()? {
                tasks.push(row_to_task(row)?);
            }
            let next_cursor = tasks.last().map(|t| t.updated_at.to_rfc3339());
            Ok(TaskListPage { tasks, next_cursor })
        })
    }
}

fn row_to_task(row: &rusqlite::Row<'_>) -> DbResult<Task> {
    let status_str: String = row.get(5)?;
    let persona_str: String = row.get(7)?;
    let schedule_str: Option<String> = row.get(8)?;
    let created_at: String = row.get(9)?;
    let updated_at: String = row.get(10)?;

    Ok(Task {
        id: parse_ulid_id(row.get::<_, String>(0)?.as_str())?,
        owner_user_id: parse_ulid_id(row.get::<_, String>(1)?.as_str())?,
        primary_device_id: parse_ulid_id(row.get::<_, String>(2)?.as_str())?,
        title: row.get(3)?,
        vertical: row.get::<_, Option<String>>(4)?.map(VerticalId::new),
        status: serde_json::from_str(&status_str).map_err(|e| DbError::Other(e.to_string()))?,
        parent_task_id: row
            .get::<_, Option<String>>(6)?
            .map(|s| parse_ulid_id::<TaskId>(s.as_str()))
            .transpose()?,
        persona_stack: serde_json::from_str(&persona_str)
            .map_err(|e| DbError::Other(e.to_string()))?,
        schedule: schedule_str
            .map(|s| serde_json::from_str::<CronSchedule>(&s))
            .transpose()
            .map_err(|e| DbError::Other(e.to_string()))?,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map_err(|e| DbError::Other(e.to_string()))?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at)
            .map_err(|e| DbError::Other(e.to_string()))?
            .with_timezone(&Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DeviceId;

    fn test_db() -> TaskDb {
        let dir = tempfile::tempdir().unwrap();
        TaskDb::open(dir.path().join("tasks.db")).unwrap()
    }

    #[test]
    fn crud_roundtrip() {
        let db = test_db();
        let repo = TaskRepository::new(db);
        let user = UserId::new();
        let device = DeviceId::new();
        let mut task = Task::new(user, device, "Test task", Some(VerticalId::from("trader")));
        repo.create(&task).unwrap();

        let loaded = repo.get(task.id).unwrap().unwrap();
        assert_eq!(loaded.title, "Test task");

        task.title = "Updated".into();
        task.updated_at = Utc::now();
        repo.update(&task).unwrap();

        let page = repo
            .list(&TaskListQuery {
                owner_user_id: Some(user),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.tasks.len(), 1);
        assert_eq!(page.tasks[0].title, "Updated");
    }
}
