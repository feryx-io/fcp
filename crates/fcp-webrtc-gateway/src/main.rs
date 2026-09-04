//! fcp-webrtc-gw — WebRTC-to-SIP gateway.
//!
//! Bridges browser-side WebRTC connections to SIP phones by combining:
//! - forge-api's WebRTC/session management (existing REST endpoints)
//! - A SIP UAC for outbound INVITE/BYE (siphon-rs IntegratedUAC)
//! - forge-media for Opus ↔ G.711 transcoding
//!
//! The gateway exposes a unified HTTP API:
//! - `/v1/webrtc/*` — WebRTC connection management (from forge-api)
//! - `/v1/calls/*` — SIP call control (this binary)
//! - `/health`, `/v1/sessions`, `/v1/metrics` — from forge-api

use anyhow::{Context, Result};
use axum::middleware as axum_middleware;
use axum::Extension;
use axum_server::tls_rustls::RustlsConfig;
use clap::Parser;
use fcp_sip::prelude::*;
use fcp_sip::transport::ConnectionPool;
use forge_api::middleware::{self, auth::AuthConfig, RateLimiter};
use forge_api::routes;
use forge_api::routes::prometheus::MetricsHandle;
use forge_api::routes::sessions::AppState;
use forge_engine::{SessionManager, SessionManagerConfig};
use forge_rtp::PortPoolConfig;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tower::ServiceBuilder;
use tracing::info;

mod call_bridge;
mod config;
mod http_api;
mod sip_client;

use config::GatewayConfig;

/// FCP WebRTC Gateway - Browser-to-SIP Call Bridge
#[derive(Parser, Debug)]
#[command(
    name = "fcp-webrtc-gw",
    version,
    about = "WebRTC-to-SIP gateway for the Ferrous Communications Platform",
    long_about = "Bridges browser WebRTC connections to SIP phones.\n\
                  Provides REST API for WebRTC management and SIP call control\n\
                  with Opus ↔ G.711 media transcoding."
)]
struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_WEBRTC_GW_CONFIG"
    )]
    config: Option<std::path::PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_WEBRTC_GW_LOG_LEVEL"
    )]
    log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_WEBRTC_GW_LOG_FORMAT"
    )]
    log_format: Option<String>,

    /// HTTP bind address override
    #[arg(
        long = "http-bind",
        value_name = "ADDR",
        help = "Override HTTP bind address (e.g., 0.0.0.0:8080)",
        env = "FCP_WEBRTC_GW_HTTP_BIND"
    )]
    http_bind: Option<String>,

    /// SIP bind address override
    #[arg(
        long = "sip-bind",
        value_name = "ADDR",
        help = "Override SIP UDP bind address (e.g., 0.0.0.0:5060)",
        env = "FCP_WEBRTC_GW_SIP_BIND"
    )]
    sip_bind: Option<String>,

    /// Public IP address for RTP/SDP
    #[arg(
        long = "public-ip",
        value_name = "IP",
        help = "Public IP for RTP and SIP (required behind NAT)",
        env = "FCP_WEBRTC_GW_PUBLIC_IP"
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
    if let Some(ref bind) = cli.sip_bind {
        config.sip.udp_bind = bind.clone();
    }
    if let Some(ref ip) = cli.public_ip {
        if !ip.is_empty() {
            config.sip.local_ip = Some(ip.clone());
            config.media.public_ip = Some(ip.clone());
        }
    }

    // Auto-detect local IP if needed
    if config.sip.local_ip.is_none() {
        let bind_unspecified = config
            .sip
            .udp_bind
            .parse::<std::net::SocketAddr>()
            .map(|a| a.ip().is_unspecified())
            .unwrap_or(true);
        if bind_unspecified {
            if let Some(detected_ip) = sip_client::auto_detect_local_ip() {
                config.sip.local_ip = Some(detected_ip.clone());
                if config.media.public_ip.is_none() {
                    config.media.public_ip = Some(detected_ip);
                }
            }
        }
    }

    // 3. Validate configuration
    config::validate_config(&config)?;

    // 4. Initialize tracing
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-webrtc-gw starting"
    );

    // 5. Build forge-api AppState directly (gives us access to WebRtcManager)
    let port_pool_config =
        PortPoolConfig::new(config.media.rtp_port_start, config.media.rtp_port_end)
            .expect("Invalid port range configuration");
    let session_manager_config = SessionManagerConfig {
        port_pool_config,
        ..Default::default()
    };
    let session_manager = SessionManager::new(session_manager_config, None);
    let metrics_handle = Arc::new(MetricsHandle::init());
    let conference_bridge = Arc::new(forge_conference::ConferenceBridge::default());
    let core_event_bus = Arc::new(forge_core::EventBus::new());

    let app_state = Arc::new(AppState::new(
        session_manager,
        metrics_handle,
        conference_bridge,
        std::path::PathBuf::from("/var/lib/fcp/webrtc-gw/recordings"),
        std::path::PathBuf::from("/var/lib/fcp/webrtc-gw/prompts"),
        forge_core::config::default_ai_allowed_endpoints(),
        core_event_bus,
    ));

    // 6. Create SIP stack
    let tcp_pool = Arc::new(ConnectionPool::new());
    let (client_tx, _client_rx) = tokio::sync::mpsc::channel(1024);
    tcp_pool.set_inbound_tx(client_tx).await;

    let dispatcher =
        sip_client::create_transport_dispatcher(&config, Arc::clone(&tcp_pool)).await?;
    let transaction_manager = TransactionManager::new_with_pool(dispatcher.clone(), tcp_pool);
    let transaction_manager_ref = Arc::new(transaction_manager);

    let uac = sip_client::create_uac(&config, transaction_manager_ref, dispatcher).await?;

    // 7. Create media manager for call bridging
    let media_mgr = call_bridge::create_media_manager(&config).await?;

    // 8. Determine public IP for SDP
    let public_ip = config
        .media
        .public_ip
        .clone()
        .unwrap_or_else(|| sip_client::derive_local_ip(&config.sip.udp_bind));

    // 9. Create call bridge manager (uses the same WebRtcManager as forge-api routes)
    let bridge_mgr = Arc::new(call_bridge::CallBridgeManager::new(
        uac,
        app_state.webrtc_manager.clone(),
        media_mgr,
        public_ip,
    ));

    // Initialize cluster membership (if configured)
    #[cfg(feature = "storage-redis")]
    let _cluster_manager = if config.cluster.enabled {
        if let Some(ref url) = config.storage.redis_url {
            match fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
                url: url.clone(),
                ..Default::default()
            })
            .await
            {
                Ok(redis) => {
                    let member = fcp_cluster::ClusterMember {
                        node_id: config.service.node_id.clone(),
                        node_type: config.service.service_type.clone(),
                        http_address: config.http.bind_address,
                        sip_address: config
                            .sip
                            .udp_bind
                            .parse()
                            .unwrap_or_else(|_| "0.0.0.0:5060".parse().unwrap()),
                        sip_tls_address: None,
                        capabilities: vec!["webrtc".to_string(), "oap_session".to_string()],
                        active_calls: 0,
                        started_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        draining: false,
                        region: fcp_cluster::resolve_region(config.cluster.region.as_deref()),
                    };
                    let cm = fcp_cluster::ClusterManager::new(redis, member).with_intervals(
                        config.cluster.heartbeat_interval_secs,
                        config.cluster.heartbeat_ttl_secs,
                    );
                    let _handle = cm.start();
                    info!("Cluster membership started");
                    Some(cm)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to connect to Redis; cluster disabled");
                    None
                }
            }
        } else {
            tracing::warn!("cluster.enabled = true but redis_url is not set; cluster disabled");
            None
        }
    } else {
        None
    };

    // 10. Build combined router: forge-api routes + call control routes
    let forge_router = routes::create_router().with_state(app_state);
    let call_router = http_api::call_routes(bridge_mgr);
    let combined_router = forge_router
        .merge(call_router)
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024));

    let auth_config = AuthConfig::new(config.auth.tokens.clone());
    let rate_limiter = RateLimiter::new(
        config.http.rate_limit_requests,
        Duration::from_secs(config.http.rate_limit_window_secs),
        Vec::new(),
    );
    let middleware_stack = ServiceBuilder::new()
        .layer(Extension(rate_limiter))
        .layer(Extension(auth_config))
        .layer(axum_middleware::from_fn(middleware::auth::auth_middleware))
        .layer(axum_middleware::from_fn(
            middleware::rate_limit::rate_limit_middleware,
        ))
        .layer(middleware::tracing_layer());
    let mut app = combined_router.layer(middleware_stack);

    if config.http.enable_cors {
        if config.http.allowed_origins.is_empty() {
            info!(
                "CORS enabled with empty allowlist; browser cross-origin requests will be blocked"
            );
        } else {
            app = app.layer(middleware::cors_layer(&config.http.allowed_origins));
        }
    }

    info!(
        http = %config.http.bind_address,
        sip = %config.sip.udp_bind,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        "fcp-webrtc-gw ready"
    );
    info!(
        "  WebRTC API: http://{}/v1/webrtc/connections",
        config.http.bind_address
    );
    info!("  Call API:   http://{}/v1/calls", config.http.bind_address);
    info!("  Health:     http://{}/health", config.http.bind_address);

    // 11. Serve with graceful shutdown
    let shutdown_timeout = Duration::from_secs(config.service.shutdown_timeout_secs);
    let shutdown_notify = Arc::new(Notify::new());
    let shutdown_task = {
        let shutdown_notify = Arc::clone(&shutdown_notify);
        tokio::spawn(async move {
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

            if !shutdown_timeout.is_zero() {
                info!(
                    timeout_secs = shutdown_timeout.as_secs(),
                    "Draining connections..."
                );
            }

            shutdown_notify.notify_waiters();
        })
    };

    let server_result = if config.tls.enabled {
        let tls = tls_config(&config).await?;
        let https_addr = config.http.bind_address;
        info!("HTTPS API listening on {}", https_addr);

        let https_service = app
            .clone()
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        let https_shutdown = Arc::clone(&shutdown_notify);
        let https_server = async move {
            tokio::select! {
                res = axum_server::bind_rustls(https_addr, tls).serve(https_service) => {
                    res.map_err(|e| anyhow::anyhow!("HTTPS server error: {}", e))
                }
                _ = https_shutdown.notified() => Ok(())
            }
        };

        https_server.await
    } else {
        let listener = tokio::net::TcpListener::bind(config.http.bind_address)
            .await
            .context("Failed to bind HTTP listener")?;
        let http_service = app
            .clone()
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        let http_shutdown = Arc::clone(&shutdown_notify);
        axum::serve(listener, http_service)
            .with_graceful_shutdown(async move {
                http_shutdown.notified().await;
            })
            .await
            .map_err(|e| anyhow::anyhow!("HTTP server error: {}", e))
    };

    shutdown_task.abort();
    server_result?;

    // Shutdown cluster manager
    #[cfg(feature = "storage-redis")]
    if let Some(ref cm) = _cluster_manager {
        cm.shutdown().await;
    }

    info!("Shutdown complete");
    Ok(())
}

async fn tls_config(config: &GatewayConfig) -> Result<RustlsConfig> {
    let cert = config
        .tls
        .cert
        .clone()
        .context("tls.enabled is true but tls.cert is not configured")?;
    let key = config
        .tls
        .key
        .clone()
        .context("tls.enabled is true but tls.key is not configured")?;

    RustlsConfig::from_pem_file(cert, key)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load TLS config: {}", e))
}

fn init_tracing(config: &GatewayConfig) {
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
