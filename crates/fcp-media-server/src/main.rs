//! fcp-media-server — Standalone REST media processing server.
//!
//! Wraps forge-api's `ApiServer` with FCP-style TOML configuration,
//! CLI argument parsing, and structured tracing. Provides session
//! management, conferencing, WebRTC, AI integration, Prometheus
//! metrics, authentication, rate limiting, TLS, and graceful shutdown.
//!
//! This binary is pure HTTP/RTP — no SIP stack is included.

use anyhow::{Context, Result};
use clap::Parser;
use forge_api::ApiServer;
use std::time::Duration;
use tracing::info;

mod config;

use config::MediaServerConfig;

/// FCP Media Server - Standalone Media Processing Engine
#[derive(Parser, Debug)]
#[command(
    name = "fcp-media",
    version,
    about = "Standalone media processing server for the Ferrous Communications Platform",
    long_about = "A REST media server wrapping forge-api with session management,\n\
                  conferencing, WebRTC, AI integration, Prometheus metrics,\n\
                  authentication, rate limiting, TLS, and graceful shutdown."
)]
struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_MEDIA_CONFIG"
    )]
    config: Option<std::path::PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_MEDIA_LOG_LEVEL"
    )]
    log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_MEDIA_LOG_FORMAT"
    )]
    log_format: Option<String>,

    /// HTTP bind address override
    #[arg(
        long = "http-bind",
        value_name = "ADDR",
        help = "Override HTTP bind address (e.g., 0.0.0.0:8080)",
        env = "FCP_MEDIA_HTTP_BIND"
    )]
    http_bind: Option<String>,

    /// Public IP address for RTP/SDP media
    #[arg(
        long = "public-ip",
        value_name = "IP",
        help = "Public IP for RTP (required in Docker or behind NAT)",
        env = "FCP_MEDIA_PUBLIC_IP"
    )]
    public_ip: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI arguments
    let cli = Cli::parse();

    // 2. Load configuration
    let mut config = config::load_config(cli.config.as_deref())?;

    // Apply CLI overrides
    if let Some(ref level) = cli.log_level {
        config.logging.level = level.clone();
    }
    if let Some(ref format) = cli.log_format {
        config.logging.format = format.clone();
    }
    if let Some(ref bind) = cli.http_bind {
        config.http.bind_address = bind
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid HTTP bind address '{}': {}", bind, e))?;
    }
    if let Some(ref ip) = cli.public_ip {
        if !ip.is_empty() {
            config.media.public_ip = Some(ip.clone());
        }
    }

    // 3. Validate configuration
    config::validate_config(&config)?;

    // 4. Initialize tracing
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-media starting"
    );

    // 5. Convert to ApiServerConfig
    let api_config = config::to_api_server_config(&config);

    // 6. Create ApiServer
    let server = ApiServer::new(api_config).await;

    info!(
        http = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        tls = config.tls.enabled,
        auth = !config.auth.disable_auth,
        "fcp-media ready — listening for API requests"
    );

    // 7. Serve with graceful shutdown on SIGINT/SIGTERM
    let shutdown_timeout = Duration::from_secs(config.service.shutdown_timeout_secs);
    let shutdown_signal = async move {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Failed to register SIGTERM handler");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown...");
            }
        }

        // Allow in-flight requests to complete within the timeout
        if !shutdown_timeout.is_zero() {
            info!(timeout_secs = shutdown_timeout.as_secs(), "Draining connections...");
        }
    };

    server
        .serve_with_shutdown(shutdown_signal)
        .await
        .context("API server error")?;

    info!("Shutdown complete");
    Ok(())
}

fn init_tracing(config: &MediaServerConfig) {
    use tracing_subscriber::EnvFilter;

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.logging.level));

    match config.logging.format.as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .json()
                .with_env_filter(env_filter)
                .init();
        }
        "compact" => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .compact()
                .with_env_filter(env_filter)
                .init();
        }
        _ => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .with_env_filter(env_filter)
                .init();
        }
    }
}
