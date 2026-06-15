//! Strategy trait and signal types.

use serde::{Deserialize, Serialize};

use hermes_vibe::types::OhlcvData;

use crate::error::StrategyError;

/// Trading signal direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Signal {
    /// Bullish / buy signal.
    Buy,
    /// Bearish / sell signal.
    Sell,
    /// No action.
    Hold,
}

/// Position side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    Long,
    Short,
    Flat,
}

/// Strategy decision for a single bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub signal: Signal,
    pub position: Position,
    /// Confidence in [0.0, 1.0].
    pub confidence: f64,
    /// Human-readable rationale.
    pub reason: String,
}

/// Trait for trading strategies.
///
/// A strategy consumes OHLCV data and produces per-bar [`Decision`]s.
pub trait Strategy: Send + Sync {
    /// Run the strategy over the provided data, returning one decision per
    /// bar (aligned to `data.rows`). Early bars may have `Signal::Hold` when
    /// indicators have not yet warmed up.
    fn run(&self, data: &OhlcvData) -> Result<Vec<Decision>, StrategyError>;

    /// Strategy name (for logging / diagnostics).
    fn name(&self) -> &str;
}
