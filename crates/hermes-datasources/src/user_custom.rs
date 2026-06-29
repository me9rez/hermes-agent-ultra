use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};

use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use url::Url;

use crate::types::{
    DataSourceAuth, DataSourceCapabilities, DataSourceError, DataSourceProvider, DataSourceQuery,
    DataSourceResponse, DataSourceResult, UserCustomDataSourceConfig,
};

pub struct UserCustomDataSource {
    config: UserCustomDataSourceConfig,
    client: Client,
}

impl UserCustomDataSource {
    pub fn new(config: UserCustomDataSourceConfig) -> DataSourceResult<Self> {
        validate_endpoint(&config.endpoint)?;
        Ok(Self {
            config,
            client: Client::new(),
        })
    }
}

pub fn validate_endpoint(endpoint: &str) -> DataSourceResult<()> {
    let url = Url::parse(endpoint)
        .map_err(|e| DataSourceError::Other(format!("invalid endpoint url: {e}")))?;
    if url.scheme() != "https" {
        return Err(DataSourceError::Other(
            "user custom datasource must use https".into(),
        ));
    }
    if let Some(host) = url.host_str()
        && is_private_host(host)
    {
        return Err(DataSourceError::Other(
            "private/localhost endpoints are not allowed".into(),
        ));
    }
    Ok(())
}

fn is_private_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_private_ip(ip);
    }
    if let Ok(addrs) = (host, 0).to_socket_addrs() {
        for addr in addrs {
            if is_private_ip(addr.ip()) {
                return true;
            }
        }
    }
    false
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4 == Ipv4Addr::new(169, 254, 0, 0)
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
    }
}

#[async_trait]
impl DataSourceProvider for UserCustomDataSource {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn display_name_key(&self) -> &str {
        "datasource.user_custom.name"
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities {
            supports_realtime: false,
            supports_historical: true,
            markets: vec![],
            asset_types: vec![],
            rate_limit_per_min: Some(30),
            data_delay_seconds: 0,
        }
    }

    async fn query(&self, q: DataSourceQuery) -> DataSourceResult<DataSourceResponse> {
        let path = match &q {
            DataSourceQuery::Custom { name, .. } => self
                .config
                .query_mapping
                .get(name)
                .cloned()
                .unwrap_or_else(|| "/query".into()),
            _ => self
                .config
                .query_mapping
                .get("default")
                .cloned()
                .unwrap_or_else(|| "/query".into()),
        };
        let url = format!("{}{}", self.config.endpoint.trim_end_matches('/'), path);
        let mut req = self.client.post(url).json(&q);
        if let Some(DataSourceAuth { header_name, token }) = &self.config.auth {
            req = req.header(header_name, token);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(DataSourceError::Other(format!(
                "user custom datasource status {}",
                resp.status()
            )));
        }
        Ok(DataSourceResponse {
            provider_id: self.config.id.clone(),
            data: resp.json().await?,
            fetched_at: Utc::now(),
        })
    }

    async fn test_connection(&self) -> DataSourceResult<()> {
        validate_endpoint(&self.config.endpoint)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_localhost() {
        assert!(validate_endpoint("http://localhost/data").is_err());
        assert!(validate_endpoint("https://127.0.0.1/data").is_err());
    }

    #[test]
    fn accepts_public_https() {
        assert!(validate_endpoint("https://api.example.com/data").is_ok());
    }
}
