//! Strategy engine and technical indicators for Vibe Research.
//!
//! This crate provides:
//! - `Strategy` trait for implementing trading strategies
//! - `Indicator` trait and built-in indicators (SMA, EMA, RSI, MACD, Bollinger)
//! - `Signal` and `Position` types for strategy output
//! - `DeclarativeStrategy` for JSON-defined strategies
//! - `StrategyRegistry` for runtime strategy management
//!
//! **0py constraint**: No Python runtime, PyO3, or Python subprocess dependencies.

pub mod builtin;
pub mod declarative;
pub mod dsl;
pub mod error;
pub mod indicators;
pub mod registry;
pub mod strategy;

pub use builtin::{all_builtin_defs, rsi_revert_def, sma_cross_def};
pub use dsl::{DeclarativeStrategyDef, IndicatorDef, RuleExpr, RuleOperand, RulesDef};
pub use error::StrategyError;
pub use indicators::Indicator;
pub use registry::{StrategyInfo, StrategyRegistry};
pub use strategy::{Decision, Position, Signal, Strategy};

// Re-export DeclarativeStrategy from declarative module
pub use declarative::DeclarativeStrategy;
