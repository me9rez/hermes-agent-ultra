//! Announcements + news via akshare.

use serde_json::{Value, json};

use crate::error::TradingError;

use super::{client, code6, map_err};

const ANNOUNCE_LIMIT: usize = 10;
const NEWS_LIMIT: usize = 8;

pub async fn fetch_events_dim_akshare(symbol: &str) -> Result<Value, TradingError> {
    let code = code6(symbol)?;
    let mut out = json!({});

    match client()
        .a_share_announcements(&code, ANNOUNCE_LIMIT)
        .await
        .map_err(map_err)
    {
        Ok(ann) => {
            let items: Vec<Value> = ann
                .into_iter()
                .map(|a| {
                    json!({
                        "title": a.title,
                        "published_at": a.published_at,
                        "url": a.url,
                        "source": a.source,
                    })
                })
                .collect();
            out["announcements"] = json!(items);
            out["announcement_count"] = json!(items.len());
        }
        Err(e) => {
            out["announcement_error"] = json!(e.to_string());
        }
    }

    match client().stock_news_em(&code).await.map_err(map_err) {
        Ok(news) => {
            let items: Vec<Value> = news
                .into_iter()
                .take(NEWS_LIMIT)
                .map(|n| {
                    json!({
                        "title": n.title,
                        "publish_time": n.publish_time,
                        "url": n.url,
                        "source": n.source,
                    })
                })
                .collect();
            out["news"] = json!(items);
            out["news_count"] = json!(items.len());
        }
        Err(e) => {
            out["news_error"] = json!(e.to_string());
        }
    }

    if out.as_object().is_some_and(|o| o.is_empty()) {
        return Err(TradingError::NoData);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_subpath_errors_without_dropping_sibling_data() {
        let data = json!({
            "announcement_count": 2,
            "announcements": [{"title": "年报"}],
            "news_error": "timeout"
        });
        assert_eq!(data["announcement_count"], 2);
        assert_eq!(data["news_error"], "timeout");
        assert!(data.get("news_count").is_none());
    }
}
