//! Vibe Research: 0py market data and backtesting library for Hermes Agent.
//!
//! This crate provides:
//! - `MarketDataProvider` trait and implementations for fetching OHLCV data
//! - `BacktestEngine` for running template-based backtests
//!
//! **0py constraint**: No Python runtime, PyO3, or Python subprocess dependencies.

pub mod backtest;
pub mod error;
pub mod indicators;
pub mod provider;
pub mod providers;
pub mod types;

pub use backtest::{BacktestEngine, Period, RunCard};
pub use error::VibeError;
pub use indicators::sma;
pub use provider::MarketDataProvider;
pub use providers::{AutoRouter, BinanceProvider, EastmoneyProvider};
pub use types::{Interval, OhlcvData, OhlcvRequest, OhlcvRow};
