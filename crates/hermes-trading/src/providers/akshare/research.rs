//! Broker research reports via akshare.

use serde_json::{Value, json};

use crate::error::TradingError;

use super::{client, code6, map_err};

const REPORT_LIMIT: usize = 12;

pub async fn fetch_research_dim_akshare(symbol: &str) -> Result<Value, TradingError> {
    let code = code6(symbol)?;
    let reports = client()
        .stock_research_report_em(&code)
        .await
        .map_err(map_err)?;
    let items: Vec<Value> = reports
        .into_iter()
        .take(REPORT_LIMIT)
        .map(|r| {
            json!({
                "title": r.title,
                "org": r.org,
                "rating": r.rating,
                "date": r.date,
                "pdf_url": r.pdf_url,
            })
        })
        .collect();
    Ok(json!({
        "research_reports": items,
        "research_count": items.len(),
    }))
}
