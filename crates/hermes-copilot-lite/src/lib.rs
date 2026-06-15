//! Copilot Lite: lightweight orchestration for Vibe Research workflows.
//!
//! This crate ties together `hermes-vibe`, `hermes-strategies`, and
//! `hermes-market-watch` into a simple copilot interface for:
//! - Running strategies against live or historical data
//! - Monitoring watchlists and generating alerts
//! - Producing human-readable analysis reports
//!
//! **0py constraint**: No Python runtime, PyO3, or Python subprocess dependencies.

pub mod error;
pub mod orchestrator;
pub mod report;

pub use error::CopilotError;
pub use orchestrator::CopilotLite;
pub use report::AnalysisReport;
