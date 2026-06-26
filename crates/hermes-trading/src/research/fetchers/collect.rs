//! Dimension collection orchestrator (mirrors UZI `pipeline/collect.py`).

use futures::future::join_all;
use serde_json::json;
use tracing::info;

use super::bridge::apply_dims_to_snapshot;
use super::context::FetchContext;
use super::registry::build_registry;
use super::schedule::{exec_layers, fetcher_map};
use super::types::CollectOutput;
use crate::quote_data::QuoteData;
use crate::research::confidence_supplement::supplement_snapshot_confidence;
use crate::research::fetchers::dim_keys;
use crate::research::profile::AnalysisProfile;
use crate::research::types::FundamentalsSnapshot;

/// Options for dimension collection.
#[derive(Debug, Clone)]
pub struct CollectOptions {
    /// When true, run web-only fetchers (they return `Skipped` stubs).
    pub include_web_dims: bool,
    /// Analysis depth label for logs / metadata (`lite` / `medium`).
    pub depth: Option<String>,
    /// When false, fetch each dimension in a layer sequentially (benchmark / debug).
    pub parallel: bool,
}

impl Default for CollectOptions {
    fn default() -> Self {
        Self {
            include_web_dims: false,
            depth: None,
            parallel: true,
        }
    }
}

/// Result of `enrich_snapshot` (raw dims + collect metadata).
#[derive(Debug, Clone)]
pub struct EnrichSnapshotResult {
    pub raw_dims: serde_json::Value,
    pub collect: CollectOutput,
}

/// Collect registered HTTP dimensions for one symbol (parallel per dependency layer).
pub async fn collect_dims(
    symbol: &str,
    opts: &CollectOptions,
    cached_quote: Option<QuoteData>,
    profile: &AnalysisProfile,
) -> CollectOutput {
    let registry = build_registry();
    let fetchers = fetcher_map(&registry);
    let mut ctx = FetchContext::new(symbol);
    if let Some(q) = cached_quote {
        ctx = ctx.with_cached_quote(q.clone());
        ctx = ctx.with_cached_basic(json!({
            "name": q.short_name,
            "price": q.price,
            "pe_ttm": q.pe_ratio,
            "change_pct": q.change_pct,
        }));
    }
    let mut output = CollectOutput {
        ticker: ctx.symbol.clone(),
        market: ctx.market,
        dims: Default::default(),
    };

    let layers = exec_layers(&registry, profile, opts, ctx.market);
    for layer in layers {
        let layer_ctx = ctx.clone();
        let results = if opts.parallel {
            let futs: Vec<_> = layer
                .iter()
                .filter_map(|dim_key| {
                    let fetcher = fetchers.get(dim_key)?.clone();
                    let c = layer_ctx.clone();
                    Some(async move {
                        let result = fetcher.fetch(&c).await;
                        (dim_key.clone(), result)
                    })
                })
                .collect();
            join_all(futs).await
        } else {
            let mut out = Vec::new();
            for dim_key in &layer {
                let Some(fetcher) = fetchers.get(dim_key) else {
                    continue;
                };
                let result = fetcher.fetch(&layer_ctx).await;
                out.push((dim_key.clone(), result));
            }
            out
        };
        for (dim_key, result) in results {
            if dim_key == dim_keys::BASIC {
                ctx = ctx.with_cached_basic(result.data.clone());
            }
            ctx.prior.insert(result.dim_key.clone(), result.clone());
            output.dims.insert(result.dim_key.clone(), result);
        }
    }

    if !output.dims.is_empty() {
        let depth = opts.depth.as_deref().unwrap_or(profile.depth_label());
        info!(
            symbol = %output.ticker,
            depth = %depth,
            dim_summary = %output.summary_line(),
            "dimension collection complete"
        );
    }

    output
}

/// Collect HTTP dims, merge snapshot, return raw_dims + collect output.
pub async fn enrich_snapshot(
    snap: &mut FundamentalsSnapshot,
    symbol: &str,
    cached_quote: Option<QuoteData>,
    profile: &AnalysisProfile,
) -> EnrichSnapshotResult {
    let opts = CollectOptions {
        depth: Some(profile.depth_label().to_string()),
        ..Default::default()
    };
    let output = collect_dims(symbol, &opts, cached_quote, profile).await;
    apply_dims_to_snapshot(snap, &output);
    supplement_snapshot_confidence(snap);
    EnrichSnapshotResult {
        raw_dims: output.build_raw_dims(),
        collect: output,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::research::fetchers::types::Market;
    use crate::research::profile::AnalysisProfile;

    #[test]
    fn market_detection() {
        assert_eq!(Market::from_symbol("600809.SH"), Market::A);
        assert_eq!(Market::from_symbol("AAPL"), Market::U);
    }

    #[tokio::test]
    #[ignore = "live network benchmark"]
    async fn benchmark_collect_parallel_vs_serial_600519_medium() {
        let profile = AnalysisProfile::medium();
        let base_opts = CollectOptions {
            depth: Some("medium".into()),
            ..Default::default()
        };

        let parallel_start = Instant::now();
        let parallel_out = collect_dims("600519.SH", &base_opts, None, &profile).await;
        let parallel_ms = parallel_start.elapsed().as_millis();

        let serial_opts = CollectOptions {
            parallel: false,
            ..base_opts
        };
        let serial_start = Instant::now();
        let serial_out = collect_dims("600519.SH", &serial_opts, None, &profile).await;
        let serial_ms = serial_start.elapsed().as_millis();

        assert_eq!(parallel_out.dims.len(), serial_out.dims.len());
        let savings = if serial_ms > 0 {
            1.0 - (parallel_ms as f64 / serial_ms as f64)
        } else {
            0.0
        };
        eprintln!(
            "collect 600519 medium: parallel={parallel_ms}ms serial={serial_ms}ms savings={:.1}% dims={}",
            savings * 100.0,
            parallel_out.dims.len()
        );
        // No hard assert — network variance; baseline documented in
        // docs/insights/COLLECT_BENCHMARK_2026-06-25.md
    }
}
