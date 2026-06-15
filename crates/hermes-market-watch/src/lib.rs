//! Market watch: real-time quotes, watchlist, and alert engine for Vibe Research.
//!
//! This crate provides:
//! - `Watchlist` for managing a set of watched symbols
//! - `QuoteProvider` trait for fetching real-time / delayed quotes
//! - `AlertEngine` for evaluating price/volume alerts
//!
//! **0py constraint**: No Python runtime, PyO3, or Python subprocess dependencies.

pub mod alert;
pub mod error;
pub mod quote;
pub mod watchlist;

pub use alert::{Alert, AlertCondition, AlertEngine, AlertTrigger};
pub use error::WatchError;
pub use quote::{Quote, QuoteProvider};
pub use watchlist::Watchlist;
