use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use hermes_core::LlmProvider;

const PROVIDER_CACHE_MAX_SIZE: usize = 128;
const PROVIDER_CACHE_IDLE_TTL: Duration = Duration::from_secs(3600);

pub(super) struct ProviderCacheEntry {
    pub(super) provider: Arc<dyn LlmProvider>,
    pub(super) last_used: Instant,
}

pub(crate) fn provider_cache()
-> &'static StdMutex<std::collections::HashMap<String, ProviderCacheEntry>> {
    static CACHE: OnceLock<StdMutex<std::collections::HashMap<String, ProviderCacheEntry>>> =
        OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
pub(crate) fn clear_provider_cache() {
    provider_cache().lock().unwrap().clear();
}

pub(crate) fn provider_cache_key(
    runtime_provider: &str,
    model_name: &str,
    base_url: Option<&str>,
    api_key: &str,
) -> String {
    format!(
        "{}|{}|{}|{}",
        runtime_provider,
        model_name,
        base_url.unwrap_or(""),
        api_key
    )
}

pub(super) fn prune_provider_cache(
    cache: &mut std::collections::HashMap<String, ProviderCacheEntry>,
) {
    let now = Instant::now();
    cache.retain(|_, entry| now.duration_since(entry.last_used) <= PROVIDER_CACHE_IDLE_TTL);
    if cache.len() <= PROVIDER_CACHE_MAX_SIZE {
        return;
    }
    let mut entries: Vec<(String, Instant)> = cache
        .iter()
        .map(|(k, v)| (k.clone(), v.last_used))
        .collect();
    entries.sort_by_key(|(_, used)| *used);
    let overflow = cache.len().saturating_sub(PROVIDER_CACHE_MAX_SIZE);
    for (key, _) in entries.into_iter().take(overflow) {
        cache.remove(&key);
    }
}
