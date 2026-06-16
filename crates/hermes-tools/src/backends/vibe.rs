//! RunCard persistence backend for vibe research.
//!
//! Stores backtest run cards to `{HERMES_HOME}/vibe/runs/{id}/run_card.json`
//! and loads them back by ID.

use std::path::PathBuf;

use async_trait::async_trait;

use hermes_core::ToolError;
use hermes_vibe::RunCard;

/// Trait for RunCard persistence operations.
#[async_trait]
pub trait RunCardStore: Send + Sync {
    /// Save a RunCard to disk, returning the assigned id.
    async fn save(&self, card: &RunCard) -> Result<String, ToolError>;

    /// Load a RunCard by id.
    async fn load(&self, id: &str) -> Result<RunCard, ToolError>;
}

/// File-based RunCard store.
///
/// Root directory: `{HERMES_HOME}/vibe/runs/`
/// Each RunCard is stored at: `{root}/{id}/run_card.json`
#[derive(Debug, Clone)]
pub struct FileRunCardStore {
    runs_dir: PathBuf,
}

impl FileRunCardStore {
    /// Create a new store with the given root directory.
    pub fn new(runs_dir: PathBuf) -> Self {
        Self { runs_dir }
    }

    /// Create a store using the default path under HERMES_HOME.
    pub fn default_path() -> Self {
        let dir = hermes_config::hermes_home().join("vibe").join("runs");
        Self::new(dir)
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.runs_dir.join(id)
    }
}

#[async_trait]
impl RunCardStore for FileRunCardStore {
    async fn save(&self, card: &RunCard) -> Result<String, ToolError> {
        if card.id.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "Cannot save RunCard with empty id; call generate_id() first".into(),
            ));
        }

        let dir = self.run_dir(&card.id);
        tokio::fs::create_dir_all(&dir).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to create run directory: {e}"))
        })?;

        let json = serde_json::to_string_pretty(card).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to serialize run_card: {e}"))
        })?;

        // Atomic write: write to .tmp then rename.
        let tmp = dir.join("run_card.json.tmp");
        let path = dir.join("run_card.json");

        tokio::fs::write(&tmp, &json).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to write run_card: {e}"))
        })?;

        tokio::fs::rename(&tmp, &path).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to rename run_card: {e}"))
        })?;

        tracing::info!(
            id = %card.id,
            symbol = %card.symbol,
            strategy = %card.strategy,
            "Saved run_card to disk"
        );

        Ok(card.id.clone())
    }

    async fn load(&self, id: &str) -> Result<RunCard, ToolError> {
        let path = self.run_dir(id).join("run_card.json");
        if !path.exists() {
            return Err(ToolError::ExecutionFailed(format!(
                "Backtest run '{id}' not found at '{}'. \
                 Check that the ID is correct.",
                path.display()
            )));
        }

        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to read run_card: {e}"))
        })?;

        serde_json::from_str(&content).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to parse run_card: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermes_vibe::{Period, RunCard};

    fn sample_card() -> RunCard {
        RunCard {
            id: "BTC-USDT-sma_cross-20260616T143022Z".into(),
            created_at: "2026-06-16T14:30:22+00:00".into(),
            symbol: "BTC-USDT".into(),
            strategy: "sma_cross".into(),
            params: serde_json::json!({"short_window": 20, "long_window": 50}),
            total_return_pct: 12.5,
            max_drawdown_pct: -3.2,
            trade_count: 4,
            sharpe_ratio: 1.8,
            win_rate_pct: 75.0,
            period: Period {
                start: "2025-12-01".into(),
                end: "2026-06-01".into(),
            },
        }
    }

    #[tokio::test]
    async fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileRunCardStore::new(dir.path().to_path_buf());
        let card = sample_card();

        let id = store.save(&card).await.unwrap();
        assert_eq!(id, "BTC-USDT-sma_cross-20260616T143022Z");

        let loaded = store.load(&id).await.unwrap();
        assert_eq!(loaded.id, card.id);
        assert_eq!(loaded.symbol, card.symbol);
        assert_eq!(loaded.strategy, card.strategy);
        assert_eq!(loaded.total_return_pct, card.total_return_pct);
    }

    #[tokio::test]
    async fn test_load_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileRunCardStore::new(dir.path().to_path_buf());

        let result = store.load("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_save_empty_id_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileRunCardStore::new(dir.path().to_path_buf());
        let mut card = sample_card();
        card.id = String::new();

        let result = store.save(&card).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty id"));
    }

    #[tokio::test]
    async fn test_creates_nested_dir() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileRunCardStore::new(dir.path().join("a").join("b"));
        let card = sample_card();

        let _id = store.save(&card).await.unwrap();
        assert!(store.run_dir(&card.id).exists());
    }
}
