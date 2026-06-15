//! Error types for the hermes-strategies crate.

use thiserror::Error;

/// Errors that can occur in the strategy engine.
#[derive(Debug, Error)]
pub enum StrategyError {
    #[error("Insufficient data: need at least {needed} rows, got {got}")]
    InsufficientData { needed: usize, got: usize },

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("Strategy execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Vibe error: {0}")]
    Vibe(#[from] hermes_vibe::VibeError),
}
