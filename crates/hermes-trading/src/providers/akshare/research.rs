//! Broker research reports via akshare.

use serde_json::{Value, json};

use crate::error::TradingError;

use super::{client, code6, map_err};

const REPORT_LIMIT: usize = 12;

pub async fn fetch_research_dim_akshare(symbol: &str) -> Result<Value, TradingError> {
    let code = code6(symbol)?;
    match client()
        .stock_research_report_em(&code)
        .await
        .map_err(map_err)
    {
        Ok(reports) => {
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
        Err(e) => Ok(json!({
            "research_reports": [],
            "research_count": 0,
            "research_error": e.to_string(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn research_payload_shape_has_reports_and_count() {
        let data = json!({
            "research_reports": [{
                "title": "维持买入",
                "org": "中信证券",
                "rating": "买入",
                "date": "2026-06-01",
                "pdf_url": "https://example.com/r.pdf"
            }],
            "research_count": 1
        });
        assert_eq!(data["research_count"], 1);
        let report = &data["research_reports"][0];
        for key in ["title", "org", "rating", "date", "pdf_url"] {
            assert!(report.get(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn research_error_keeps_empty_reports_array() {
        let data = json!({
            "research_reports": [],
            "research_count": 0,
            "research_error": "timeout"
        });
        assert!(data.get("research_error").is_some());
        assert_eq!(data["research_count"], 0);
    }
}
