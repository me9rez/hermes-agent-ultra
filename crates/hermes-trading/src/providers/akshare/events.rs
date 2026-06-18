//! Announcements + news via akshare.

use serde_json::{Value, json};

use crate::error::TradingError;

use super::{client, code6, map_err};

const ANNOUNCE_LIMIT: usize = 10;
const NEWS_LIMIT: usize = 8;

pub async fn fetch_events_dim_akshare(symbol: &str) -> Result<Value, TradingError> {
    let code = code6(symbol)?;
    let mut out = json!({});

    if let Ok(ann) = client()
        .a_share_announcements(&code, ANNOUNCE_LIMIT)
        .await
        .map_err(map_err)
    {
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

    if let Ok(news) = client().stock_news_em(&code).await.map_err(map_err) {
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

    if out.as_object().is_some_and(|o| o.is_empty()) {
        return Err(TradingError::NoData);
    }
    Ok(out)
}
