//! Dimension 16 · 龙虎榜.

use async_trait::async_trait;

use super::super::r#trait::DimFetcher;
use super::super::types::{DimQuality, DimResult, FetcherSpec, Market};
use crate::providers::akshare::fetch_lhb_dim_akshare;
use crate::research::fetchers::context::FetchContext;
use crate::research::fetchers::dim_keys;
use crate::settlement::is_a_share;

pub struct LhbFetcher;

impl LhbFetcher {
    pub const SPEC: FetcherSpec = FetcherSpec {
        dim_key: dim_keys::LHB,
        depends_on: &[],
        markets: &[Market::A],
        sources: &["akshare", "eastmoney_lhb"],
        web_only: false,
    };
}

#[async_trait]
impl DimFetcher for LhbFetcher {
    fn spec(&self) -> &FetcherSpec {
        &Self::SPEC
    }

    async fn fetch(&self, ctx: &FetchContext) -> DimResult {
        if !is_a_share(&ctx.symbol) {
            return DimResult::skipped(dim_keys::LHB, &ctx.symbol, "龙虎榜仅 A 股");
        }
        match fetch_lhb_dim_akshare(&ctx.symbol).await {
            Ok((data, source)) => {
                let count = data
                    .get("lhb_count_30d")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let has_error = data.get("lhb_error").is_some();
                DimResult::ok(
                    dim_keys::LHB,
                    &ctx.symbol,
                    data,
                    source,
                    if count > 0 && !has_error {
                        DimQuality::Partial
                    } else {
                        DimQuality::Missing
                    },
                )
            }
            Err(e) => DimResult::error(dim_keys::LHB, &ctx.symbol, "akshare", e.to_string()),
        }
    }
}
