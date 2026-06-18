//! A-share quote chain: akshare → eastmoney_http.

use akshare::QuoteSnapshot;

use crate::error::TradingError;
use crate::http::default_client;
use crate::providers::eastmoney_http::{self};
use crate::providers::eastmoney_quote::EastmoneyQuoteProvider;
use crate::quote_data::QuoteData;
use crate::symbol::normalize_symbol;

use super::{client, code6, map_err, try_or_fallback};

/// Shared quote chain for `get_quote` and basic dim.
pub async fn fetch_a_share_quote_chain(symbol: &str) -> Result<QuoteData, TradingError> {
    let canonical = normalize_symbol(symbol);
    try_or_fallback(
        async {
            let code = code6(&canonical)?;
            let snap = client().a_share_quote(&code).await.map_err(map_err)?;
            Ok(quote_snapshot_to_data(&canonical, snap, "akshare"))
        },
        async {
            let http = default_client();
            let em = eastmoney_http::fetch_a_share_snapshot(&http, &canonical).await?;
            Ok(EastmoneyQuoteProvider::snapshot_to_quote(em))
        },
    )
    .await
}

fn quote_snapshot_to_data(symbol: &str, snap: QuoteSnapshot, source: &str) -> QuoteData {
    let mut out = QuoteData::new(symbol.to_string(), source);
    out.price = Some(snap.close);
    out.change_pct = None;
    out.volume = Some(snap.volume as f64);
    out.currency = Some("CNY".into());
    out.finalize_partial();
    out
}
