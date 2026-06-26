//! Dimension 0 · basic quote / identity.

use async_trait::async_trait;
use serde_json::json;
use tracing::{debug, warn};

use super::super::r#trait::DimFetcher;
use super::super::types::{DimQuality, DimResult, FetcherSpec, Market};
use crate::providers::EastmoneyBasicProvider;
use crate::providers::FundamentalsProvider;
use crate::providers::QuoteRouter;
use crate::providers::QuoteSource;
use crate::providers::akshare::{
    apply_supplement, fetch_a_share_quote_chain, fetch_basic_info_supplement,
};
use crate::quote_data::QuoteData;
use crate::research::fetchers::context::FetchContext;
use crate::research::fetchers::dim_keys;
use crate::research::types::FundamentalsSnapshot;
use crate::settlement::is_a_share;

pub struct BasicFetcher {
    basic: EastmoneyBasicProvider,
    quotes: QuoteRouter,
}

impl BasicFetcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            basic: EastmoneyBasicProvider::new(),
            quotes: QuoteRouter::new(),
        }
    }

    pub const SPEC: FetcherSpec = FetcherSpec {
        dim_key: dim_keys::BASIC,
        depends_on: &[],
        markets: &[Market::A, Market::H, Market::U],
        sources: &["akshare", "eastmoney_push2", "tencent_qt", "yahoo"],
        web_only: false,
    };

    /// QuoteData has no market_cap / shares — always merge push2 or individual_info for Full.
    fn needs_push2_merge(_q: &QuoteData) -> bool {
        true
    }

    fn snap_needs_supplement(snap: &FundamentalsSnapshot) -> bool {
        snap.market_cap_yi.is_none()
            || snap.shares_outstanding_yi.is_none()
            || snap.industry.is_none()
    }

    async fn supplement_snap_if_needed(
        ticker: &str,
        snap: &mut FundamentalsSnapshot,
        source: &mut String,
    ) {
        if !Self::snap_needs_supplement(snap) && snap.name.is_some() {
            return;
        }
        match fetch_basic_info_supplement(ticker).await {
            Ok(sup) => {
                apply_supplement(snap, &sup);
                if sup.market_cap_yi.is_some() || sup.industry.is_some() || sup.name.is_some() {
                    if source.contains("akshare") {
                        *source = format!("{source}+akshare_info");
                    } else if source.is_empty() {
                        *source = "akshare_info".into();
                    } else {
                        *source = format!("{source}+akshare_info");
                    }
                }
            }
            Err(e) => {
                warn!(symbol = %ticker, error = %e, "basic individual_info supplement failed");
            }
        }
        Self::fill_name_from_tencent(ticker, snap, source).await;
    }

    async fn fill_name_from_tencent(
        ticker: &str,
        snap: &mut FundamentalsSnapshot,
        source: &mut String,
    ) {
        if snap.name.is_some() {
            return;
        }
        let http = crate::http::default_client();
        match crate::providers::eastmoney_http::fetch_tencent_qt(&http, ticker).await {
            Ok(qt) if qt.name.is_some() => {
                snap.name.clone_from(&qt.name);
                if !source.contains("tencent_qt") {
                    *source = format!("{source}+tencent_qt");
                }
            }
            Ok(_) => {}
            Err(e) => {
                warn!(symbol = %ticker, error = %e, "tencent qt name fallback failed");
            }
        }
    }

    fn snap_has_core(snap: &FundamentalsSnapshot) -> bool {
        snap.name.is_some() && snap.price.is_some()
    }

    fn dim_from_snap(snap: &FundamentalsSnapshot, source: &str) -> DimResult {
        let ticker = snap.symbol.clone();
        let data = json!({
            "name": snap.name,
            "price": snap.price,
            "pe_ttm": snap.pe,
            "pb": snap.pb,
            "market_cap_yi": snap.market_cap_yi,
            "shares_outstanding_yi": snap.shares_outstanding_yi,
            "change_pct": snap.change_pct,
            "industry": snap.industry,
        });
        DimResult::ok(
            dim_keys::BASIC,
            &ticker,
            data,
            source,
            if snap.market_cap_yi.is_some() {
                DimQuality::Full
            } else {
                DimQuality::Partial
            },
        )
    }

    fn dim_from_quote(ticker: &str, q: &QuoteData) -> DimResult {
        DimResult::ok(
            dim_keys::BASIC,
            ticker,
            json!({
                "name": q.short_name,
                "price": q.price,
                "pe_ttm": q.pe_ratio,
                "change_pct": q.change_pct,
            }),
            q.source.as_str(),
            DimQuality::Partial,
        )
    }

    async fn resolve_quote(ctx: &FetchContext) -> Option<QuoteData> {
        if let Some(q) = ctx.cached_quote.clone() {
            debug!(symbol = %ctx.symbol, "basic dim reusing cached quote");
            return Some(q);
        }
        fetch_a_share_quote_chain(&ctx.symbol).await.ok()
    }

    async fn fetch_a_share(&self, ctx: &FetchContext) -> DimResult {
        let ticker = &ctx.symbol;

        if let Some(q) = Self::resolve_quote(ctx).await
            && q.price.is_some()
        {
            if Self::needs_push2_merge(&q) {
                match self.basic.fetch(ticker).await {
                    Ok(mut snap) => {
                        Self::merge_snap_fields(&mut snap, &q);
                        let mut source = if q.source == "akshare" {
                            "akshare+eastmoney_push2".into()
                        } else if q.source == "tencent_qt" {
                            "eastmoney_push2+tencent_qt".into()
                        } else {
                            q.source.clone()
                        };
                        Self::supplement_snap_if_needed(ticker, &mut snap, &mut source).await;
                        return Self::dim_from_snap(&snap, &source);
                    }
                    Err(e) => {
                        warn!(symbol = %ticker, error = %e, "eastmoney basic merge skipped");
                    }
                }
            }
            let mut snap = FundamentalsSnapshot {
                symbol: ticker.to_string(),
                name: q.short_name.clone(),
                price: q.price,
                pe: q.pe_ratio,
                change_pct: q.change_pct,
                ..Default::default()
            };
            let mut source = q.source.clone();
            Self::supplement_snap_if_needed(ticker, &mut snap, &mut source).await;
            return Self::dim_from_snap(&snap, &source);
        }

        self.fetch_a_share_fallback(ticker).await
    }

    async fn fetch_a_share_fallback(&self, ticker: &str) -> DimResult {
        match self.basic.fetch(ticker).await {
            Ok(mut snap) if Self::snap_has_core(&snap) => {
                let mut source = "eastmoney_push2".to_string();
                Self::supplement_snap_if_needed(ticker, &mut snap, &mut source).await;
                return Self::dim_from_snap(&snap, &source);
            }
            Ok(snap) => {
                warn!(
                    symbol = %ticker,
                    "basic dim partial from eastmoney, trying quote router"
                );
                if let Ok(q) = self
                    .quotes
                    .fetch_quote_with_source(ticker, QuoteSource::Auto, false)
                    .await
                {
                    return Self::merge_snap_and_quote(ticker, snap, &q).await;
                }
            }
            Err(e) => {
                warn!(
                    symbol = %ticker,
                    error = %e,
                    "eastmoney basic failed, trying quote router"
                );
            }
        }

        match self
            .quotes
            .fetch_quote_with_source(ticker, QuoteSource::Auto, false)
            .await
        {
            Ok(q) => {
                let mut snap = FundamentalsSnapshot {
                    symbol: ticker.to_string(),
                    name: q.short_name.clone(),
                    price: q.price,
                    pe: q.pe_ratio,
                    change_pct: q.change_pct,
                    ..Default::default()
                };
                let mut source = q.source.clone();
                Self::supplement_snap_if_needed(ticker, &mut snap, &mut source).await;
                Self::dim_from_snap(&snap, &source)
            }
            Err(e) => DimResult::error(dim_keys::BASIC, ticker, "quote_router", e.to_string()),
        }
    }

    fn merge_snap_fields(snap: &mut FundamentalsSnapshot, q: &QuoteData) {
        if snap.name.is_none() {
            snap.name.clone_from(&q.short_name);
        }
        if snap.price.is_none() {
            snap.price = q.price;
        }
        if snap.pe.is_none() {
            snap.pe = q.pe_ratio;
        }
        if snap.change_pct.is_none() {
            snap.change_pct = q.change_pct;
        }
    }

    async fn merge_snap_and_quote(
        ticker: &str,
        mut snap: FundamentalsSnapshot,
        q: &QuoteData,
    ) -> DimResult {
        Self::merge_snap_fields(&mut snap, q);
        let mut source = if q.source == "akshare" {
            if snap.market_cap_yi.is_some() {
                "akshare+eastmoney_push2".into()
            } else {
                "akshare".into()
            }
        } else if q.source == "tencent_qt" {
            "eastmoney_push2+tencent_qt".into()
        } else {
            q.source.clone()
        };
        Self::supplement_snap_if_needed(ticker, &mut snap, &mut source).await;
        Self::dim_from_snap(&snap, &source)
    }
}

impl Default for BasicFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DimFetcher for BasicFetcher {
    fn spec(&self) -> &FetcherSpec {
        &Self::SPEC
    }

    async fn fetch(&self, ctx: &FetchContext) -> DimResult {
        let ticker = &ctx.symbol;
        if is_a_share(ticker) {
            return self.fetch_a_share(ctx).await;
        }

        match self
            .quotes
            .fetch_quote_with_source(ticker, QuoteSource::Auto, false)
            .await
        {
            Ok(q) => Self::dim_from_quote(ticker, &q),
            Err(e) => DimResult::error(dim_keys::BASIC, ticker, "quote_router", e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_quote(source: &str, name: Option<&str>, pe: Option<f64>) -> QuoteData {
        QuoteData {
            symbol: "002714.SZ".into(),
            market_date: None,
            as_of: None,
            price: Some(49.0),
            change: None,
            change_pct: Some(-0.5),
            volume: None,
            currency: Some("CNY".into()),
            exchange: None,
            short_name: name.map(str::to_string),
            pe_ratio: pe,
            high_52w: None,
            low_52w: None,
            source: source.into(),
            partial: false,
        }
    }

    #[test]
    fn needs_push2_merge_always_for_capital_fields() {
        assert!(BasicFetcher::needs_push2_merge(&sample_quote(
            "akshare",
            Some("牧原股份"),
            Some(12.0)
        )));
    }

    #[tokio::test]
    async fn merge_snap_and_quote_fills_gaps() {
        let snap = FundamentalsSnapshot {
            symbol: "600519.SH".into(),
            market_cap_yi: Some(1500.0),
            ..Default::default()
        };
        let q = QuoteData {
            symbol: "600519.SH".into(),
            market_date: None,
            as_of: None,
            price: Some(1407.0),
            change: None,
            change_pct: Some(0.1),
            volume: None,
            currency: None,
            exchange: None,
            short_name: Some("贵州茅台".into()),
            pe_ratio: Some(18.0),
            high_52w: None,
            low_52w: None,
            source: "akshare".into(),
            partial: false,
        };
        let dim = BasicFetcher::merge_snap_and_quote("600519.SH", snap, &q).await;
        assert!(dim.error.is_none());
        assert!(dim.source.contains("akshare"));
        assert!(dim.source.contains("eastmoney_push2"));
        assert_eq!(dim.data.get("price").and_then(|v| v.as_f64()), Some(1407.0));
        assert_eq!(
            dim.data.get("name").and_then(|v| v.as_str()),
            Some("贵州茅台")
        );
    }
}
