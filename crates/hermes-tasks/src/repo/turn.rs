use chrono::{DateTime, Utc};
use rusqlite::params;
use serde_json;

use crate::db::{DbError, DbResult, TaskDb, parse_ulid_id};
use crate::types::{
    EventKind, TaskEvent, TaskId, TaskTurn, TokenUsage, TurnId, anchor_slug_from_label,
    truncate_label,
};

#[derive(Clone)]
pub struct TurnRepository {
    db: TaskDb,
}

impl TurnRepository {
    pub fn new(db: TaskDb) -> Self {
        Self { db }
    }

    pub fn create(&self, turn: &TaskTurn) -> DbResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO task_turns (
                    id, task_id, instruction_event_id, label, started_at, ended_at, status,
                    artifact_count, approval_count, error_count,
                    input_tokens, output_tokens, cost_usd_cents, sub_task_ids_json
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![
                    turn.id.to_string(),
                    turn.task_id.to_string(),
                    turn.instruction_event_id.to_string(),
                    turn.label,
                    turn.started_at.to_rfc3339(),
                    turn.ended_at.map(|t| t.to_rfc3339()),
                    serde_json::to_string(&turn.status)
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    turn.artifact_count,
                    turn.approval_count,
                    turn.error_count,
                    turn.cost_tokens.input_tokens,
                    turn.cost_tokens.output_tokens,
                    turn.cost_tokens.cost_usd_cents,
                    serde_json::to_string(&turn.sub_task_ids)
                        .map_err(|e| DbError::Other(e.to_string()))?,
                ],
            )?;
            Ok(())
        })
    }

    pub fn update(&self, turn: &TaskTurn) -> DbResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE task_turns SET
                    label = ?2, ended_at = ?3, status = ?4,
                    artifact_count = ?5, approval_count = ?6, error_count = ?7,
                    input_tokens = ?8, output_tokens = ?9, cost_usd_cents = ?10,
                    sub_task_ids_json = ?11
                 WHERE id = ?1",
                params![
                    turn.id.to_string(),
                    turn.label,
                    turn.ended_at.map(|t| t.to_rfc3339()),
                    serde_json::to_string(&turn.status)
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    turn.artifact_count,
                    turn.approval_count,
                    turn.error_count,
                    turn.cost_tokens.input_tokens,
                    turn.cost_tokens.output_tokens,
                    turn.cost_tokens.cost_usd_cents,
                    serde_json::to_string(&turn.sub_task_ids)
                        .map_err(|e| DbError::Other(e.to_string()))?,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_for_task(&self, task_id: TaskId) -> DbResult<Vec<TaskTurn>> {
        self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, task_id, instruction_event_id, label, started_at, ended_at, status,
                        artifact_count, approval_count, error_count,
                        input_tokens, output_tokens, cost_usd_cents, sub_task_ids_json
                 FROM task_turns WHERE task_id = ?1 ORDER BY started_at ASC",
            )?;
            let mut rows = stmt.query(params![task_id.to_string()])?;
            let mut turns = Vec::new();
            while let Some(row) = rows.next()? {
                turns.push(row_to_turn(row)?);
            }
            Ok(turns)
        })
    }

    pub fn get(&self, turn_id: TurnId) -> DbResult<Option<TaskTurn>> {
        self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, task_id, instruction_event_id, label, started_at, ended_at, status,
                        artifact_count, approval_count, error_count,
                        input_tokens, output_tokens, cost_usd_cents, sub_task_ids_json
                 FROM task_turns WHERE id = ?1",
            )?;
            let mut rows = stmt.query(params![turn_id.to_string()])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row_to_turn(row)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn bind_instruction_event(
        &self,
        event_repo: &super::event::EventRepository,
        event: &mut TaskEvent,
        instruction_text: &str,
    ) -> DbResult<TaskTurn> {
        if event.kind != EventKind::Instruction {
            return Err(DbError::Other(
                "auto-bind requires Instruction event kind".into(),
            ));
        }
        let label = truncate_label(instruction_text, 80);
        let turn = TaskTurn::new(event.task_id, event.id, label.clone());
        event.turn_id = Some(turn.id);
        event.toc_label = Some(label.clone());
        event.anchor_slug = anchor_slug_from_label(&label, turn.id);
        self.create(&turn)?;
        event_repo.append(event)?;
        Ok(turn)
    }
}

fn row_to_turn(row: &rusqlite::Row<'_>) -> DbResult<TaskTurn> {
    let status_str: String = row.get(6)?;
    let started_at: String = row.get(4)?;
    let ended_at: Option<String> = row.get(5)?;
    let sub_tasks_str: String = row.get(13)?;

    Ok(TaskTurn {
        id: parse_ulid_id(row.get::<_, String>(0)?.as_str())?,
        task_id: parse_ulid_id(row.get::<_, String>(1)?.as_str())?,
        instruction_event_id: parse_ulid_id(row.get::<_, String>(2)?.as_str())?,
        label: row.get(3)?,
        started_at: DateTime::parse_from_rfc3339(&started_at)
            .map_err(|e| DbError::Other(e.to_string()))?
            .with_timezone(&Utc),
        ended_at: ended_at
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| DbError::Other(e.to_string()))
            })
            .transpose()?,
        status: serde_json::from_str(&status_str).map_err(|e| DbError::Other(e.to_string()))?,
        artifact_count: row.get(7)?,
        approval_count: row.get(8)?,
        error_count: row.get(9)?,
        cost_tokens: TokenUsage {
            input_tokens: row.get(10)?,
            output_tokens: row.get(11)?,
            cost_usd_cents: row.get(12)?,
        },
        sub_task_ids: serde_json::from_str(&sub_tasks_str)
            .map_err(|e| DbError::Other(e.to_string()))?,
    })
}
