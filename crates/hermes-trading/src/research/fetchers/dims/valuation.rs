//! Dimension 10 · valuation.

use async_trait::async_trait;
use serde_json::json;
use tracing::warn;

use super::super::r#trait::DimFetcher;
use super::super::types::{DimQuality, DimResult, FetcherSpec, Market};
use crate::providers::EastmoneyBasicProvider;
use crate::providers::FundamentalsProvider;
use crate::providers::akshare::fetch_valuation_percentiles;
use crate::research::fetchers::context::FetchContext;
use crate::research::fetchers::dim_keys;
use crate::settlement::is_a_share;

pub struct ValuationFetcher {
    basic: EastmoneyBasicProvider,
}

impl ValuationFetcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            basic: EastmoneyBasicProvider::new(),
        }
    }

    pub const SPEC: FetcherSpec = FetcherSpec {
        dim_key: dim_keys::VALUATION,
        depends_on: &[dim_keys::BASIC],
        markets: &[Market::A, Market::H, Market::U],
        sources: &["0_basic", "akshare", "eastmoney_push2"],
        web_only: false,
    };

    fn f64_from_basic(basic: &serde_json::Value, key: &str) -> Option<f64> {
        basic.get(key).and_then(|v| v.as_f64())
    }
}

impl Default for ValuationFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DimFetcher for ValuationFetcher {
    fn spec(&self) -> &FetcherSpec {
        &Self::SPEC
    }

    async fn fetch(&self, ctx: &FetchContext) -> DimResult {
        let ticker = &ctx.symbol;
        if !is_a_share(ticker) {
            return DimResult::skipped(
                dim_keys::VALUATION,
                ticker,
                "港美股估值分位需 web_search / yahoo",
            );
        }

        let (mut pe, mut pb, mut source) = if let Some(basic) = ctx.cached_basic_data() {
            (
                Self::f64_from_basic(basic, "pe_ttm"),
                Self::f64_from_basic(basic, "pb"),
                ctx.prior_basic()
                    .map(|(_, s)| s.to_string())
                    .unwrap_or_else(|| "0_basic".into()),
            )
        } else if let Some((basic, src)) = ctx.prior_basic() {
            (
                Self::f64_from_basic(basic, "pe_ttm"),
                Self::f64_from_basic(basic, "pb"),
                src.to_string(),
            )
        } else {
            (None, None, "0_basic".into())
        };

        if (pe.is_none() || pb.is_none())
            && ctx.cached_basic_data().is_none()
            && let Ok(snap) = self.basic.fetch(ticker).await
        {
            pe = pe.or(snap.pe);
            pb = pb.or(snap.pb);
            if source == "0_basic" {
                source = "eastmoney_push2".into();
            }
        }

        let mut pe_percentile = None;
        let mut pb_percentile = None;
        match fetch_valuation_percentiles(ticker, pe, pb).await {
            Ok(pct) => {
                pe_percentile = pct.get("pe_percentile").and_then(|v| v.as_f64());
                pb_percentile = pct.get("pb_percentile").and_then(|v| v.as_f64());
                if source == "0_basic" || source == "eastmoney_push2" {
                    source = format!("{source}+akshare");
                } else {
                    source = "akshare".into();
                }
            }
            Err(e) => {
                warn!(symbol = %ticker, error = %e, "valuation percentile fetch failed");
            }
        }

        let quality = if pe.is_some() && pe_percentile.is_some() {
            DimQuality::Full
        } else if pe.is_some() {
            DimQuality::Partial
        } else {
            DimQuality::Missing
        };

        DimResult::ok(
            dim_keys::VALUATION,
            ticker,
            json!({
                "pe_ttm": pe,
                "pb": pb,
                "pe_percentile": pe_percentile,
                "pb_percentile": pb_percentile,
            }),
            &source,
            quality,
        )
    }
}
