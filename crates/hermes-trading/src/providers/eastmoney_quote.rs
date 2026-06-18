//! Eastmoney realtime quote API for A-shares (via shared `eastmoney_http`).

use async_trait::async_trait;

use crate::error::TradingError;
use crate::providers::eastmoney_http::AshareSnapshot;
use crate::quote_data::QuoteData;
use crate::quote_provider::QuoteProvider;
use crate::settlement::is_a_share;
use crate::symbol::normalize_symbol;

/// Realtime A-share quote via akshare primary + Eastmoney push2 / Tencent qt fallback.
#[derive(Debug, Clone, Default)]
pub struct EastmoneyQuoteProvider;

impl EastmoneyQuoteProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn snapshot_to_quote(snap: AshareSnapshot) -> QuoteData {
        let mut out = QuoteData::new(snap.symbol, snap.source);
        out.short_name = snap.name;
        out.price = snap.price;
        out.change = snap.change;
        out.change_pct = snap.change_pct;
        out.volume = snap.volume;
        out.pe_ratio = snap.pe;
        out.currency = Some("CNY".to_string());
        out.finalize_partial();
        out
    }
}

#[async_trait]
impl QuoteProvider for EastmoneyQuoteProvider {
    async fn fetch_quote(&self, symbol: &str) -> Result<QuoteData, TradingError> {
        let canonical = normalize_symbol(symbol);
        if !is_a_share(&canonical) {
            return Err(TradingError::SymbolNotFound(format!(
                "Eastmoney quote only supports A-shares: {symbol}"
            )));
        }
        crate::providers::akshare::fetch_a_share_quote_chain(&canonical).await
    }

    fn name(&self) -> &str {
        "eastmoney"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::eastmoney_http;

    #[test]
    fn snapshot_to_quote_maps_fields() {
        let snap = eastmoney_http::AshareSnapshot {
            symbol: "600519.SH".into(),
            source: "eastmoney".into(),
            name: Some("贵州茅台".into()),
            price: Some(1407.04),
            change: Some(0.04),
            change_pct: Some(0.01),
            volume: Some(1000.0),
            pe: Some(18.0),
            pb: None,
            market_cap_yi: None,
            circulating_cap_yi: None,
            shares_outstanding_yi: None,
        };
        let q = EastmoneyQuoteProvider::snapshot_to_quote(snap);
        assert_eq!(q.price, Some(1407.04));
        assert_eq!(q.short_name.as_deref(), Some("贵州茅台"));
    }
}
