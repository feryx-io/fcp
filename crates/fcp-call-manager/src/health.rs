//! HTTP health/status/calls API for fcp-call-manager.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use fcp_b2bua_core::CallManager;
use fcp_core::storage::{CallDisposition, CdrQuery, CdrStore};
use fcp_core::{CallId, DynStorage};
use fcp_media::MediaManager;
use fcp_queue::QueueManager;
use crate::queue_handler::{QueueCallHandler, SupervisorMode};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

/// Shared state for the HTTP API.
pub struct AppState {
    pub call_manager: Arc<CallManager>,
    pub storage: Arc<DynStorage>,
    pub media_manager: Arc<MediaManager>,
    pub queue_manager: Option<Arc<QueueManager>>,
    pub queue_handler: Option<Arc<QueueCallHandler>>,
    pub start_time: Instant,
}

/// Run the HTTP health/status/calls server.
pub async fn run_http_server(bind_addr: SocketAddr, state: Arc<AppState>) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        .route("/calls", get(calls_handler))
        .route("/registrations", get(registrations_handler))
        .route("/cdrs", get(cdrs_handler))
        .route("/media-sessions", get(media_sessions_handler))
        .route("/transcoding", get(transcoding_handler))
        // Queue/Agent management endpoints
        .route("/queues", get(list_queues_handler))
        .route("/queues/{name}", get(get_queue_handler))
        .route("/agents", get(list_agents_handler))
        .route("/agents/{id}", get(get_agent_handler))
        .route("/agents/{id}/pause", post(pause_agent_handler))
        .route("/agents/{id}/unpause", post(unpause_agent_handler))
        .route("/agents/{id}/kick", post(kick_agent_handler))
        .route("/agents/{id}/login", post(login_agent_handler))
        .route("/agents/{id}/logout", post(logout_agent_handler))
        // Supervisor endpoints
        .route("/calls/{id}/monitor", post(monitor_call_handler))
        .route("/calls/{id}/whisper", post(whisper_call_handler))
        .route("/calls/{id}/barge", post(barge_call_handler))
        .route("/calls/{id}/supervisor/leave", post(supervisor_leave_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "HTTP API server started");
    axum::serve(listener, app).await?;
    Ok(())
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
    let transcoding_sessions: Vec<&fcp_media::RtpSessionInfo> = sessions
        .iter()
        .filter(|s| s.transcoding_active)
        .collect();

    // Build codec pairs summary
    let mut codec_pairs: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
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
            }))
        }
        None => Json(serde_json::json!({"error": format!("Queue '{}' not found", name)})),
    }
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
    match qm.pause_agent(&id, body.duration_secs).await {
        Ok(()) => Json(serde_json::json!({"status": "paused", "agent_id": id})),
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
    let tenant_id = fcp_core::TenantId::nil();

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
            .agent_login(&id, extension, &auto_queues, tenant_id, skills)
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
            .agent_login(&id, extension, &queues, tenant_id, skills)
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
    match qh.start_supervisor(&call_id, &body.supervisor_extension, SupervisorMode::Monitor).await {
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
    match qh.start_supervisor(&call_id, &body.supervisor_extension, SupervisorMode::Whisper).await {
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
    match qh.start_supervisor(&call_id, &body.supervisor_extension, SupervisorMode::Barge).await {
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
