//! Shared fetch context passed between dimension fetchers.

use std::collections::BTreeMap;

use super::types::{DimResult, Market};
use crate::quote_data::QuoteData;
use crate::research::fetchers::dim_keys;

/// Context for a single symbol collection run (prior dims for `depends_on`).
#[derive(Debug, Clone)]
pub struct FetchContext {
    pub symbol: String,
    pub market: Market,
    pub prior: BTreeMap<String, DimResult>,
    /// Quote already fetched by caller (e.g. `analyze_stock`) — basic dim reuses it.
    pub cached_quote: Option<QuoteData>,
    /// Cached `0_basic` payload after basic dim completes (valuation/peers reuse).
    pub cached_basic: Option<serde_json::Value>,
}

impl FetchContext {
    #[must_use]
    pub fn new(symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        let market = Market::from_symbol(&symbol);
        Self {
            symbol,
            market,
            prior: BTreeMap::new(),
            cached_quote: None,
            cached_basic: None,
        }
    }

    #[must_use]
    pub fn with_cached_basic(mut self, data: serde_json::Value) -> Self {
        self.cached_basic = Some(data);
        self
    }

    /// Basic dim data: explicit cache or completed `0_basic` in `prior`.
    #[must_use]
    pub fn cached_basic_data(&self) -> Option<&serde_json::Value> {
        self.cached_basic
            .as_ref()
            .or_else(|| self.prior_data(dim_keys::BASIC))
    }

    #[must_use]
    pub fn with_cached_quote(mut self, quote: QuoteData) -> Self {
        self.cached_quote = Some(quote);
        self
    }

    #[must_use]
    pub fn prior_data(&self, dim_key: &str) -> Option<&serde_json::Value> {
        self.prior.get(dim_key).map(|r| &r.data)
    }

    #[must_use]
    pub fn prior_industry(&self) -> Option<String> {
        self.prior_data("0_basic")
            .and_then(|d| d.get("industry"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    /// Prior `0_basic` dim payload and source tag.
    #[must_use]
    pub fn prior_basic(&self) -> Option<(&serde_json::Value, &str)> {
        self.prior
            .get("0_basic")
            .map(|r| (&r.data, r.source.as_str()))
    }
}
