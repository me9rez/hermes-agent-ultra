//! Cross-vertical read-only queries between bundled verticals.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossQueryRequest {
    pub from_vertical: String,
    pub to_vertical: String,
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossQueryResult {
    pub items: Vec<serde_json::Value>,
    pub note: Option<String>,
}

pub fn cross_vertical_query(req: &CrossQueryRequest) -> CrossQueryResult {
    CrossQueryResult {
        items: vec![],
        note: Some(format!(
            "stub: {} -> {} ({})",
            req.from_vertical, req.to_vertical, req.query
        )),
    }
}
