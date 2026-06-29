use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;

use crate::types::{
    DataSourceCapabilities, DataSourceError, DataSourceProvider, DataSourceQuery,
    DataSourceResponse, DataSourceResult,
};

pub struct AkshareCloudDataSource {
    client: Client,
    base_url: String,
}

impl AkshareCloudDataSource {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
        }
    }

    pub fn from_env() -> Self {
        let base_url = std::env::var("TERRA_CLOUD_BASE_URL")
            .unwrap_or_else(|_| "https://api.terra.app".into());
        Self::new(format!("{base_url}/v1/datasource/akshare"))
    }
}

#[async_trait]
impl DataSourceProvider for AkshareCloudDataSource {
    fn id(&self) -> &str {
        "akshare"
    }

    fn display_name_key(&self) -> &str {
        "datasource.akshare.name"
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities {
            supports_realtime: false,
            supports_historical: true,
            markets: vec!["CN-A".into()],
            asset_types: vec!["Equity".into()],
            rate_limit_per_min: Some(60),
            data_delay_seconds: 900,
        }
    }

    async fn query(&self, q: DataSourceQuery) -> DataSourceResult<DataSourceResponse> {
        let resp = self
            .client
            .post(format!("{}/query", self.base_url.trim_end_matches('/')))
            .json(&q)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(DataSourceError::Other(format!(
                "akshare cloud status {}",
                resp.status()
            )));
        }
        let data = resp.json().await?;
        Ok(DataSourceResponse {
            provider_id: self.id().to_string(),
            data,
            fetched_at: Utc::now(),
        })
    }

    async fn test_connection(&self) -> DataSourceResult<()> {
        let resp = self
            .client
            .get(format!("{}/health", self.base_url.trim_end_matches('/')))
            .send()
            .await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(DataSourceError::Other(format!(
                "health check failed: {}",
                resp.status()
            )))
        }
    }
}
