use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::params;
use serde_json::Value;

use crate::db::{DbError, DbResult, TaskDb, parse_ulid_id};
use crate::types::{ArtifactId, TaskId, UserId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArtifactRecord {
    pub id: ArtifactId,
    pub task_id: TaskId,
    pub owner_user_id: UserId,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub ext: String,
    pub relative_path: String,
    pub created_at: DateTime<Utc>,
    pub metadata: Option<Value>,
}

pub struct ArtifactStore {
    db: TaskDb,
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(db: TaskDb, root: impl AsRef<Path>) -> DbResult<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        Ok(Self { db, root })
    }

    pub fn default_root() -> PathBuf {
        directories::ProjectDirs::from("app", "terra", "terra")
            .map(|d| d.data_dir().join("artifacts"))
            .unwrap_or_else(|| std::env::temp_dir().join("terra-artifacts"))
    }

    pub fn open(db: TaskDb) -> DbResult<Self> {
        Self::new(db, Self::default_root())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn task_dir(&self, task_id: TaskId) -> PathBuf {
        self.root.join(task_id.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write_bytes(
        &self,
        task_id: TaskId,
        owner_user_id: UserId,
        name: impl Into<String>,
        mime_type: impl Into<String>,
        ext: impl Into<String>,
        bytes: &[u8],
        metadata: Option<Value>,
    ) -> DbResult<ArtifactRecord> {
        let name = name.into();
        let mime_type = mime_type.into();
        let ext = ext.into();
        let id = ArtifactId::new();
        let task_dir = self.task_dir(task_id);
        std::fs::create_dir_all(&task_dir)?;
        let filename = format!("{id}.{ext}");
        let relative_path = format!("{}/{filename}", task_id);
        let full_path = self.root.join(&relative_path);
        std::fs::write(&full_path, bytes)?;

        let record = ArtifactRecord {
            id,
            task_id,
            owner_user_id,
            name,
            mime_type,
            size_bytes: bytes.len() as u64,
            ext,
            relative_path,
            created_at: Utc::now(),
            metadata,
        };
        self.insert_record(&record)?;
        Ok(record)
    }

    pub fn read_bytes(&self, artifact_id: ArtifactId) -> DbResult<Vec<u8>> {
        let record = self
            .get(artifact_id)?
            .ok_or_else(|| DbError::Other("artifact not found".into()))?;
        let path = self.root.join(&record.relative_path);
        Ok(std::fs::read(path)?)
    }

    pub fn get(&self, artifact_id: ArtifactId) -> DbResult<Option<ArtifactRecord>> {
        self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, task_id, owner_user_id, name, mime_type, size_bytes, ext,
                        relative_path, created_at, metadata_json
                 FROM artifacts WHERE id = ?1",
            )?;
            let mut rows = stmt.query(params![artifact_id.to_string()])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row_to_record(row)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn list_for_task(&self, task_id: TaskId) -> DbResult<Vec<ArtifactRecord>> {
        self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, task_id, owner_user_id, name, mime_type, size_bytes, ext,
                        relative_path, created_at, metadata_json
                 FROM artifacts WHERE task_id = ?1 ORDER BY created_at DESC",
            )?;
            let mut rows = stmt.query(params![task_id.to_string()])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(row_to_record(row)?);
            }
            Ok(out)
        })
    }

    fn insert_record(&self, record: &ArtifactRecord) -> DbResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO artifacts (
                    id, task_id, owner_user_id, name, mime_type, size_bytes, ext,
                    relative_path, created_at, metadata_json
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    record.id.to_string(),
                    record.task_id.to_string(),
                    record.owner_user_id.to_string(),
                    record.name,
                    record.mime_type,
                    record.size_bytes,
                    record.ext,
                    record.relative_path,
                    record.created_at.to_rfc3339(),
                    record
                        .metadata
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|e| DbError::Other(e.to_string()))?,
                ],
            )?;
            Ok(())
        })
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> DbResult<ArtifactRecord> {
    let created_at: String = row.get(8)?;
    let metadata_str: Option<String> = row.get(9)?;
    Ok(ArtifactRecord {
        id: parse_ulid_id(row.get::<_, String>(0)?.as_str())?,
        task_id: parse_ulid_id(row.get::<_, String>(1)?.as_str())?,
        owner_user_id: parse_ulid_id(row.get::<_, String>(2)?.as_str())?,
        name: row.get(3)?,
        mime_type: row.get(4)?,
        size_bytes: row.get(5)?,
        ext: row.get(6)?,
        relative_path: row.get(7)?,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map_err(|e| DbError::Other(e.to_string()))?
            .with_timezone(&Utc),
        metadata: metadata_str
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| DbError::Other(e.to_string()))?,
    })
}
