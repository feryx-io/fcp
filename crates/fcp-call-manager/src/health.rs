//! HTTP health/status/calls API for fcp-call-manager.

use axum::{extract::Query, extract::State, routing::get, Json, Router};
use fcp_b2bua_core::CallManager;
use fcp_core::storage::{CallDisposition, CdrQuery, CdrStore};
use fcp_core::{CallId, DynStorage};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

/// Shared state for the HTTP API.
pub struct AppState {
    pub call_manager: Arc<CallManager>,
    pub storage: Arc<DynStorage>,
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

    let aor = match query.aor {
        Some(aor) => aor,
        None => {
            return Json(serde_json::json!({
                "registrations": [],
                "hint": "Pass ?aor=user@domain to query registrations for a specific AOR"
            }));
        }
    };

    match state.storage.get_registrations(&aor).await {
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

            Json(serde_json::json!({ "registrations": reg_list }))
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
