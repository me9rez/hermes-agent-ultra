use std::path::{Path, PathBuf};

use serde::Deserialize;

use hermes_tasks::TaskCategory;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerticalLoadError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct VerticalMeta {
    pub id: String,
    pub display_name_key: String,
    pub description_key: String,
    pub icon: String,
    pub category: String,
    pub order: u32,
    pub task_category: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VerticalDefinition {
    pub meta: VerticalMeta,
    pub dir: PathBuf,
    pub starters: serde_json::Value,
    pub datasources: serde_json::Value,
}

pub struct VerticalLoader {
    bundled_root: PathBuf,
}

impl VerticalLoader {
    pub fn bundled() -> Self {
        Self {
            bundled_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bundled"),
        }
    }

    pub fn with_root(root: impl AsRef<Path>) -> Self {
        Self {
            bundled_root: root.as_ref().to_path_buf(),
        }
    }

    pub fn list(&self) -> Result<Vec<VerticalDefinition>, VerticalLoadError> {
        let mut out = Vec::new();
        if !self.bundled_root.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&self.bundled_root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                out.push(self.load_vertical(entry.path())?);
            }
        }
        out.sort_by_key(|v| v.meta.order);
        Ok(out)
    }

    pub fn load(&self, id: &str) -> Result<VerticalDefinition, VerticalLoadError> {
        self.load_vertical(self.bundled_root.join(id))
    }

    fn load_vertical(&self, dir: PathBuf) -> Result<VerticalDefinition, VerticalLoadError> {
        let vertical_md = dir.join("VERTICAL.md");
        let content = std::fs::read_to_string(&vertical_md)?;
        let (frontmatter, _) = split_frontmatter(&content);
        let doc: toml::Table = toml::from_str(frontmatter)?;
        let meta: VerticalMeta = doc
            .get("meta")
            .ok_or_else(|| VerticalLoadError::Other("missing [meta]".into()))?
            .clone()
            .try_into()
            .map_err(VerticalLoadError::Toml)?;

        let starters = read_json_or_default(&dir.join("starters.json"));
        let datasources = read_json_or_default(&dir.join("datasources.json"));

        Ok(VerticalDefinition {
            meta,
            dir,
            starters,
            datasources,
        })
    }
}

impl VerticalMeta {
    pub fn task_category_enum(&self) -> Option<TaskCategory> {
        self.task_category
            .as_deref()
            .and_then(TaskCategory::from_str_loose)
    }
}

fn split_frontmatter(content: &str) -> (&str, &str) {
    if let Some(rest) = content.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        let fm = &rest[..end];
        let body = &rest[end + 4..];
        return (fm, body);
    }
    ("", content)
}

fn read_json_or_default(path: &Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!([]))
}
