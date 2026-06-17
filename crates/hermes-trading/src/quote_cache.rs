//! Short-TTL disk cache for spot quotes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hermes_config::hermes_home;

use crate::error::TradingError;
use crate::quote_data::QuoteData;

const DEFAULT_TTL_SECS: u64 = 5 * 60;

fn quote_ttl() -> Duration {
    std::env::var("HERMES_TRADING_QUOTE_CACHE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_TTL_SECS))
}

/// File-backed quote cache under `{HERMES_HOME}/trading/quote-cache/`.
#[derive(Debug, Clone)]
pub struct QuoteCache {
    dir: Option<PathBuf>,
    ttl: Duration,
}

impl QuoteCache {
    #[must_use]
    pub fn default_path() -> Self {
        Self {
            dir: Some(hermes_home().join("trading").join("quote-cache")),
            ttl: quote_ttl(),
        }
    }

    #[must_use]
    pub fn disabled() -> Self {
        Self {
            dir: None,
            ttl: quote_ttl(),
        }
    }

    #[must_use]
    pub fn with_dir(dir: PathBuf) -> Self {
        Self {
            dir: Some(dir),
            ttl: quote_ttl(),
        }
    }

    #[must_use]
    pub fn with_dir_and_ttl(dir: PathBuf, ttl: Duration) -> Self {
        Self {
            dir: Some(dir),
            ttl,
        }
    }

    #[must_use]
    pub fn cache_key(source: &str, symbol: &str) -> String {
        let safe = symbol.replace(['/', '\\'], "_");
        format!("{source}-{safe}.json")
    }

    pub async fn get(&self, key: &str) -> Option<QuoteData> {
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

    pub async fn put(&self, key: &str, data: &QuoteData) -> Result<(), TradingError> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| TradingError::InvalidResponse("Quote cache is disabled".into()))?;
        tokio::fs::create_dir_all(dir).await.map_err(|e| {
            TradingError::InvalidResponse(format!("Failed to create quote cache directory: {e}"))
        })?;

        let path = dir.join(key);
        let tmp = dir.join(format!("{key}.tmp"));
        let json = serde_json::to_string_pretty(data)?;
        tokio::fs::write(&tmp, json).await.map_err(|e| {
            TradingError::InvalidResponse(format!("Failed to write quote cache file: {e}"))
        })?;
        tokio::fs::rename(&tmp, &path).await.map_err(|e| {
            TradingError::InvalidResponse(format!("Failed to rename quote cache file: {e}"))
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
    use std::time::Duration;

    #[tokio::test]
    async fn quote_cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cache = QuoteCache::with_dir(dir.path().to_path_buf());
        let mut data = QuoteData::new("000001.SZ", "eastmoney");
        data.price = Some(10.5);
        data.partial = false;
        let key = QuoteCache::cache_key("eastmoney", "000001.SZ");
        cache.put(&key, &data).await.unwrap();
        let loaded = cache.get(&key).await.unwrap();
        assert_eq!(loaded.price, Some(10.5));
    }

    #[tokio::test]
    async fn expired_quote_cache_misses() {
        let dir = tempfile::tempdir().unwrap();
        let cache =
            QuoteCache::with_dir_and_ttl(dir.path().to_path_buf(), Duration::from_millis(1));
        let mut data = QuoteData::new("000001.SZ", "eastmoney");
        data.price = Some(1.0);
        let key = QuoteCache::cache_key("eastmoney", "000001.SZ");
        cache.put(&key, &data).await.unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(cache.get(&key).await.is_none());
    }
}
