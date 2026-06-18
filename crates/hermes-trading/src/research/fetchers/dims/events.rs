//! Dimension 15 · announcements / news events.

use async_trait::async_trait;

use super::super::r#trait::DimFetcher;
use super::super::types::{DimQuality, DimResult, FetcherSpec, Market};
use crate::providers::akshare::fetch_events_dim_akshare;
use crate::research::fetchers::context::FetchContext;
use crate::research::fetchers::dim_keys;
use crate::settlement::is_a_share;

pub struct EventsFetcher;

impl EventsFetcher {
    pub const SPEC: FetcherSpec = FetcherSpec {
        dim_key: dim_keys::EVENTS,
        depends_on: &[],
        markets: &[Market::A, Market::H, Market::U],
        sources: &["akshare", "web_search"],
        web_only: false,
    };
}

#[async_trait]
impl DimFetcher for EventsFetcher {
    fn spec(&self) -> &FetcherSpec {
        &Self::SPEC
    }

    async fn fetch(&self, ctx: &FetchContext) -> DimResult {
        if !is_a_share(&ctx.symbol) {
            return DimResult::skipped(
                dim_keys::EVENTS,
                &ctx.symbol,
                "非 A 股事件/新闻用 web_search",
            );
        }
        match fetch_events_dim_akshare(&ctx.symbol).await {
            Ok(data) => {
                let has_data = data
                    .get("announcement_count")
                    .or_else(|| data.get("news_count"))
                    .and_then(|v| v.as_u64())
                    .is_some_and(|n| n > 0);
                DimResult::ok(
                    dim_keys::EVENTS,
                    &ctx.symbol,
                    data,
                    "akshare",
                    if has_data {
                        DimQuality::Partial
                    } else {
                        DimQuality::Missing
                    },
                )
            }
            Err(e) => DimResult::error(dim_keys::EVENTS, &ctx.symbol, "akshare", e.to_string()),
        }
    }
}
