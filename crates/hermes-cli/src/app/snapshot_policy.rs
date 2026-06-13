use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(crate) struct SnapshotPersistGate {
    pub(super) last_persist: Instant,
    pub(super) pending_mutations: u32,
    pub(super) backoff_ms: u64,
}

impl SnapshotPersistGate {
    const MIN_INTERVAL_MS: u64 = 500;
    const MAX_BACKOFF_MS: u64 = 8_000;
    const MUTATION_THRESHOLD: u32 = 3;

    pub(super) fn new() -> Self {
        Self {
            last_persist: Instant::now(),
            pending_mutations: 0,
            backoff_ms: Self::MIN_INTERVAL_MS,
        }
    }

    pub(super) fn record_mutation(&mut self) {
        self.pending_mutations += 1;
    }

    pub(super) fn should_persist(&self) -> bool {
        let elapsed = self.last_persist.elapsed();
        elapsed >= Duration::from_millis(Self::MIN_INTERVAL_MS)
            || self.pending_mutations >= Self::MUTATION_THRESHOLD
            || elapsed >= Duration::from_millis(self.backoff_ms)
    }

    pub(super) fn mark_persisted(&mut self) {
        self.last_persist = Instant::now();
        self.pending_mutations = 0;
        self.backoff_ms = (self.backoff_ms.saturating_mul(2)).min(Self::MAX_BACKOFF_MS);
    }
}
