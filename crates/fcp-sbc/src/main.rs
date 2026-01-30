//! fcp-sbc — Session Border Controller for the Ferrous Communications Platform.
//!
//! Enforces IP ACLs, message size limits, rate limiting, and admission control
//! before delegating to the inner B2BUA SipMessageHandler. Supports trunk
//! routing, health monitoring, and topology hiding.

use anyhow::{Context, Result};
use clap::Parser;
use fcp_sip::prelude::*;
use fcp_sip::registrar::FcpRegistrar;
use fcp_sip::service::{DialogService, TransactionService};
use fcp_sip::transport::ConnectionPool;
use sip_observe::{set_transport_metrics, TracingTransportMetrics};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

mod admission;
mod cli;
mod config;
mod enforcement;
mod health;
mod health_probes;
mod hooks;
mod metrics;
mod registration_store;
mod topology;
mod trunk_health;
mod trunk_router;

use admission::AdmissionController;
use cli::Cli;
use enforcement::SbcEnforcementHandler;
use health::HealthState;
use hooks::SbcEventHooks;
use metrics::SbcRuntimeMetrics;
use registration_store::InMemoryRegistrationStore;
use trunk_health::TrunkHealthMonitor;
use trunk_router::TrunkRouter;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI arguments
    let cli = Cli::parse();

    // 2. Load configuration from TOML (or defaults)
    let sbc_config = config::load_config(cli.config.as_deref())?;

    // 3. Initialize tracing subscriber
    init_tracing(&cli.log_level, &cli.log_format);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        bind = %cli.bind,
        health_bind = %cli.health_bind,
        "fcp-sbc starting"
    );

    // 4. Validate config (best-effort; warn but don't abort if external_domain empty)
    if let Err(e) = sbc_config.validate() {
        warn!(%e, "SBC configuration validation warning");
    }

    // 5. Create UDP socket with SO_REUSEADDR
    let bind_addr: std::net::SocketAddr = cli.bind.parse().context("Invalid bind address")?;

    let domain = if bind_addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };

    let socket = socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
        .context("Failed to create UDP socket")?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;

    // For IPv6: dual-stack when binding to [::], v6-only for specific addresses
    if bind_addr.is_ipv6() {
        let v6_only = !bind_addr.ip().is_unspecified();
        socket.set_only_v6(v6_only)?;
    }

    socket.bind(&bind_addr.into())?;
    let std_socket: std::net::UdpSocket = socket.into();
    let udp_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    // 6. Create TCP ConnectionPool
    let tcp_pool = Arc::new(ConnectionPool::new());
    let (client_tx, client_rx) = tokio::sync::mpsc::channel(1024);
    tcp_pool.set_inbound_tx(client_tx).await;

    // 7. Create RealTransportDispatcher
    let dispatcher = Arc::new(fcp_sip::transport::RealTransportDispatcher::new_with_pool(
        Some(Arc::new(udp_socket)),
        Arc::clone(&tcp_pool),
    ));

    // Set up transport metrics
    if !set_transport_metrics(Arc::new(TracingTransportMetrics)) {
        warn!("transport metrics already configured");
    }

    // 8. Create TransactionManager
    let transaction_manager = TransactionManager::new_with_pool(dispatcher.clone(), tcp_pool);
    let transaction_service = TransactionService::new(transaction_manager);
    let transaction_manager_ref = Arc::clone(transaction_service.manager());

    // 9. Create DialogManager
    let dialog_manager = DialogManager::new();
    let dialog_service = DialogService::new(dialog_manager);
    let dialog_manager_ref = Arc::clone(dialog_service.manager());

    // 10. Create UAC
    let local_addr = derive_local_addr(&cli.bind);
    let local_uri = create_local_uri(&cli.bind)?;

    let resolver = Arc::new(SipResolver::from_system().context("Failed to create DNS resolver")?);
    let integrated_uac = IntegratedUAC::builder()
        .local_uri(local_uri.as_str())
        .local_addr(&local_addr)?
        .via_advertised_addr(&local_addr)?
        .transaction_manager(Arc::clone(&transaction_manager_ref))
        .resolver(resolver)
        .dispatcher(dispatcher)
        .build()
        .context("Failed to build IntegratedUAC")?;
    let uac = Arc::new(SipUserAgentClient::new(Arc::new(integrated_uac)));

    // 11. Create AdmissionController, TrunkHealthMonitor, SbcRuntimeMetrics FIRST
    let admission = Arc::new(AdmissionController::new(sbc_config.admission.clone()));
    let trunk_health = Arc::new(TrunkHealthMonitor::new(&sbc_config.trunks));
    let sbc_metrics = Arc::new(SbcRuntimeMetrics::new());

    // 12. Create event hooks so call/registration lifecycle updates admission/metrics
    let sbc_hooks: Arc<dyn fcp_events::EventPublisher> = Arc::new(SbcEventHooks::new(
        Arc::clone(&admission),
        Arc::clone(&sbc_metrics),
    ));
    let call_manager = Arc::new(fcp_sip::CallManager::with_events(Arc::clone(&sbc_hooks)));

    // 12b. Create registrar with event publishing so registration lifecycle
    //      updates admission counters (registration_added / registration_removed)
    let reg_store: Arc<dyn fcp_core::RegistrationStore> =
        Arc::new(InMemoryRegistrationStore::default());
    let registrar = Arc::new(FcpRegistrar::with_events(
        reg_store,
        None, // no credential store — SBC does not authenticate registrations itself
        Some(sbc_config.external_domain.clone()),
        sbc_hooks,
    ));

    // 13. Create TrunkRouter (built but not integrated into handler in v1)
    let _trunk_router = TrunkRouter::new(sbc_config.trunks.clone(), Arc::clone(&trunk_health));

    // 14. Create RateLimiterSet from sip-ratelimit
    let rate_limiter = Arc::new(sip_ratelimit::RateLimiterSet::new());

    // 15. Create SipMessageHandler (with B2buaConfig from SbcConfig)
    let local_ip = derive_local_ip(&cli.bind);
    let media_profile = Some(MediaProfileBuilder::audio_only());
    let handler = Arc::new(SipMessageHandler::new(
        Arc::clone(&transaction_manager_ref),
        Arc::clone(&dialog_manager_ref),
        local_uri,
        media_profile,
        local_ip,
        10000, // RTP port start
        Some(registrar),
        None, // no routing service
        Arc::clone(&call_manager),
        Some(Arc::clone(&uac)),
        None, // no storage
        None, // no media manager
        None, // no media config
        None, // no flow handler
        sbc_config.b2bua.clone(),
    ));

    // 19. Create SbcEnforcementHandler (wraps SipMessageHandler)
    let enforcement_handler = Arc::new(SbcEnforcementHandler::new(
        Arc::clone(&handler),
        sbc_config.security.clone(),
        Arc::clone(&rate_limiter),
        Arc::clone(&admission),
        Arc::clone(&sbc_metrics),
    ));

    // 20. Create TransportListener (wired to SbcEnforcementHandler)
    let listener_config = ListenerConfig {
        udp_bind: Some(bind_addr),
        tcp_bind: Some(bind_addr),
        #[cfg(feature = "tls")]
        tls_bind: None,
        #[cfg(feature = "tls")]
        tls_cert: None,
        #[cfg(feature = "tls")]
        tls_key: None,
    };
    let handler_dyn: Arc<dyn InboundMessageHandler> =
        enforcement_handler as Arc<dyn InboundMessageHandler>;
    let mut listener = TransportListener::new(listener_config, handler_dyn);
    listener
        .start()
        .await
        .context("Failed to start transport listeners")?;

    // Spawn TCP response routing task
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
                tracing::error!("Failed to handle TCP response: {}", e);
            }
        }
    });

    // 21. Spawn health HTTP server
    let health_bind: std::net::SocketAddr = cli
        .health_bind
        .parse()
        .context("Invalid health bind address")?;
    let health_state = Arc::new(HealthState {
        admission: Arc::clone(&admission),
        health_monitor: Arc::clone(&trunk_health),
        metrics: Arc::clone(&sbc_metrics),
    });
    tokio::spawn(async move {
        if let Err(e) = health::run_health_server(health_bind, health_state).await {
            tracing::error!(%e, "SBC health server exited");
        }
    });

    // 22. Spawn metrics snapshot background task
    let metrics_admission = Arc::clone(&admission);
    let metrics_ref = Arc::clone(&sbc_metrics);
    let snapshot_interval = sbc_config.metrics.snapshot_interval_secs;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(snapshot_interval));
        loop {
            interval.tick().await;
            let cps = metrics_admission.current_cps();
            let calls = metrics_admission.concurrent_calls();
            let regs = metrics_admission.concurrent_registrations();
            metrics_ref.update_snapshot(cps, calls, regs);
        }
    });

    // Spawn trunk health OPTIONS probe task
    let probe_monitor = Arc::clone(&trunk_health);
    let probe_uac = Arc::clone(&uac);
    let probe_trunks = sbc_config.trunks.clone();
    tokio::spawn(async move {
        health_probes::run_health_probes(probe_monitor, probe_trunks, probe_uac).await;
    });

    info!(
        udp = %cli.bind,
        tcp = %cli.bind,
        health = %cli.health_bind,
        trunks = sbc_config.trunks.len(),
        "fcp-sbc ready — listening for SIP messages"
    );

    // 23. Wait for shutdown signal
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

    // 24. Graceful shutdown
    listener.stop().await?;
    info!("Shutdown complete");
    Ok(())
}

/// Initialize tracing subscriber.
fn init_tracing(level: &str, format: &str) {
    use tracing_subscriber::EnvFilter;

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    match format {
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

/// Create a local SIP URI from the bind address.
fn create_local_uri(bind_str: &str) -> Result<SipUri> {
    let bind_addr: std::net::SocketAddr = bind_str.parse().context("Invalid bind address")?;
    let ip = if bind_addr.ip().is_unspecified() {
        auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind_addr.ip().to_string()
    };

    let uri_str = if bind_addr.port() != 5060 && bind_addr.port() != 0 {
        format!("sip:sbc@{}:{}", ip, bind_addr.port())
    } else {
        format!("sip:sbc@{}", ip)
    };

    let uri = Uri::parse(&uri_str).context("Failed to parse local URI")?;
    uri.as_sip()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

/// Derive a local address string for UAC configuration.
fn derive_local_addr(bind_str: &str) -> String {
    if let Ok(addr) = bind_str.parse::<std::net::SocketAddr>() {
        let ip = if addr.ip().is_unspecified() {
            auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
        } else {
            addr.ip().to_string()
        };
        let port = if addr.port() == 0 { 5060 } else { addr.port() };
        format!("{}:{}", ip, port)
    } else {
        "127.0.0.1:5060".to_string()
    }
}

/// Derive local IP string from bind address.
fn derive_local_ip(bind_str: &str) -> String {
    if let Ok(addr) = bind_str.parse::<std::net::SocketAddr>() {
        if !addr.ip().is_unspecified() {
            return addr.ip().to_string();
        }
    }
    auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Auto-detect the local LAN IP.
fn auto_detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:53").ok()?;
    let addr = socket.local_addr().ok()?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    Some(addr.ip().to_string())
}
