//! Telegram DM topic-mode persistence (Python `SessionDB` topic APIs).

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use super::error::StateDbError;

fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramTopicBinding {
    pub chat_id: String,
    pub thread_id: String,
    pub user_id: String,
    pub session_key: String,
    pub session_id: String,
    pub managed_mode: String,
    pub linked_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnlinkedTelegramSession {
    pub id: String,
    pub title: Option<String>,
    pub preview: Option<String>,
    pub started_at: f64,
    pub last_active: f64,
    pub message_count: i64,
}

pub fn apply_telegram_topic_migration(conn: &Arc<Mutex<Connection>>) -> Result<(), StateDbError> {
    let guard = conn
        .lock()
        .map_err(|_| StateDbError("state db lock poisoned".into()))?;
    guard.execute_batch(
        "CREATE TABLE IF NOT EXISTS telegram_dm_topic_mode (
            chat_id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            activated_at REAL NOT NULL,
            updated_at REAL NOT NULL,
            has_topics_enabled INTEGER,
            allows_users_to_create_topics INTEGER,
            capability_checked_at REAL,
            intro_message_id TEXT,
            pinned_message_id TEXT
        );
        CREATE TABLE IF NOT EXISTS telegram_dm_topic_bindings (
            chat_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            session_key TEXT NOT NULL,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            managed_mode TEXT NOT NULL DEFAULT 'auto',
            linked_at REAL NOT NULL,
            updated_at REAL NOT NULL,
            PRIMARY KEY (chat_id, thread_id)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_telegram_dm_topic_bindings_session
        ON telegram_dm_topic_bindings(session_id);
        CREATE INDEX IF NOT EXISTS idx_telegram_dm_topic_bindings_user
        ON telegram_dm_topic_bindings(user_id, chat_id);",
    )?;

    let current: i64 = guard
        .query_row(
            "SELECT value FROM state_meta WHERE key = 'telegram_dm_topic_schema_version'",
            [],
            |r| {
                let s: String = r.get(0)?;
                Ok(s.parse::<i64>().unwrap_or(0))
            },
        )
        .unwrap_or(0);

    if current < 2 {
        let fk_rows: Vec<(String, String)> = guard
            .prepare("PRAGMA foreign_key_list('telegram_dm_topic_bindings')")?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        let needs_rebuild = fk_rows
            .iter()
            .any(|(table, on_delete)| table == "sessions" && on_delete.as_str() != "CASCADE");
        if needs_rebuild {
            guard.execute_batch(
                "CREATE TABLE telegram_dm_topic_bindings_new (
                    chat_id TEXT NOT NULL,
                    thread_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    session_key TEXT NOT NULL,
                    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    managed_mode TEXT NOT NULL DEFAULT 'auto',
                    linked_at REAL NOT NULL,
                    updated_at REAL NOT NULL,
                    PRIMARY KEY (chat_id, thread_id)
                );
                INSERT INTO telegram_dm_topic_bindings_new
                SELECT chat_id, thread_id, user_id, session_key, session_id,
                       managed_mode, linked_at, updated_at
                FROM telegram_dm_topic_bindings;
                DROP TABLE telegram_dm_topic_bindings;
                ALTER TABLE telegram_dm_topic_bindings_new RENAME TO telegram_dm_topic_bindings;
                CREATE UNIQUE INDEX IF NOT EXISTS idx_telegram_dm_topic_bindings_session
                ON telegram_dm_topic_bindings(session_id);
                CREATE INDEX IF NOT EXISTS idx_telegram_dm_topic_bindings_user
                ON telegram_dm_topic_bindings(user_id, chat_id);",
            )?;
        }
        guard.execute(
            "INSERT INTO state_meta (key, value) VALUES ('telegram_dm_topic_schema_version', '2')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
    }
    Ok(())
}

pub fn enable_telegram_topic_mode(
    conn: &Arc<Mutex<Connection>>,
    chat_id: &str,
    user_id: &str,
    has_topics_enabled: Option<bool>,
    allows_users_to_create_topics: Option<bool>,
) -> Result<(), StateDbError> {
    apply_telegram_topic_migration(conn)?;
    let now = now_unix();
    let to_int = |v: Option<bool>| v.map(|b| if b { 1i64 } else { 0 });
    let guard = conn
        .lock()
        .map_err(|_| StateDbError("state db lock poisoned".into()))?;
    guard.execute(
        "INSERT INTO telegram_dm_topic_mode (
            chat_id, user_id, enabled, activated_at, updated_at,
            has_topics_enabled, allows_users_to_create_topics, capability_checked_at
        ) VALUES (?1, ?2, 1, ?3, ?3, ?4, ?5, ?3)
        ON CONFLICT(chat_id) DO UPDATE SET
            user_id = excluded.user_id,
            enabled = 1,
            updated_at = excluded.updated_at,
            has_topics_enabled = excluded.has_topics_enabled,
            allows_users_to_create_topics = excluded.allows_users_to_create_topics,
            capability_checked_at = excluded.capability_checked_at",
        params![
            chat_id,
            user_id,
            now,
            to_int(has_topics_enabled),
            to_int(allows_users_to_create_topics),
        ],
    )?;
    Ok(())
}

pub fn disable_telegram_topic_mode(
    conn: &Arc<Mutex<Connection>>,
    chat_id: &str,
    clear_bindings: bool,
) -> Result<(), StateDbError> {
    let guard = conn
        .lock()
        .map_err(|_| StateDbError("state db lock poisoned".into()))?;
    let now = now_unix();
    let _ = guard.execute(
        "UPDATE telegram_dm_topic_mode SET enabled = 0, updated_at = ?1 WHERE chat_id = ?2",
        params![now, chat_id],
    );
    if clear_bindings {
        let _ = guard.execute(
            "DELETE FROM telegram_dm_topic_bindings WHERE chat_id = ?1",
            params![chat_id],
        );
    }
    Ok(())
}

pub fn is_telegram_topic_mode_enabled(
    conn: &Arc<Mutex<Connection>>,
    chat_id: &str,
    user_id: &str,
) -> bool {
    let Ok(guard) = conn.lock() else {
        return false;
    };
    guard
        .query_row(
            "SELECT enabled FROM telegram_dm_topic_mode WHERE chat_id = ?1 AND user_id = ?2",
            params![chat_id, user_id],
            |r| r.get::<_, i64>(0),
        )
        .map(|v| v != 0)
        .unwrap_or(false)
}

pub fn get_telegram_topic_binding(
    conn: &Arc<Mutex<Connection>>,
    chat_id: &str,
    thread_id: &str,
) -> Result<Option<TelegramTopicBinding>, StateDbError> {
    let guard = conn
        .lock()
        .map_err(|_| StateDbError("state db lock poisoned".into()))?;
    match guard.query_row(
        "SELECT chat_id, thread_id, user_id, session_key, session_id,
                managed_mode, linked_at, updated_at
         FROM telegram_dm_topic_bindings WHERE chat_id = ?1 AND thread_id = ?2",
        params![chat_id, thread_id],
        |r| {
            Ok(TelegramTopicBinding {
                chat_id: r.get(0)?,
                thread_id: r.get(1)?,
                user_id: r.get(2)?,
                session_key: r.get(3)?,
                session_id: r.get(4)?,
                managed_mode: r.get(5)?,
                linked_at: r.get(6)?,
                updated_at: r.get(7)?,
            })
        },
    ) {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) if e.to_string().contains("no such table") => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn bind_telegram_topic(
    conn: &Arc<Mutex<Connection>>,
    chat_id: &str,
    thread_id: &str,
    user_id: &str,
    session_key: &str,
    session_id: &str,
    managed_mode: &str,
) -> Result<(), StateDbError> {
    apply_telegram_topic_migration(conn)?;
    let now = now_unix();
    let guard = conn
        .lock()
        .map_err(|_| StateDbError("state db lock poisoned".into()))?;
    if let Ok((linked_chat, linked_thread)) = guard.query_row(
        "SELECT chat_id, thread_id FROM telegram_dm_topic_bindings WHERE session_id = ?1",
        params![session_id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    ) {
        if linked_chat != chat_id || linked_thread != thread_id {
            return Err(StateDbError(
                "session is already linked to another Telegram topic".into(),
            ));
        }
    }
    guard.execute(
        "INSERT INTO telegram_dm_topic_bindings (
            chat_id, thread_id, user_id, session_key, session_id,
            managed_mode, linked_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
        ON CONFLICT(chat_id, thread_id) DO UPDATE SET
            user_id = excluded.user_id,
            session_key = excluded.session_key,
            session_id = excluded.session_id,
            managed_mode = excluded.managed_mode,
            updated_at = excluded.updated_at",
        params![
            chat_id,
            thread_id,
            user_id,
            session_key,
            session_id,
            managed_mode,
            now,
        ],
    )?;
    Ok(())
}

pub fn is_telegram_session_linked_to_topic(
    conn: &Arc<Mutex<Connection>>,
    session_id: &str,
) -> bool {
    let Ok(guard) = conn.lock() else {
        return false;
    };
    guard
        .query_row(
            "SELECT 1 FROM telegram_dm_topic_bindings WHERE session_id = ?1 LIMIT 1",
            params![session_id],
            |_| Ok(true),
        )
        .unwrap_or(false)
}

pub fn list_unlinked_telegram_sessions_for_user(
    conn: &Arc<Mutex<Connection>>,
    user_id: &str,
    limit: usize,
) -> Result<Vec<UnlinkedTelegramSession>, StateDbError> {
    let guard = conn
        .lock()
        .map_err(|_| StateDbError("state db lock poisoned".into()))?;
    let sql_with_bindings = "
        SELECT s.id, s.title, s.started_at, s.message_count,
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
        WHERE s.source = 'telegram'
          AND s.user_id = ?
          AND NOT EXISTS (
              SELECT 1 FROM telegram_dm_topic_bindings b WHERE b.session_id = s.id
          )
        ORDER BY last_active DESC, s.started_at DESC
        LIMIT ?";
    let sql_fallback = "
        SELECT s.id, s.title, s.started_at, s.message_count,
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
        WHERE s.source = 'telegram' AND s.user_id = ?
        ORDER BY last_active DESC, s.started_at DESC
        LIMIT ?";

    let rows = match guard.prepare(sql_with_bindings) {
        Ok(mut stmt) => stmt
            .query_map(params![user_id, limit as i64], map_unlinked_row)?
            .filter_map(|r| r.ok())
            .collect(),
        Err(_) => guard
            .prepare(sql_fallback)?
            .query_map(params![user_id, limit as i64], map_unlinked_row)?
            .filter_map(|r| r.ok())
            .collect(),
    };
    Ok(rows)
}

fn map_unlinked_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UnlinkedTelegramSession> {
    let raw: String = row.get("_preview_raw").unwrap_or_default();
    let raw = raw.trim();
    let preview = if raw.is_empty() {
        None
    } else if raw.chars().count() > 60 {
        Some(format!("{}...", raw.chars().take(60).collect::<String>()))
    } else {
        Some(raw.to_string())
    };
    Ok(UnlinkedTelegramSession {
        id: row.get("id")?,
        title: row.get("title").ok(),
        preview,
        started_at: row.get("started_at")?,
        last_active: row.get("last_active")?,
        message_count: row.get("message_count").unwrap_or(0),
    })
}

/// System commands allowed in Telegram DM root lobby when topic mode is active.
pub fn is_telegram_lobby_system_command(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return false;
    }
    let cmd = trimmed.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    matches!(
        cmd.as_str(),
        "/topic" | "/status" | "/sessions" | "/usage" | "/cost" | "/help" | "/platforms"
    )
}
