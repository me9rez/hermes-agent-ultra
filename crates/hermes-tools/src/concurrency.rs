//! Bounded concurrent task execution.
//!
//! A single `Semaphore` controls in-flight count so we never exceed `concurrency`
//! running tasks at once, yet start the next task as soon as any one completes —
//! unlike the manual `if join_set.len() >= concurrency { join_next }` pattern,
//! which starves the queue whenever a slow task blocks the drain loop.

use std::future::Future;
use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Run `tasks` with at most `concurrency` running concurrently.
///
/// Returns all outputs in the order tasks complete (not input order).
/// A `concurrency` of 0 is treated as 1.
pub async fn run_bounded<F, T>(concurrency: usize, tasks: impl IntoIterator<Item = F>) -> Vec<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let concurrency = concurrency.max(1);
    let sem = Arc::new(Semaphore::new(concurrency));
    let mut join_set = JoinSet::new();

    for task in tasks {
        let permit = Arc::clone(&sem)
            .acquire_owned()
            .await
            .expect("semaphore closed");
        join_set.spawn(async move {
            let result = task.await;
            drop(permit);
            result
        });
    }

    let mut results = Vec::with_capacity(join_set.len());
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(v) => results.push(v),
            Err(e) => std::panic::resume_unwind(e.into_panic()),
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::run_bounded;

    #[tokio::test]
    async fn peak_concurrency_respects_bound() {
        const TASK_COUNT: usize = 10;
        const CONCURRENCY: usize = 2;

        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let tasks = (0..TASK_COUNT).map(|_| {
            let in_flight = Arc::clone(&in_flight);
            let peak = Arc::clone(&peak);
            async move {
                let current = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
            }
        });

        run_bounded(CONCURRENCY, tasks).await;

        assert!(
            peak.load(Ordering::SeqCst) <= CONCURRENCY,
            "peak concurrency exceeded bound: {}",
            peak.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn all_tasks_complete() {
        let results = run_bounded(3, (0u32..12).map(|i| async move { i * 2 })).await;
        assert_eq!(results.len(), 12);
    }
}
