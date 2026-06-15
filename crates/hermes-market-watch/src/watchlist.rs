//! Watchlist management.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::WatchError;

/// A managed list of symbols to monitor.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Watchlist {
    symbols: BTreeSet<String>,
}

impl Watchlist {
    /// Create an empty watchlist.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a symbol to the watchlist.
    pub fn add(&mut self, symbol: impl Into<String>) -> Result<(), WatchError> {
        let sym = symbol.into();
        if !self.symbols.insert(sym.clone()) {
            return Err(WatchError::DuplicateSymbol(sym));
        }
        Ok(())
    }

    /// Remove a symbol from the watchlist.
    pub fn remove(&mut self, symbol: &str) -> Result<(), WatchError> {
        if !self.symbols.remove(symbol) {
            return Err(WatchError::SymbolNotWatched(symbol.to_string()));
        }
        Ok(())
    }

    /// Check whether a symbol is in the watchlist.
    pub fn contains(&self, symbol: &str) -> bool {
        self.symbols.contains(symbol)
    }

    /// Return an iterator over watched symbols.
    pub fn symbols(&self) -> impl Iterator<Item = &str> {
        self.symbols.iter().map(String::as_str)
    }

    /// Number of symbols in the watchlist.
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Whether the watchlist is empty.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
}
