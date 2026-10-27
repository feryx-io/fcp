//! HTTP health/status/calls API for fcp-call-manager.

use crate::queue_handler::{QueueCallHandler, SupervisorMode};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use fcp_b2bua_core::CallManager;
use fcp_core::storage::{CallDisposition, CdrQuery, CdrStore};
use fcp_core::{CallId, CredentialStore, DynStorage, TenantId};
use fcp_events::filter::EventType;
use fcp_events::{EventFilter, EventSubscriber, InMemoryEventBus, PlatformEvent};
use fcp_media::MediaManager;
use fcp_queue::QueueManager;
use serde::Deserialize;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

/// Shared state for the HTTP API.
pub struct AppState {
    pub call_manager: Arc<CallManager>,
    pub storage: Arc<DynStorage>,
    /// The read replica as of its last probe (`docs/DEPLOYMENT_GAPS.md` §1.7).
    pub replica: fcp_storage_postgres::ReplicaHealth,
    /// Whether the primary answers (`docs/DEEP_TESTING.md` D5): a node
    /// whose database is away is not ready, since it refuses new
    /// registrations and calls.
    pub primary: Arc<fcp_storage_postgres::primary_health::PrimaryHealth>,
    pub realm: String,
    pub default_tenant_id: TenantId,
    pub media_manager: Arc<MediaManager>,
    pub queue_manager: Option<Arc<QueueManager>>,
    pub queue_handler: Option<Arc<QueueCallHandler>>,
    pub presence_manager: Option<fcp_presence::PresenceManager>,
    pub event_bus: Arc<InMemoryEventBus>,
    pub start_time: Instant,
    pub config_path: Option<std::path::PathBuf>,
    pub api_token: Option<String>,
    /// PBX handler for parking operations (used by /internal/unpark).
    pub pbx_handler: Option<Arc<fcp_pbx::PbxHandler>>,
    /// Cluster parking store for cross-node park/pickup.
    pub cluster_parking: Option<Arc<dyn fcp_core::ClusterParkingStore>>,
    /// What call recordings start through, with the tenant's consent step.
    pub call_recorder: Arc<fcp_sip::recording::CallRecorder>,
    /// Calls with a supervisor (`docs/ENTERPRISE_CALLING.md` §3.11).
    pub supervisions: Option<Arc<fcp_sip::supervision::Supervisions>>,
    /// What live call control acts through (`docs/DEVELOPER_SURFACE.md`
    /// phase 8): hold, transfer, redirect, play, hang up.
    pub sip_handler: Option<Arc<fcp_sip::handler::SipMessageHandler>>,
    /// This node's `[[trunks]]`, which outbound and emergency routing use.
    pub trunks: Vec<fcp_sbc_core::TrunkConfig>,
    pub trunk_health: Option<Arc<fcp_sbc_core::TrunkHealthMonitor>>,
    /// Every trunk this node routes by, the tenants' included.
    pub trunk_set: Option<Arc<fcp_sbc_core::TrunkSet>>,
    /// What this node's REGISTER bindings are doing.
    pub trunk_registrar: Option<Arc<fcp_sbc_core::TrunkRegistrar>>,
    /// What chooses this node's trunks, for a route preview
    /// (`docs/TENANT_TRUNKS.md` §12, phase 6d).
    pub trunk_router: Option<Arc<fcp_sbc_core::TrunkRouter>>,
    /// Every tenant's own routing rules, as this node decides them.
    pub tenant_rules: Option<Arc<fcp_policy::TenantRuleSet>>,
    /// The node's own YAML policy, when it has one.
    pub routing_service: Option<Arc<fcp_routing::RoutingService>>,
    /// What this node will carry, and what it is carrying
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §9).
    pub admission: Arc<fcp_sbc_core::AdmissionController>,
    /// Leaving service without dropping calls (§6).
    pub drain: Arc<fcp_sip::drain::Drain>,
    pub node_id: String,
    /// Registration lookups kept for a TTL, when the node keeps them
    /// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1, phase 2).
    pub registration_cache: Option<Arc<fcp_sip::registration_cache::RegistrationCache>>,
    /// Cluster membership, when the node is clustered: what `/status`
    /// reports of its view (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3).
    #[cfg(feature = "storage-redis")]
    pub cluster: Option<Arc<fcp_cluster::ClusterManager>>,
}

fn queue_visible_to_service_tenant(
    qm: &QueueManager,
    tenant_id: &TenantId,
    queue_name: &str,
) -> bool {
    qm.get_queue_config_for_tenant(queue_name, tenant_id)
        .is_some()
}

fn queue_not_found(queue_name: &str) -> Json<serde_json::Value> {
    Json(serde_json::json!({"error": format!("Queue '{}' not found", queue_name)}))
}

fn agent_not_found(agent_id: &str) -> Json<serde_json::Value> {
    Json(serde_json::json!({"error": format!("Agent '{}' not found", agent_id)}))
}

/// The trunks this node may route emergency calls to: its own
/// emergency-capable `[[trunks]]` and every tenant's
/// (`docs/E911_BLOCK_2.md` §12.1, `docs/TENANT_TRUNKS.md` §5). A call tries
/// its own tenant's first and the platform's after them, so the list is by
/// priority then weight across all of them and each names its tenant.
async fn emergency_trunks_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Every emergency-capable trunk this node routes by, the tenants'
    // included: since `docs/TENANT_TRUNKS.md` phase 6 a tenant's own trunk
    // carries its 911 calls, so a list of the node's `[[trunks]]` alone no
    // longer answers "what would carry an emergency call from here".
    let all: Vec<std::sync::Arc<fcp_sbc_core::TrunkConfig>> = match state.trunk_set.as_ref() {
        Some(set) => set.all(),
        None => state
            .trunks
            .iter()
            .cloned()
            .map(std::sync::Arc::new)
            .collect(),
    };
    let mut trunks: Vec<&std::sync::Arc<fcp_sbc_core::TrunkConfig>> =
        all.iter().filter(|t| t.emergency_capable).collect();
    trunks.sort_by_key(|t| (t.priority, std::cmp::Reverse(t.weight)));
    let listed: Vec<serde_json::Value> = trunks
        .iter()
        .map(|t| {
            let healthy = state
                .trunk_health
                .as_ref()
                .map(|h| h.is_healthy(&t.key()))
                .unwrap_or(true);
            serde_json::json!({
                "name": t.name,
                "trunk_id": t.key(),
                // Whose trunk it is: a tenant's own carries that tenant's
                // emergency calls ahead of the platform's, which carry
                // anyone's.
                "tenant_id": t.tenant_id.map(|id| id.to_string()),
                "enabled": t.enabled,
                "healthy": healthy,
                "priority": t.priority,
                "hosts": t.hosts,
            })
        })
        .collect();
    let routable = trunks.iter().any(|t| {
        t.enabled
            && state
                .trunk_health
                .as_ref()
                .map(|h| h.is_healthy(&t.key()))
                .unwrap_or(true)
    });
    Json(serde_json::json!({
        "trunks": listed,
        "routable": routable,
        "configured_trunks": all.len(),
    }))
}

/// What this node sees of the trunks it routes by
/// (`docs/TENANT_TRUNKS.md` §9): its own probes, its own calls and its own
/// bindings, which is what "health per node" means. Another node answers
/// this differently, and that disagreement is the point.
async fn trunks_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (Some(trunks), Some(health)) = (state.trunk_set.as_ref(), state.trunk_health.as_ref())
    else {
        return Json(serde_json::json!({
            "node_id": state.node_id,
            "trunks": [],
            "routes_by_trunks": false,
        }));
    };
    let report = fcp_sbc_core::node_view(
        &state.node_id,
        trunks,
        health,
        state.trunk_registrar.as_deref(),
    );
    let routable = report.trunks.iter().any(|t| t.routable);
    Json(serde_json::json!({
        "node_id": report.node_id,
        "trunks": report.trunks,
        "routable": routable,
        "routes_by_trunks": true,
    }))
}

/// Run the HTTP health/status/calls server.
pub async fn run_http_server(bind_addr: SocketAddr, state: Arc<AppState>) -> anyhow::Result<()> {
    let protected = Router::new()
        .route("/status", get(status_handler))
        // What this node would do with a call, for the admin API's route
        // preview (`docs/TENANT_TRUNKS.md` §12, phase 6d): it names tenants'
        // carriers, so it takes the token.
        .route(
            "/routes/preview",
            get(crate::route_preview::route_preview_handler),
        )
        .route("/calls", get(calls_handler))
        .route("/calls/:id", get(call_handler))
        .route(
            "/calls/:id/recording/start",
            post(start_call_recording_handler),
        )
        .route(
            "/calls/:id/recording/stop",
            post(stop_call_recording_handler),
        )
        .route("/registrations", get(registrations_handler))
        .route("/cdrs", get(cdrs_handler))
        .route("/cdr/status", get(cdr_status_handler))
        .route("/media-sessions", get(media_sessions_handler))
        .route("/transcoding", get(transcoding_handler))
        .route("/emergency/trunks", get(emergency_trunks_handler))
        .route("/trunks", get(trunks_handler))
        // Queue/Agent management endpoints
        .route(
            "/queues",
            get(list_queues_handler).post(create_queue_handler),
        )
        .route(
            "/queues/:name",
            get(get_queue_handler)
                .put(update_queue_handler)
                .delete(delete_queue_handler),
        )
        .route(
            "/queues/:name/drain",
            post(drain_queue_handler).delete(undrain_queue_handler),
        )
        .route("/queues/reload", post(reload_queues_handler))
        .route("/queues/:name/history", get(queue_history_handler))
        .route("/agents", get(list_agents_handler))
        .route("/agents/:id", get(get_agent_handler))
        .route("/agents/:id/pause", post(pause_agent_handler))
        .route("/agents/:id/unpause", post(unpause_agent_handler))
        .route("/agents/:id/kick", post(kick_agent_handler))
        .route("/agents/:id/login", post(login_agent_handler))
        .route("/agents/:id/logout", post(logout_agent_handler))
        .route("/agents/:id/extend-wrapup", post(extend_wrapup_handler))
        // Callback endpoints
        .route("/callbacks", get(list_callbacks_handler))
        .route("/callbacks/:call_id", delete(cancel_callback_handler))
        // Campaign / predictive dialer endpoints
        .route(
            "/campaigns/:queue/numbers",
            post(campaign_load_numbers_handler),
        )
        .route("/campaigns/:queue/stats", get(campaign_stats_handler))
        .route("/campaigns/:queue/start", post(campaign_start_handler))
        .route("/campaigns/:queue/stop", post(campaign_stop_handler))
        // Queue transfer endpoint
        .route(
            "/queues/:from/transfer/:call_id",
            post(transfer_call_handler),
        )
        // Live call control (`docs/DEVELOPER_SURFACE.md` phase 8)
        .merge(crate::call_control::routes())
        // Supervisor endpoints
        .route("/calls/:id/monitor", post(monitor_call_handler))
        .route("/calls/:id/whisper", post(whisper_call_handler))
        .route("/calls/:id/barge", post(barge_call_handler))
        .route("/calls/:id/supervisor", get(supervisor_get_handler))
        .route(
            "/calls/:id/supervisor/leave",
            post(supervisor_leave_handler),
        )
        // Presence endpoints
        .route("/presence", get(list_presence_handler))
        .route(
            "/presence/:aor",
            get(get_presence_handler).put(update_presence_handler),
        )
        // Prometheus metrics endpoint
        .route("/metrics", get(metrics_handler))
        // Leaving service: start a drain, or call one off (§6).
        .route(
            "/drain",
            post(start_drain_handler).delete(stop_drain_handler),
        )
        // Internal cluster endpoints (authenticated via cluster secret)
        .route("/internal/hangup", post(internal_hangup_handler))
        .route("/internal/supervisor", post(internal_supervisor_handler))
        .route("/internal/unpark", post(internal_unpark_handler))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_api_token,
        ));

    let app = Router::new()
        .route("/health", get(health_handler))
        // Whether this node should be sent work
        // (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). A load balancer reads
        // it without a token, as it reads `/health`.
        .route("/ready", get(ready_handler))
        // Callback endpoints
        // WebSocket wallboard (real-time queue stats)
        .route("/ws/wallboard", get(wallboard_ws_handler))
        .merge(protected)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "HTTP API server started");
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

    let provided_token = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(extract_bearer_token);

    match provided_token {
        Some(token) if fcp_core::secret::constant_time_eq(token, expected_token) => {
            next.run(request).await
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid or missing authentication token"})),
        )
            .into_response(),
    }
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-call-manager"
    }))
}

async fn status_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let uptime = state.start_time.elapsed();
    let call_stats = state.call_manager.stats();
    let admission_cfg = state.admission.config_snapshot();
    let refused = |reason| fcp_sbc_core::AdmissionCounters::global().refused_for(reason);

    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-call-manager",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime.as_secs(),
        "active_calls": call_stats.active,
        "total_calls": call_stats.total,
        "established_calls": call_stats.established,
        "terminated_calls": call_stats.terminated,
        "failed_calls": call_stats.failed,
        // What this node is carrying against what it allows
        // (docs/CAPACITY_AND_SURVIVABILITY.md §9). Zero is no limit.
        "draining": state.drain.is_draining(),
        // Registration lookups since start, by reason
        // (docs/MULTI_REGION_FOLLOWUPS.md §2.1). A region away from the
        // primary pays a crossing for each.
        //
        // `total` is round trips to the store; `cached` is lookups the
        // registration cache answered instead (phase 2).
        "registration_reads": {
            "direct": fcp_core::storage::RegistrationReads::global().counts().0,
            "alias": fcp_core::storage::RegistrationReads::global().counts().1,
            "housekeeping": fcp_core::storage::RegistrationReads::global().counts().2,
            "total": fcp_core::storage::RegistrationReads::global().total(),
            "cached": fcp_core::storage::RegistrationReads::global().cached(),
        },
        // Whether this node's view of the cluster looks whole, and why
        // not (docs/MULTI_REGION_FOLLOWUPS.md §2.3). Absent outside a
        // cluster.
        "cluster_view": cluster_view(&state),
        // Which release every node runs, and whether more than one
        // (docs/BACKUP_AND_RESTORE.md §5). Absent outside a cluster.
        "cluster_versions": cluster_versions(&state),
        "replica": state.replica.json(),
        "database": state.primary.json(),
        "registration_cache": state.registration_cache.as_ref().map(|cache| {
            let stats = cache.stats();
            serde_json::json!({
                "ttl_secs": cache.ttl().as_secs(),
                "entries": stats.size,
                "hits": stats.hits,
                "misses": stats.misses,
                "evictions": stats.evictions,
            })
        }),
        "admission": {
            "concurrent_calls": state.admission.concurrent_calls(),
            "current_cps": state.admission.current_cps(),
            "concurrent_registrations": state.admission.concurrent_registrations(),
            "limits": {
                "max_cps": admission_cfg.max_cps,
                "max_concurrent_calls": admission_cfg.max_concurrent_calls,
                "max_concurrent_registrations": admission_cfg.max_concurrent_registrations,
                "max_register_cps_per_source": admission_cfg.max_register_cps_per_source,
                "max_register_cps_per_aor": admission_cfg.max_register_cps_per_aor,
            },
            "refused": {
                "node_cps": refused(fcp_sip::admission::Refusal::NodeCps),
                "node_calls": refused(fcp_sip::admission::Refusal::NodeCalls),
                "tenant_calls": refused(fcp_sip::admission::Refusal::TenantCalls),
                "registrations": refused(fcp_sip::admission::Refusal::Registrations),
                "register_rate": refused(fcp_sip::admission::Refusal::RegisterRate),
                "draining": refused(fcp_sip::admission::Refusal::Draining),
            },
            "tenants": state
                .admission
                .tenant_calls_snapshot()
                .into_iter()
                .map(|(tenant, calls)| serde_json::json!({
                    "tenant_id": tenant.to_string(),
                    "concurrent_calls": calls,
                }))
                .collect::<Vec<_>>(),
        },
    }))
}

/// Whether this node should be sent work: `200` while it is serving,
/// `503` while it is draining, so a load balancer stops choosing it
/// before it stops taking calls (`docs/CAPACITY_AND_SURVIVABILITY.md`
/// §6). `/health` keeps answering `200` throughout, so a liveness probe
/// does not kill the process mid-drain.
async fn ready_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (mut code, mut body) = fcp_sip::drain::ready_body(&state.drain).await;
    // A node whose database is away refuses new registrations and calls
    // (`docs/DEEP_TESTING.md` D5), so a balancer should send them elsewhere;
    // so does one whose primary's address leads to a standby (D5c), which
    // refuses registrations.
    body["database"] = state.primary.json();
    if !state.primary.is_writable() {
        code = 503;
        body["ready"] = serde_json::Value::Bool(false);
    }
    answer((code, body))
}

/// Start a drain: this node takes no new calls, keeps the ones it has
/// until `drain_wait_secs`, and then ends what is left with a BYE that
/// says why. The node keeps running — a drain is not a shutdown.
async fn start_drain_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    answer(fcp_sip::drain::start_body(&state.drain).await)
}

/// Call a drain off: the node serves again, and the calls it was carrying
/// — which the drain never touched — carry on.
async fn stop_drain_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    answer(fcp_sip::drain::stop_body(&state.drain).await)
}

/// What the shared drain answered, as this router's response.
fn answer((code, body): (u16, serde_json::Value)) -> (StatusCode, Json<serde_json::Value>) {
    let code = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(body))
}

async fn calls_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let calls: Vec<_> = state
        .call_manager
        .all_calls()
        .into_iter()
        .filter(|call| call.tenant_id == state.default_tenant_id)
        .collect();
    let call_list: Vec<serde_json::Value> = calls
        .iter()
        .map(|call| {
            let elapsed = call.created_at.elapsed().map(|d| d.as_secs()).unwrap_or(0);
            serde_json::json!({
                "call_id": call.id.to_string(),
                "state": format!("{:?}", call.state),
                "held": call.held.map(|leg| leg.as_str()),
                "direction": format!("{:?}", call.a_leg.direction),
                "caller": call.caller(),
                "callee": call.callee().unwrap_or(""),
                "duration_seconds": elapsed,
            })
        })
        .collect();

    Json(serde_json::json!({ "calls": call_list }))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct StartRecordingBody {
    /// Under the tenant's directory; absent, a dated file named for the call.
    #[serde(default)]
    path: Option<String>,
    /// The call's tenant, when the caller scopes it (the admin API does);
    /// the node's default tenant otherwise.
    #[serde(default)]
    tenant_id: Option<TenantId>,
    /// The party asking from their app (`/me/calls/:id/recording`): the
    /// call must be theirs, and the recording is `on_demand`. The API has
    /// checked that they may record.
    #[serde(default)]
    for_user: Option<fcp_core::UserId>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct StopRecordingBody {
    #[serde(default)]
    tenant_id: Option<TenantId>,
    /// The party asking: only a recording they started on demand stops.
    #[serde(default)]
    for_user: Option<fcp_core::UserId>,
}

/// GET /calls/:id — one of the tenant's calls (`?tenant_id=`, the node's
/// default tenant otherwise), as the admin API's `GET /api/v1/calls/:id`
/// shows it.
async fn call_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<SupervisorQuery>,
) -> Response {
    let Some(call) =
        scoped_call(&state, &id, query.tenant_id).and_then(|id| state.call_manager.get_call(&id))
    else {
        return call_not_found(&id);
    };
    Json(call_view(&call)).into_response()
}

/// A call as `GET /calls/:id` answers it: the admin API's `CallInfo`.
fn call_view(call: &fcp_b2bua_core::Call) -> serde_json::Value {
    serde_json::json!({
        "id": call.id.to_string(),
        "state": format!("{:?}", call.state),
        "from_uri": call.a_leg.remote_uri,
        "to_uri": call
            .b_leg
            .as_ref()
            .map(|leg| leg.remote_uri.clone())
            .unwrap_or_else(|| call.callee().unwrap_or("").to_string()),
        "from_display_name": call.a_leg.remote_display_name,
        "to_display_name": call.b_leg.as_ref().and_then(|leg| leg.remote_display_name.clone()),
        "created_at": rfc3339(call.created_at),
        "established_at": call.established_at.map(rfc3339),
        "ended_at": call.ended_at.map(rfc3339),
        "duration_seconds": call.duration().map(|d| d.as_secs()),
        "media_anchored": call.media_anchored,
        "failure_reason": call.failure_reason,
    })
}

fn rfc3339(time: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn call_not_found(id: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": format!("Call not found: {id}")})),
    )
        .into_response()
}

/// Whether `user` is a party to the call: the effective user of either
/// leg, so a hoteling guest is their host.
fn party_to(state: &AppState, call_id: &fcp_b2bua_core::CallId, user: &fcp_core::UserId) -> bool {
    state
        .call_manager
        .get_call(call_id)
        .is_some_and(|c| c.caller_user_id == Some(*user) || c.callee_user_id == Some(*user))
}

fn recording_error_response(e: fcp_sip::recording::RecordingError) -> Response {
    use fcp_sip::recording::RecordingError as E;
    let status = match e {
        E::CallNotFound => StatusCode::NOT_FOUND,
        E::InvalidPath => StatusCode::BAD_REQUEST,
        E::NoMedia => StatusCode::SERVICE_UNAVAILABLE,
        E::CallEnded
        | E::NotAnchored
        | E::AlreadyRecording
        | E::NotRecording
        | E::NotStartedByUser
        | E::ConsentRefused => StatusCode::CONFLICT,
        E::Media(_) | E::NotRegistered(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
}

/// Record a call through the tenant's consent step
/// (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1); returns once the announcement has
/// played and the recorder runs.
///
/// POST /calls/{id}/recording/start
async fn start_call_recording_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let req: StartRecordingBody = if body.iter().all(u8::is_ascii_whitespace) {
        StartRecordingBody::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(req) => req,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("Invalid request body: {e}")})),
                )
                    .into_response()
            }
        }
    };
    let Some(call_id) = scoped_call(&state, &id, req.tenant_id) else {
        return call_not_found(&id);
    };
    let started = match req.for_user {
        Some(user) => {
            if !party_to(&state, &call_id, &user) {
                return call_not_found(&id);
            }
            state.call_recorder.start_on_demand(&call_id, user).await
        }
        None => {
            let path = match req.path {
                Some(p) => fcp_sip::recording::RecordingPath::Supplied(p),
                None => fcp_sip::recording::RecordingPath::wav(),
            };
            state
                .call_recorder
                .start(
                    &call_id,
                    fcp_core::recording::CallRecordingSource::Api,
                    path,
                )
                .await
        }
    };
    match started {
        Ok(started) => Json(serde_json::json!({
            "call_id": id,
            "status": "recording",
            "recording_id": started.recording_id.to_string(),
            "path": started.file,
            "consent": started.consent.as_str(),
        }))
        .into_response(),
        Err(e) => recording_error_response(e),
    }
}

/// POST /calls/{id}/recording/stop
async fn stop_call_recording_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<StopRecordingBody>>,
) -> Response {
    let (tenant, for_user) = body
        .map(|Json(b)| (b.tenant_id, b.for_user))
        .unwrap_or_default();
    let Some(call_id) = scoped_call(&state, &id, tenant) else {
        return call_not_found(&id);
    };
    let stopped = match for_user {
        Some(user) => {
            if !party_to(&state, &call_id, &user) {
                return call_not_found(&id);
            }
            state.call_recorder.stop_on_demand(&call_id, user).await
        }
        None => state.call_recorder.stop(&call_id).await,
    };
    match stopped {
        Ok(()) => Json(serde_json::json!({"call_id": id, "status": "stopped"})).into_response(),
        Err(e) => recording_error_response(e),
    }
}

#[derive(Deserialize)]
struct RegistrationQuery {
    aor: Option<String>,
}

async fn registrations_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RegistrationQuery>,
) -> Json<serde_json::Value> {
    use fcp_core::RegistrationStore;

    let visible_aors = match service_visible_aors(&state).await {
        Ok(visible_aors) => visible_aors,
        Err(e) => {
            return Json(serde_json::json!({
                "error": format!("Failed to resolve tenant-visible registrations: {}", e),
                "registrations": []
            }));
        }
    };

    if let Some(ref aor) = query.aor {
        if !visible_aors.contains(&normalize_aor(aor)) {
            return Json(serde_json::json!({
                "registrations": [],
                "count": 0
            }));
        }
    }

    // If AOR is provided, filter by that AOR; otherwise return all registrations
    let registrations_result = match query.aor {
        Some(ref aor) => {
            state
                .storage
                .get_registrations_in_tenant(&state.default_tenant_id, aor)
                .await
        }
        None => {
            state
                .storage
                .get_all_registrations_in_tenant(&state.default_tenant_id)
                .await
        }
    };

    match registrations_result {
        Ok(registrations) => {
            let reg_list: Vec<serde_json::Value> = registrations
                .iter()
                .map(|reg| {
                    let expires_secs = reg
                        .expires_at
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    serde_json::json!({
                        "aor": reg.aor,
                        "contact": reg.contact,
                        "expires_at_epoch": expires_secs,
                        "user_agent": reg.user_agent,
                    })
                })
                .collect();

            Json(serde_json::json!({
                "registrations": reg_list,
                "count": reg_list.len()
            }))
        }
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to get registrations: {}", e),
            "registrations": []
        })),
    }
}

/// CDR query parameters.
#[derive(Deserialize)]
struct CdrQueryParams {
    /// Filter by call ID
    call_id: Option<String>,
    /// Filter by tenant ID
    tenant_id: Option<String>,
    /// Filter by caller URI (substring match)
    from_uri: Option<String>,
    /// Filter by callee URI (substring match)
    to_uri: Option<String>,
    /// Filter by disposition (answered, busy, no_answer, failed, cancelled)
    disposition: Option<String>,
    /// `call` (default), `queue`, `conference`, `gateway` or `all`
    kind: Option<String>,
    /// Start time range begin (Unix timestamp in seconds)
    start_time_from: Option<u64>,
    /// Start time range end (Unix timestamp in seconds)
    start_time_to: Option<u64>,
    /// Maximum number of records to return (default: 100, max: 1000)
    limit: Option<usize>,
    /// Number of records to skip (default: 0)
    offset: Option<usize>,
}

async fn cdrs_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CdrQueryParams>,
) -> Json<serde_json::Value> {
    use fcp_core::TenantId;
    use std::time::{Duration, UNIX_EPOCH};

    // Build CDR query from parameters
    let mut query = CdrQuery::default();
    match fcp_core::storage::CdrKind::listing_filter(
        params.kind.as_deref(),
        params.call_id.is_some(),
    ) {
        Ok(kind) => query.kind = kind,
        Err(error) => return Json(serde_json::json!({ "error": error, "cdrs": [] })),
    }

    // Parse call_id if provided
    if let Some(ref call_id_str) = params.call_id {
        match call_id_str.parse::<CallId>() {
            Ok(call_id) => query.call_id = Some(call_id),
            Err(_) => {
                return Json(serde_json::json!({
                    "error": format!("Invalid call_id format: {}", call_id_str),
                    "cdrs": []
                }));
            }
        }
    }

    query.tenant_id = Some(state.default_tenant_id);

    // Parse tenant_id if provided
    if let Some(ref tenant_id_str) = params.tenant_id {
        match tenant_id_str.parse::<TenantId>() {
            Ok(tenant_id) if tenant_id == state.default_tenant_id => {}
            Ok(_) => {
                return Json(serde_json::json!({
                    "error": "tenant_id outside service tenant scope",
                    "cdrs": []
                }));
            }
            Err(_) => {
                return Json(serde_json::json!({
                    "error": format!("Invalid tenant_id format: {}", tenant_id_str),
                    "cdrs": []
                }));
            }
        }
    }

    query.from_uri = params.from_uri;
    query.to_uri = params.to_uri;

    // Apply limit with bounds (default 100, max 1000)
    query.limit = Some(params.limit.unwrap_or(100).min(1000));
    query.offset = params.offset;

    // Parse time range filters
    if let Some(ts) = params.start_time_from {
        query.start_time_from = Some(UNIX_EPOCH + Duration::from_secs(ts));
    }
    if let Some(ts) = params.start_time_to {
        query.start_time_to = Some(UNIX_EPOCH + Duration::from_secs(ts));
    }

    // Parse disposition if provided
    if let Some(ref disposition_str) = params.disposition {
        let disposition = match disposition_str.to_lowercase().as_str() {
            "answered" => Some(CallDisposition::Answered),
            "busy" => Some(CallDisposition::Busy),
            "no_answer" | "noanswer" => Some(CallDisposition::NoAnswer),
            "failed" => Some(CallDisposition::Failed),
            "cancelled" | "canceled" => Some(CallDisposition::Cancelled),
            _ => {
                return Json(serde_json::json!({
                    "error": format!("Invalid disposition: {}. Valid values: answered, busy, no_answer, failed, cancelled", disposition_str),
                    "cdrs": []
                }));
            }
        };
        query.disposition = disposition;
    }

    // Query CDRs and count from storage (for pagination)
    let count_query = query.clone();
    let (cdrs_result, count_result) = tokio::join!(
        state.storage.query_cdrs(query),
        state.storage.count_cdrs(count_query)
    );

    match cdrs_result {
        Ok(cdrs) => {
            let total = count_result.unwrap_or(cdrs.len());
            let cdr_list: Vec<serde_json::Value> = cdrs
                .iter()
                .map(|cdr| {
                    let start_time_epoch = cdr
                        .start_time
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let answer_time_epoch = cdr.answer_time.and_then(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .ok()
                    });
                    let end_time_epoch = cdr.end_time.and_then(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .ok()
                    });

                    serde_json::json!({
                        "cdr_id": cdr.cdr_id.to_string(),
                        "kind": cdr.kind.as_str(),
                        "call_id": cdr.call_id.to_string(),
                        "tenant_id": cdr.tenant_id.to_string(),
                        "sip_call_id": cdr.sip_call_id,
                        "direction": cdr.direction.map(|d| format!("{:?}", d).to_lowercase()),
                        "node_id": cdr.node_id,
                        "region": cdr.region,
                        "trunk_name": cdr.trunk_name,
                        "correlation_id": cdr.correlation_id.map(|c| c.to_string()),
                        "rtp_packets_sent": cdr.rtp_packets_sent,
                        "rtp_packets_received": cdr.rtp_packets_received,
                        "rtp_bytes_sent": cdr.rtp_bytes_sent,
                        "rtp_bytes_received": cdr.rtp_bytes_received,
                        "rtp_packets_lost": cdr.rtp_packets_lost,
                        "cost": cdr.cost,
                        "currency": cdr.currency,
                        "rate_plan_id": cdr.rate_plan_id,
                        "billable_seconds": cdr.billable_seconds,
                        "recording_url": cdr.recording_url,
                        "recording_consent": cdr.recording_consent.map(|c| c.as_str()),
                        "from_uri": cdr.from_uri,
                        "from_display_name": cdr.from_display_name,
                        "to_uri": cdr.to_uri,
                        "to_display_name": cdr.to_display_name,
                        "start_time_epoch": start_time_epoch,
                        "answer_time_epoch": answer_time_epoch,
                        "end_time_epoch": end_time_epoch,
                        "setup_duration_ms": cdr.setup_duration_ms,
                        "call_duration_ms": cdr.call_duration_ms,
                        "total_duration_ms": cdr.total_duration_ms,
                        "disposition": format!("{:?}", cdr.disposition),
                        "hangup_cause": format!("{:?}", cdr.hangup_cause),
                        "hangup_party": cdr.hangup_party.as_ref().map(|p| format!("{:?}", p)),
                        "sip_status_code": cdr.sip_status_code,
                        "sip_reason": cdr.sip_reason,
                        "media_anchored": cdr.media_anchored,
                        "codec_a_leg": cdr.codec_a_leg,
                        "codec_b_leg": cdr.codec_b_leg,
                    })
                })
                .collect();

            Json(serde_json::json!({
                "cdrs": cdr_list,
                "count": cdr_list.len(),
                "total": total,
                "limit": params.limit.unwrap_or(100).min(1000),
                "offset": params.offset.unwrap_or(0)
            }))
        }
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to query CDRs: {}", e),
            "cdrs": []
        })),
    }
}

fn normalize_aor(aor: &str) -> String {
    fcp_sip::presence::extract_aor(aor)
        .unwrap_or_else(|| {
            aor.trim_start_matches("sip:")
                .trim_start_matches("sips:")
                .to_string()
        })
        .to_ascii_lowercase()
}

async fn service_visible_aors(state: &AppState) -> anyhow::Result<HashSet<String>> {
    use fcp_core::storage::DeviceStore;

    let mut visible = HashSet::new();

    for device in state.storage.list_devices(&state.default_tenant_id).await? {
        if let Some(sip_uri) = device.sip_uri {
            visible.insert(normalize_aor(&sip_uri));
        }
    }

    for user in state
        .storage
        .list_users()
        .await?
        .into_iter()
        .filter(|user| user.tenant_id == state.default_tenant_id)
    {
        visible.insert(format!(
            "{}@{}",
            user.username.to_ascii_lowercase(),
            state.realm.to_ascii_lowercase()
        ));
    }

    Ok(visible)
}

/// List all active RTP/media sessions with details
async fn media_sessions_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let sessions = state.media_manager.list_sessions().await;

    // Convert to JSON-friendly format
    let session_list: Vec<serde_json::Value> = sessions
        .iter()
        .map(|session| {
            serde_json::json!({
                "call_id": session.call_id,
                "dialog_id": session.dialog_id,
                "state": session.state,
                "local_rtp_port": session.local_rtp_port,
                "local_rtp_port_b": session.local_rtp_port_b,
                "duration_secs": session.duration_secs,
                "transcoding_active": session.transcoding_active,
                "a_leg": {
                    "remote_addr": session.a_leg.remote_addr,
                    "codec": session.a_leg.codec,
                    "clock_rate": session.a_leg.clock_rate,
                    "payload_type": session.a_leg.payload_type,
                    "packets_received": session.a_leg.packets_received,
                    "packets_sent": session.a_leg.packets_sent,
                    "bytes_received": session.a_leg.bytes_received,
                    "bytes_sent": session.a_leg.bytes_sent,
                },
                "b_leg": {
                    "remote_addr": session.b_leg.remote_addr,
                    "codec": session.b_leg.codec,
                    "clock_rate": session.b_leg.clock_rate,
                    "payload_type": session.b_leg.payload_type,
                    "packets_received": session.b_leg.packets_received,
                    "packets_sent": session.b_leg.packets_sent,
                    "bytes_received": session.b_leg.bytes_received,
                    "bytes_sent": session.b_leg.bytes_sent,
                },
            })
        })
        .collect();

    Json(serde_json::json!({
        "sessions": session_list,
        "count": session_list.len()
    }))
}

/// Get transcoding statistics summary
async fn transcoding_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let sessions = state.media_manager.list_sessions().await;

    // Count sessions with transcoding active
    let transcoding_sessions: Vec<&fcp_media::RtpSessionInfo> =
        sessions.iter().filter(|s| s.transcoding_active).collect();

    // Build codec pairs summary
    let mut codec_pairs: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for session in &transcoding_sessions {
        let pair = format!("{} <-> {}", session.a_leg.codec, session.b_leg.codec);
        *codec_pairs.entry(pair).or_insert(0) += 1;
    }

    // Detailed transcoding sessions
    let transcoding_details: Vec<serde_json::Value> = transcoding_sessions
        .iter()
        .map(|session| {
            serde_json::json!({
                "call_id": session.call_id,
                "a_leg": {
                    "remote_addr": session.a_leg.remote_addr,
                    "codec": session.a_leg.codec,
                    "clock_rate": session.a_leg.clock_rate,
                },
                "b_leg": {
                    "remote_addr": session.b_leg.remote_addr,
                    "codec": session.b_leg.codec,
                    "clock_rate": session.b_leg.clock_rate,
                },
                "duration_secs": session.duration_secs,
            })
        })
        .collect();

    Json(serde_json::json!({
        "total_sessions": sessions.len(),
        "transcoding_sessions": transcoding_sessions.len(),
        "direct_forward_sessions": sessions.len() - transcoding_sessions.len(),
        "codec_pairs": codec_pairs,
        "sessions": transcoding_details,
    }))
}

// ---- Prometheus Metrics Endpoint ----

/// Serve Prometheus metrics in text exposition format.
///
/// Returns all registered Prometheus metrics. When the `storage-redis`
/// feature is enabled with `metrics`, Redis operation and cache metrics
/// are included.
///
/// GET /metrics
/// Which release every node runs, when this one is clustered.
fn cluster_versions(state: &AppState) -> Option<serde_json::Value> {
    #[cfg(feature = "storage-redis")]
    {
        state
            .cluster
            .as_ref()
            .and_then(|c| serde_json::to_value(c.versions()).ok())
    }
    #[cfg(not(feature = "storage-redis"))]
    {
        let _ = state;
        None
    }
}

/// This node's view of the cluster, when it is clustered.
fn cluster_view(state: &AppState) -> Option<serde_json::Value> {
    #[cfg(feature = "storage-redis")]
    {
        state
            .cluster
            .as_ref()
            .and_then(|c| serde_json::to_value(c.view()).ok())
    }
    #[cfg(not(feature = "storage-redis"))]
    {
        let _ = state;
        None
    }
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let families = prometheus::default_registry().gather();
    let mut buffer = Vec::new();
    encoder.encode(&families, &mut buffer).unwrap_or_default();
    let mut body = String::from_utf8(buffer).unwrap_or_default();
    let node = state.call_manager.node_id().unwrap_or("").replace('"', "");
    // The CDR pipeline's node side (docs/CDR_PIPELINE.md §11).
    if let Some(writer) = state.call_manager.cdr_writer() {
        body.push_str(&writer.prometheus(&format!("node=\"{node}\"")));
    }
    // Pushes by provider, kind and outcome (docs/PUSH.md §2.3).
    body.push_str(&fcp_push::PushCounters::global().prometheus(&format!("node=\"{node}\"")));
    // Calls by trunk, tenant and outcome (docs/TENANT_TRUNKS.md §9).
    body.push_str(
        &fcp_core::trunk_report::TrunkCounters::global().prometheus(&format!("node=\"{node}\"")),
    );
    // What this node admitted and refused, and what it is carrying
    // (docs/CAPACITY_AND_SURVIVABILITY.md §9).
    body.push_str(
        &fcp_sbc_core::AdmissionCounters::global().prometheus(&format!("node=\"{node}\"")),
    );
    body.push_str(&admission_gauges(&state.admission, &node));
    // Call recordings by source and consent outcome
    // (docs/COMPLIANCE_FOLLOWUPS.md §9).
    body.push_str(
        &fcp_core::compliance::RecordingCounters::global().prometheus(&format!("node=\"{node}\"")),
    );
    // Whether this node's view of the cluster looks whole
    // (docs/MULTI_REGION_FOLLOWUPS.md §2.3).
    #[cfg(feature = "storage-redis")]
    if let Some(cluster) = &state.cluster {
        body.push_str(&cluster.view().prometheus(&format!("node=\"{node}\"")));
        body.push_str(&cluster.versions().prometheus(&format!("node=\"{node}\"")));
    }
    // Registration lookups in the store, by reason: the one hot-path read
    // that is neither cached nor replica-served, so each is a crossing in
    // a region away from the primary (docs/MULTI_REGION_FOLLOWUPS.md §2.1).
    body.push_str(
        &fcp_core::storage::RegistrationReads::global().prometheus(&format!("node=\"{node}\"")),
    );
    body.push_str(&state.replica.prometheus(&format!("node=\"{node}\"")));
    body.push_str(&state.primary.prometheus(&format!("node=\"{node}\"")));
    // Emergency calls by kind, location source and delivery, the location
    // gaps, and the trunks' Geolocation-Error answers
    // (docs/E911_BLOCK_2.md §9).
    body.push_str(
        &fcp_core::e911::EmergencyCounters::global().prometheus(&format!("node=\"{node}\"")),
    );
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

/// What this node is carrying against what it allows, so headroom is a
/// query rather than a look in someone's TOML
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §9).
fn admission_gauges(admission: &fcp_sbc_core::AdmissionController, node: &str) -> String {
    let cfg = admission.config_snapshot();
    let mut out = String::from(
        "# HELP fcp_admission_concurrent_calls Calls this node is carrying.\n\
         # TYPE fcp_admission_concurrent_calls gauge\n",
    );
    out.push_str(&format!(
        "fcp_admission_concurrent_calls{{node=\"{node}\"}} {}\n",
        admission.concurrent_calls()
    ));
    out.push_str(
        "# HELP fcp_admission_current_cps Calls started on this node in the last second.\n\
         # TYPE fcp_admission_current_cps gauge\n",
    );
    out.push_str(&format!(
        "fcp_admission_current_cps{{node=\"{node}\"}} {}\n",
        admission.current_cps()
    ));
    out.push_str(
        "# HELP fcp_admission_concurrent_registrations Registrations this node holds.\n\
         # TYPE fcp_admission_concurrent_registrations gauge\n",
    );
    out.push_str(&format!(
        "fcp_admission_concurrent_registrations{{node=\"{node}\"}} {}\n",
        admission.concurrent_registrations()
    ));
    out.push_str(
        "# HELP fcp_admission_capacity What this node allows. Zero is no limit.\n\
         # TYPE fcp_admission_capacity gauge\n",
    );
    for (limit, value) in [
        ("max_cps", cfg.max_cps),
        ("max_concurrent_calls", cfg.max_concurrent_calls),
        (
            "max_concurrent_registrations",
            cfg.max_concurrent_registrations,
        ),
    ] {
        out.push_str(&format!(
            "fcp_admission_capacity{{node=\"{node}\",limit=\"{limit}\"}} {value}\n"
        ));
    }
    let tenants = admission.tenant_calls_snapshot();
    if !tenants.is_empty() {
        out.push_str(
            "# HELP fcp_tenant_concurrent_calls Calls a tenant is carrying on this node.\n\
             # TYPE fcp_tenant_concurrent_calls gauge\n",
        );
        for (tenant, calls) in tenants {
            out.push_str(&format!(
                "fcp_tenant_concurrent_calls{{node=\"{node}\",tenant=\"{tenant}\"}} {calls}\n"
            ));
        }
    }
    out
}

/// The CDR pipeline's node side: what the spool holds and what the
/// in-process processor has done (`docs/CDR_PIPELINE.md` §11).
///
/// GET /cdr/status
async fn cdr_status_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(match state.call_manager.cdr_writer() {
        Some(writer) => writer.status(),
        None => serde_json::json!({ "spool": null, "processor": null }),
    })
}

// ---- Queue/Agent Management Endpoints ----

/// List all queues with stats.
async fn list_queues_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled", "queues": []}));
    };
    let stats = qm.all_queue_stats_for_tenant(&state.default_tenant_id);
    Json(serde_json::json!({ "queues": stats }))
}

/// Get a single queue's stats + waiting calls.
async fn get_queue_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    match qm.queue_stats_for_tenant(&name, &state.default_tenant_id) {
        Some(stats) => {
            let agents = qm.list_agents_for_queue_and_tenant(&name, &state.default_tenant_id);
            let is_draining = qm.is_queue_draining(&name);
            let agent_list: Vec<serde_json::Value> = agents
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "id": a.id,
                        "extension": a.extension,
                        "state": a.state.to_string(),
                    })
                })
                .collect();
            Json(serde_json::json!({
                "queue": stats,
                "agents": agent_list,
                "is_draining": is_draining,
            }))
        }
        None => queue_not_found(&name),
    }
}

/// Create a new queue at runtime.
async fn create_queue_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateQueueBody>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    match qm.create_queue(body.key.clone(), body.config, state.default_tenant_id) {
        Ok(()) => Json(serde_json::json!({
            "status": "created",
            "queue": body.key,
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Update a queue's configuration at runtime.
async fn update_queue_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(config): Json<fcp_queue::QueueConfig>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &name) {
        return queue_not_found(&name);
    }
    match qm.update_queue(&name, config) {
        Ok(()) => Json(serde_json::json!({
            "status": "updated",
            "queue": name,
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Delete a queue at runtime.
async fn delete_queue_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &name) {
        return queue_not_found(&name);
    }
    match qm.delete_queue(&name) {
        Ok(()) => Json(serde_json::json!({
            "status": "deleted",
            "queue": name,
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Start draining a queue — reject new calls, let existing ones finish.
async fn drain_queue_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &name) {
        return queue_not_found(&name);
    }
    match qm.drain_queue(&name).await {
        Ok(()) => Json(serde_json::json!({
            "status": "draining",
            "queue": name,
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Stop draining a queue — resume accepting calls.
async fn undrain_queue_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &name) {
        return queue_not_found(&name);
    }
    match qm.undrain_queue(&name).await {
        Ok(()) => Json(serde_json::json!({
            "status": "active",
            "queue": name,
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Reload queue configuration from disk.
async fn reload_queues_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };

    let config_path = state.config_path.as_deref();
    let new_config = match crate::config::load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            return Json(serde_json::json!({
                "error": format!("Failed to load config: {}", e),
            }));
        }
    };

    let (effective_queue_config, queue_definitions) =
        match crate::queue_runtime_config::build_effective_queue_runtime(
            &new_config.queue,
            &state.storage,
            state.default_tenant_id,
        )
        .await
        {
            Ok(result) => result,
            Err(e) => {
                return Json(serde_json::json!({
                    "error": format!("Failed to build queue runtime config: {}", e),
                }));
            }
        };

    let new_queues = match qm.sync_queue_definitions(queue_definitions, effective_queue_config) {
        Ok(new_queues) => new_queues,
        Err(e) => {
            return Json(serde_json::json!({
                "error": format!("Failed to synchronize queue configuration: {}", e),
            }));
        }
    };

    // Spawn distribution loops for any newly added queues
    if let Some(ref qh) = state.queue_handler {
        for name in &new_queues {
            info!(queue = %name, "Spawning distribution loop for new queue");
            qh.spawn_distribution_loop_for(name.clone());
        }
    }

    Json(serde_json::json!({
        "status": "reloaded",
        "new_queues": new_queues,
    }))
}

#[derive(Deserialize)]
struct CreateQueueBody {
    key: String,
    #[serde(flatten)]
    config: fcp_queue::QueueConfig,
}

/// List all agents with stats.
async fn list_agents_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled", "agents": []}));
    };
    let stats = qm.all_agent_stats_for_tenant(&state.default_tenant_id);
    Json(serde_json::json!({ "agents": stats }))
}

/// Get a single agent's stats.
async fn get_agent_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    match qm.get_agent_for_tenant(&id, &state.default_tenant_id) {
        Some(_) => match qm.agent_stats(&id) {
            Some(stats) => Json(serde_json::json!({ "agent": stats })),
            None => agent_not_found(&id),
        },
        None => agent_not_found(&id),
    }
}

#[derive(Deserialize)]
struct PauseBody {
    duration_secs: Option<u32>,
    reason: Option<String>,
}

/// Pause an agent.
async fn pause_agent_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PauseBody>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if qm
        .get_agent_for_tenant(&id, &state.default_tenant_id)
        .is_none()
    {
        return agent_not_found(&id);
    }
    match qm
        .pause_agent(&id, body.duration_secs, body.reason.clone())
        .await
    {
        Ok(()) => Json(serde_json::json!({
            "status": "paused",
            "agent_id": id,
            "reason": body.reason,
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Unpause an agent.
async fn unpause_agent_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if qm
        .get_agent_for_tenant(&id, &state.default_tenant_id)
        .is_none()
    {
        return agent_not_found(&id);
    }
    match qm.unpause_agent(&id).await {
        Ok(()) => Json(serde_json::json!({"status": "available", "agent_id": id})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

#[derive(Deserialize)]
struct KickBody {
    reason: Option<String>,
}

/// Kick (force-logout) an agent.
async fn kick_agent_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<KickBody>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if qm
        .get_agent_for_tenant(&id, &state.default_tenant_id)
        .is_none()
    {
        return agent_not_found(&id);
    }
    let reason = body.reason.as_deref().unwrap_or("admin_kick");
    match qm.kick_agent(&id, reason).await {
        Ok(()) => Json(serde_json::json!({"status": "kicked", "agent_id": id})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

#[derive(Deserialize)]
struct LoginBody {
    extension: Option<String>,
    queues: Option<Vec<String>>,
    skills: Option<Vec<String>>,
}

/// Log an agent in via REST API.
async fn login_agent_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<LoginBody>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    let extension = body.extension.as_deref().unwrap_or(&id);
    let queues = body.queues.unwrap_or_default();
    let skills = body.skills.unwrap_or_default();
    if queues.is_empty() {
        // Auto-discover queues from config (same as feature code login)
        let config = qm.get_system_config();
        let auto_queues: Vec<String> = config
            .queues
            .iter()
            .filter(|(_, qc)| qc.agents.contains(&extension.to_string()))
            .filter(|(name, _)| queue_visible_to_service_tenant(qm, &state.default_tenant_id, name))
            .map(|(name, _)| name.clone())
            .collect();
        if auto_queues.is_empty() {
            return Json(serde_json::json!({
                "error": format!("Extension '{}' is not configured in any queue", extension)
            }));
        }
        match qm
            .agent_login(
                &id,
                extension,
                &auto_queues,
                state.default_tenant_id,
                skills,
            )
            .await
        {
            Ok(()) => Json(serde_json::json!({
                "status": "logged_in",
                "agent_id": id,
                "queues": auto_queues,
            })),
            Err(e) => Json(serde_json::json!({"error": e.to_string()})),
        }
    } else {
        if let Some(invalid_queue) = queues
            .iter()
            .find(|queue| !queue_visible_to_service_tenant(qm, &state.default_tenant_id, queue))
        {
            return queue_not_found(invalid_queue);
        }
        match qm
            .agent_login(&id, extension, &queues, state.default_tenant_id, skills)
            .await
        {
            Ok(()) => Json(serde_json::json!({
                "status": "logged_in",
                "agent_id": id,
                "queues": queues,
            })),
            Err(e) => Json(serde_json::json!({"error": e.to_string()})),
        }
    }
}

/// Log an agent out via REST API.
async fn logout_agent_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if qm
        .get_agent_for_tenant(&id, &state.default_tenant_id)
        .is_none()
    {
        return agent_not_found(&id);
    }
    match qm.agent_logout(&id).await {
        Ok(()) => Json(serde_json::json!({"status": "logged_out", "agent_id": id})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Extend an agent's wrap-up timer.
async fn extend_wrapup_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if qm
        .get_agent_for_tenant(&id, &state.default_tenant_id)
        .is_none()
    {
        return agent_not_found(&id);
    }
    match qm.extend_wrap_up(&id).await {
        Ok(extensions_used) => Json(serde_json::json!({
            "status": "extended",
            "agent_id": id,
            "extensions_used": extensions_used,
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

// ---- Campaign / Predictive Dialer Endpoints ----

#[derive(Deserialize)]
struct CampaignNumbersBody {
    numbers: Vec<fcp_queue::CampaignNumber>,
}

/// Upload a list of numbers for a campaign.
async fn campaign_load_numbers_handler(
    State(state): State<Arc<AppState>>,
    Path(queue): Path<String>,
    Json(body): Json<CampaignNumbersBody>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &queue) {
        return queue_not_found(&queue);
    }
    let count = body.numbers.len();
    qm.load_campaign_numbers(&queue, body.numbers);
    Json(serde_json::json!({
        "status": "loaded",
        "queue": queue,
        "numbers_loaded": count,
    }))
}

/// Get campaign statistics for a queue.
async fn campaign_stats_handler(
    State(state): State<Arc<AppState>>,
    Path(queue): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &queue) {
        return queue_not_found(&queue);
    }
    match qm.campaign_stats(&queue) {
        Some(stats) => Json(serde_json::json!({
            "queue": queue,
            "stats": stats,
        })),
        None => Json(serde_json::json!({
            "error": "No campaign data for this queue",
        })),
    }
}

/// Start the campaign for a queue.
async fn campaign_start_handler(
    State(state): State<Arc<AppState>>,
    Path(queue): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &queue) {
        return queue_not_found(&queue);
    }
    qm.set_campaign_running(&queue, true);
    Json(serde_json::json!({
        "status": "started",
        "queue": queue,
    }))
}

/// Stop the campaign for a queue.
async fn campaign_stop_handler(
    State(state): State<Arc<AppState>>,
    Path(queue): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &queue) {
        return queue_not_found(&queue);
    }
    qm.set_campaign_running(&queue, false);
    Json(serde_json::json!({
        "status": "stopped",
        "queue": queue,
    }))
}

// ---- Queue Transfer Endpoint ----

#[derive(Deserialize)]
struct TransferBody {
    to_queue: String,
}

/// Transfer a call from one queue to another.
async fn transfer_call_handler(
    State(state): State<Arc<AppState>>,
    Path((from, call_id)): Path<(String, String)>,
    Json(body): Json<TransferBody>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &from) {
        return queue_not_found(&from);
    }
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &body.to_queue) {
        return queue_not_found(&body.to_queue);
    }
    match qm.transfer_to_queue(&call_id, &from, &body.to_queue).await {
        Ok(position) => {
            // Update the session's queue_name if a handler is available
            if let Some(ref qh) = state.queue_handler {
                qh.update_session_queue(&call_id, &body.to_queue);
            }
            Json(serde_json::json!({
                "status": "transferred",
                "call_id": call_id,
                "from_queue": from,
                "to_queue": body.to_queue,
                "position": position,
            }))
        }
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

// ---- Supervisor Endpoints ----

#[derive(Deserialize)]
struct SupervisorBody {
    /// The supervisor: a user's extension or name, or a SIP URI.
    #[serde(alias = "supervisor")]
    supervisor_extension: String,
    /// For a whisper, who hears it: `a` (the caller) or `b` (the callee,
    /// the default — a queue call's agent).
    #[serde(default)]
    heard_by: Option<String>,
    /// The call's tenant, when the caller of this API scopes it (the
    /// admin API does); the node's default tenant otherwise.
    #[serde(default)]
    tenant_id: Option<TenantId>,
}

/// Start monitoring a call (listen only).
async fn monitor_call_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
    Json(body): Json<SupervisorBody>,
) -> Response {
    supervise_call(state, call_id, body, "monitor").await
}

/// Whisper on a call: one party hears the supervisor, the other does not.
async fn whisper_call_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
    Json(body): Json<SupervisorBody>,
) -> Response {
    supervise_call(state, call_id, body, "whisper").await
}

/// Barge into a call: both parties hear the supervisor.
async fn barge_call_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
    Json(body): Json<SupervisorBody>,
) -> Response {
    supervise_call(state, call_id, body, "barge").await
}

/// A supervisor onto a call (`docs/ENTERPRISE_CALLING.md` §3.11): a queue
/// call through the queue's own media, every other anchored call through
/// its media session's third party.
async fn supervise_call(
    state: Arc<AppState>,
    call_id: String,
    body: SupervisorBody,
    kind: &'static str,
) -> Response {
    if let Some(qh) = state
        .queue_handler
        .as_ref()
        .filter(|qh| qh.has_session(&call_id))
    {
        let mode = match kind {
            "monitor" => SupervisorMode::Monitor,
            "whisper" => SupervisorMode::Whisper,
            _ => SupervisorMode::Barge,
        };
        return match qh
            .start_supervisor(&call_id, &body.supervisor_extension, mode)
            .await
        {
            Ok(()) => Json(serde_json::json!({
                "status": "joined",
                "call_id": call_id,
                "supervisor": body.supervisor_extension,
                "mode": kind,
            }))
            .into_response(),
            Err(e) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response(),
        };
    }
    let Some(supervisions) = state.supervisions.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "supervision is not available on this node"})),
        )
            .into_response();
    };
    let Some(id) = scoped_call(&state, &call_id, body.tenant_id) else {
        return call_not_found(&call_id);
    };
    let heard_by =
        match body
            .heard_by
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("b") => fcp_media::MediaLeg::B,
            Some("a") => fcp_media::MediaLeg::A,
            Some(other) => return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("heard_by: `a` or `b`, not `{other}`")})),
            )
                .into_response(),
        };
    let mode = match kind {
        "monitor" => fcp_media::Supervision::Monitor,
        "whisper" => fcp_media::Supervision::Whisper(heard_by),
        _ => fcp_media::Supervision::Barge,
    };
    match supervisions
        .supervise(&id, &body.supervisor_extension, mode)
        .await
    {
        Ok(()) => Json(serde_json::json!({
            "status": "joined",
            "call_id": call_id,
            "supervisor": body.supervisor_extension,
            "mode": kind,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::from_u16(e.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
struct SupervisorQuery {
    #[serde(default)]
    tenant_id: Option<TenantId>,
}

/// The call `id` of `tenant` (the node's default tenant when unnamed).
pub(crate) fn scoped_call(
    state: &AppState,
    id: &str,
    tenant: Option<TenantId>,
) -> Option<fcp_b2bua_core::CallId> {
    let call_id = id.parse::<fcp_b2bua_core::CallId>().ok()?;
    let call = state.call_manager.get_call(&call_id)?;
    (call.tenant_id == tenant.unwrap_or(state.default_tenant_id)).then_some(call_id)
}

/// The call's supervisor, if it has one.
async fn supervisor_get_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
    Query(query): Query<SupervisorQuery>,
) -> Response {
    let Some(id) = scoped_call(&state, &call_id, query.tenant_id) else {
        return call_not_found(&call_id);
    };
    match state.supervisions.as_ref().and_then(|s| s.of(&id)) {
        Some(view) => Json(view).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "the call has no supervisor"})),
        )
            .into_response(),
    }
}

/// Remove the supervisor from a call.
async fn supervisor_leave_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
    Query(query): Query<SupervisorQuery>,
) -> Response {
    if let Some(qh) = state
        .queue_handler
        .as_ref()
        .filter(|qh| qh.has_session(&call_id))
    {
        return match qh.stop_supervisor(&call_id).await {
            Ok(()) => {
                Json(serde_json::json!({"status": "left", "call_id": call_id})).into_response()
            }
            Err(e) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response(),
        };
    }
    let Some(id) = scoped_call(&state, &call_id, query.tenant_id) else {
        return call_not_found(&call_id);
    };
    match state.supervisions.as_ref() {
        Some(s) if s.leave(&id).await => {
            Json(serde_json::json!({"status": "left", "call_id": call_id})).into_response()
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "the call has no supervisor"})),
        )
            .into_response(),
    }
}

// ---- Queue History Endpoint ----

#[derive(Deserialize)]
struct HistoryQuery {
    from: Option<u64>,
    to: Option<u64>,
    limit: Option<usize>,
}

/// Get historical stats snapshots for a queue.
async fn queue_history_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if !queue_visible_to_service_tenant(qm, &state.default_tenant_id, &name) {
        return queue_not_found(&name);
    }
    let limit = query.limit.unwrap_or(500).min(1440);
    let snapshots = qm.query_stats_history(&name, query.from, query.to, limit);
    Json(serde_json::json!({
        "queue": name,
        "snapshots": snapshots,
        "count": snapshots.len(),
    }))
}

// ---- Callback Endpoints ----

/// List all pending callbacks across all queues.
async fn list_callbacks_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled", "callbacks": []}));
    };
    let callbacks = qm.list_all_callbacks();
    Json(serde_json::json!({ "callbacks": callbacks }))
}

/// Cancel a pending callback by call_id.
async fn cancel_callback_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    if qm.cancel_callback(&call_id) {
        Json(serde_json::json!({
            "status": "cancelled",
            "call_id": call_id,
        }))
    } else {
        Json(serde_json::json!({
            "error": format!("Callback '{}' not found", call_id),
        }))
    }
}

// ---- WebSocket Wallboard Endpoint ----

#[derive(Deserialize)]
struct WallboardQuery {
    token: Option<String>,
}

/// Extract a bearer token from the Authorization header value.
fn extract_bearer_token(header_value: &str) -> Option<&str> {
    header_value
        .strip_prefix("Bearer ")
        .or_else(|| header_value.strip_prefix("bearer "))
        .map(|t| t.trim())
}

/// WebSocket upgrade handler for real-time wallboard stats.
///
/// Subscribes to `QueueEvent::StatsSnapshot` on the event bus and streams
/// each snapshot as JSON to the connected WebSocket client.
///
/// If `api_token` is configured, validates bearer token from the
/// `Authorization` header or `?token=` query parameter before upgrading.
async fn wallboard_ws_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WallboardQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Check auth if api_token is configured
    if let Some(ref expected_token) = state.api_token {
        // Try Authorization header first, then query param
        let provided_token = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(extract_bearer_token)
            .map(|s| s.to_string())
            .or(query.token);

        match provided_token {
            Some(ref token) if fcp_core::secret::constant_time_eq(token, expected_token) => {
                // Auth passed
            }
            _ => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "Invalid or missing authentication token"})),
                )
                    .into_response();
            }
        }
    }

    ws.on_upgrade(move |socket| wallboard_ws_session(socket, state))
        .into_response()
}

/// Run a wallboard WebSocket session.
async fn wallboard_ws_session(mut socket: WebSocket, state: Arc<AppState>) {
    // Subscribe to Queue events only
    let filter = EventFilter::new()
        .tenant(state.default_tenant_id)
        .types(vec![EventType::Queue]);
    let mut stream = match state.event_bus.subscribe(filter).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "Failed to subscribe to event bus for wallboard");
            let _ = socket.close().await;
            return;
        }
    };

    info!("Wallboard WebSocket client connected");

    loop {
        tokio::select! {
            // Receive events from the bus
            event = stream.next() => {
                match event {
                    Some(PlatformEvent::Queue(fcp_events::QueueEvent::StatsSnapshot { stats, .. })) => {
                        let json = serde_json::json!({
                            "type": "stats_snapshot",
                            "stats": stats,
                        });
                        if let Err(e) = socket.send(Message::Text(json.to_string())).await {
                            warn!(error = %e, "Wallboard WebSocket send failed, closing");
                            break;
                        }
                    }
                    Some(_) => {
                        // Non-StatsSnapshot queue event — skip
                    }
                    None => {
                        // Event stream ended
                        warn!("Wallboard event stream ended unexpectedly");
                        break;
                    }
                }
            }
            // Handle incoming WebSocket messages (ping/pong/close)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        info!("Wallboard WebSocket client disconnected");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(_)) => {} // Ignore text/binary from client
                    Some(Err(e)) => {
                        warn!(error = %e, "Wallboard WebSocket recv error");
                        break;
                    }
                }
            }
        }
    }
}

// ── Presence endpoints ──────────────────────────────────────────────────

/// List all tracked presentities.
async fn list_presence_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let Some(ref pm) = state.presence_manager else {
        return Json(
            serde_json::json!({"error": "Presence service not enabled", "presentities": []}),
        );
    };

    let scoped_presences = pm.get_presences_by_tenant(&state.default_tenant_id);
    let total_watchers: usize = scoped_presences
        .iter()
        .map(|presence| {
            pm.get_watchers_for_tenant(&presence.aor, &state.default_tenant_id)
                .len()
        })
        .sum();
    let presentities: Vec<serde_json::Value> = scoped_presences
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "aor": p.aor,
                "state": p.state.to_string(),
                "note": p.note,
            })
        })
        .collect();
    let total_presentities = presentities.len();

    Json(serde_json::json!({
        "presentities": presentities,
        "total": total_presentities,
        "total_watchers": total_watchers,
    }))
}

/// Get presence for a specific AOR.
async fn get_presence_handler(
    State(state): State<Arc<AppState>>,
    Path(aor): Path<String>,
) -> impl IntoResponse {
    let Some(ref pm) = state.presence_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Presence service not enabled"})),
        )
            .into_response();
    };

    // URL-decode the AOR (@ may be encoded as %40)

    match pm.get_presence_for_tenant(&aor, &state.default_tenant_id) {
        Some(info) => {
            let watchers: Vec<serde_json::Value> = pm
                .get_watchers_for_tenant(&aor, &state.default_tenant_id)
                .into_iter()
                .map(|w| serde_json::json!({"watcher_aor": w.watcher_aor, "subscription_id": w.subscription_id}))
                .collect();

            let pidf = pm.generate_pidf_for_tenant(&aor, &state.default_tenant_id);

            Json(serde_json::json!({
                "aor": info.aor,
                "state": info.state.to_string(),
                "note": info.note,
                "watchers": watchers,
                "pidf_xml": pidf,
            }))
            .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Presentity '{}' not found", aor)})),
        )
            .into_response(),
    }
}

/// Update presence for a specific AOR.
async fn update_presence_handler(
    State(state): State<Arc<AppState>>,
    Path(aor): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(ref pm) = state.presence_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Presence service not enabled"})),
        )
            .into_response();
    };

    let state_str = body["state"].as_str().unwrap_or("open");
    let presence_state = match state_str {
        "open" => fcp_presence::PresenceState::Open,
        "closed" => fcp_presence::PresenceState::Closed,
        "away" => fcp_presence::PresenceState::Away,
        "busy" => fcp_presence::PresenceState::Busy,
        "on_the_phone" => fcp_presence::PresenceState::OnThePhone,
        "ringing" => fcp_presence::PresenceState::Ringing,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Unknown state: {}", other)})),
            )
                .into_response();
        }
    };

    let note = body["note"].as_str().map(|s| s.to_string());

    let update = fcp_presence::PresenceUpdate {
        aor,
        state: presence_state,
        note,
    };

    match pm.update_presence(update, &state.default_tenant_id).await {
        Ok(info) => Json(serde_json::json!({
            "aor": info.aor,
            "state": info.state.to_string(),
            "note": info.note,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Internal cluster endpoints (cross-node operations)
// ---------------------------------------------------------------------------

/// Log distributed tracing context from incoming internal request headers.
///
/// Extracts `traceparent` (W3C Trace Context) and `x-fcp-node-id` headers
/// and logs them for cross-node request correlation.
fn log_trace_context(headers: &HeaderMap) {
    let trace_id = headers
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");
    let source_node = headers
        .get("x-fcp-node-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    tracing::debug!(
        trace_id = %trace_id,
        source_node = %source_node,
        "Internal cluster request received"
    );
}

/// Request body for POST /internal/hangup
#[derive(Deserialize)]
struct InternalHangupRequest {
    call_id: String,
    reason: Option<String>,
}

/// Hang up a call on this node. Used by other nodes for cross-node call control.
///
/// POST /internal/hangup
async fn internal_hangup_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<InternalHangupRequest>,
) -> impl IntoResponse {
    // Authenticate via cluster secret
    if let Err(resp) = verify_cluster_secret(&state, &headers) {
        return resp;
    }

    log_trace_context(&headers);

    let call_id = match req.call_id.parse::<fcp_b2bua_core::CallId>() {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid call_id: {}", e)})),
            )
                .into_response();
        }
    };

    let _ = state.call_manager.terminate_call_with(
        &call_id,
        fcp_sip::TerminationReason::system(
            fcp_sip::SystemCause::Admin,
            req.reason
                .unwrap_or_else(|| "Internal hangup request".to_string()),
        ),
    );

    info!(call_id = %call_id, "Internal hangup request processed");
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// Request body for POST /internal/supervisor
#[derive(Deserialize)]
struct InternalSupervisorRequest {
    call_id: String,
    mode: String, // "monitor", "whisper", "barge"
}

/// Inject a supervisor session into a call on this node.
///
/// POST /internal/supervisor
async fn internal_supervisor_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<InternalSupervisorRequest>,
) -> impl IntoResponse {
    // Authenticate via cluster secret
    if let Err(resp) = verify_cluster_secret(&state, &headers) {
        return resp;
    }

    log_trace_context(&headers);

    let mode = match req.mode.as_str() {
        "monitor" => SupervisorMode::Monitor,
        "whisper" => SupervisorMode::Whisper,
        "barge" => SupervisorMode::Barge,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid mode: {}", req.mode)})),
            )
                .into_response();
        }
    };

    if let Some(ref qh) = state.queue_handler {
        match qh.change_supervisor_mode(&req.call_id, mode).await {
            Ok(()) => {
                info!(call_id = %req.call_id, mode = %req.mode, "Internal supervisor request processed");
                (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Queue handler not available"})),
        )
            .into_response()
    }
}

/// Verify the cluster secret from the Authorization header.
// The error type is an axum `Response`, which is large by nature; boxing it
// here would only add indirection at every call site.
#[allow(clippy::result_large_err)]
fn verify_cluster_secret(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), axum::response::Response> {
    // If no api_token is configured, skip auth (single-node mode)
    let secret = match &state.api_token {
        Some(s) => s,
        None => return Ok(()),
    };

    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);

    if fcp_core::secret::constant_time_eq(token, secret) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid cluster secret"})),
        )
            .into_response())
    }
}

// ---------------------------------------------------------------------------
// POST /internal/unpark — unpark a call on this node (cross-node pickup)
// ---------------------------------------------------------------------------

/// Request body for POST /internal/unpark
#[derive(Deserialize)]
struct InternalUnparkRequest {
    orbit: u16,
}

/// Unpark a call on this node. Called by another node's retrieve handler.
///
/// This endpoint:
/// 1. Retrieves the call from the local ParkingManager
/// 2. Removes it from the cluster parking store (Redis)
/// 3. Returns the parked call info so the calling node knows it succeeded
///
/// The actual SIP bridging (unhold A-leg, INVITE retriever) is handled by the
/// calling node or a subsequent step — this endpoint only releases the parking
/// state on the owning node.
///
/// POST /internal/unpark
async fn internal_unpark_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<InternalUnparkRequest>,
) -> impl IntoResponse {
    // Authenticate via cluster secret
    if let Err(resp) = verify_cluster_secret(&state, &headers) {
        return resp;
    }

    log_trace_context(&headers);

    let pbx = match &state.pbx_handler {
        Some(p) => p,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "PBX handler not available"})),
            )
                .into_response();
        }
    };

    // Retrieve from local parking manager
    match pbx.retrieve_parked_call(req.orbit) {
        Ok(parked) => {
            info!(
                orbit = req.orbit,
                call_id = %parked.call_id,
                "Internal unpark: retrieved parked call"
            );

            // Also remove from cluster store
            if let Some(ref cp) = state.cluster_parking {
                let cp = Arc::clone(cp);
                let orbit = req.orbit;
                tokio::spawn(async move {
                    if let Err(e) = cp.cluster_unpark(orbit).await {
                        tracing::warn!(orbit = orbit, error = %e, "Failed to clean up cluster parking entry");
                    }
                });
            }

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "call_id": parked.call_id.to_string(),
                    "orbit": parked.orbit,
                    "parker_extension": parked.parker_extension,
                })),
            )
                .into_response()
        }
        Err(e) => {
            info!(orbit = req.orbit, error = %e, "Internal unpark: orbit not occupied");
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Orbit not occupied: {}", e)})),
            )
                .into_response()
        }
    }
}
