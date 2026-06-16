//! Runtime strategy registry.
//!
//! Manages built-in and user-defined strategies. On construction, it loads
//! built-in strategies, then scans `$HERMES_HOME/vibe/strategies/` for
//! user-defined JSON files.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::builtin::all_builtin_defs;
use crate::declarative::DeclarativeStrategy;
use crate::dsl::DeclarativeStrategyDef;
use crate::error::StrategyError;
use crate::strategy::Strategy;

/// Metadata about a registered strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyInfo {
    /// Strategy name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Author: "builtin" or "user".
    pub author: String,
    /// Default parameter values.
    pub default_params: serde_json::Value,
}

/// Runtime registry of available backtest strategies.
pub struct StrategyRegistry {
    strategies: HashMap<String, Arc<dyn Strategy>>,
    /// Metadata for each strategy.
    info: HashMap<String, StrategyMeta>,
}

impl Default for StrategyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal metadata kept alongside the strategy.
struct StrategyMeta {
    info: StrategyInfo,
}

impl StrategyRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            strategies: HashMap::new(),
            info: HashMap::new(),
        }
    }

    /// Create a registry with built-in strategies loaded.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register_builtins();
        reg
    }

    /// Create a registry with built-ins + user strategies from a directory.
    pub async fn with_builtins_and_dir(dir: &Path) -> Self {
        let mut reg = Self::with_builtins();
        reg.load_from_dir(dir).await;
        reg
    }

    /// Register all built-in strategies.
    pub fn register_builtins(&mut self) {
        for def in all_builtin_defs() {
            let name = def.name.clone();
            let info = StrategyInfo {
                name: def.name.clone(),
                description: def.description.clone(),
                author: def.author.clone(),
                default_params: def.default_params.clone(),
            };
            match DeclarativeStrategy::from_def(def) {
                Ok(strategy) => {
                    self.info.insert(name.clone(), StrategyMeta { info });
                    self.strategies.insert(name, Arc::new(strategy));
                }
                Err(e) => {
                    tracing::error!(strategy = %name, error = %e, "Failed to compile built-in strategy");
                }
            }
        }
    }

    /// Load user strategies from a directory of JSON files.
    ///
    /// Files that fail validation are skipped with a warning log.
    pub async fn load_from_dir(&mut self, dir: &Path) {
        if !dir.exists() {
            return;
        }
        let entries = match fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(path = %dir.display(), error = %e, "Failed to read strategies directory");
                return;
            }
        };

        let mut entries = entries;
        while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match Self::load_strategy_file(&path).await {
                Ok((def, strategy)) => {
                    let name = def.name.clone();
                    if self.strategies.contains_key(&name) {
                        tracing::warn!(strategy = %name, "Strategy already registered, skipping file {}", path.display());
                        continue;
                    }
                    let info = StrategyInfo {
                        name: def.name.clone(),
                        description: def.description.clone(),
                        author: def.author.clone(),
                        default_params: def.default_params.clone(),
                    };
                    self.info.insert(name.clone(), StrategyMeta { info });
                    self.strategies.insert(name, Arc::new(strategy));
                    tracing::info!(strategy = %path.display(), "Loaded user strategy");
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Failed to load strategy file");
                }
            }
        }
    }

    /// Load a single strategy file, validate, and compile.
    async fn load_strategy_file(
        path: &Path,
    ) -> Result<(DeclarativeStrategyDef, DeclarativeStrategy), StrategyError> {
        let content = fs::read_to_string(path).await?;
        let def: DeclarativeStrategyDef = serde_json::from_str(&content)?;
        def.validate()?;
        let strategy = DeclarativeStrategy::from_def(def.clone())?;
        Ok((def, strategy))
    }

    /// Register a strategy.
    pub fn register(&mut self, strategy: Arc<dyn Strategy>, info: StrategyInfo) {
        let name = info.name.clone();
        self.info.insert(name.clone(), StrategyMeta { info });
        self.strategies.insert(name, strategy);
    }

    /// Get a strategy by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Strategy>> {
        self.strategies.get(name).cloned()
    }

    /// List all registered strategies' metadata.
    pub fn list(&self) -> Vec<StrategyInfo> {
        let mut infos: Vec<StrategyInfo> = self.info.values().map(|m| m.info.clone()).collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    /// Unregister a strategy (only user strategies can be removed).
    ///
    /// Returns `true` if the strategy was found and removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        // Fix 5: Check info first to prevent bypassing builtin protection.
        if self.info.get(name).is_some_and(|m| m.info.author == "builtin") {
            return false;
        }
        let removed_s = self.strategies.remove(name).is_some();
        let removed_i = self.info.remove(name).is_some();
        removed_s || removed_i
    }

    /// Check if a strategy name already exists.
    pub fn contains(&self, name: &str) -> bool {
        self.strategies.contains_key(name)
    }
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_with_builtins() {
        let reg = StrategyRegistry::with_builtins();
        assert!(reg.get("sma_cross").is_some());
        assert!(reg.get("rsi_revert").is_some());
        assert!(reg.get("nonexistent").is_none());
        let list = reg.list();
        assert!(list.len() >= 2);
    }

    #[test]
    fn test_registry_cannot_remove_builtin() {
        let mut reg = StrategyRegistry::with_builtins();
        assert!(!reg.unregister("sma_cross"));
        assert!(reg.get("sma_cross").is_some());
    }

    #[tokio::test]
    async fn test_registry_load_from_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = StrategyRegistry::with_builtins();
        reg.load_from_dir(dir.path()).await;
        assert!(reg.get("sma_cross").is_some()); // builtins still there
    }

    #[tokio::test]
    async fn test_registry_load_from_dir_with_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let strategy_json = r#"{
            "name": "macd_cross",
            "description": "MACD crossover",
            "indicators": [
                {"id": "macd_line", "type": "macd", "params": {"fast": 12, "slow": 26}},
                {"id": "signal_line", "type": "ema", "params": {"period": 9}, "source": "macd_line"}
            ],
            "rules": {
                "buy": "macd_line crosses_above signal_line",
                "sell": "macd_line crosses_below signal_line"
            }
        }"#;
        let file_path = dir.path().join("macd_cross.json");
        tokio::fs::write(&file_path, strategy_json).await.unwrap();

        let mut reg = StrategyRegistry::with_builtins();
        reg.load_from_dir(dir.path()).await;
        assert!(reg.get("macd_cross").is_some());
    }

    #[tokio::test]
    async fn test_registry_skip_invalid_file() {
        let dir = tempfile::tempdir().unwrap();
        let bad_json = r#"{"name": "bad", "description": "test"}"#; // missing indicators/rules
        let file_path = dir.path().join("bad.json");
        tokio::fs::write(&file_path, bad_json).await.unwrap();

        let mut reg = StrategyRegistry::with_builtins();
        reg.load_from_dir(dir.path()).await;
        assert!(reg.get("bad").is_none()); // skipped
    }
}
