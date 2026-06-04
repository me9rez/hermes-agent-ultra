//! Session listing helpers for tools and gateway.

use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use super::error::StateDbError;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionListRow {
    pub id: String,
    pub source: String,
    pub model: Option<String>,
    pub title: Option<String>,
    pub started_at: f64,
    pub message_count: i64,
    pub preview: Option<String>,
    pub last_active: Option<f64>,
    pub lineage_root_id: Option<String>,
}

fn format_preview(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        None
    } else if raw.chars().count() > 60 {
        Some(format!("{}...", raw.chars().take(60).collect::<String>()))
    } else {
        Some(raw.to_string())
    }
}

fn row_to_list(row: &Row<'_>) -> rusqlite::Result<SessionListRow> {
    let source: Option<String> = row.get("source").ok();
    let platform: Option<String> = row.get("platform").ok();
    let effective = source
        .filter(|s| !s.trim().is_empty())
        .or(platform.filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "cli".into());
    let preview_raw: Option<String> = row.get("_preview_raw").ok();
    Ok(SessionListRow {
        id: row.get("id")?,
        source: effective,
        model: row.get("model").ok(),
        title: row.get("title").ok(),
        started_at: row.get("started_at")?,
        message_count: row.get("message_count").unwrap_or(0),
        preview: preview_raw.as_deref().and_then(format_preview),
        last_active: row.get("last_active").ok(),
        lineage_root_id: None,
    })
}

pub fn get_compression_tip(
    conn: &Arc<Mutex<Connection>>,
    session_id: &str,
) -> Result<String, StateDbError> {
    let guard = conn
        .lock()
        .map_err(|_| StateDbError("state db lock poisoned".into()))?;
    let mut current = session_id.to_string();
    for _ in 0..100 {
        let child: Option<String> = guard
            .query_row(
                "SELECT id FROM sessions
                 WHERE parent_session_id = ?1
                   AND started_at >= (
                       SELECT ended_at FROM sessions
                       WHERE id = ?2 AND end_reason = 'compression'
                   )
                 ORDER BY started_at DESC LIMIT 1",
                params![current, current],
                |r| r.get(0),
            )
            .ok();
        let Some(next) = child else {
            return Ok(current);
        };
        current = next;
    }
    Ok(current)
}

pub fn list_sessions_rich(
    conn: &Arc<Mutex<Connection>>,
    source: Option<&str>,
    exclude_sources: &[&str],
    limit: usize,
    offset: usize,
    min_message_count: i64,
    order_by_last_active: bool,
) -> Result<Vec<SessionListRow>, StateDbError> {
    let guard = conn
        .lock()
        .map_err(|_| StateDbError("state db lock poisoned".into()))?;

    let mut where_clauses = vec![
        "(s.parent_session_id IS NULL OR EXISTS (
            SELECT 1 FROM sessions p
            WHERE p.id = s.parent_session_id
              AND p.end_reason = 'branched'
              AND s.started_at >= p.ended_at
        ))"
            .to_string(),
    ];
    let mut params_vec: Vec<rusqlite::types::Value> = Vec::new();

    if let Some(src) = source {
        where_clauses.push(
            "COALESCE(NULLIF(s.source, ''), NULLIF(s.platform, ''), 'cli') = ?".into(),
        );
        params_vec.push(src.to_string().into());
    }
    if !exclude_sources.is_empty() {
        let ph: Vec<_> = exclude_sources.iter().map(|_| "?").collect();
        where_clauses.push(format!(
            "COALESCE(NULLIF(s.source, ''), NULLIF(s.platform, ''), 'cli') NOT IN ({})",
            ph.join(", ")
        ));
        for s in exclude_sources {
            params_vec.push((*s).to_string().into());
        }
    }
    if min_message_count > 0 {
        where_clauses.push("s.message_count >= ?".into());
        params_vec.push(min_message_count.into());
    }

    let where_sql = format!("WHERE {}", where_clauses.join(" AND "));
    let order = if order_by_last_active {
        "ORDER BY last_active DESC, s.started_at DESC"
    } else {
        "ORDER BY s.started_at DESC"
    };

    let sql = format!(
        "SELECT s.*,
            COALESCE(
                (SELECT SUBSTR(REPLACE(REPLACE(m.content, char(10), ' '), char(13), ' '), 1, 63)
                 FROM messages m
                 WHERE m.session_id = s.id AND m.role = 'user' AND m.content IS NOT NULL
                 ORDER BY m.timestamp, m.id LIMIT 1),
                ''
            ) AS _preview_raw,
            COALESCE(
                (SELECT MAX(m2.timestamp) FROM messages m2 WHERE m2.session_id = s.id),
                s.started_at
            ) AS last_active
         FROM sessions s
         {where_sql}
         {order}
         LIMIT ? OFFSET ?"
    );
    params_vec.push((limit as i64).into());
    params_vec.push((offset as i64).into());

    let mut stmt = guard
        .prepare(&sql)
        .map_err(|e| StateDbError(format!("list_sessions_rich: {e}")))?;
    stmt.query_map(rusqlite::params_from_iter(params_vec.iter()), row_to_list)
        .map_err(|e| StateDbError(format!("list_sessions_rich query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StateDbError(format!("list_sessions_rich read: {e}")))
}

pub fn load_session_messages(
    conn: &Arc<Mutex<Connection>>,
    session_id: &str,
) -> Result<Vec<(String, String, Option<String>)>, StateDbError> {
    let guard = conn
        .lock()
        .map_err(|_| StateDbError("state db lock poisoned".into()))?;
    let mut stmt = guard
        .prepare(
            "SELECT role, COALESCE(content, ''), tool_calls
             FROM messages WHERE session_id = ?1 ORDER BY id ASC",
        )
        .map_err(|e| StateDbError(format!("load messages: {e}")))?;
    stmt.query_map(params![session_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })
    .map_err(|e| StateDbError(format!("load messages query: {e}")))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| StateDbError(format!("load messages read: {e}")))
}
