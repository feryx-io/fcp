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
use tracing::info;

mod cache;
mod cli;
mod config;
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

/// Gateway status handler with backend health
async fn gateway_status(State(proxy_state): State<proxy::ProxyState>) -> impl IntoResponse {
    // TODO: Check backend health
    Json(json!({
        "status": "operational",
        "service": "fcp-api-gateway",
        "version": env!("CARGO_PKG_VERSION"),
        "backends": {
            "admin": {
                "url": proxy_state.config.backends.admin.url,
                "status": "unknown"  // TODO: Implement health checks
            }
        }
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Load configuration
    let mut config = load_config(cli.config.as_deref())?;

    // Apply CLI overrides
    if let Some(log_level) = cli.log_level {
        config.logging.level = log_level;
    }
    if let Some(log_format) = cli.log_format {
        config.logging.format = log_format;
    }
    if let Some(bind_addr) = cli.bind_addr {
        config.gateway.bind_address = bind_addr
            .parse()
            .context("Invalid bind address format")?;
    }
    if let Some(redis_url) = cli.redis_url {
        config.redis.url = redis_url;
    }

    // Initialize tracing/logging
    init_tracing(&config).context("Failed to initialize tracing")?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        service = "fcp-api-gateway",
        "Starting FCP API Gateway"
    );

    // Create shared state
    let config = Arc::new(config);
    let proxy_state = proxy::ProxyState::new(Arc::clone(&config));
    let cache_state = cache::CacheState::new(&config.redis, config.cache.clone());
    let rate_limiter_state = rate_limit::RateLimiterState::new(config.rate_limit.clone());

    // Configure CORS
    let cors = if config.cors.enabled {
        if config.cors.allow_all_origins {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::PATCH])
                .allow_headers(Any)
        } else {
            // TODO: Implement specific origin handling
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::PATCH])
                .allow_headers(Any)
        }
    } else {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::PATCH])
            .allow_headers(Any)
    };

    // Build router with middleware stack
    let app = Router::new()
        // Gateway health/status endpoints
        .route("/health", get(health_check))
        .route("/gateway/status", get(gateway_status))
        // All other routes go through proxy
        .fallback(proxy::proxy_handler)
        // Add middleware (executed in reverse order)
        .layer(middleware::from_fn_with_state(cache_state, cache::cache_middleware))
        .layer(middleware::from_fn_with_state(rate_limiter_state, rate_limit::rate_limit_middleware))
        .layer(if config.gateway.compression_enabled {
            Some(CompressionLayer::new())
        } else {
            None
        }.unwrap_or_else(|| CompressionLayer::new()))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(proxy_state.clone());

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
        axum::serve(listener, app).await
    });

    // Wait for shutdown signal
    info!("FCP API Gateway running. Press Ctrl+C to shutdown.");
    signal::ctrl_c()
        .await
        .context("Failed to listen for shutdown signal")?;
    info!("Received shutdown signal (Ctrl+C)");

    // Shutdown server
    server_handle.abort();

    info!("FCP API Gateway shutdown complete");
    Ok(())
}
