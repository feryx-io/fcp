//! fcp-registrar — Standalone SIP registration server.
//!
//! Handles only SIP REGISTER requests with persistent storage.
//! Returns 405 Method Not Allowed for all other SIP methods.

use anyhow::Result;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{
    extract::{Request, State},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
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
    /// Leaving service (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    drain: Arc<fcp_sip::drain::Drain>,
    /// `[http] api_token`: what `/status` and `/drain` ask for. `None`
    /// only on a loopback bind, where configuration refuses to run
    /// without one otherwise.
    api_token: Option<String>,
    /// The read replica as of its last probe (`docs/DEPLOYMENT_GAPS.md` §1.7).
    replica: fcp_storage_postgres::ReplicaHealth,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI arguments
    let cli = Cli::parse();

    // 2. Load configuration
    let mut config = fcp_core::config::check_stage(
        config::load_config(cli.config.as_deref()),
        cli.check_config,
    )?;
    // Secrets from the environment win over the file
    // (`docs/SHIP_AND_OPERATE.md` phase 2).
    if let Some(v) = &cli.database_url {
        config.storage.database_url = v.clone();
    }
    if let Some(v) = &cli.api_token {
        config.http.api_token = Some(v.clone());
    }

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

    fcp_core::config::check_stage(config::validate_config(&config), cli.check_config)?;

    // 3. Initialize tracing
    if cli.check_config {
        println!(
            "{}",
            fcp_core::config::masked_toml(&config).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }
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
    let storage::StorageResult {
        dyn_storage,
        replica: replica_health,
    } = storage::initialize_storage(&config.storage).await?;

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
    // What one source may send, and the sources exempt
    // (`docs/DEEP_TESTING.md` D2c).
    fcp_sip::ingress::set_limit(config.sip.max_messages_per_source)?;
    fcp_sip::ingress::Exemptions::new()
        .networks(
            "[sip] trusted_sources",
            fcp_sip::emergency::TrustedEdges::parse(&config.sip.trusted_sources)
                .map_err(|e| anyhow::anyhow!("[sip] trusted_sources: {e}"))?,
        )
        .install();
    // A source that keeps failing authentication is blocked
    // (`docs/EDGE_SECURITY.md` §2.8).
    fcp_sip::auth_block::install(
        config.sip.max_auth_failures_per_source,
        config.sip.auth_block_secs,
    );
    let (transport_dispatcher, _udp_socket) =
        transport::start_transports(&transport_config, tx).await?;

    // 8. Create transaction manager
    let transaction_mgr = Arc::new(TransactionManager::new(transport_dispatcher));

    // 8b. What this node accepts, and how it leaves service
    // (`docs/CAPACITY_AND_SURVIVABILITY.md` §5, §6). Every limit is
    // optional: a registrar that writes no `[admission]` section keeps
    // none but REGISTERs per source (#314), from which `[sip]
    // trusted_sources` are exempt.
    let admission = Arc::new(fcp_sbc_core::AdmissionController::new(
        config.admission.resolve(),
    ));
    let drain = Arc::new(fcp_sip::drain::Drain::new(
        Arc::clone(&admission) as Arc<dyn fcp_sip::admission::NodeAdmission>,
        Arc::new(RegisteredDevices {
            storage: Arc::clone(&dyn_storage) as Arc<dyn fcp_core::RegistrationStore>,
        }) as Arc<dyn fcp_sip::drain::DrainTarget>,
        Arc::new(fcp_events::InMemoryEventBus::new()),
        config.service.node_id.clone(),
        config.service.drain_wait_secs,
    ));

    // Say this process is there (`docs/SERVICES.md`).
    let beacon = config.storage.redis_url.as_ref().map(|url| {
        let (load, leaving) = (Arc::clone(&drain), Arc::clone(&drain));
        let handle = fcp_cluster::ServiceBeacon::new(
            fcp_cluster::ServiceIdentity::new(&config.service.node_id, "registrar")
                .http(config.http.bind_address)
                .sip(&config.sip.udp_bind),
        )
        .check(fcp_cluster::Check::tenants(
            Arc::clone(&dyn_storage) as Arc<dyn fcp_core::storage::TenantStore>
        ))
        .load(move || {
            let drain = Arc::clone(&load);
            async move {
                std::collections::BTreeMap::from([(
                    "registrations".to_string(),
                    drain.carrying().await as u64,
                )])
            }
        })
        .draining(move || leaving.is_draining())
        .start(fcp_storage_redis::RedisConfig {
            url: url.clone(),
            ..Default::default()
        });
        drain.set_health(handle.health());
        handle
    });

    // 9. Spawn HTTP health server
    let app_state = Arc::new(AppState {
        start_time: Instant::now(),
        registrations_handled: AtomicU64::new(0),
        drain: Arc::clone(&drain),
        api_token: config.http.api_token.clone(),
        replica: replica_health,
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

    let gate: Arc<dyn fcp_sip::admission::NodeAdmission> = Arc::clone(&admission) as _;

    // 11. Set up graceful shutdown signal handling
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to register SIGTERM handler");

    // 12. Main event loop
    loop {
        tokio::select! {
            Some(packet) = rx.recv() => {
                handle_packet(&transaction_mgr, &registrar, &gate, packet).await;
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

    // Leave service before stopping (`docs/CAPACITY_AND_SURVIVABILITY.md`
    // §6): take no new registrations, and give the devices registered here
    // the wait to come back somewhere that is serving. With
    // `drain_wait_secs = 0` — the default — this is what it always was.
    if config.service.drain_wait_secs > 0 || drain.is_draining() {
        fcp_sip::drain::drain_for_shutdown(&drain).await;
    } else {
        drain.start().await;
    }
    if let Some(beacon) = &beacon {
        beacon.shutdown().await;
    }

    info!("Shutdown complete");
    Ok(())
}

/// What a drain means to a registrar: the devices registered here
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
///
/// It carries no calls, so there is nothing to hang up. What a drain does
/// is stop taking new registrations, grant the ones it does take a minute
/// at most — the shared REGISTER gate does that from
/// `[admission] drain_register_expiry_secs` — and wait for the devices to
/// come back somewhere that is serving. They leave on their own as their
/// registrations lapse, which is why the wait watches the count rather
/// than ending anything.
struct RegisteredDevices {
    storage: Arc<dyn fcp_core::RegistrationStore>,
}

#[async_trait::async_trait]
impl fcp_sip::drain::DrainTarget for RegisteredDevices {
    async fn carrying(&self) -> usize {
        self.storage.registration_count().await.unwrap_or(0)
    }

    /// Nothing is ended: a registration that is not renewed expires, and
    /// the device is already registering elsewhere by then.
    async fn end_remaining(&self) -> usize {
        0
    }
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

    let registrar = FcpRegistrar::new(
        registration_store,
        credential_store,
        realm,
        config.registrar.auth_enabled,
    )
    .with_tenant_store(Arc::clone(storage) as _)
    .with_default_tenant_id(config.service.default_tenant_id)
    // Devices' own credentials, and who a credential may register as
    // (docs/ENTERPRISE_CALLING.md §3.8).
    .with_identity(Arc::clone(storage))
    .with_digest_uri_check(config.registrar.digest_uri_check);
    // Phone apps the call managers can wake (docs/PUSH.md §2.1).
    Arc::new(match &config.push {
        Some(push) => registrar.with_push(push.clone()),
        None => registrar,
    })
}

/// Handle an incoming SIP packet from the transport layer.
async fn handle_packet(
    transaction_mgr: &Arc<TransactionManager>,
    registrar: &Arc<FcpRegistrar>,
    admission: &Arc<dyn fcp_sip::admission::NodeAdmission>,
    packet: InboundPacket,
) {
    // A source that kept failing authentication is told nothing until its
    // block ends (`docs/EDGE_SECURITY.md` §2.8).
    if !fcp_sip::auth_block::admit(packet.peer().ip()) {
        return;
    }
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
        let admission = Arc::clone(admission);
        let peer = packet.peer();
        tokio::spawn(async move {
            dispatcher::dispatch_request(&req, handle, &registrar, Some(&admission), peer).await;
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
    // One setup for every binary, its lines shipped to Redis once the
    // service beacon starts (`docs/LOG_VIEWER.md`).
    if let Err(e) = fcp_cluster::logging::init(&config.logging.level, &config.logging.format, false)
    {
        eprintln!("logging: {e:#}");
    }
}

/// Run the HTTP health/status server. The probes — `/health`, `/ready` —
/// are open; the operator routes — `/status`, `/drain` — are behind
/// `[http] api_token`, as on every other binary
/// (`docs/DEPLOYMENT_GAPS.md` §1.3).
async fn run_health_server(bind_addr: std::net::SocketAddr, state: Arc<AppState>) -> Result<()> {
    let public = Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler));
    let protected = Router::new()
        .route("/status", get(status_handler))
        .route(
            "/drain",
            post(start_drain_handler).delete(stop_drain_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_api_token,
        ));
    let app = public.merge(protected).with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "HTTP health server started");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn require_api_token(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(expected_token) = state.api_token.as_deref() else {
        return next.run(request).await;
    };

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let token = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);

    // Compared in time that does not depend on how much of the token the
    // caller guessed right (`docs/EDGE_SECURITY.md` §2.6).
    if !token.is_empty() && fcp_core::secret::constant_time_eq(token, expected_token) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid or missing authentication token"})),
        )
            .into_response()
    }
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
        "status": if state.drain.is_draining() { "draining" } else { "healthy" },
        "service": "fcp-registrar",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime.as_secs(),
        "registrations_handled": registrations_handled,
        "draining": state.drain.is_draining(),
        "registrations_held": state.drain.carrying().await,
        "replica": state.replica.json(),
    }))
}

/// Whether this node should be sent devices: `503` while it is draining,
/// so a load balancer or a DNS SRV weight stops choosing it
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). `/health` keeps answering
/// `200`, so a liveness probe does not kill the process mid-drain.
async fn ready_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    answer(fcp_sip::drain::ready_body(&state.drain).await)
}

/// Start a drain: this node grants short registrations so its devices come
/// back somewhere that is serving, and waits for them to go.
async fn start_drain_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    answer(fcp_sip::drain::start_body(&state.drain).await)
}

/// Call a drain off: the node registers devices normally again.
async fn stop_drain_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    answer(fcp_sip::drain::stop_body(&state.drain).await)
}

/// What the shared drain answered, as this router's response.
fn answer((code, body): (u16, serde_json::Value)) -> (StatusCode, Json<serde_json::Value>) {
    let code = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(body))
}
