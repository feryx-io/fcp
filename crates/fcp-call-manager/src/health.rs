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
use fcp_core::{CallId, DynStorage, TenantId};
use fcp_events::filter::EventType;
use fcp_events::{EventFilter, EventSubscriber, InMemoryEventBus, PlatformEvent};
use fcp_media::MediaManager;
use fcp_queue::QueueManager;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

/// Shared state for the HTTP API.
pub struct AppState {
    pub call_manager: Arc<CallManager>,
    pub storage: Arc<DynStorage>,
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
}

/// Run the HTTP health/status/calls server.
pub async fn run_http_server(bind_addr: SocketAddr, state: Arc<AppState>) -> anyhow::Result<()> {
    let protected = Router::new()
        .route("/status", get(status_handler))
        .route("/calls", get(calls_handler))
        .route("/registrations", get(registrations_handler))
        .route("/cdrs", get(cdrs_handler))
        .route("/media-sessions", get(media_sessions_handler))
        .route("/transcoding", get(transcoding_handler))
        // Queue/Agent management endpoints
        .route(
            "/queues",
            get(list_queues_handler).post(create_queue_handler),
        )
        .route(
            "/queues/{name}",
            get(get_queue_handler)
                .put(update_queue_handler)
                .delete(delete_queue_handler),
        )
        .route(
            "/queues/{name}/drain",
            post(drain_queue_handler).delete(undrain_queue_handler),
        )
        .route("/queues/reload", post(reload_queues_handler))
        .route("/queues/{name}/history", get(queue_history_handler))
        .route("/agents", get(list_agents_handler))
        .route("/agents/{id}", get(get_agent_handler))
        .route("/agents/{id}/pause", post(pause_agent_handler))
        .route("/agents/{id}/unpause", post(unpause_agent_handler))
        .route("/agents/{id}/kick", post(kick_agent_handler))
        .route("/agents/{id}/login", post(login_agent_handler))
        .route("/agents/{id}/logout", post(logout_agent_handler))
        .route("/agents/{id}/extend-wrapup", post(extend_wrapup_handler))
        // Callback endpoints
        .route("/callbacks", get(list_callbacks_handler))
        .route("/callbacks/{call_id}", delete(cancel_callback_handler))
        // Campaign / predictive dialer endpoints
        .route(
            "/campaigns/{queue}/numbers",
            post(campaign_load_numbers_handler),
        )
        .route("/campaigns/{queue}/stats", get(campaign_stats_handler))
        .route("/campaigns/{queue}/start", post(campaign_start_handler))
        .route("/campaigns/{queue}/stop", post(campaign_stop_handler))
        // Queue transfer endpoint
        .route(
            "/queues/{from}/transfer/{call_id}",
            post(transfer_call_handler),
        )
        // Supervisor endpoints
        .route("/calls/{id}/monitor", post(monitor_call_handler))
        .route("/calls/{id}/whisper", post(whisper_call_handler))
        .route("/calls/{id}/barge", post(barge_call_handler))
        .route(
            "/calls/{id}/supervisor/leave",
            post(supervisor_leave_handler),
        )
        // Presence endpoints
        .route("/presence", get(list_presence_handler))
        .route(
            "/presence/{aor}",
            get(get_presence_handler).put(update_presence_handler),
        )
        // Prometheus metrics endpoint
        .route("/metrics", get(metrics_handler))
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
        Some(token) if constant_time_compare(token, expected_token) => next.run(request).await,
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
    }))
}

async fn calls_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let calls = state.call_manager.all_calls();
    let call_list: Vec<serde_json::Value> = calls
        .iter()
        .map(|call| {
            let elapsed = call.created_at.elapsed().map(|d| d.as_secs()).unwrap_or(0);
            serde_json::json!({
                "call_id": call.id.to_string(),
                "state": format!("{:?}", call.state),
                "direction": format!("{:?}", call.a_leg.direction),
                "caller": call.caller(),
                "callee": call.callee().unwrap_or(""),
                "duration_seconds": elapsed,
            })
        })
        .collect();

    Json(serde_json::json!({ "calls": call_list }))
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

    // If AOR is provided, filter by that AOR; otherwise return all registrations
    let registrations_result = match query.aor {
        Some(ref aor) => state.storage.get_registrations(aor).await,
        None => state.storage.get_all_registrations().await,
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

    // Parse tenant_id if provided
    if let Some(ref tenant_id_str) = params.tenant_id {
        match tenant_id_str.parse::<TenantId>() {
            Ok(tenant_id) => query.tenant_id = Some(tenant_id),
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
                        "call_id": cdr.call_id.to_string(),
                        "tenant_id": cdr.tenant_id.to_string(),
                        "sip_call_id": cdr.sip_call_id,
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
async fn metrics_handler() -> impl IntoResponse {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let families = prometheus::default_registry().gather();
    let mut buffer = Vec::new();
    encoder.encode(&families, &mut buffer).unwrap_or_default();
    let body = String::from_utf8(buffer).unwrap_or_default();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

// ---- Queue/Agent Management Endpoints ----

/// List all queues with stats.
async fn list_queues_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let Some(ref qm) = state.queue_manager else {
        return Json(serde_json::json!({"error": "Queue system not enabled", "queues": []}));
    };
    let stats = qm.all_queue_stats();
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
    match qm.queue_stats(&name) {
        Some(stats) => {
            let agents = qm.list_agents_for_queue(&name);
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
        None => Json(serde_json::json!({"error": format!("Queue '{}' not found", name)})),
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

    let new_queues = qm.reload_config(new_config.queue.clone());

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
    let stats = qm.all_agent_stats();
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
    match qm.agent_stats(&id) {
        Some(stats) => Json(serde_json::json!({ "agent": stats })),
        None => Json(serde_json::json!({"error": format!("Agent '{}' not found", id)})),
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
    supervisor_extension: String,
}

/// Start monitoring a call (listen only).
async fn monitor_call_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
    Json(body): Json<SupervisorBody>,
) -> Json<serde_json::Value> {
    let Some(ref qh) = state.queue_handler else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    match qh
        .start_supervisor(
            &call_id,
            &body.supervisor_extension,
            SupervisorMode::Monitor,
        )
        .await
    {
        Ok(()) => Json(serde_json::json!({
            "status": "monitoring",
            "call_id": call_id,
            "supervisor_extension": body.supervisor_extension,
            "mode": "monitor",
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Start whispering to the agent on a call (agent hears supervisor, caller does not).
async fn whisper_call_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
    Json(body): Json<SupervisorBody>,
) -> Json<serde_json::Value> {
    let Some(ref qh) = state.queue_handler else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    match qh
        .start_supervisor(
            &call_id,
            &body.supervisor_extension,
            SupervisorMode::Whisper,
        )
        .await
    {
        Ok(()) => Json(serde_json::json!({
            "status": "whispering",
            "call_id": call_id,
            "supervisor_extension": body.supervisor_extension,
            "mode": "whisper",
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Barge into a call (full three-way conference).
async fn barge_call_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
    Json(body): Json<SupervisorBody>,
) -> Json<serde_json::Value> {
    let Some(ref qh) = state.queue_handler else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    match qh
        .start_supervisor(&call_id, &body.supervisor_extension, SupervisorMode::Barge)
        .await
    {
        Ok(()) => Json(serde_json::json!({
            "status": "barging",
            "call_id": call_id,
            "supervisor_extension": body.supervisor_extension,
            "mode": "barge",
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Remove the supervisor from a call.
async fn supervisor_leave_handler(
    State(state): State<Arc<AppState>>,
    Path(call_id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref qh) = state.queue_handler else {
        return Json(serde_json::json!({"error": "Queue system not enabled"}));
    };
    match qh.stop_supervisor(&call_id).await {
        Ok(()) => Json(serde_json::json!({
            "status": "supervisor_removed",
            "call_id": call_id,
        })),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
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

/// Constant-time string comparison to prevent timing attacks.
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
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
            Some(ref token) if constant_time_compare(token, expected_token) => {
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
    let filter = EventFilter::new().types(vec![EventType::Queue]);
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

    let presences: Vec<serde_json::Value> = pm
        .get_all_presences()
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "aor": p.aor,
                "state": p.state.to_string(),
                "note": p.note,
            })
        })
        .collect();

    Json(serde_json::json!({
        "presentities": presences,
        "total": pm.presentity_count(),
        "total_watchers": pm.total_watcher_count(),
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

    match pm.get_presence(&aor) {
        Some(info) => {
            let watchers: Vec<serde_json::Value> = pm
                .get_watchers(&aor)
                .into_iter()
                .map(|w| serde_json::json!({"watcher_aor": w.watcher_aor, "subscription_id": w.subscription_id}))
                .collect();

            let pidf = pm.generate_pidf(&aor);

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

    let _ = state.call_manager.terminate_call(&call_id, req.reason);

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

    if constant_time_compare(token, secret) {
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
