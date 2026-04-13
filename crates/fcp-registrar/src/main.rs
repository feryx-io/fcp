//! fcp-registrar — Standalone SIP registration server.
//!
//! Handles only SIP REGISTER requests with persistent storage.
//! Returns 405 Method Not Allowed for all other SIP methods.

use anyhow::Result;
use axum::{extract::State, routing::get, Json, Router};
use clap::Parser;
use fcp_sip::registrar::FcpRegistrar;
use sip_core::Method;
use sip_observe::{set_transport_metrics, TracingTransportMetrics};
use sip_parse::{parse_request, parse_response};
use sip_transaction::{request_branch_id, TransactionKey, TransactionManager, TransportContext};
use sip_transport::InboundPacket;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{info, warn};

mod cli;
mod config;
mod dispatcher;
mod storage;
mod transport;

use cli::Cli;
use config::RegistrarConfig;

/// Shared state for the HTTP health endpoint.
struct AppState {
    start_time: Instant,
    registrations_handled: AtomicU64,
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
    if let Some(ref bind) = cli.sip_bind {
        config.sip.udp_bind = bind.clone();
        config.sip.tcp_bind = bind.clone();
    }
    if let Some(ref bind) = cli.sips_bind {
        config.sip.sips_bind = bind.clone();
    }
    if let Some(ref bind) = cli.ws_bind {
        config.sip.ws_bind = Some(bind.clone());
    }
    if let Some(ref bind) = cli.wss_bind {
        config.sip.wss_bind = Some(bind.clone());
    }
    if let Some(ref cert) = cli.tls_cert {
        config.sip.tls_cert_path = Some(cert.display().to_string());
    }
    if let Some(ref key) = cli.tls_key {
        config.sip.tls_key_path = Some(key.display().to_string());
    }
    if let Some(ref bind) = cli.http_bind {
        config.http.bind_address = bind
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid HTTP bind address '{}': {}", bind, e))?;
    }

    config::validate_config(&config)?;

    // 3. Initialize tracing
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-registrar starting"
    );

    // Set up observability
    if !set_transport_metrics(Arc::new(TracingTransportMetrics)) {
        warn!("transport metrics already configured");
    }

    // 4. Initialize storage
    let dyn_storage = storage::initialize_storage(&config.storage).await?;

    // 5. Create FcpRegistrar
    let registrar = create_registrar(&config, &dyn_storage);

    // 6. Create message channel
    let (tx, mut rx) = mpsc::channel::<InboundPacket>(1024);

    // 7. Start SIP transports
    let transport_config = transport::RegistrarTransportConfig {
        udp_bind: config.sip.udp_bind.clone(),
        tcp_bind: config.sip.tcp_bind.clone(),
        tls_bind: config
            .sip
            .tls_cert_path
            .as_ref()
            .zip(config.sip.tls_key_path.as_ref())
            .map(|_| config.sip.sips_bind.clone()),
        ws_bind: config.sip.ws_bind.clone(),
        wss_bind: config
            .sip
            .tls_cert_path
            .as_ref()
            .zip(config.sip.tls_key_path.as_ref())
            .and(config.sip.wss_bind.clone()),
        tls_cert_path: config.sip.tls_cert_path.clone(),
        tls_key_path: config.sip.tls_key_path.clone(),
    };
    let (transport_dispatcher, _udp_socket) =
        transport::start_transports(&transport_config, tx).await?;

    // 8. Create transaction manager
    let transaction_mgr = Arc::new(TransactionManager::new(transport_dispatcher));

    // 9. Spawn HTTP health server
    let app_state = Arc::new(AppState {
        start_time: Instant::now(),
        registrations_handled: AtomicU64::new(0),
    });
    let http_bind = config.http.bind_address;
    tokio::spawn(async move {
        if let Err(e) = run_health_server(http_bind, app_state).await {
            tracing::error!(%e, "HTTP health server exited");
        }
    });

    // 10. Spawn registration cleanup task
    let cleanup_storage: Arc<dyn fcp_core::RegistrationStore> = Arc::clone(&dyn_storage) as _;
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

    info!(
        udp = %config.sip.udp_bind,
        tcp = %config.sip.tcp_bind,
        tls = %config.sip.sips_bind,
        ws = ?config.sip.ws_bind,
        wss = ?config.sip.wss_bind,
        http = %config.http.bind_address,
        auth = config.registrar.auth_enabled,
        realm = %config.registrar.realm,
        "fcp-registrar ready — listening for REGISTER requests"
    );

    // 11. Set up graceful shutdown signal handling
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to register SIGTERM handler");

    // 12. Main event loop
    loop {
        tokio::select! {
            Some(packet) = rx.recv() => {
                handle_packet(&transaction_mgr, &registrar, packet).await;
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
                break;
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown...");
                break;
            }
        }
    }

    info!("Shutdown complete");
    Ok(())
}

/// Create the FcpRegistrar from configuration and storage.
fn create_registrar(
    config: &RegistrarConfig,
    storage: &Arc<fcp_core::DynStorage>,
) -> Arc<FcpRegistrar> {
    // DynStorage implements RegistrationStore and CredentialStore directly
    let registration_store: Arc<dyn fcp_core::RegistrationStore> = Arc::clone(storage) as _;
    // Always provide credential store so auth can be toggled at runtime
    let credential_store: Option<Arc<dyn fcp_core::CredentialStore>> =
        Some(Arc::clone(storage) as _);
    let realm = Some(config.registrar.realm.clone());

    Arc::new(
        FcpRegistrar::new(
            registration_store,
            credential_store,
            realm,
            config.registrar.auth_enabled,
        )
        .with_tenant_store(Arc::clone(storage) as _)
        .with_default_tenant_id(config.service.default_tenant_id),
    )
}

/// Handle an incoming SIP packet from the transport layer.
async fn handle_packet(
    transaction_mgr: &Arc<TransactionManager>,
    registrar: &Arc<FcpRegistrar>,
    packet: InboundPacket,
) {
    // Try parsing as a request
    if let Some(req) = parse_request(packet.payload()) {
        let ctx = TransportContext::new(
            transport::map_transport(packet.transport()),
            packet.peer(),
            packet.stream().cloned(),
        );

        // ACK doesn't create a transaction
        if req.method() == &Method::Ack {
            if let Some(branch) = request_branch_id(&req) {
                let key = TransactionKey::new(branch, Method::Invite, true);
                transaction_mgr.ack_received(&key).await;
            }
            return;
        }

        let handle = transaction_mgr.receive_request(req.clone(), ctx).await;

        let registrar = Arc::clone(registrar);
        tokio::spawn(async move {
            dispatcher::dispatch_request(&req, handle, &registrar).await;
        });
        return;
    }

    // Try parsing as a response (pass to transaction manager)
    if let Some(response) = parse_response(packet.payload()) {
        transaction_mgr.receive_response(response).await;
        return;
    }

    // SIP keep-alive (CRLF per RFC 5626)
    if matches!(packet.payload().as_ref(), b"\r\n" | b"\r\n\r\n") {
        tracing::trace!(
            peer = %packet.peer(),
            "SIP keep-alive received"
        );
        return;
    }

    warn!(
        peer = %packet.peer(),
        len = packet.payload().len(),
        "Unparsable packet received"
    );
}

/// Initialize tracing subscriber based on configuration.
fn init_tracing(config: &RegistrarConfig) {
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
            // "pretty" or anything else
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .with_env_filter(env_filter)
                .init();
        }
    }
}

/// Run the HTTP health/status server.
async fn run_health_server(bind_addr: std::net::SocketAddr, state: Arc<AppState>) -> Result<()> {
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "HTTP health server started");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-registrar"
    }))
}

async fn status_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let uptime = state.start_time.elapsed();
    let registrations_handled = state.registrations_handled.load(Ordering::Relaxed);

    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-registrar",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime.as_secs(),
        "registrations_handled": registrations_handled,
    }))
}
