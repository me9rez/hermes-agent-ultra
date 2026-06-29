use chrono::{DateTime, Utc};
use rusqlite::params;
use serde_json;

use crate::db::{DbError, DbResult, TaskDb, parse_ulid_id};
use crate::types::{EventId, TaskEvent, TaskId, TokenUsage, TurnId};

#[derive(Clone)]
pub struct EventRepository {
    db: TaskDb,
}

impl EventRepository {
    pub fn new(db: TaskDb) -> Self {
        Self { db }
    }

    pub fn append(&self, event: &TaskEvent) -> DbResult<()> {
        self.db.with_conn(|conn| {
            let actor_json =
                serde_json::to_string(&event.actor).map_err(|e| DbError::Other(e.to_string()))?;
            let payload_json =
                serde_json::to_string(&event.payload).map_err(|e| DbError::Other(e.to_string()))?;
            let (input_tokens, output_tokens, cost_usd_cents) = event
                .cost_tokens
                .as_ref()
                .map(|t| {
                    (
                        Some(t.input_tokens),
                        Some(t.output_tokens),
                        Some(t.cost_usd_cents),
                    )
                })
                .unwrap_or((None, None, None));

            conn.execute(
                "INSERT INTO task_events (
                    id, task_id, parent_event_id, kind, actor_json, payload_json,
                    collapsed_by_default, streaming, created_at, duration_ms,
                    input_tokens, output_tokens, cost_usd_cents,
                    turn_id, toc_label, toc_icon, anchor_slug
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                params![
                    event.id.to_string(),
                    event.task_id.to_string(),
                    event.parent_event_id.map(|id| id.to_string()),
                    serde_json::to_string(&event.kind)
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    actor_json,
                    payload_json,
                    event.collapsed_by_default as i32,
                    event.streaming as i32,
                    event.created_at.to_rfc3339(),
                    event.duration_ms,
                    input_tokens,
                    output_tokens,
                    cost_usd_cents,
                    event.turn_id.map(|id| id.to_string()),
                    event.toc_label,
                    event
                        .toc_icon
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|e| DbError::Other(e.to_string()))?,
                    event.anchor_slug,
                ],
            )?;
            Ok(())
        })
    }

    pub fn set_streaming(&self, event_id: EventId, streaming: bool) -> DbResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE task_events SET streaming = ?2 WHERE id = ?1",
                params![event_id.to_string(), streaming as i32],
            )?;
            Ok(())
        })
    }

    pub fn list_for_task(&self, task_id: TaskId) -> DbResult<Vec<TaskEvent>> {
        self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, task_id, parent_event_id, kind, actor_json, payload_json,
                        collapsed_by_default, streaming, created_at, duration_ms,
                        input_tokens, output_tokens, cost_usd_cents,
                        turn_id, toc_label, toc_icon, anchor_slug
                 FROM task_events WHERE task_id = ?1 ORDER BY created_at ASC",
            )?;
            let mut rows = stmt.query(params![task_id.to_string()])?;
            let mut events = Vec::new();
            while let Some(row) = rows.next()? {
                events.push(row_to_event(row)?);
            }
            Ok(events)
        })
    }

    pub fn list_for_turn(&self, turn_id: TurnId) -> DbResult<Vec<TaskEvent>> {
        self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, task_id, parent_event_id, kind, actor_json, payload_json,
                        collapsed_by_default, streaming, created_at, duration_ms,
                        input_tokens, output_tokens, cost_usd_cents,
                        turn_id, toc_label, toc_icon, anchor_slug
                 FROM task_events WHERE turn_id = ?1 ORDER BY created_at ASC",
            )?;
            let mut rows = stmt.query(params![turn_id.to_string()])?;
            let mut events = Vec::new();
            while let Some(row) = rows.next()? {
                events.push(row_to_event(row)?);
            }
            Ok(events)
        })
    }

    pub fn get(&self, event_id: EventId) -> DbResult<Option<TaskEvent>> {
        self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, task_id, parent_event_id, kind, actor_json, payload_json,
                        collapsed_by_default, streaming, created_at, duration_ms,
                        input_tokens, output_tokens, cost_usd_cents,
                        turn_id, toc_label, toc_icon, anchor_slug
                 FROM task_events WHERE id = ?1",
            )?;
            let mut rows = stmt.query(params![event_id.to_string()])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row_to_event(row)?))
            } else {
                Ok(None)
            }
        })
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> DbResult<TaskEvent> {
    let kind_str: String = row.get(3)?;
    let actor_str: String = row.get(4)?;
    let payload_str: String = row.get(5)?;
    let created_at: String = row.get(8)?;
    let input_tokens: Option<u64> = row.get(10)?;
    let output_tokens: Option<u64> = row.get(11)?;
    let cost_usd_cents: Option<u32> = row.get(12)?;
    let toc_icon_str: Option<String> = row.get(15)?;

    let cost_tokens = match (input_tokens, output_tokens, cost_usd_cents) {
        (Some(i), Some(o), Some(c)) => Some(TokenUsage {
            input_tokens: i,
            output_tokens: o,
            cost_usd_cents: c,
        }),
        _ => None,
    };

    Ok(TaskEvent {
        id: parse_ulid_id(row.get::<_, String>(0)?.as_str())?,
        task_id: parse_ulid_id(row.get::<_, String>(1)?.as_str())?,
        parent_event_id: row
            .get::<_, Option<String>>(2)?
            .map(|s| parse_ulid_id::<EventId>(s.as_str()))
            .transpose()?,
        kind: serde_json::from_str(&kind_str).map_err(|e| DbError::Other(e.to_string()))?,
        actor: serde_json::from_str(&actor_str).map_err(|e| DbError::Other(e.to_string()))?,
        payload: serde_json::from_str(&payload_str).map_err(|e| DbError::Other(e.to_string()))?,
        collapsed_by_default: row.get::<_, i32>(6)? != 0,
        streaming: row.get::<_, i32>(7)? != 0,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map_err(|e| DbError::Other(e.to_string()))?
            .with_timezone(&Utc),
        duration_ms: row.get(9)?,
        cost_tokens,
        turn_id: row
            .get::<_, Option<String>>(13)?
            .map(|s| parse_ulid_id::<TurnId>(s.as_str()))
            .transpose()?,
        toc_label: row.get(14)?,
        toc_icon: toc_icon_str
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| DbError::Other(e.to_string()))?,
        anchor_slug: row.get(16)?,
    })
}
