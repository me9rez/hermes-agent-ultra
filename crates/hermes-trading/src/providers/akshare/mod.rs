//! akshare-rs adapter — primary A-share data path with eastmoney fallback.

mod basic_info;
mod candles;
mod capital_flow;
mod events;
mod financials;
mod fund_holders;
mod labels;
mod lhb;
mod peers;
mod quote;
mod research;
mod symbol_resolve;
mod valuation;

use std::sync::OnceLock;

use akshare::AkShareClient;

use crate::error::TradingError;
use crate::settlement::is_a_share;
use crate::symbol::normalize_symbol;

static CLIENT: OnceLock<AkShareClient> = OnceLock::new();

pub(crate) fn client() -> &'static AkShareClient {
    CLIENT.get_or_init(AkShareClient::new)
}

pub(crate) fn map_err(e: akshare::Error) -> TradingError {
    TradingError::InvalidResponse(e.to_string())
}

/// Six-digit A-share code from `600519.SH`.
pub(crate) fn code6(symbol: &str) -> Result<String, TradingError> {
    let canon = normalize_symbol(symbol);
    if !is_a_share(&canon) {
        return Err(TradingError::SymbolNotFound(format!(
            "akshare A-share only: {symbol}"
        )));
    }
    Ok(canon.split('.').next().unwrap_or(&canon).to_string())
}

/// Sina paper code prefix (`sh600519` / `sz000001`).
pub(crate) fn sina_paper_code(symbol: &str) -> Result<String, TradingError> {
    let code = code6(symbol)?;
    let canon = normalize_symbol(symbol);
    Ok(if canon.ends_with(".SZ") {
        format!("sz{code}")
    } else {
        format!("sh{code}")
    })
}

pub(crate) async fn try_or_fallback<T, P, F>(primary: P, fallback: F) -> Result<T, TradingError>
where
    P: std::future::Future<Output = Result<T, TradingError>>,
    F: std::future::Future<Output = Result<T, TradingError>>,
{
    match primary.await {
        Ok(v) => Ok(v),
        Err(e) => {
            tracing::warn!(error = %e, "akshare path failed, using fallback");
            fallback.await
        }
    }
}

pub use basic_info::{
    BasicInfoSupplement, apply_supplement, fetch_basic_info_supplement, map_individual_info,
};
pub use candles::{CHART_CANDLE_COUNT, OhlcBar, fetch_a_share_closes, fetch_a_share_ohlc_bars};
pub use capital_flow::fetch_capital_flow_dim_akshare;
pub use events::fetch_events_dim_akshare;
pub use financials::fetch_financials_dim_akshare;
pub use fund_holders::fetch_fund_holders_dim;
pub use labels::DEFAULT_INDUSTRY;
pub use lhb::fetch_lhb_dim_akshare;
pub use peers::{
    em_prefix_symbol, fetch_industry_growth, fetch_peer_table, map_peer_table, median_peer_pe,
};
pub use quote::fetch_a_share_quote_chain;
pub use research::fetch_research_dim_akshare;
pub use symbol_resolve::resolve_a_share_symbol;
pub use valuation::{fetch_valuation_percentiles, percentile_rank};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code6_and_sina_paper() {
        assert_eq!(code6("600519.SH").unwrap(), "600519");
        assert_eq!(sina_paper_code("600519.SH").unwrap(), "sh600519");
        assert_eq!(sina_paper_code("000001.SZ").unwrap(), "sz000001");
    }

    #[tokio::test]
    #[ignore = "live akshare network"]
    async fn live_akshare_quote_600519() {
        let q = super::quote::fetch_a_share_quote_chain("600519.SH")
            .await
            .expect("quote");
        assert!(q.price.unwrap_or(0.0) > 0.0);
    }

    #[tokio::test]
    #[ignore = "live akshare network"]
    async fn live_akshare_candles_600519() {
        let (closes, source) = super::candles::fetch_a_share_closes("600519.SH")
            .await
            .expect("candles");
        assert!(closes.len() >= 20);
        assert!(closes.last().copied().unwrap_or(0.0) > 0.0);
        assert!(source == "akshare" || source == "eastmoney_push2his");
    }

    #[tokio::test]
    #[ignore = "live akshare network"]
    async fn live_events_dim_600519() {
        let data = fetch_events_dim_akshare("600519.SH")
            .await
            .expect("events dim");
        let ann = data
            .get("announcement_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let news = data.get("news_count").and_then(|v| v.as_u64()).unwrap_or(0);
        eprintln!("live events 600519: {}", data);
        if let Some(err) = data.get("announcement_error") {
            eprintln!("announcement_error: {err}");
        }
        if let Some(err) = data.get("news_error") {
            eprintln!("news_error: {err}");
        }
        assert!(
            ann > 0 || news > 0,
            "expected announcements or news; got {data}"
        );
    }

    #[tokio::test]
    #[ignore = "live akshare network"]
    async fn live_research_dim_600519() {
        let data = super::research::fetch_research_dim_akshare("600519.SH")
            .await
            .expect("research dim");
        let count = data
            .get("research_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        eprintln!("live research 600519: {}", data);
        if let Some(err) = data.get("research_error") {
            eprintln!("research_error: {err}");
        }
        assert!(count > 0, "expected broker reports for Moutai; got {data}");
        let reports = data
            .get("research_reports")
            .and_then(|v| v.as_array())
            .expect("research_reports array");
        assert!(!reports.is_empty());
        let first = &reports[0];
        assert!(first.get("title").and_then(|v| v.as_str()).is_some());
        assert!(first.get("org").and_then(|v| v.as_str()).is_some());
    }

    #[tokio::test]
    #[ignore = "live akshare network"]
    async fn live_lhb_dim_600519() {
        let (data, source) = fetch_lhb_dim_akshare("600519.SH").await.expect("lhb dim");
        eprintln!("live lhb 600519 ({source}): {}", data);
        if let Some(err) = data.get("lhb_error") {
            eprintln!("lhb_error: {err}");
        }
        assert!(
            data.get("lhb_count_30d").is_some(),
            "expected lhb_count_30d field; got {data}"
        );
        assert!(
            data.get("matched_youzi")
                .and_then(|v| v.as_array())
                .is_some(),
            "expected matched_youzi array; got {data}"
        );
        let count = data
            .get("lhb_count_30d")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let youzi = data
            .get("matched_youzi")
            .and_then(|v| v.as_array())
            .map_or(0, |a| a.len());
        eprintln!("lhb_count_30d={count}, matched_youzi={youzi}");
    }
}
