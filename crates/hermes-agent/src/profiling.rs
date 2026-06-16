//! Runtime profiling utilities for hot-path instrumentation.
//!
//! # Usage
//!
//! ## Heap profiling (`dhat-heap` feature)
//!
//! ```bash
//! cargo test -p hermes-agent --features dhat-heap -- --nocapture hotpath_200_turn_baseline
//! # Writes dhat-heap.json; open with https://nnethercote.github.io/dh_view/dh_view.html
//! ```
//!
//! ## CPU flamegraph
//!
//! ```bash
//! RUST_LOG="hermes_agent[build_turn_api_messages]=debug,hermes_agent[execute_tool_calls]=debug" \
//!   cargo run -p hermes-cli
//! # Pipe to inferno or use pprof-rs to capture a flamegraph.
//! ```
//!
//! ## Interpreting spans
//!
//! Hot-path spans and their key fields (stable names):
//!
//! | Span name                 | Fields                                     |
//! |---------------------------|--------------------------------------------|
//! | `build_turn_api_messages` | `msg_count`, `total_chars`                 |
//! | `assemble_api_messages`   | `source_len`, `prefetch_bytes`             |
//! | `tool_call`               | `tool`, `id`                               |
//! | `tool batch complete`     | `turn`, `tool_count`, `elapsed_ms` (event) |
//!
//! Filter them:
//! ```text
//! RUST_LOG="hermes_agent[build_turn_api_messages]=debug"
//! ```

/// Hold a `dhat::Profiler` for the lifetime of a test or binary.
///
/// Construct with [`DhatGuard::new`] at the top of your test / `main`, keep
/// the binding alive until the scope ends, and DHAT will write `dhat-heap.json`
/// on drop.
///
/// Outside the `dhat-heap` feature this is a zero-sized no-op type.
#[cfg(feature = "dhat-heap")]
pub struct DhatGuard(dhat::Profiler);

#[cfg(feature = "dhat-heap")]
impl DhatGuard {
    pub fn new() -> Self {
        Self(dhat::Profiler::new_heap())
    }
}

#[cfg(not(feature = "dhat-heap"))]
pub struct DhatGuard;

#[cfg(not(feature = "dhat-heap"))]
impl DhatGuard {
    #[inline(always)]
    pub fn new() -> Self {
        Self
    }
}

impl Default for DhatGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use hermes_core::Message;

    use super::*;
    use crate::api_messages::assemble_api_messages_from_ctx;

    fn make_conversation(turns: usize, chars_per_message: usize) -> Vec<Message> {
        let content = "x".repeat(chars_per_message);
        let mut msgs = Vec::with_capacity(turns * 2);
        for _ in 0..turns {
            msgs.push(Message::user(content.clone()));
            msgs.push(Message::assistant(content.clone()));
        }
        msgs
    }

    /// Synthetic 200-turn hot-path benchmark.
    ///
    /// Run with `--features dhat-heap` to capture allocation profile:
    /// ```bash
    /// cargo test -p hermes-agent --features dhat-heap \
    ///   -- --nocapture hotpath_200_turn_baseline
    /// ```
    ///
    /// With the default feature set this test still verifies correctness and
    /// reports wall-clock timing so the result can be used as a baseline.
    #[test]
    fn hotpath_200_turn_baseline() {
        let _guard = DhatGuard::new();

        const TURNS: usize = 200;
        const CHARS_PER_MSG: usize = 500;

        let messages = make_conversation(TURNS, CHARS_PER_MSG);
        let total_input_chars: usize = messages
            .iter()
            .filter_map(|m| m.content.as_deref())
            .map(|c| c.len())
            .sum();

        let t0 = Instant::now();
        let result = assemble_api_messages_from_ctx(
            &messages,
            "",
            None,
            "gpt-4o",
            "ephemeral",
            false,
            false,
            false,
        );
        let elapsed = t0.elapsed();

        assert_eq!(result.len(), TURNS * 2);

        let total_output_chars: usize = result
            .iter()
            .filter_map(|m| m.content.as_deref())
            .map(|c| c.len())
            .sum();
        assert_eq!(total_input_chars, total_output_chars);

        println!(
            "[hotpath_200_turn_baseline] {} messages × {} chars → {} ms  \
             (input {:.1} KB, output {:.1} KB)",
            messages.len(),
            CHARS_PER_MSG,
            elapsed.as_millis(),
            total_input_chars as f64 / 1024.0,
            total_output_chars as f64 / 1024.0,
        );

        // 200 KB of message content: assembly must complete in <50 ms even in
        // debug builds on slow CI hardware.  The actual time on a modern laptop
        // in release mode is <1 ms, confirming the Knuth analysis: LLM network
        // latency (seconds) dominates; clone overhead is negligible.
        assert!(
            elapsed.as_millis() < 50,
            "assembly took {}ms — unexpectedly slow, investigate allocations",
            elapsed.as_millis()
        );
    }

    /// Simulates the cache-key hit path: message list unchanged, second call
    /// should be identical in cost to the first (no re-assembly in the real
    /// path because `build_turn_api_messages` short-circuits on cache hit).
    #[test]
    fn hotpath_cache_miss_vs_assembly() {
        const TURNS: usize = 200;
        const CHARS_PER_MSG: usize = 500;

        let messages = make_conversation(TURNS, CHARS_PER_MSG);

        let t0 = Instant::now();
        let result1 = assemble_api_messages_from_ctx(
            &messages,
            "",
            None,
            "gpt-4o",
            "ephemeral",
            false,
            false,
            false,
        );
        let first_call_ms = t0.elapsed().as_millis();

        let t1 = Instant::now();
        let result2 = assemble_api_messages_from_ctx(
            &messages,
            "",
            None,
            "gpt-4o",
            "ephemeral",
            false,
            false,
            false,
        );
        let second_call_ms = t1.elapsed().as_millis();

        assert_eq!(result1.len(), result2.len());
        println!(
            "[cache_miss_vs_assembly] first={}ms second={}ms",
            first_call_ms, second_call_ms
        );
    }

    #[test]
    fn dhat_guard_is_zero_sized_without_feature() {
        #[cfg(not(feature = "dhat-heap"))]
        assert_eq!(std::mem::size_of::<DhatGuard>(), 0);
    }

    #[test]
    fn dhat_guard_default_works() {
        let _g = DhatGuard::default();
    }
}
