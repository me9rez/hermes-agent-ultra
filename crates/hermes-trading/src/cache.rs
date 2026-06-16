//! Disk cache for OHLCV market data responses.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hermes_config::hermes_home;

use crate::error::TradingError;
use crate::types::{Interval, OhlcvData, OhlcvRequest};

/// Default cache TTL: 24 hours.
const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// File-backed OHLCV cache under `{HERMES_HOME}/trading/cache/`.
#[derive(Debug, Clone)]
pub struct DiskCache {
    dir: Option<PathBuf>,
    ttl: Duration,
}

impl DiskCache {
    /// Cache at `{HERMES_HOME}/trading/cache/` with 24h TTL.
    #[must_use]
    pub fn default_path() -> Self {
        Self {
            dir: Some(hermes_home().join("trading").join("cache")),
            ttl: DEFAULT_TTL,
        }
    }

    /// Disabled cache (parity tests and explicit opt-out).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            dir: None,
            ttl: DEFAULT_TTL,
        }
    }

    /// Cache at a custom directory (unit tests).
    #[must_use]
    pub fn with_dir(dir: PathBuf) -> Self {
        Self {
            dir: Some(dir),
            ttl: DEFAULT_TTL,
        }
    }

    /// Cache with a custom TTL (unit tests).
    #[must_use]
    pub fn with_dir_and_ttl(dir: PathBuf, ttl: Duration) -> Self {
        Self {
            dir: Some(dir),
            ttl,
        }
    }

    /// Build cache filename key: `{source}-{symbol}-{interval}-{start}-{end}.json`.
    #[must_use]
    pub fn cache_key(source: &str, req: &OhlcvRequest) -> String {
        let safe_symbol = req.symbol.replace('/', "_");
        let interval = match req.interval {
            Interval::Daily => "daily",
            Interval::Weekly => "weekly",
        };
        format!(
            "{source}-{safe_symbol}-{interval}-{}-{}.json",
            req.start.format("%Y-%m-%d"),
            req.end.format("%Y-%m-%d")
        )
    }

    /// Read cached data if present and not expired.
    pub async fn get(&self, key: &str) -> Option<OhlcvData> {
        let path = self.path_for(key)?;
        if !path.exists() {
            return None;
        }
        if self.is_expired(&path).await {
            return None;
        }
        let content = tokio::fs::read_to_string(&path).await.ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Write data to cache (atomic tmp + rename).
    pub async fn put(&self, key: &str, data: &OhlcvData) -> Result<(), TradingError> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| TradingError::InvalidResponse("Cache is disabled".into()))?;
        tokio::fs::create_dir_all(dir).await.map_err(|e| {
            TradingError::InvalidResponse(format!("Failed to create cache directory: {e}"))
        })?;

        let path = dir.join(key);
        let tmp = dir.join(format!("{key}.tmp"));
        let json = serde_json::to_string_pretty(data)?;
        tokio::fs::write(&tmp, json).await.map_err(|e| {
            TradingError::InvalidResponse(format!("Failed to write cache file: {e}"))
        })?;
        tokio::fs::rename(&tmp, &path).await.map_err(|e| {
            TradingError::InvalidResponse(format!("Failed to rename cache file: {e}"))
        })?;
        Ok(())
    }

    fn path_for(&self, key: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(key))
    }

    async fn is_expired(&self, path: &Path) -> bool {
        let meta = match tokio::fs::metadata(path).await {
            Ok(m) => m,
            Err(_) => return true,
        };
        let modified = match meta.modified() {
            Ok(t) => t,
            Err(_) => return true,
        };
        modified
            .elapsed()
            .map(|elapsed| elapsed > self.ttl)
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    use crate::types::OhlcvRow;

    fn sample_req() -> OhlcvRequest {
        OhlcvRequest {
            symbol: "BTC-USDT".to_string(),
            start: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 5, 10).unwrap(),
            interval: Interval::Daily,
        }
    }

    fn sample_data() -> OhlcvData {
        OhlcvData {
            symbol: "BTC-USDT".to_string(),
            interval: Interval::Daily,
            rows: vec![OhlcvRow {
                date: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            }],
            partial: false,
        }
    }

    #[test]
    fn cache_key_format() {
        let key = DiskCache::cache_key("binance", &sample_req());
        assert_eq!(key, "binance-BTC-USDT-daily-2026-05-01-2026-05-10.json");
    }

    #[tokio::test]
    async fn put_and_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::with_dir(dir.path().to_path_buf());
        let key = DiskCache::cache_key("binance", &sample_req());
        let data = sample_data();
        cache.put(&key, &data).await.unwrap();
        let loaded = cache.get(&key).await.unwrap();
        assert_eq!(loaded.symbol, "BTC-USDT");
        assert!(!loaded.partial);
    }

    #[tokio::test]
    async fn disabled_cache_returns_none() {
        let cache = DiskCache::disabled();
        assert!(cache.get("any.json").await.is_none());
    }

    #[tokio::test]
    async fn expired_cache_misses() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::with_dir_and_ttl(dir.path().to_path_buf(), Duration::from_millis(1));
        let key = DiskCache::cache_key("binance", &sample_req());
        cache.put(&key, &sample_data()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(cache.get(&key).await.is_none());
    }
}
