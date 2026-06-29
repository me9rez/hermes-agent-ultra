use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;

use crate::types::{
    DataSourceCapabilities, DataSourceError, DataSourceProvider, DataSourceQuery,
    DataSourceResponse, DataSourceResult,
};

pub struct AkshareLocalDataSource {
    client: Client,
    bridge_url: String,
}

impl AkshareLocalDataSource {
    pub fn new(bridge_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            bridge_url: bridge_url.into(),
        }
    }

    pub fn default_bridge() -> Self {
        Self::new("http://127.0.0.1:8799")
    }

    pub fn probe_available(&self) -> bool {
        let url = self.bridge_url.trim_end_matches('/');
        if let Ok(parsed) = url::Url::parse(url)
            && let Some(host) = parsed.host_str()
        {
            let port = parsed.port().unwrap_or(8799);
            if let Ok(addr) = format!("{host}:{port}").parse() {
                return std::net::TcpStream::connect_timeout(
                    &addr,
                    std::time::Duration::from_millis(500),
                )
                .is_ok();
            }
        }
        false
    }
}

#[async_trait]
impl DataSourceProvider for AkshareLocalDataSource {
    fn id(&self) -> &str {
        "akshare_local"
    }

    fn display_name_key(&self) -> &str {
        "datasource.akshare_local.name"
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities {
            supports_realtime: false,
            supports_historical: true,
            markets: vec!["CN-A".into()],
            asset_types: vec!["Equity".into()],
            rate_limit_per_min: None,
            data_delay_seconds: 900,
        }
    }

    async fn query(&self, q: DataSourceQuery) -> DataSourceResult<DataSourceResponse> {
        let resp = self
            .client
            .post(format!("{}/query", self.bridge_url.trim_end_matches('/')))
            .json(&q)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(DataSourceError::Other(format!(
                "akshare local status {}",
                resp.status()
            )));
        }
        Ok(DataSourceResponse {
            provider_id: self.id().to_string(),
            data: resp.json().await?,
            fetched_at: Utc::now(),
        })
    }

    async fn test_connection(&self) -> DataSourceResult<()> {
        if self.probe_available() {
            Ok(())
        } else {
            Err(DataSourceError::Other(
                "local akshare bridge not available".into(),
            ))
        }
    }
}
