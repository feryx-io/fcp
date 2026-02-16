//! fcp-conference-server — Standalone multi-party audio conference bridge.
//!
//! Accepts SIP INVITEs for configured meeting room numbers, authenticates
//! participants via PIN, mixes audio using forge-conference, and provides
//! a REST API for meeting management and runtime conference control.
//!
//! Key differences from fcp-voicemail-server:
//! - Uses ConferenceCallHandler instead of FlowCallHandler
//! - Creates ConferenceBridge (forge-conference) for multi-party audio mixing
//! - REST API manages persistent Meeting configs + runtime conference operations
//! - No flow engine — conference calls are multi-party, not sequential single-call flows

use anyhow::{Context, Result};
use clap::Parser;
use fcp_events::{EventFilter, EventSubscriber, InMemoryEventBus};
use fcp_media::{BackendType, MediaConfig};
use fcp_sbc_core::{
    CpsChecker, NoCpsLimit, TrunkCredentialProvider, TrunkHealthMonitor, TrunkRegistrar,
    TrunkRouter,
};
use fcp_sip::prelude::*;
use fcp_sip::service::{DialogService, SubscriptionService, TransactionService};
use fcp_sip::transport::ConnectionPool;
use forge_conference::ConferenceBridge;
use forge_core::{AudioCodec as ForgeAudioCodec, AudioFormat};
use forge_rtp::{PortPool, PortPoolConfig};
use forge_mixer::MixerOptions;
use sip_observe::{set_transport_metrics, TracingTransportMetrics};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

mod audio_bridge;
mod cli;
mod conference_handler;
mod health;
mod storage;

use cli::Cli;
use fcp_conference_server::config;
use config::ConferenceServerConfig;
use conference_handler::ConferenceCallHandler;

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
    if let Some(ref ip) = cli.public_ip {
        if !ip.is_empty() {
            config.sip.local_ip = Some(ip.clone());
            config.media.public_ip = Some(ip.clone());
        }
    }

    // Auto-detect local IP if not explicitly configured and bind is 0.0.0.0
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

    // 2b. Validate configuration
    config::validate_config(&config)?;

    // 3. Initialize tracing
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-conference starting"
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

    // 10. Build credential provider from trunks (if any)
    let trunk_cred_provider: Option<Arc<TrunkCredentialProvider>> = if !config.trunks.is_empty() {
        Some(Arc::new(TrunkCredentialProvider::from_trunks(
            &config.trunks,
        )))
    } else {
        None
    };

    // 11. Create UAC for outbound calls
    let uac = create_uac(
        &config,
        Arc::clone(&transaction_manager_ref),
        dispatcher,
        trunk_cred_provider,
    )
    .await?;

    // 12. Create event bus
    let event_bus = Arc::new(InMemoryEventBus::new());
    info!("Event bus initialized (in-memory)");

    // 13. Trunk infrastructure (optional)
    let trunk_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>;
    let outbound_route_provider: Option<Arc<dyn fcp_sip::OutboundRouteProvider>> =
        if !config.trunks.is_empty() {
            let health = Arc::new(TrunkHealthMonitor::new(&config.trunks));
            let cps: Arc<dyn CpsChecker> = Arc::new(NoCpsLimit);
            let router = Arc::new(TrunkRouter::new(
                config.trunks.clone(),
                Arc::clone(&health),
                cps,
            ));

            let health_clone = Arc::clone(&health);
            let trunks_clone = config.trunks.clone();
            let uac_clone = Arc::clone(&uac);
            tokio::spawn(fcp_sbc_core::run_health_probes(
                health_clone,
                trunks_clone,
                uac_clone,
            ));

            let trunk_registrar = Arc::new(TrunkRegistrar::new(&config.trunks));
            if trunk_registrar.has_registerable_trunks() {
                let (tx, rx) = tokio::sync::watch::channel(false);
                let reg = Arc::clone(&trunk_registrar);
                let reg_health = Arc::clone(&health);
                let reg_uac = Arc::clone(&uac);
                tokio::spawn(fcp_sbc_core::run_trunk_registration(
                    reg, reg_health, reg_uac, rx,
                ));
                trunk_shutdown_tx = Some(tx);
            } else {
                trunk_shutdown_tx = None;
            }

            info!(count = config.trunks.len(), "Trunk routing enabled");
            Some(router as Arc<dyn fcp_sip::OutboundRouteProvider>)
        } else {
            trunk_shutdown_tx = None;
            None
        };

    // 14. Create media manager
    let media_manager = create_media_manager(&config).await?;

    // 15. Create CallManager for B2BUA call tracking
    let call_manager = Arc::new(fcp_sip::CallManager::with_storage_and_events(
        Arc::clone(&dyn_storage),
        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
    ));

    // 16. Create RTP port pool for conference participants
    let port_pool = Arc::new(PortPool::new(
        PortPoolConfig::new(config.media.rtp_port_start, config.media.rtp_port_end)
            .context("Invalid RTP port range")?,
    ));

    // 17. Create ConferenceBridge + ConferenceCallHandler
    // Use config sample_rate for the mixer (8kHz for G.711, 16kHz for G.722 wideband)
    let audio_format = AudioFormat::new(config.conference.sample_rate, 1, ForgeAudioCodec::PCM);
    let mixer_options = MixerOptions::default();
    let bridge = Arc::new(
        ConferenceBridge::new(audio_format, config.conference.frame_size, mixer_options)
            .context("Failed to create ConferenceBridge")?,
    );

    let local_uri = create_local_uri(&config)?;
    let local_addr = config
        .sip
        .local_ip
        .clone()
        .or_else(|| config.media.public_ip.clone())
        .unwrap_or_else(|| derive_local_ip(&config.sip.udp_bind));

    // Ensure recordings directory exists
    if let Err(e) = std::fs::create_dir_all(&config.conference.recordings_dir) {
        warn!(
            path = %config.conference.recordings_dir,
            error = %e,
            "Failed to create recordings directory"
        );
    } else {
        info!(path = %config.conference.recordings_dir, "Recordings directory ready");
    }

    let conference_handler = Arc::new(ConferenceCallHandler::new(
        Arc::clone(&bridge),
        Arc::clone(&dyn_storage),
        config.conference.clone(),
        local_uri.as_str().to_string(),
        local_addr.clone(),
        Arc::clone(&port_pool),
        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
        Arc::clone(&uac),
        config.ai.clone(),
    ));

    if config.ai.enabled {
        info!(model = %config.ai.model, "AI participant support enabled");
    }

    info!(
        sample_rate = config.conference.sample_rate,
        frame_size = config.conference.frame_size,
        "ConferenceBridge created"
    );

    // 18. Create SipMessageHandler
    //     flow_handler = None (conference doesn't use flows)
    //     The conference handler is wired via the conference_handler field
    let handler = Arc::new(SipMessageHandler::new(
        Arc::clone(&transaction_manager_ref),
        Arc::clone(&dialog_manager_ref),
        local_uri,
        Some(MediaProfileBuilder::audio_only()),
        local_addr,
        config.media.rtp_port_start,
        None, // registrar = None
        None, // routing_service = None
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
        None, // flow_handler = None
        config.b2bua.clone(),
        outbound_route_provider,
        None, // No PBX handler
        Some(Arc::clone(&conference_handler) as Arc<dyn fcp_sip::conference::ConferenceHandler>),
    )?);

    // 19. Create and start TransportListener
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
        #[cfg(feature = "tls")]
        tls_bind: None,
        #[cfg(feature = "tls")]
        tls_cert: None,
        #[cfg(feature = "tls")]
        tls_key: None,
    };

    let handler_dyn: Arc<dyn InboundMessageHandler> =
        Arc::clone(&handler) as Arc<dyn InboundMessageHandler>;
    let mut listener = TransportListener::new(listener_config, handler_dyn);
    listener
        .start()
        .await
        .context("Failed to start transport listeners")?;

    // 20. Spawn TCP response routing task
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
                tracing::error!("Failed to handle inbound TCP response: {}", e);
            }
        }
    });

    // 21. Spawn HTTP server (conference API)
    let events_dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let conf_state = Arc::new(health::ConferenceState {
        bridge: Arc::clone(&bridge),
        storage: Arc::clone(&dyn_storage),
        conference_handler: Arc::clone(&conference_handler),
        call_manager: Arc::clone(&call_manager),
        event_bus: Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
        start_time: Instant::now(),
        auth_enabled: config.http.auth_enabled,
        auth_token: config.http.auth_token.clone(),
        rate_limit_rps: config.http.rate_limit_rps,
        event_subscriber: Arc::clone(&event_bus),
        events_dropped: Arc::clone(&events_dropped),
    });
    let http_bind = config.http.bind_address;
    tokio::spawn(supervise_task("http-server", async move {
        if let Err(e) = health::run_http_server(http_bind, conf_state).await {
            tracing::error!(%e, "HTTP API server exited");
        }
    }));

    // 22. Spawn recording cleanup task (if retention configured)
    if let Some(days) = config.conference.recording_retention_days {
        if days > 0 {
            let recordings_dir = config.conference.recordings_dir.clone();
            let retention_days = days;
            tokio::spawn(supervise_task("recording-cleanup", async move {
                run_recording_cleanup(recordings_dir, retention_days).await;
            }));
            info!(
                retention_days = days,
                "Recording cleanup task started"
            );
        }
    }

    // 23. Spawn config hot-reload watcher
    {
        let config_path = cli
            .config
            .as_ref()
            .map(|p| p.clone())
            .unwrap_or_else(|| std::path::PathBuf::from("fcp-conference-server.toml"));
        if config_path.exists() {
            let handler_ref = Arc::clone(&conference_handler);
            tokio::spawn(supervise_task("config-watcher", async move {
                run_config_watcher(config_path, handler_ref).await;
            }));
            info!("Config hot-reload watcher started");
        }
    }

    // 24a. Spawn session timer reaper (RFC 4028)
    if config.conference.session_expires_secs > 0 {
        let reaper_handler = Arc::clone(&conference_handler);
        let reap_interval = config.conference.session_expires_secs;
        tokio::spawn(supervise_task("session-reaper", async move {
            // Check every half the session-expires interval
            let interval = Duration::from_secs(reap_interval / 2).max(Duration::from_secs(30));
            loop {
                tokio::time::sleep(interval).await;
                reaper_handler.reap_expired_sessions().await;
            }
        }));
        info!(session_expires_secs = config.conference.session_expires_secs, "Session timer reaper started");
    }

    // 24b. Spawn recording size monitor
    if config.conference.max_recording_size_mb > 0 {
        let monitor_bridge = Arc::clone(&bridge);
        let max_bytes = config.conference.max_recording_size_mb * 1024 * 1024;
        let recordings_dir = config.conference.recordings_dir.clone();
        let monitor_event_bus = Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>;
        tokio::spawn(supervise_task("recording-size-monitor", async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                // Check each actively-recording room by scanning for .wav files
                // that are currently being written (modified in the last 60s)
                let mut dir = match tokio::fs::read_dir(&recordings_dir).await {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                while let Ok(Some(entry)) = dir.next_entry().await {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("wav") {
                        continue;
                    }
                    if let Ok(meta) = tokio::fs::metadata(&path).await {
                        if meta.len() <= max_bytes {
                            continue;
                        }
                        // File exceeds limit — find the room by filename prefix and stop recording
                        let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        // Filename format: {room}-{timestamp}-{uuid} — extract room as first segment
                        let room_id = filename.split('-').next().unwrap_or(filename);
                        if let Ok(room) = monitor_bridge.get_room(room_id) {
                            if room.is_recording() {
                                warn!(
                                    room = %room_id,
                                    size_mb = meta.len() / (1024 * 1024),
                                    max_mb = max_bytes / (1024 * 1024),
                                    "Recording size limit reached, stopping"
                                );
                                let _ = room.stop_recording();
                                let event = fcp_events::PlatformEvent::Conference(
                                    fcp_events::ConferenceEvent::RecordingStopped {
                                        room_number: room_id.to_string(),
                                        tenant_id: fcp_core::TenantId::nil(),
                                        timestamp: std::time::SystemTime::now(),
                                    },
                                );
                                let _ = monitor_event_bus.publish(event).await;
                            }
                        }
                    }
                }
            }
        }));
        info!(max_mb = config.conference.max_recording_size_mb, "Recording size monitor started");
    }

    // 24c. Spawn event forwarding task (if configured)
    if let Some(ref events_url) = config.events.server_url {
        let events_url = events_url.clone();
        let event_bus_clone = Arc::clone(&event_bus);
        let dropped_counter = Arc::clone(&events_dropped);
        tokio::spawn(supervise_task("event-forwarder", async move {
            if let Err(e) = run_event_forwarder(event_bus_clone, &events_url, dropped_counter).await {
                tracing::error!(%e, "Event forwarder task failed");
            }
        }));
        info!(
            events_url = %config.events.server_url.as_deref().unwrap_or(""),
            "Event forwarding enabled"
        );
    }

    // 23. Log startup summary
    info!(
        udp = %config.sip.udp_bind,
        tcp = %config.sip.tcp_bind,
        http = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        trunks = config.trunks.len(),
        events_forwarding = config.events.server_url.is_some(),
        "fcp-conference ready — listening for SIP INVITE messages"
    );

    // 24. Wait for shutdown signal
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

    // 25. Graceful shutdown with timeout
    // First, stop accepting new INVITEs (drain mode)
    conference_handler.start_draining();

    let shutdown_timeout = Duration::from_secs(config.service.shutdown_timeout_secs);
    info!(timeout_secs = config.service.shutdown_timeout_secs, "Shutting down conference sessions...");
    if shutdown_timeout.is_zero() {
        conference_handler.shutdown().await;
    } else {
        match tokio::time::timeout(shutdown_timeout, conference_handler.shutdown()).await {
            Ok(()) => info!("All conference sessions shut down cleanly"),
            Err(_) => warn!(
                timeout_secs = config.service.shutdown_timeout_secs,
                remaining_sessions = conference_handler.session_count(),
                "Shutdown timeout reached, force-closing remaining sessions"
            ),
        }
    }

    if let Some(tx) = trunk_shutdown_tx {
        let _ = tx.send(true);
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
    listener.stop().await?;
    info!("Shutdown complete");
    Ok(())
}

/// Create a transport dispatcher with shared TCP pool.
async fn create_transport_dispatcher(
    config: &ConferenceServerConfig,
    tcp_pool: Arc<ConnectionPool>,
) -> Result<Arc<fcp_sip::transport::RealTransportDispatcher>> {
    use fcp_sip::transport::RealTransportDispatcher;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

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

    let dispatcher = RealTransportDispatcher::new_with_pool(Some(Arc::new(udp_socket)), tcp_pool);

    Ok(Arc::new(dispatcher))
}

/// Create a UAC for outbound calls.
async fn create_uac(
    config: &ConferenceServerConfig,
    transaction_manager: Arc<TransactionManager>,
    dispatcher: Arc<fcp_sip::transport::RealTransportDispatcher>,
    credential_provider: Option<Arc<TrunkCredentialProvider>>,
) -> Result<Arc<SipUserAgentClient>> {
    let local_uri = create_local_uri(config)?;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    let local_addr = if let Some(ref local_ip) = config.sip.local_ip {
        let port = if bind_addr.port() == 0 {
            5060
        } else {
            bind_addr.port()
        };
        format!("{}:{}", local_ip, port)
    } else {
        let port = if bind_addr.port() == 0 {
            5060
        } else {
            bind_addr.port()
        };
        format!("{}:{}", bind_addr.ip(), port)
    };

    let resolver =
        Arc::new(SipResolver::from_system().context("Failed to create DNS resolver")?);

    let mut builder = IntegratedUAC::builder();
    builder = builder
        .local_uri(local_uri.as_str())
        .local_addr(&local_addr)?
        .via_advertised_addr(&local_addr)?
        .transaction_manager(transaction_manager)
        .resolver(resolver)
        .dispatcher(dispatcher);

    if let Some(cred_provider) = credential_provider {
        builder = builder.credential_provider(cred_provider);
    }

    let integrated_uac = builder.build().context("Failed to build IntegratedUAC")?;

    Ok(Arc::new(SipUserAgentClient::new(Arc::new(integrated_uac))))
}

/// Create a local SIP URI from configuration.
fn create_local_uri(config: &ConferenceServerConfig) -> Result<SipUri> {
    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    if let Some(ref uri_str) = config.sip.local_uri {
        let uri = Uri::parse(uri_str).context("Failed to parse configured local_uri")?;
        return uri
            .as_sip()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Configured local_uri is not a SIP URI"));
    }

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
        format!("sip:conference@{}:{}", domain, bind_addr.port())
    } else {
        format!("sip:conference@{}", domain)
    };

    let uri = Uri::parse(&uri_str).context("Failed to parse local URI")?;
    uri.as_sip()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

/// Create a media manager from configuration.
async fn create_media_manager(
    config: &ConferenceServerConfig,
) -> Result<Arc<fcp_media::MediaManager>> {
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
        rtp_ports = format!(
            "{}-{}",
            config.media.rtp_port_start, config.media.rtp_port_end
        ),
        "Media manager created"
    );

    Ok(Arc::new(manager))
}

fn derive_local_ip(bind_addr: &str) -> String {
    if let Ok(addr) = bind_addr.parse::<std::net::SocketAddr>() {
        if !addr.ip().is_unspecified() {
            return addr.ip().to_string();
        }
    }
    auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
}

fn auto_detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:53").ok()?;
    let addr = socket.local_addr().ok()?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    Some(addr.ip().to_string())
}

fn init_tracing(config: &ConferenceServerConfig) {
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

/// Periodically delete recordings older than the retention period.
async fn run_recording_cleanup(recordings_dir: String, retention_days: u32) {
    let retention = Duration::from_secs(retention_days as u64 * 86400);
    // Run cleanup on startup, then every hour
    loop {
        let dir = match std::fs::read_dir(&recordings_dir) {
            Ok(d) => d,
            Err(e) => {
                warn!(error = %e, "Recording cleanup: cannot read directory");
                tokio::time::sleep(Duration::from_secs(3600)).await;
                continue;
            }
        };

        let mut deleted = 0u32;
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wav") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let age = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .unwrap_or_default();
                if age > retention {
                    if let Err(e) = std::fs::remove_file(&path) {
                        warn!(path = %path.display(), error = %e, "Failed to delete old recording");
                    } else {
                        deleted += 1;
                    }
                }
            }
        }

        if deleted > 0 {
            info!(
                deleted = deleted,
                retention_days = retention_days,
                "Recording cleanup completed"
            );
        }

        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

async fn run_event_forwarder(
    event_bus: Arc<InMemoryEventBus>,
    events_url: &str,
    dropped_metric: Arc<std::sync::atomic::AtomicU64>,
) -> Result<()> {
    use std::sync::atomic::{AtomicU64, AtomicBool};

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("Failed to create HTTP client for event forwarding")?;

    let mut stream = event_bus
        .subscribe(EventFilter::new())
        .await
        .context("Failed to subscribe to event bus")?;

    info!(url = %events_url, "Event forwarder started");

    // Circuit breaker state (shared with spawned delivery tasks)
    let consecutive_failures = Arc::new(AtomicU64::new(0));
    let circuit_open = Arc::new(AtomicBool::new(false));
    let circuit_opened_at = Arc::new(tokio::sync::Mutex::new(Instant::now()));
    const FAILURE_THRESHOLD: u64 = 5;
    const COOLDOWN: Duration = Duration::from_secs(30);

    // Concurrency limiter: max 50 in-flight HTTP POST tasks
    let semaphore = Arc::new(tokio::sync::Semaphore::new(50));

    let mut delivered_events: u64 = 0;

    loop {
        match stream.next().await {
            Some(event) => {
                // Circuit breaker check
                if circuit_open.load(std::sync::atomic::Ordering::Relaxed) {
                    let opened = *circuit_opened_at.lock().await;
                    if opened.elapsed() < COOLDOWN {
                        // Circuit open — increment shared dropped metric (visible via /metrics)
                        let total_dropped = dropped_metric.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        if total_dropped % 100 == 1 {
                            warn!(
                                dropped = total_dropped,
                                cooldown_remaining_secs = (COOLDOWN - opened.elapsed()).as_secs(),
                                "Circuit breaker OPEN — dropping events"
                            );
                        }
                        continue;
                    }
                    // Cooldown expired — transition to half-open (allow one probe)
                    info!("Circuit breaker half-open, probing...");
                    circuit_open.store(false, std::sync::atomic::Ordering::Relaxed);
                    consecutive_failures.store(0, std::sync::atomic::Ordering::Relaxed);
                }

                tracing::debug!(?event, "Forwarding event to events server");
                let client = client.clone();
                let url = events_url.to_string();
                let failures = Arc::clone(&consecutive_failures);
                let cb_open = Arc::clone(&circuit_open);
                let cb_opened_at = Arc::clone(&circuit_opened_at);
                let permit = Arc::clone(&semaphore);
                let drop_counter = Arc::clone(&dropped_metric);
                tokio::spawn(async move {
                    // Acquire semaphore permit to bound concurrency
                    let _permit = match permit.acquire().await {
                        Ok(p) => p,
                        Err(_) => return, // Semaphore closed (shutdown)
                    };
                    if forward_event_with_retry(&client, &url, &event).await {
                        // Success — reset failure counter
                        failures.store(0, std::sync::atomic::Ordering::Relaxed);
                    } else {
                        // Failure — increment dropped metric and maybe trip circuit
                        drop_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let prev = failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if prev + 1 >= FAILURE_THRESHOLD
                            && !cb_open.load(std::sync::atomic::Ordering::Relaxed)
                        {
                            cb_open.store(true, std::sync::atomic::Ordering::Relaxed);
                            *cb_opened_at.lock().await = Instant::now();
                            tracing::error!(
                                consecutive_failures = prev + 1,
                                cooldown_secs = COOLDOWN.as_secs(),
                                "Circuit breaker OPENED — event forwarding paused"
                            );
                        }
                    }
                });
                delivered_events += 1;
            }
            None => {
                warn!(
                    delivered = delivered_events,
                    dropped = dropped_metric.load(std::sync::atomic::Ordering::Relaxed),
                    "Event stream ended unexpectedly, resubscribing..."
                );
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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

/// Forward a single event with exponential backoff retry (max 3 attempts).
/// Returns `true` on success, `false` if all retries exhausted.
async fn forward_event_with_retry(
    client: &reqwest::Client,
    url: &str,
    event: &fcp_events::PlatformEvent,
) -> bool {
    let max_retries = 3u32;
    let mut delay = Duration::from_millis(200);

    for attempt in 1..=max_retries {
        match client.post(url).json(event).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    return true;
                }
                tracing::warn!(
                    status = %resp.status(),
                    attempt = attempt,
                    "Event forward received non-2xx response"
                );
            }
            Err(e) => {
                tracing::warn!(
                    %e,
                    attempt = attempt,
                    max = max_retries,
                    "Failed to forward event"
                );
            }
        }

        if attempt < max_retries {
            tokio::time::sleep(delay).await;
            delay *= 2; // Exponential backoff
        }
    }

    tracing::error!("Event dropped after {} failed attempts", max_retries);
    false
}

/// Supervisor wrapper for long-running background tasks.
///
/// If the inner future panics, catches the panic, logs it, and does NOT restart
/// (the task is considered dead). This prevents silent task disappearance.
async fn supervise_task(name: &'static str, task: impl std::future::Future<Output = ()> + Send + 'static) {
    match tokio::task::spawn(task).await {
        Ok(()) => {
            warn!(task = name, "Background task exited normally");
        }
        Err(e) => {
            if e.is_panic() {
                tracing::error!(task = name, "Background task PANICKED: {}", e);
            } else {
                tracing::error!(task = name, "Background task cancelled: {}", e);
            }
        }
    }
}

/// Watch config file for changes and hot-reload conference/AI settings.
/// Polls file mtime every 10 seconds. Only reloads [conference] and [ai] sections.
async fn run_config_watcher(
    config_path: std::path::PathBuf,
    handler: Arc<ConferenceCallHandler>,
) {
    let mut last_mtime = std::fs::metadata(&config_path)
        .and_then(|m| m.modified())
        .ok();

    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;

        let current_mtime = match std::fs::metadata(&config_path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "Config watcher: cannot stat config file");
                continue;
            }
        };

        if last_mtime == Some(current_mtime) {
            continue;
        }
        last_mtime = Some(current_mtime);

        info!(path = %config_path.display(), "Config file changed, reloading...");

        match config::load_config(Some(&config_path)) {
            Ok(new_config) => {
                // Validate before applying
                if let Err(e) = config::validate_config(&new_config) {
                    warn!(error = %e, "Config reload rejected: validation failed, keeping current config");
                    continue;
                }
                handler.reload_config(new_config.conference, new_config.ai);
            }
            Err(e) => {
                warn!(error = %e, "Config reload failed, keeping current config");
            }
        }
    }
}
