use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataSourceError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Other(String),
}

pub type DataSourceResult<T> = Result<T, DataSourceError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceCapabilities {
    pub supports_realtime: bool,
    pub supports_historical: bool,
    pub markets: Vec<String>,
    pub asset_types: Vec<String>,
    pub rate_limit_per_min: Option<u32>,
    pub data_delay_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DataSourceQuery {
    Quote {
        symbol: String,
    },
    Klines {
        symbol: String,
        period: String,
        count: u32,
    },
    SectorRotation {
        window: String,
    },
    CapitalFlow {
        symbol: Option<String>,
        window: String,
    },
    News {
        symbols: Vec<String>,
        since: DateTime<Utc>,
    },
    Search {
        keyword: String,
        asset_type: String,
    },
    Custom {
        name: String,
        params: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceResponse {
    pub provider_id: String,
    pub data: Value,
    pub fetched_at: DateTime<Utc>,
}

#[async_trait]
pub trait DataSourceProvider: Send + Sync {
    fn id(&self) -> &str;
    fn display_name_key(&self) -> &str;
    fn capabilities(&self) -> DataSourceCapabilities;
    async fn query(&self, q: DataSourceQuery) -> DataSourceResult<DataSourceResponse>;
    async fn test_connection(&self) -> DataSourceResult<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceAuth {
    pub header_name: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserCustomDataSourceConfig {
    pub id: String,
    pub display_name: String,
    pub endpoint: String,
    pub auth: Option<DataSourceAuth>,
    pub query_mapping: HashMap<String, String>,
}
