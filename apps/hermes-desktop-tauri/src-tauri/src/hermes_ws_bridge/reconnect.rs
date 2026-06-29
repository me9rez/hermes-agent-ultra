use std::time::Duration;

pub struct ExponentialBackoff {
    attempt: u32,
    base_ms: u64,
    max_ms: u64,
}

impl ExponentialBackoff {
    pub fn new() -> Self {
        Self {
            attempt: 0,
            base_ms: 500,
            max_ms: 30_000,
        }
    }

    pub fn next_delay(&mut self) -> Duration {
        let exp = self.base_ms.saturating_mul(1u64 << self.attempt.min(6));
        let delay = exp.min(self.max_ms);
        self.attempt = self.attempt.saturating_add(1);
        Duration::from_millis(delay)
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new()
    }
}
