//! fcp-call-manager — Combined SIP registrar and B2BUA call handler.
//!
//! Users register their devices and place/receive calls through the same binary.
//! Local user-to-user calls are handled directly via B2BUA; the architecture
//! supports future fcp-sme integration for cross-node and PSTN routing.
//!
//! This binary wires up existing fcp-sip components (SipMessageHandler,
//! TransportListener, FcpRegistrar) with storage, config, and an HTTP API.

use anyhow::{Context, Result};
use clap::Parser;
use fcp_events::{EventFilter, EventSubscriber, InMemoryEventBus};
use fcp_media::{BackendType, MediaConfig};
use fcp_routing::{RoutingService, YamlRoutingStore};
use fcp_sip::prelude::*;
use fcp_sip::service::{DialogService, SubscriptionService, TransactionService};
use fcp_sip::transport::ConnectionPool;
use sip_observe::{set_transport_metrics, TracingTransportMetrics};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

mod cli;
mod config;
mod health;
mod storage;

use cli::Cli;
use config::CallManagerConfig;

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
    if let Some(ref bind) = cli.sip_bind {
        config.sip.udp_bind = bind.clone();
        config.sip.tcp_bind = bind.clone();
    }
    if let Some(ref bind) = cli.http_bind {
        config.http.bind_address = bind
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid HTTP bind address '{}': {}", bind, e))?;
    }
    // Only apply public_ip override if it's non-empty
    // (clap treats empty env vars as Some("") instead of None)
    if let Some(ref ip) = cli.public_ip {
        if !ip.is_empty() {
            config.sip.local_ip = Some(ip.clone());
            config.media.public_ip = Some(ip.clone());
        }
    }

    // Auto-detect local IP if not explicitly configured and bind is 0.0.0.0
    // This prevents using unroutable addresses (0.0.0.0, 127.0.0.1) in SIP
    // messages when running inside Docker or binding to all interfaces.
    if config.sip.local_ip.is_none() {
        let bind_unspecified = config
            .sip
            .udp_bind
            .parse::<std::net::SocketAddr>()
            .map(|a| a.ip().is_unspecified())
            .unwrap_or(true);
        if bind_unspecified {
            if let Some(detected_ip) = auto_detect_local_ip() {
                config.sip.local_ip = Some(detected_ip.clone());
                if config.media.public_ip.is_none() {
                    config.media.public_ip = Some(detected_ip);
                }
            }
        }
    }

    // 3. Initialize tracing
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-call-manager starting"
    );

    if let Some(ref ip) = config.sip.local_ip {
        info!(
            local_ip = %ip,
            source = if cli.public_ip.as_ref().map_or(false, |p| !p.is_empty()) { "PUBLIC_IP env" } else { "auto-detected" },
            "Using local IP for SIP signaling and media"
        );
    }

    // Set up observability
    if !set_transport_metrics(Arc::new(TracingTransportMetrics)) {
        warn!("transport metrics already configured");
    }

    // 4. Initialize storage
    let dyn_storage = storage::initialize_storage(&config.storage).await?;

    // 5. Create SIP components
    let dialog_manager = DialogManager::new();
    let subscription_manager = SubscriptionManager::new();

    // 6. Create shared TCP connection pool
    let tcp_pool = Arc::new(ConnectionPool::new());

    // Set up TCP response routing
    let (client_tx, client_rx) = tokio::sync::mpsc::channel(1024);
    tcp_pool.set_inbound_tx(client_tx).await;

    // 7. Create transport dispatcher
    let dispatcher = create_transport_dispatcher(&config, Arc::clone(&tcp_pool)).await?;

    // 8. Create transaction manager with shared pool
    let transaction_manager = TransactionManager::new_with_pool(dispatcher.clone(), tcp_pool);

    // 9. Create services
    let transaction_service = TransactionService::new(transaction_manager);
    let dialog_service = DialogService::new(dialog_manager);
    let _subscription_service = SubscriptionService::new(subscription_manager);

    let transaction_manager_ref = Arc::clone(transaction_service.manager());
    let dialog_manager_ref = Arc::clone(dialog_service.manager());

    // 10. Create UAC for outbound calls (B-leg)
    let uac = create_uac(
        &config,
        Arc::clone(&transaction_manager_ref),
        dispatcher,
    )
    .await?;

    // 11. Create event bus (needed by registrar and call manager)
    let event_bus = Arc::new(InMemoryEventBus::new());
    info!("Event bus initialized (in-memory)");

    // 12. Create FcpRegistrar (with event publishing)
    let registrar = create_registrar(&config, &dyn_storage, Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>);

    // 13. Create routing service (if configured)
    let routing_service = if let Some(ref rules_file) = config.routing.rules_file {
        match YamlRoutingStore::new(PathBuf::from(rules_file), config.routing.hot_reload).await {
            Ok(store) => {
                info!(rules_file = %rules_file, hot_reload = config.routing.hot_reload, "Routing service initialized");
                Some(Arc::new(RoutingService::new(Arc::new(store))))
            }
            Err(e) => {
                warn!(%e, rules_file = %rules_file, "Failed to load routing rules, routing disabled");
                None
            }
        }
    } else {
        info!("No routing rules file configured, routing disabled");
        None
    };

    // 14. Create media manager
    let media_manager = create_media_manager(&config).await?;

    // 15. Create CallManager for B2BUA functionality (with event publishing)
    let call_manager = Arc::new(fcp_sip::CallManager::with_storage_and_events(
        Arc::clone(&dyn_storage),
        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
    ));

    // 16. Create SipMessageHandler (wires all components together)
    let local_uri = create_local_uri(&config)?;
    let media_profile = if config.calls.enable_sdp {
        Some(MediaProfileBuilder::audio_only())
    } else {
        None
    };
    let local_addr = config
        .sip
        .local_ip
        .clone()
        .or_else(|| config.media.public_ip.clone())
        .unwrap_or_else(|| derive_local_ip(&config.sip.udp_bind));

    let handler = Arc::new(SipMessageHandler::new(
        Arc::clone(&transaction_manager_ref),
        Arc::clone(&dialog_manager_ref),
        local_uri,
        media_profile,
        local_addr,
        config.media.rtp_port_start,
        Some(Arc::clone(&registrar)),
        routing_service, // routing service from config
        Arc::clone(&call_manager),
        Some(Arc::clone(&uac)),
        Some(Arc::clone(&dyn_storage)),
        Some(Arc::clone(&media_manager)),
        Some(fcp_core::config::MediaConfig {
            enabled: true,
            rtp_port_start: config.media.rtp_port_start,
            rtp_port_end: config.media.rtp_port_end,
            public_ip: config.media.public_ip.clone(),
            ..Default::default()
        }),
        None, // flow_handler — add when flow config is present
    ));

    // 16. Create and start TransportListener
    let udp_bind: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;
    let tcp_bind: std::net::SocketAddr = config
        .sip
        .tcp_bind
        .parse()
        .context("Invalid TCP bind address")?;

    let listener_config = ListenerConfig {
        udp_bind: Some(udp_bind),
        tcp_bind: Some(tcp_bind),
    };


    let handler_dyn: Arc<dyn InboundMessageHandler> =
        Arc::clone(&handler) as Arc<dyn InboundMessageHandler>;
    let mut listener = TransportListener::new(listener_config, handler_dyn);
    listener.start().await.context("Failed to start transport listeners")?;

    // 17. Spawn TCP response routing task
    let handler_clone = Arc::clone(&handler);
    let mut client_rx = client_rx;
    tokio::spawn(async move {
        while let Some(packet) = client_rx.recv().await {
            let txn_transport = match packet.transport() {
                SipTransportKind::Udp => SipTransactionTransportKind::Udp,
                SipTransportKind::Tcp => SipTransactionTransportKind::Tcp,
                SipTransportKind::Tls => SipTransactionTransportKind::Tls,
                SipTransportKind::Ws => SipTransactionTransportKind::Ws,
                SipTransportKind::Wss => SipTransactionTransportKind::Wss,
                SipTransportKind::Sctp => SipTransactionTransportKind::Sctp,
                SipTransportKind::TlsSctp => SipTransactionTransportKind::TlsSctp,
            };

            if let Err(e) = handler_clone
                .handle_inbound(
                    txn_transport,
                    packet.peer(),
                    packet.payload().clone(),
                    packet.stream().cloned(),
                )
                .await
            {
                tracing::error!("Failed to handle outbound TCP response: {}", e);
            }
        }
    });

    // 18. Spawn HTTP health/status/calls server
    let app_state = Arc::new(health::AppState {
        call_manager: Arc::clone(&call_manager),
        storage: Arc::clone(&dyn_storage),
        start_time: Instant::now(),
    });
    let http_bind = config.http.bind_address;
    tokio::spawn(async move {
        if let Err(e) = health::run_http_server(http_bind, app_state).await {
            tracing::error!(%e, "HTTP API server exited");
        }
    });

    // 19. Spawn registration cleanup task
    let cleanup_storage: Arc<dyn fcp_core::RegistrationStore> =
        Arc::clone(&dyn_storage) as _;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            match cleanup_storage.cleanup_expired().await {
                Ok(count) => {
                    if count > 0 {
                        info!(count, "Cleaned up expired registrations");
                    }
                }
                Err(e) => {
                    warn!(%e, "Failed to cleanup expired registrations");
                }
            }
        }
    });

    // 20. Spawn event forwarding task (if configured)
    if let Some(ref events_url) = config.events.server_url {
        let events_url = events_url.clone();
        let event_bus_clone = Arc::clone(&event_bus);
        tokio::spawn(async move {
            if let Err(e) = run_event_forwarder(event_bus_clone, &events_url).await {
                tracing::error!(%e, "Event forwarder task failed");
            }
        });
        info!(events_url = %config.events.server_url.as_deref().unwrap_or(""), "Event forwarding enabled");
    }

    info!(
        udp = %config.sip.udp_bind,
        tcp = %config.sip.tcp_bind,
        http = %config.http.bind_address,
        auth = config.registrar.auth_enabled,
        realm = %config.registrar.realm,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        routing = config.routing.rules_file.is_some(),
        events_forwarding = config.events.server_url.is_some(),
        "fcp-call-manager ready — listening for SIP messages"
    );

    // 21. Wait for shutdown signal
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to register SIGTERM handler");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, initiating graceful shutdown...");
        }
    }

    // 21. Graceful shutdown
    listener.stop().await?;
    info!("Shutdown complete");
    Ok(())
}

/// Create the FcpRegistrar from configuration and storage.
fn create_registrar(
    config: &CallManagerConfig,
    storage: &Arc<fcp_core::DynStorage>,
    event_publisher: Arc<dyn fcp_events::EventPublisher>,
) -> Arc<fcp_sip::registrar::FcpRegistrar> {
    let registration_store: Arc<dyn fcp_core::RegistrationStore> = Arc::clone(storage) as _;
    let credential_store: Option<Arc<dyn fcp_core::CredentialStore>> =
        if config.registrar.auth_enabled {
            Some(Arc::clone(storage) as _)
        } else {
            None
        };
    let realm = Some(config.registrar.realm.clone());

    Arc::new(fcp_sip::registrar::FcpRegistrar::with_events(
        registration_store,
        credential_store,
        realm,
        event_publisher,
    ))
}

/// Create a transport dispatcher with shared TCP pool.
async fn create_transport_dispatcher(
    config: &CallManagerConfig,
    tcp_pool: Arc<ConnectionPool>,
) -> Result<Arc<fcp_sip::transport::RealTransportDispatcher>> {
    use fcp_sip::transport::RealTransportDispatcher;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    // Create UDP socket with SO_REUSEADDR
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .context("Failed to create UDP socket")?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&bind_addr.into())?;
    let std_socket: std::net::UdpSocket = socket.into();
    let udp_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    let dispatcher = RealTransportDispatcher::new_with_pool(
        Some(Arc::new(udp_socket)),
        tcp_pool,
    );

    Ok(Arc::new(dispatcher))
}

/// Create a UAC for outbound calls (B-leg).
async fn create_uac(
    config: &CallManagerConfig,
    transaction_manager: Arc<TransactionManager>,
    dispatcher: Arc<fcp_sip::transport::RealTransportDispatcher>,
) -> Result<Arc<SipUserAgentClient>> {
    let local_uri = create_local_uri(config)?;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    let local_addr = if let Some(ref local_ip) = config.sip.local_ip {
        let port = if bind_addr.port() == 0 { 5060 } else { bind_addr.port() };
        format!("{}:{}", local_ip, port)
    } else {
        let port = if bind_addr.port() == 0 { 5060 } else { bind_addr.port() };
        format!("{}:{}", bind_addr.ip(), port)
    };

    let resolver = Arc::new(
        SipResolver::from_system().context("Failed to create DNS resolver")?,
    );

    let integrated_uac = IntegratedUAC::builder()
        .local_uri(local_uri.as_str())
        .local_addr(&local_addr)?
        .via_advertised_addr(&local_addr)?
        .transaction_manager(transaction_manager)
        .resolver(resolver)
        .dispatcher(dispatcher)
        .build()
        .context("Failed to build IntegratedUAC")?;

    Ok(Arc::new(SipUserAgentClient::new(Arc::new(integrated_uac))))
}

/// Create a local SIP URI from configuration.
fn create_local_uri(config: &CallManagerConfig) -> Result<SipUri> {
    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    // Use explicit local_uri if configured
    if let Some(ref uri_str) = config.sip.local_uri {
        let uri = Uri::parse(uri_str).context("Failed to parse configured local_uri")?;
        return uri
            .as_sip()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Configured local_uri is not a SIP URI"));
    }

    // Build from local_ip or bind address
    let domain = if bind_addr.ip().is_unspecified() {
        config
            .sip
            .local_ip
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind_addr.ip().to_string()
    };

    let uri_str = if bind_addr.port() != 0 && bind_addr.port() != 5060 {
        format!("sip:call-manager@{}:{}", domain, bind_addr.port())
    } else {
        format!("sip:call-manager@{}", domain)
    };

    let uri = Uri::parse(&uri_str).context("Failed to parse local URI")?;
    uri.as_sip()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

/// Create a media manager from configuration.
async fn create_media_manager(config: &CallManagerConfig) -> Result<Arc<fcp_media::MediaManager>> {
    let media_config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: config.media.rtp_port_start,
        rtp_port_end: config.media.rtp_port_end,
        public_ip: config.media.public_ip.clone(),
        ..Default::default()
    };

    let manager = fcp_media::MediaManager::new(media_config)
        .await
        .context("Failed to create media manager")?;

    info!(
        backend = manager.backend_name(),
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        "Media manager created"
    );

    Ok(Arc::new(manager))
}

/// Derive local IP from bind address string.
///
/// If the bind address is unspecified (0.0.0.0), falls back to auto-detection.
fn derive_local_ip(bind_addr: &str) -> String {
    if let Ok(addr) = bind_addr.parse::<std::net::SocketAddr>() {
        if !addr.ip().is_unspecified() {
            return addr.ip().to_string();
        }
    }
    auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Auto-detect the local LAN IP by asking the OS which interface routes to the internet.
///
/// Opens a UDP socket to a well-known address (no actual traffic is sent) and checks
/// which local address the OS selected. This is the standard technique for determining
/// the machine's routable IP address.
fn auto_detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // connect() on a UDP socket doesn't send anything — it just sets the
    // default destination so the OS selects the appropriate source address.
    socket.connect("8.8.8.8:53").ok()?;
    let addr = socket.local_addr().ok()?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    Some(addr.ip().to_string())
}

/// Initialize tracing subscriber based on configuration.
fn init_tracing(config: &CallManagerConfig) {
    use tracing_subscriber::EnvFilter;

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.logging.level));

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
            // "pretty" or anything else
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .with_env_filter(env_filter)
                .init();
        }
    }
}

/// Run the event forwarder task that subscribes to local events and POSTs them to fcp-events-server.
async fn run_event_forwarder(
    event_bus: Arc<InMemoryEventBus>,
    events_url: &str,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("Failed to create HTTP client for event forwarding")?;

    // Subscribe to all events
    let mut stream = event_bus
        .subscribe(EventFilter::new())
        .await
        .context("Failed to subscribe to event bus")?;

    info!(url = %events_url, "Event forwarder started");

    loop {
        match stream.next().await {
            Some(event) => {
                tracing::debug!(?event, "Forwarding event to events server");
                // Fire-and-forget POST to events server
                let client = client.clone();
                let url = events_url.to_string();
                tokio::spawn(async move {
                    match client.post(&url).json(&event).send().await {
                        Ok(resp) => {
                            if resp.status().is_success() {
                                tracing::debug!(status = %resp.status(), "Event forwarded");
                            } else {
                                tracing::warn!(
                                    status = %resp.status(),
                                    "Event forward received non-2xx response"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(%e, "Failed to forward event to events server");
                        }
                    }
                });
            }
            None => {
                warn!("Event stream ended unexpectedly, resubscribing...");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                // Try to resubscribe
                match event_bus.subscribe(EventFilter::new()).await {
                    Ok(new_stream) => {
                        stream = new_stream;
                        info!("Resubscribed to event bus");
                    }
                    Err(e) => {
                        warn!(%e, "Failed to resubscribe to event bus");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}
