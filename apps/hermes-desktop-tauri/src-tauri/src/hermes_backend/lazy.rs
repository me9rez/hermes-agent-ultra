use std::sync::atomic::{AtomicBool, Ordering};

pub struct LazyBackendGate {
    started: AtomicBool,
}

impl LazyBackendGate {
    pub fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
        }
    }

    pub fn should_start(&self) -> bool {
        !self.started.swap(true, Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.started.store(false, Ordering::SeqCst);
    }
}

impl Default for LazyBackendGate {
    fn default() -> Self {
        Self::new()
    }
}
