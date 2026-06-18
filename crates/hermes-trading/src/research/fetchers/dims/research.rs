//! Dimension 6 · broker research reports.

use async_trait::async_trait;

use super::super::r#trait::DimFetcher;
use super::super::types::{DimQuality, DimResult, FetcherSpec, Market};
use crate::providers::akshare::fetch_research_dim_akshare;
use crate::research::fetchers::context::FetchContext;
use crate::research::fetchers::dim_keys;
use crate::settlement::is_a_share;

pub struct ResearchFetcher;

impl ResearchFetcher {
    pub const SPEC: FetcherSpec = FetcherSpec {
        dim_key: dim_keys::RESEARCH,
        depends_on: &[],
        markets: &[Market::A, Market::H, Market::U],
        sources: &["akshare", "web_search"],
        web_only: false,
    };
}

#[async_trait]
impl DimFetcher for ResearchFetcher {
    fn spec(&self) -> &FetcherSpec {
        &Self::SPEC
    }

    async fn fetch(&self, ctx: &FetchContext) -> DimResult {
        if !is_a_share(&ctx.symbol) {
            return DimResult::skipped(dim_keys::RESEARCH, &ctx.symbol, "非 A 股研报用 web_search");
        }
        match fetch_research_dim_akshare(&ctx.symbol).await {
            Ok(data) => {
                let count = data
                    .get("research_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                DimResult::ok(
                    dim_keys::RESEARCH,
                    &ctx.symbol,
                    data,
                    "akshare",
                    if count > 0 {
                        DimQuality::Partial
                    } else {
                        DimQuality::Missing
                    },
                )
            }
            Err(e) => DimResult::error(dim_keys::RESEARCH, &ctx.symbol, "akshare", e.to_string()),
        }
    }
}
