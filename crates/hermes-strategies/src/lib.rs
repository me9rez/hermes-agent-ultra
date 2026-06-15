//! Strategy engine and technical indicators for Vibe Research.
//!
//! This crate provides:
//! - `Strategy` trait for implementing trading strategies
//! - `Indicator` trait and built-in indicators (SMA, EMA, RSI, MACD, Bollinger)
//! - `Signal` and `Position` types for strategy output
//!
//! **0py constraint**: No Python runtime, PyO3, or Python subprocess dependencies.

pub mod error;
pub mod indicators;
pub mod strategy;

pub use error::StrategyError;
pub use indicators::Indicator;
pub use strategy::{Decision, Position, Signal, Strategy};
