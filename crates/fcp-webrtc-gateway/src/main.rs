//! fcp-webrtc-gw — WebRTC-to-SIP gateway.
//!
//! Puts browsers in conference rooms and on SIP calls: each session owns a
//! WebRTC peer connection (forge-webrtc) and a SIP leg (siphon-rs
//! IntegratedUAC) and relays media between them — audio in the codec both
//! sides negotiated (G.722 or G.711, no transcoding), video packets, and
//! RTCP feedback.
//!
//! The gateway exposes a unified HTTP API:
//! - `/v1/rooms/{room}/join`, `/v1/rooms/*` — room joins (this binary)
//! - `/v1/calls/*` — SIP calls (this binary)
//! - `/v1/webrtc/connections/{id}/answer`, `/ice-candidate` — the browser's
//!   half of a session's signalling (from forge-api)
//! - `/health`, `/v1/sessions`, `/v1/metrics` — from forge-api
//! - `/ready`, `/status`, `/drain` — leaving service
//!   (`docs/CAPACITY_AND_SURVIVABILITY.md` §6)

use anyhow::{Context, Result};
use axum::middleware as axum_middleware;
use axum::Extension;
use axum_server::tls_rustls::RustlsConfig;
use clap::Parser;
use fcp_sip::listener::{InboundMessageHandler, ListenerConfig, TransportListener};
use fcp_sip::prelude::*;
use fcp_sip::transport::ConnectionPool;
use fcp_webrtc_gateway::config::{self, GatewayConfig};
use fcp_webrtc_gateway::inbound::GatewayInboundHandler;
use fcp_webrtc_gateway::placement::ConferencePlacement;
use fcp_webrtc_gateway::session::{SessionConfig, SessionManager};
use fcp_webrtc_gateway::{http_api, sip_client};
use forge_api::middleware::{self, auth::AuthConfig, RateLimiter};
use forge_api::routes;
use forge_api::routes::prometheus::MetricsHandle;
use forge_api::routes::sessions::AppState;
use forge_engine::{SessionManager as ForgeSessionManager, SessionManagerConfig};
use forge_rtp::{PortPool, PortPoolConfig};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tower::ServiceBuilder;
use tracing::info;

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

    // 5. Build forge-api AppState directly (gives us access to WebRtcManager).
    //    The media port range is split: the lower half feeds the relay (ICE
    //    sockets and SIP-side RTP), the upper half forge-api's own media
    //    sessions, so the two pools never bind the same port.
    let (relay_ports, forge_ports) = split_port_range(&config)?;
    let relay_range = format!("{}-{}", relay_ports.min_port, relay_ports.max_port);
    let forge_range = format!("{}-{}", forge_ports.min_port, forge_ports.max_port);
    let session_manager_config = SessionManagerConfig {
        port_pool_config: forge_ports,
        ..Default::default()
    };
    let session_manager = ForgeSessionManager::new(session_manager_config, None);
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

    let uac = sip_client::create_uac(&config, transaction_manager_ref.clone(), dispatcher).await?;

    // 7. Determine public IP for SDP
    let public_ip = config
        .media
        .public_ip
        .clone()
        .unwrap_or_else(|| sip_client::derive_local_ip(&config.sip.udp_bind));

    // 8. Cluster membership and conference placement (if configured)
    #[cfg(feature = "storage-redis")]
    let mut cluster_placement = None;
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
                        capacity: None,
                        draining: false,
                        // Where a draining peer sends its browsers
                        // (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
                        public_url: config.http.public_url.clone(),
                        region: fcp_cluster::resolve_region(config.cluster.region.as_deref()),
                        video_budget: Vec::new(),
                        registering_trunks: Vec::new(),
                    };
                    let cm = Arc::new(
                        fcp_cluster::ClusterManager::new(redis.clone(), member).with_intervals(
                            config.cluster.heartbeat_interval_secs,
                            config.cluster.heartbeat_ttl_secs,
                        ),
                    );
                    let _handle = cm.start();
                    info!("Cluster membership started");
                    cluster_placement = Some(fcp_webrtc_gateway::placement::ClusterPlacement {
                        registry: fcp_cluster::ConferenceRegistry::new(
                            redis,
                            config.service.node_id.clone(),
                        ),
                        members: Arc::clone(cm.members_map()),
                        capacity: config.conference.node_capacity,
                        region: cm.region().map(str::to_owned),
                        tenant_id: config.conference.tenant_id.clone(),
                    });
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

    let placement = {
        let server_uri = config.conference.server_uri.as_deref();
        #[cfg(feature = "storage-redis")]
        let placement = match cluster_placement.take() {
            Some(cluster) => ConferencePlacement::with_cluster(server_uri, cluster),
            None => ConferencePlacement::static_only(server_uri),
        };
        #[cfg(not(feature = "storage-redis"))]
        let placement = ConferencePlacement::static_only(server_uri);
        Arc::new(placement)
    };
    if !placement.is_configured() {
        tracing::warn!(
            "no conference server configured ([conference] server_uri or a cluster); room joins will fail"
        );
    }

    // 9. Sessions: the peer connections, SIP legs and relays.
    let session_cfg = SessionConfig {
        public_ip: public_ip.clone(),
        local_uri: sip_client::local_uri_string(&config),
        stun_servers: config.webrtc.stun_servers.clone(),
        turn_servers: config
            .webrtc
            .turn_servers
            .iter()
            .map(|t| forge_webrtc::TurnServer::new(&t.uri, &t.username, &t.password))
            .collect(),
        gather_timeout: Duration::from_millis(config.webrtc.gather_timeout_ms),
        ice_timeout: Duration::from_secs(config.webrtc.ice_timeout_secs),
        invite_timeout: Duration::from_secs(config.conference.invite_timeout_secs),
        inactivity_timeout: Duration::from_secs(config.webrtc.inactivity_timeout_secs),
        srtp: config.conference.srtp,
        browser_ice_servers: config
            .webrtc
            .browser_ice_servers
            .iter()
            .map(|s| fcp_webrtc_gateway::session::IceServer {
                urls: s.urls.clone(),
                username: s.username.clone(),
                credential: s.credential.clone(),
            })
            .collect(),
        outbound_proxy: config.sip.outbound_proxy.clone(),
        emergency: config.e911.classifier(),
    };
    // What this node accepts, and what it refuses
    // (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). Every limit is optional:
    // a gateway that writes no `[admission]` section keeps none, as it
    // always did.
    let admission = Arc::new(fcp_sbc_core::AdmissionController::new(
        config.admission.resolve(),
    ));
    // Where a browser is sent when this node leaves service (§6): a
    // serving gateway from the cluster, or nowhere in particular, which
    // tells the browser to come back where it is.
    #[cfg(feature = "storage-redis")]
    let handover: Arc<dyn fcp_webrtc_gateway::session::HandoverAdvice> =
        match _cluster_manager.as_ref() {
            Some(cm) => Arc::new(fcp_webrtc_gateway::drain::ClusterHandover::new(Arc::clone(
                cm,
            ))),
            None => Arc::new(fcp_webrtc_gateway::drain::NoHandover),
        };
    #[cfg(not(feature = "storage-redis"))]
    let handover: Arc<dyn fcp_webrtc_gateway::session::HandoverAdvice> =
        Arc::new(fcp_webrtc_gateway::drain::NoHandover);

    let mut session_manager = SessionManager::new(
        uac,
        app_state.webrtc_manager.clone(),
        PortPool::new(relay_ports),
        placement,
        session_cfg,
    )
    .with_admission(Arc::clone(&admission) as Arc<dyn fcp_sip::admission::NodeAdmission>)
    .with_handover(handover);
    // Session records (docs/CDR_PIPELINE.md): through the spool to the
    // Redis stream, the only sink a node without a database has.
    #[cfg(feature = "storage-redis")]
    if let Some(writer) = start_cdr(&config).await? {
        session_manager = session_manager.with_cdr(fcp_webrtc_gateway::session::CdrSettings {
            writer,
            node_id: config.service.node_id.clone(),
            region: config
                .cluster
                .region
                .clone()
                .or_else(|| std::env::var("FCP_REGION").ok()),
            default_tenant: config
                .conference
                .tenant_id
                .as_deref()
                .and_then(|t| t.parse().ok()),
        });
    }
    let sessions = Arc::new(session_manager);

    // How this node leaves service (`docs/CAPACITY_AND_SURVIVABILITY.md`
    // §6): the same sequence whether it starts at `POST /drain` or at
    // SIGTERM. The browsers in rooms are told where to go as they poll;
    // whatever is left at the deadline is hung up.
    #[cfg(feature = "storage-redis")]
    let announcer =
        fcp_sbc_core::ClusterAnnouncer::maybe(_cluster_manager.as_ref().map(Arc::clone));
    #[cfg(not(feature = "storage-redis"))]
    let announcer: Option<Arc<dyn fcp_sip::drain::DrainAnnouncer>> = None;
    let drain = Arc::new(fcp_sip::drain::Drain::new(
        Arc::clone(&admission) as Arc<dyn fcp_sip::admission::NodeAdmission>,
        Arc::new(fcp_webrtc_gateway::drain::GatewaySessions::new(
            Arc::clone(&sessions),
            announcer,
        )) as Arc<dyn fcp_sip::drain::DrainTarget>,
        Arc::new(fcp_events::InMemoryEventBus::new()),
        config.service.node_id.clone(),
        config.service.drain_wait_secs,
    ));

    // What this node is carrying, in the heartbeat: a draining peer picks
    // the emptiest gateway to hand its browsers to, and conference
    // placement reads the same field
    // (`docs/CAPACITY_AND_SURVIVABILITY.md` §7).
    #[cfg(feature = "storage-redis")]
    if let Some(cm) = _cluster_manager.as_ref().map(Arc::clone) {
        let sessions = Arc::clone(&sessions);
        let every = Duration::from_secs(config.cluster.heartbeat_interval_secs.max(1));
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                cm.update_active_calls(sessions.live_count().await);
            }
        });
    }

    // 10. Inbound SIP: responses to our INVITEs, BYE from the far end.
    let sip_bind: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid SIP UDP bind address")?;
    let inbound: Arc<dyn InboundMessageHandler> = Arc::new(GatewayInboundHandler::new(
        transaction_manager_ref,
        Arc::clone(&sessions),
    ));
    let mut sip_listener = TransportListener::new(
        ListenerConfig {
            udp_bind: Some(sip_bind),
            tcp_bind: None,
            tls_bind: None,
            tls_cert: None,
            tls_key: None,
            tls_client_ca: None,
            tls_client_auth: None,
            ws_bind: None,
            wss_bind: None,
        },
        inbound,
    );
    sip_listener
        .start()
        .await
        .context("Failed to start the SIP listener")?;

    // 11. Build combined router: forge-api's routes behind its operator
    //     auth; the gateway's own routes behind theirs (operator or session
    //     token, the join page open); rate limiting and tracing over both.
    let auth_config = AuthConfig::new(config.auth.tokens.clone());
    let forge_router = routes::create_router().with_state(app_state).layer(
        ServiceBuilder::new()
            .layer(Extension(auth_config))
            .layer(axum_middleware::from_fn(middleware::auth::auth_middleware)),
    );
    let gateway_router = http_api::routes(http_api::ApiState {
        sessions: Arc::clone(&sessions),
        drain: Arc::clone(&drain),
        operator_tokens: Arc::new(config.auth.tokens.clone()),
        trusted_proxies: Arc::new(config.http.trusted_proxies.clone()),
        public_join: config.http.public_join,
        serve_join_page: config.http.serve_join_page,
    });
    let combined_router = forge_router
        .merge(gateway_router)
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024));

    let rate_limiter = RateLimiter::new(
        config.http.rate_limit_requests,
        Duration::from_secs(config.http.rate_limit_window_secs),
        Vec::new(),
    );
    let middleware_stack = ServiceBuilder::new()
        .layer(Extension(rate_limiter))
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
        public_ip = %public_ip,
        relay_ports = %relay_range,
        forge_ports = %forge_range,
        conference = ?config.conference.server_uri,
        srtp = config.conference.srtp,
        "fcp-webrtc-gw ready"
    );
    info!(
        "  Rooms API:  http://{}/v1/rooms/{{room}}/join",
        config.http.bind_address
    );
    if config.http.serve_join_page {
        info!("  Join page:  http://{}/join", config.http.bind_address);
    }
    info!("  Calls API:  http://{}/v1/calls", config.http.bind_address);
    info!(
        "  WebRTC API: http://{}/v1/webrtc/connections/{{id}}/answer",
        config.http.bind_address
    );
    info!("  Health:     http://{}/health", config.http.bind_address);
    info!(
        "  Ready:      http://{}/ready  (503 while draining)",
        config.http.bind_address
    );

    // 12. Serve with graceful shutdown
    let shutdown_timeout = Duration::from_secs(config.service.shutdown_timeout_secs);
    let shutdown_notify = Arc::new(Notify::new());
    let shutdown_task = {
        let shutdown_notify = Arc::clone(&shutdown_notify);
        let drain = Arc::clone(&drain);
        let drain_wait_secs = config.service.drain_wait_secs;
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

            // Leave service before the HTTP server stops
            // (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): take no new
            // sessions, and let the browsers in rooms read — from the
            // status they are already polling — that they should join
            // another node. That polling is why the drain runs while the
            // server is still answering. With `drain_wait_secs = 0` —
            // the default — this is what it always was.
            if drain_wait_secs > 0 || drain.is_draining() {
                fcp_sip::drain::drain_for_shutdown(&drain).await;
            } else {
                drain.start().await;
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

    sip_listener.stop().await.ok();

    // Shutdown cluster manager
    #[cfg(feature = "storage-redis")]
    if let Some(ref cm) = _cluster_manager {
        cm.shutdown().await;
    }

    // The last sessions' records leave before we do.
    if let Some(writer) = sessions.cdr_writer() {
        writer.flush(Duration::from_secs(5)).await;
    }
    info!("Shutdown complete");
    Ok(())
}

/// The media port range in two halves: the relay's (ICE sockets and
/// SIP-side RTP) and forge-api's media sessions'.
fn split_port_range(config: &GatewayConfig) -> Result<(PortPoolConfig, PortPoolConfig)> {
    let start = config.media.rtp_port_start;
    let end = config.media.rtp_port_end;
    let mid = start + (end - start) / 2;
    let mid = mid & !1;
    let relay = PortPoolConfig::new(start & !1, mid - 1)
        .map_err(|e| anyhow::anyhow!("relay media port range: {e}"))?;
    let forge = PortPoolConfig::new(mid, end)
        .map_err(|e| anyhow::anyhow!("forge-api media port range: {e}"))?;
    Ok((relay, forge))
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

/// The gateway's CDR writer, when `[cdr]` asks for one: a spool shipping
/// to the Redis stream. Anything else is refused with a warning, since the
/// gateway has no database to store records itself.
#[cfg(feature = "storage-redis")]
async fn start_cdr(
    config: &fcp_webrtc_gateway::config::GatewayConfig,
) -> anyhow::Result<Option<fcp_sip::CdrWriter>> {
    let cdr = &config.cdr;
    if cdr.spool_dir.is_none() {
        return Ok(None);
    }
    if cdr.sink != fcp_cdr::SinkKind::Stream {
        tracing::warn!("[cdr] spool_dir is set but sink is not \"stream\": the gateway has no database, so it writes no session records");
        return Ok(None);
    }
    let Some(url) = &config.storage.redis_url else {
        tracing::warn!("[cdr] sink = \"stream\" needs storage.redis_url; no session records");
        return Ok(None);
    };
    let redis = fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
        url: url.clone(),
        ..Default::default()
    })
    .await
    .context("Failed to connect to Redis for the CDR stream")?;
    let stream: std::sync::Arc<dyn fcp_cdr::sink::Sink> =
        std::sync::Arc::new(fcp_storage_redis::cdr_stream::RedisCdrStream::new(
            redis,
            config.service.node_id.clone(),
            Duration::from_secs(24 * 3600),
        ));
    let writer =
        fcp_cdr::start_node(cdr, None, Some(stream)).context("Failed to start the CDR spool")?;
    Ok(Some(writer))
}
