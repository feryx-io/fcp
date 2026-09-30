//! The WebSocket client the streaming providers share.

use anyhow::{Context, Result};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

pub(crate) type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// An `http(s)://` base as `ws(s)://`.
pub(crate) fn ws_url(base: &str) -> String {
    if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    }
}

/// Connect with an `Authorization` header.
pub(crate) async fn connect(url: &str, authorization: &str) -> Result<Socket> {
    let mut request = url
        .into_client_request()
        .with_context(|| format!("the stream URL {url}"))?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(authorization).context("the authorization header")?,
    );
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .with_context(|| format!("connecting to {}", url.split('?').next().unwrap_or(url)))?;
    Ok(socket)
}
