//! Reverse proxy implementation for forwarding requests to backend services.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tracing::{debug, error};

use crate::config::GatewayConfig;

/// Proxy state shared across requests
#[derive(Clone)]
pub struct ProxyState {
    /// Configuration
    pub config: Arc<GatewayConfig>,

    /// HTTP client for proxying requests
    pub client: reqwest::Client,
}

impl ProxyState {
    /// Create a new proxy state
    pub fn new(config: Arc<GatewayConfig>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.gateway.request_timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self { config, client }
    }
}

/// Proxy handler that forwards requests to backend services
pub async fn proxy_handler(
    State(state): State<ProxyState>,
    req: Request<Body>,
) -> Result<Response, StatusCode> {
    let path = req.uri().path();
    let query = req.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();

    // Determine which backend to route to based on path
    let backend_url = determine_backend(&state.config, path)
        .ok_or(StatusCode::NOT_FOUND)?;

    // Construct target URL
    let target_url = format!("{}{}{}", backend_url, path, query);

    debug!(
        method = %req.method(),
        path = %path,
        target = %target_url,
        "Proxying request"
    );

    // Forward the request
    match forward_request(&state.client, req, &target_url).await {
        Ok(response) => Ok(response),
        Err(e) => {
            error!(error = %e, target = %target_url, "Failed to proxy request");
            Err(StatusCode::BAD_GATEWAY)
        }
    }
}

/// Determine which backend service to route to based on request path
fn determine_backend(config: &GatewayConfig, path: &str) -> Option<String> {
    // All API requests go to admin backend for now
    // In future, can add routing logic for /api/v1/events -> events backend, etc.
    if path.starts_with("/api/v1/") || path.starts_with("/metrics") {
        Some(config.backends.admin.url.clone())
    } else {
        None
    }
}

/// Forward request to backend service
async fn forward_request(
    client: &reqwest::Client,
    req: Request<Body>,
    target_url: &str,
) -> Result<Response, reqwest::Error> {
    let method = req.method().clone();
    let headers = req.headers().clone();

    // Convert axum::Body to bytes
    let body_bytes = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!(error = %e, "Failed to read request body");
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Failed to read request body"))
                .unwrap()
                .into_response());
        }
    };

    // Build backend request
    let mut backend_req = client
        .request(method.clone(), target_url)
        .body(body_bytes.to_vec());

    // Copy relevant headers (excluding Host, Connection, etc.)
    for (name, value) in headers.iter() {
        let name_str = name.as_str();
        // Skip hop-by-hop headers
        if !matches!(name_str, "host" | "connection" | "keep-alive" | "transfer-encoding" | "upgrade" | "te" | "trailer" | "proxy-authorization" | "proxy-authenticate") {
            backend_req = backend_req.header(name, value);
        }
    }

    // Send request to backend
    let backend_resp = backend_req.send().await?;

    // Convert backend response to axum response
    let status = backend_resp.status();
    let headers = backend_resp.headers().clone();
    let body_bytes = backend_resp.bytes().await?;

    let mut response = Response::builder()
        .status(status);

    // Copy response headers
    for (name, value) in headers.iter() {
        response = response.header(name, value);
    }

    Ok(response
        .body(Body::from(body_bytes))
        .unwrap())
}
