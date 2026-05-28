//! WebSocket connect helpers (HTTP CONNECT proxy for platforms that use tokio-tungstenite).

use std::sync::Arc;

use hermes_core::errors::GatewayError;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::Response;
use tokio_tungstenite::{client_async, MaybeTlsStream, WebSocketStream};
use url::Url;

use crate::adapter::AdapterProxyConfig;

/// Connect to a `wss://` URL, optionally via HTTP proxy (`CONNECT` tunnel).
pub async fn connect_websocket(
    gateway_url: &str,
    proxy: &AdapterProxyConfig,
) -> Result<
    (
        WebSocketStream<MaybeTlsStream<TcpStream>>,
        Response<Option<Vec<u8>>>,
    ),
    GatewayError,
> {
    if let Some(ref http_proxy) = proxy.http_proxy {
        return connect_websocket_via_http_proxy(gateway_url, http_proxy).await;
    }
    tokio_tungstenite::connect_async(gateway_url)
        .await
        .map_err(|e| GatewayError::ConnectionFailed(format!("WebSocket connect: {e}")))
}

async fn connect_websocket_via_http_proxy(
    gateway_url: &str,
    proxy_url: &str,
) -> Result<
    (
        WebSocketStream<MaybeTlsStream<TcpStream>>,
        Response<Option<Vec<u8>>>,
    ),
    GatewayError,
> {
    let target = Url::parse(gateway_url).map_err(|e| {
        GatewayError::ConnectionFailed(format!("Invalid WebSocket URL {gateway_url}: {e}"))
    })?;
    let proxy = Url::parse(proxy_url).map_err(|e| {
        GatewayError::ConnectionFailed(format!("Invalid HTTP proxy {proxy_url}: {e}"))
    })?;

    let target_host = target
        .host_str()
        .ok_or_else(|| GatewayError::ConnectionFailed("WebSocket URL missing host".into()))?;
    let target_port = target.port_or_known_default().unwrap_or(443);

    let proxy_host = proxy
        .host_str()
        .ok_or_else(|| GatewayError::ConnectionFailed("Proxy URL missing host".into()))?;
    let proxy_port = proxy.port_or_known_default().unwrap_or(8080);

    let mut stream = TcpStream::connect((proxy_host, proxy_port))
        .await
        .map_err(|e| {
            GatewayError::ConnectionFailed(format!(
                "Proxy TCP connect {proxy_host}:{proxy_port}: {e}"
            ))
        })?;

    let connect_req = format!(
        "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\n\r\n"
    );
    stream
        .write_all(connect_req.as_bytes())
        .await
        .map_err(|e| GatewayError::ConnectionFailed(format!("Proxy CONNECT write: {e}")))?;

    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 256];
    loop {
        let n = stream.read(&mut chunk).await.map_err(|e| {
            GatewayError::ConnectionFailed(format!("Proxy CONNECT read: {e}"))
        })?;
        if n == 0 {
            return Err(GatewayError::ConnectionFailed(
                "Proxy closed before CONNECT response".into(),
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 8192 {
            return Err(GatewayError::ConnectionFailed(
                "Proxy CONNECT response too large".into(),
            ));
        }
    }

    let status_line = std::str::from_utf8(&buf)
        .map_err(|_| GatewayError::ConnectionFailed("Proxy CONNECT non-UTF8 response".into()))?
        .lines()
        .next()
        .unwrap_or_default();
    if !status_line.contains(" 200 ") {
        return Err(GatewayError::ConnectionFailed(format!(
            "Proxy CONNECT failed: {status_line}"
        )));
    }

    let tls = build_tls_connector()?;
    let server_name = ServerName::try_from(target_host.to_string()).map_err(|e| {
        GatewayError::ConnectionFailed(format!("Invalid TLS server name {target_host}: {e}"))
    })?;
    let tls_stream = tls
        .connect(server_name, stream)
        .await
        .map_err(|e| GatewayError::ConnectionFailed(format!("TLS after proxy: {e}")))?;

    let request = gateway_url
        .into_client_request()
        .map_err(|e| GatewayError::ConnectionFailed(format!("WebSocket request: {e}")))?;

    let stream = MaybeTlsStream::Rustls(tls_stream);
    client_async(request, stream)
        .await
        .map_err(|e| GatewayError::ConnectionFailed(format!("WebSocket handshake: {e}")))
}

fn build_tls_connector() -> Result<TlsConnector, GatewayError> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}
