use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use thiserror::Error;

const BASELINE_MIGRATION: &str = include_str!("../migrations/0001_baseline.sql");

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub type DbResult<T> = Result<T, DbError>;

pub fn parse_ulid_id<T>(s: &str) -> DbResult<T>
where
    T: std::str::FromStr<Err = ulid::DecodeError>,
{
    s.parse::<T>().map_err(|e| DbError::Other(e.to_string()))
}

#[derive(Clone)]
pub struct TaskDb {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl TaskDb {
    pub fn open(path: impl AsRef<Path>) -> DbResult<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(BASELINE_MIGRATION)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        })
    }

    pub fn open_default() -> DbResult<Self> {
        let path = default_tasks_db_path();
        Self::open(path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    pub fn with_conn<F, T>(&self, f: F) -> DbResult<T>
    where
        F: FnOnce(&Connection) -> DbResult<T>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|_| DbError::Other("db lock poisoned".into()))?;
        f(&conn)
    }
}

pub fn default_tasks_db_path() -> PathBuf {
    let base = directories::ProjectDirs::from("app", "terra", "terra")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("terra"));
    base.join("tasks.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let db = TaskDb::open(dir.path().join("tasks.db")).unwrap();
        db.with_conn(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tasks'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(count, 1);
            Ok(())
        })
        .unwrap();
    }
}
