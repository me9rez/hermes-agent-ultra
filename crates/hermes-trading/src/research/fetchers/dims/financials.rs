//! Dimension 1 · financials (三表摘要).

use async_trait::async_trait;

use super::super::r#trait::DimFetcher;
use super::super::types::{DimQuality, DimResult, FetcherSpec, Market};
use crate::providers::akshare::fetch_financials_dim_akshare;
use crate::research::fetchers::context::FetchContext;
use crate::research::fetchers::dim_keys;

pub struct FinancialsFetcher;

impl FinancialsFetcher {
    pub const SPEC: FetcherSpec = FetcherSpec {
        dim_key: dim_keys::FINANCIALS,
        depends_on: &[],
        markets: &[Market::A, Market::H, Market::U],
        sources: &["akshare", "eastmoney_f10", "yahoo"],
        web_only: false,
    };
}

#[async_trait]
impl DimFetcher for FinancialsFetcher {
    fn spec(&self) -> &FetcherSpec {
        &Self::SPEC
    }

    async fn fetch(&self, ctx: &FetchContext) -> DimResult {
        let ticker = &ctx.symbol;
        match fetch_financials_dim_akshare(ticker).await {
            Ok((data, source)) => {
                let quality = if data.get("roe").and_then(|v| v.as_f64()).is_some()
                    && data.get("net_margin").and_then(|v| v.as_f64()).is_some()
                {
                    DimQuality::Full
                } else if data
                    .get("revenue_latest_yi")
                    .and_then(|v| v.as_f64())
                    .is_some()
                {
                    DimQuality::Partial
                } else {
                    DimQuality::Missing
                };
                DimResult::ok(dim_keys::FINANCIALS, ticker, data, source, quality)
            }
            Err(e) => DimResult::error(dim_keys::FINANCIALS, ticker, "akshare", e.to_string()),
        }
    }
}
