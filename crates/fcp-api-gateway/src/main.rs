//! FCP API Gateway - Public-facing API with caching and rate limiting
//!
//! This binary provides the API Gateway for FCP, handling:
//!
//! - Request routing to backend services (admin, events, jobs)
//! - Response caching (Redis-backed for GET requests)
//! - Rate limiting (token bucket algorithm)
//! - CORS handling
//! - Request/response logging
//! - Health checks

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::Method,
    middleware,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use clap::Parser;
use serde_json::json;
use std::sync::Arc;
use tokio::signal;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{info, warn};

mod cache;
mod cli;
mod config;
mod health;
mod proxy;
mod rate_limit;

use cli::Cli;
use config::load_config;

/// Initialize tracing/logging for fcp-api-gateway.
fn init_tracing(config: &config::GatewayConfig) -> Result<()> {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let log_level = &config.logging.level;
    let log_format = &config.logging.format;

    // Create the EnvFilter from the log level
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .context("Failed to create log filter")?;

    // Build the subscriber based on format
    match log_format.as_str() {
        "json" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())
                .init();
        }
        "pretty" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().pretty())
                .init();
        }
        "compact" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().compact())
                .init();
        }
        _ => {
            // Default to JSON for production
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())
                .init();
        }
    }

    Ok(())
}

/// Health check handler
async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "service": "fcp-api-gateway",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Gateway status: the last background probe of every backend
/// (`crate::health`), never a probe made on request.
async fn gateway_status(State(proxy_state): State<proxy::ProxyState>) -> impl IntoResponse {
    Json(proxy_state.health.report(&proxy_state.config))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Load configuration
    let mut config =
        fcp_core::config::check_stage(load_config(cli.config.as_deref()), cli.check_config)?;

    // Apply CLI overrides
    if let Some(log_level) = cli.log_level {
        config.logging.level = log_level;
    }
    if let Some(log_format) = cli.log_format {
        config.logging.format = log_format;
    }
    if let Some(bind_addr) = cli.bind_addr {
        config.gateway.bind_address = bind_addr.parse().context("Invalid bind address format")?;
    }
    if let Some(redis_url) = cli.redis_url {
        config.redis.url = redis_url;
    }

    // Initialize tracing/logging
    if cli.check_config {
        println!(
            "{}",
            fcp_core::config::masked_toml(&config).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }
    init_tracing(&config).context("Failed to initialize tracing")?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        service = "fcp-api-gateway",
        "Starting FCP API Gateway"
    );

    // Create shared state
    let config = Arc::new(config);
    let proxy_state = proxy::ProxyState::new(Arc::clone(&config));
    proxy_state
        .health
        .spawn_probes(proxy_state.client.clone(), Arc::clone(&config));
    let cache_state = cache::CacheState::new(&config.redis, config.cache.clone());
    let rate_limiter_state = rate_limit::RateLimiterState::new(config.rate_limit.clone());

    // Configure CORS
    let allowed_methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
    ];
    let cors = if config.cors.enabled {
        if config.cors.allow_all_origins {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(allowed_methods)
                .allow_headers(Any)
        } else if !config.cors.allowed_origins.is_empty() {
            // Parse configured origins into HeaderValues
            let origins: Vec<axum::http::HeaderValue> = config
                .cors
                .allowed_origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(allowed_methods)
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ])
        } else {
            // No origins configured and allow_all_origins is false — deny all cross-origin
            warn!("CORS enabled but no origins configured and allow_all_origins=false; cross-origin requests will be rejected");
            CorsLayer::new()
                .allow_methods(allowed_methods)
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ])
        }
    } else {
        // CORS disabled — no CORS headers at all (browser same-origin policy enforced)
        CorsLayer::new()
    };

    // Build router with middleware stack
    let app = Router::new()
        // Gateway health/status endpoints
        .route("/health", get(health_check))
        .route("/gateway/status", get(gateway_status))
        // All other routes go through proxy
        .fallback(proxy::proxy_handler)
        // Add middleware (executed in reverse order)
        .layer(middleware::from_fn_with_state(
            cache_state,
            cache::cache_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            rate_limiter_state,
            rate_limit::rate_limit_middleware,
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(proxy_state.clone());
    let app = if config.gateway.compression_enabled {
        app.layer(CompressionLayer::new())
    } else {
        app
    };

    // Start server
    let bind_addr = config.gateway.bind_address;
    info!(
        bind = %bind_addr,
        cache_enabled = config.cache.enabled,
        rate_limit_enabled = config.rate_limit.enabled,
        compression_enabled = config.gateway.compression_enabled,
        "Starting API Gateway server"
    );

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!("API Gateway listening on {}", bind_addr);

    // Spawn server task
    let server_handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
    });

    // Wait for shutdown signal
    info!("FCP API Gateway running. Press Ctrl+C or send SIGTERM to shutdown.");
    shutdown_signal().await;

    // Shutdown server
    server_handle.abort();

    info!("FCP API Gateway shutdown complete");
    Ok(())
}

/// Resolve on Ctrl+C or SIGTERM so container orchestrators can stop the
/// service gracefully (`docker stop`, Kubernetes pod termination).
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("Received Ctrl+C"),
        _ = terminate => info!("Received SIGTERM"),
    }
}
