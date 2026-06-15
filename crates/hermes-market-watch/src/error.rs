//! Error types for the hermes-market-watch crate.

use thiserror::Error;

/// Errors that can occur in the market watch engine.
#[derive(Debug, Error)]
pub enum WatchError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Symbol already in watchlist: {0}")]
    DuplicateSymbol(String),

    #[error("Symbol not in watchlist: {0}")]
    SymbolNotWatched(String),

    #[error("Quote unavailable: {0}")]
    QuoteUnavailable(String),

    #[error("Invalid alert condition: {0}")]
    InvalidCondition(String),

    #[error("Vibe error: {0}")]
    Vibe(#[from] hermes_vibe::VibeError),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}
