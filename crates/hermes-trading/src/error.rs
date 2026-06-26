//! Error types for the hermes-trading crate.

use thiserror::Error;

/// Errors that can occur in the trading library.
#[derive(Debug, Error)]
pub enum TradingError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Data source returned invalid response: {0}")]
    InvalidResponse(String),

    #[error("Symbol not found or not supported: {0}")]
    SymbolNotFound(String),

    #[error("No data available for the requested period")]
    NoData,

    #[error("Backtest error: {0}")]
    Backtest(String),

    #[error("Unsupported strategy: {0}")]
    UnsupportedStrategy(String),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
