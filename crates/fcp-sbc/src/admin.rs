//! Admin API endpoints for the SBC.
//!
//! Provides runtime management of trunks, calls, and admission control
//! under the `/admin/` path prefix.

use crate::health::HealthState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct TrunkSummary {
    pub name: String,
    pub enabled: bool,
    pub admin_disabled: bool,
    pub health_status: String,
    pub direction: String,
    pub concurrent_calls: u64,
    pub max_concurrent_calls: u32,
    pub priority: u32,
    pub weight: u32,
}

#[derive(Serialize)]
pub struct TrunkDetail {
    pub name: String,
    pub enabled: bool,
    pub admin_disabled: bool,
    pub health_status: String,
    pub direction: String,
    pub concurrent_calls: u64,
    pub max_concurrent_calls: u32,
    pub priority: u32,
    pub weight: u32,
    pub hosts: Vec<String>,
    pub transport_protocol: String,
    pub transport_port: Option<u16>,
    pub consecutive_failures: u32,
    pub has_credentials: bool,
    pub register: bool,
}

#[derive(Serialize)]
pub struct TrunkActionResponse {
    pub name: String,
    pub action: String,
    pub admin_disabled: bool,
}

#[derive(Serialize)]
pub struct CallSummary {
    pub call_id: String,
    pub state: String,
    pub caller: String,
    pub callee: String,
    pub created_at: f64,
    pub duration_secs: Option<f64>,
}

#[derive(Serialize)]
pub struct CallDetail {
    pub call_id: String,
    pub state: String,
    pub caller: String,
    pub callee: String,
    pub created_at: f64,
    pub duration_secs: Option<f64>,
    pub established_at: Option<f64>,
    pub a_leg_remote_uri: String,
    pub a_leg_contact: Option<String>,
    pub b_leg_remote_uri: Option<String>,
    pub b_leg_contact: Option<String>,
    pub media_anchored: bool,
    pub failure_reason: Option<String>,
}

#[derive(Serialize)]
pub struct CallStatsResponse {
    pub total: usize,
    pub active: usize,
    pub established: usize,
    pub terminated: usize,
    pub failed: usize,
}

#[derive(Serialize)]
pub struct AdmissionStatus {
    pub config: AdmissionConfigView,
    pub counters: AdmissionCounters,
}

#[derive(Serialize)]
pub struct AdmissionConfigView {
    pub max_cps: u32,
    pub max_concurrent_calls: u32,
    pub max_concurrent_registrations: u32,
    pub per_trunk_cps: Vec<TrunkCpsView>,
    pub overload_action: String,
    pub retry_after_secs: u64,
}

#[derive(Serialize)]
pub struct TrunkCpsView {
    pub trunk_name: String,
    pub max_cps: u32,
}

#[derive(Serialize)]
pub struct AdmissionCounters {
    pub current_cps: u64,
    pub concurrent_calls: u64,
    pub concurrent_registrations: u64,
}

#[derive(Deserialize)]
pub struct UpdateAdmissionRequest {
    pub max_cps: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub max_concurrent_registrations: Option<u32>,
    pub per_trunk_cps: Option<Vec<TrunkCpsUpdate>>,
    pub overload_action: Option<String>,
    pub retry_after_secs: Option<u64>,
}

#[derive(Deserialize)]
pub struct TrunkCpsUpdate {
    pub trunk_name: String,
    pub max_cps: u32,
}

#[derive(Serialize)]
pub struct AdminError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

// ── Router ──────────────────────────────────────────────────────────────────

pub fn admin_router() -> Router<Arc<HealthState>> {
    Router::new()
        .route("/trunks", get(list_trunks))
        .route("/trunks/:name", get(get_trunk))
        .route("/trunks/:name/disable", axum::routing::post(disable_trunk))
        .route("/trunks/:name/enable", axum::routing::post(enable_trunk))
        .route("/calls", get(list_calls))
        .route("/calls/stats", get(call_stats))
        .route("/calls/:id", get(get_call).delete(terminate_call))
        .route("/admission", get(get_admission).put(update_admission))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn system_time_to_epoch_secs(t: SystemTime) -> f64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn call_state_str(state: fcp_b2bua_core::CallState) -> &'static str {
    use fcp_b2bua_core::CallState;
    match state {
        CallState::Initiated => "initiated",
        CallState::RoutingPending => "routing_pending",
        CallState::LookingUpDestination => "looking_up_destination",
        CallState::Inviting => "inviting",
        CallState::Proceeding => "proceeding",
        CallState::Established => "established",
        CallState::Terminating => "terminating",
        CallState::Terminated => "terminated",
        CallState::Failed => "failed",
    }
}

fn health_status_str(status: fcp_sbc_core::metrics::TrunkHealthStatus) -> &'static str {
    use fcp_sbc_core::metrics::TrunkHealthStatus;
    match status {
        TrunkHealthStatus::Healthy => "healthy",
        TrunkHealthStatus::Degraded => "degraded",
        TrunkHealthStatus::Down => "down",
        TrunkHealthStatus::Unknown => "unknown",
    }
}

fn direction_str(dir: fcp_sbc_core::trunk::TrunkDirection) -> &'static str {
    use fcp_sbc_core::trunk::TrunkDirection;
    match dir {
        TrunkDirection::Upstream => "upstream",
        TrunkDirection::Downstream => "downstream",
        TrunkDirection::Bidirectional => "bidirectional",
    }
}

fn admin_err(status: StatusCode, msg: &str) -> (StatusCode, Json<AdminError>) {
    (
        status,
        Json(AdminError {
            error: msg.to_string(),
            details: None,
        }),
    )
}

fn admin_err_detail(
    status: StatusCode,
    msg: &str,
    detail: &str,
) -> (StatusCode, Json<AdminError>) {
    (
        status,
        Json(AdminError {
            error: msg.to_string(),
            details: Some(detail.to_string()),
        }),
    )
}

// ── Trunk handlers ──────────────────────────────────────────────────────────

async fn list_trunks(
    State(state): State<Arc<HealthState>>,
) -> Json<Vec<TrunkSummary>> {
    let summaries = state
        .trunk_configs
        .iter()
        .map(|tc| {
            let status = state
                .health_monitor
                .status(&tc.name)
                .unwrap_or(fcp_sbc_core::metrics::TrunkHealthStatus::Unknown);
            TrunkSummary {
                name: tc.name.clone(),
                enabled: tc.enabled,
                admin_disabled: state.health_monitor.is_admin_disabled(&tc.name),
                health_status: health_status_str(status).to_string(),
                direction: direction_str(tc.direction).to_string(),
                concurrent_calls: state.health_monitor.concurrent_calls(&tc.name),
                max_concurrent_calls: tc.max_concurrent_calls,
                priority: tc.priority,
                weight: tc.weight,
            }
        })
        .collect();
    Json(summaries)
}

async fn get_trunk(
    State(state): State<Arc<HealthState>>,
    Path(name): Path<String>,
) -> Result<Json<TrunkDetail>, (StatusCode, Json<AdminError>)> {
    let tc = state
        .trunk_configs
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| admin_err(StatusCode::NOT_FOUND, "trunk not found"))?;

    let status = state
        .health_monitor
        .status(&tc.name)
        .unwrap_or(fcp_sbc_core::metrics::TrunkHealthStatus::Unknown);

    Ok(Json(TrunkDetail {
        name: tc.name.clone(),
        enabled: tc.enabled,
        admin_disabled: state.health_monitor.is_admin_disabled(&tc.name),
        health_status: health_status_str(status).to_string(),
        direction: direction_str(tc.direction).to_string(),
        concurrent_calls: state.health_monitor.concurrent_calls(&tc.name),
        max_concurrent_calls: tc.max_concurrent_calls,
        priority: tc.priority,
        weight: tc.weight,
        hosts: tc.hosts.clone(),
        transport_protocol: format!("{:?}", tc.transport.protocol),
        transport_port: tc.transport.port,
        consecutive_failures: state.health_monitor.consecutive_failures(&tc.name),
        has_credentials: tc.credentials.is_some(),
        register: tc.register,
    }))
}

async fn disable_trunk(
    State(state): State<Arc<HealthState>>,
    Path(name): Path<String>,
) -> Result<Json<TrunkActionResponse>, (StatusCode, Json<AdminError>)> {
    if !state.health_monitor.admin_disable(&name) {
        return Err(admin_err(StatusCode::NOT_FOUND, "trunk not found"));
    }
    Ok(Json(TrunkActionResponse {
        name,
        action: "disabled".to_string(),
        admin_disabled: true,
    }))
}

async fn enable_trunk(
    State(state): State<Arc<HealthState>>,
    Path(name): Path<String>,
) -> Result<Json<TrunkActionResponse>, (StatusCode, Json<AdminError>)> {
    if !state.health_monitor.admin_enable(&name) {
        return Err(admin_err(StatusCode::NOT_FOUND, "trunk not found"));
    }
    Ok(Json(TrunkActionResponse {
        name,
        action: "enabled".to_string(),
        admin_disabled: false,
    }))
}

// ── Call handlers ───────────────────────────────────────────────────────────

async fn list_calls(
    State(state): State<Arc<HealthState>>,
) -> Json<Vec<CallSummary>> {
    let calls = state.call_manager.active_calls();
    let summaries = calls
        .into_iter()
        .map(|c| CallSummary {
            call_id: c.id.to_string(),
            state: call_state_str(c.state).to_string(),
            caller: c.caller().to_string(),
            callee: c.callee().unwrap_or("").to_string(),
            created_at: system_time_to_epoch_secs(c.created_at),
            duration_secs: c.duration().map(|d| d.as_secs_f64()),
        })
        .collect();
    Json(summaries)
}

async fn call_stats(
    State(state): State<Arc<HealthState>>,
) -> Json<CallStatsResponse> {
    let stats = state.call_manager.stats();
    Json(CallStatsResponse {
        total: stats.total,
        active: stats.active,
        established: stats.established,
        terminated: stats.terminated,
        failed: stats.failed,
    })
}

async fn get_call(
    State(state): State<Arc<HealthState>>,
    Path(id): Path<String>,
) -> Result<Json<CallDetail>, (StatusCode, Json<AdminError>)> {
    let call_id: fcp_b2bua_core::CallId = id
        .parse()
        .map_err(|_| admin_err(StatusCode::BAD_REQUEST, "invalid call ID"))?;

    let call = state
        .call_manager
        .get_call(&call_id)
        .ok_or_else(|| admin_err(StatusCode::NOT_FOUND, "call not found"))?;

    Ok(Json(CallDetail {
        call_id: call.id.to_string(),
        state: call_state_str(call.state).to_string(),
        caller: call.caller().to_string(),
        callee: call.callee().unwrap_or("").to_string(),
        created_at: system_time_to_epoch_secs(call.created_at),
        duration_secs: call.duration().map(|d| d.as_secs_f64()),
        established_at: call.established_at.map(system_time_to_epoch_secs),
        a_leg_remote_uri: call.a_leg.remote_uri.clone(),
        a_leg_contact: call.a_leg.contact.map(|a| a.to_string()),
        b_leg_remote_uri: call.b_leg.as_ref().map(|l| l.remote_uri.clone()),
        b_leg_contact: call.b_leg.as_ref().and_then(|l| l.contact.map(|a| a.to_string())),
        media_anchored: call.media_anchored,
        failure_reason: call.failure_reason.clone(),
    }))
}

async fn terminate_call(
    State(state): State<Arc<HealthState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<AdminError>)> {
    let call_id: fcp_b2bua_core::CallId = id
        .parse()
        .map_err(|_| admin_err(StatusCode::BAD_REQUEST, "invalid call ID"))?;

    state
        .call_manager
        .terminate_call(&call_id, Some("admin API termination".to_string()))
        .map_err(|e| admin_err_detail(StatusCode::NOT_FOUND, "call not found", &e.to_string()))?;

    Ok(Json(serde_json::json!({
        "call_id": call_id.to_string(),
        "action": "terminated",
        "note": "Internal state updated. On-wire BYE signaling is a future enhancement."
    })))
}

// ── Admission handlers ──────────────────────────────────────────────────────

async fn get_admission(
    State(state): State<Arc<HealthState>>,
) -> Json<AdmissionStatus> {
    let cfg = state.admission.config_snapshot();
    Json(AdmissionStatus {
        config: AdmissionConfigView {
            max_cps: cfg.max_cps,
            max_concurrent_calls: cfg.max_concurrent_calls,
            max_concurrent_registrations: cfg.max_concurrent_registrations,
            per_trunk_cps: cfg
                .per_trunk_cps
                .iter()
                .map(|e| TrunkCpsView {
                    trunk_name: e.trunk_name.clone(),
                    max_cps: e.max_cps,
                })
                .collect(),
            overload_action: format!("{:?}", cfg.overload_action),
            retry_after_secs: cfg.retry_after_secs,
        },
        counters: AdmissionCounters {
            current_cps: state.admission.current_cps(),
            concurrent_calls: state.admission.concurrent_calls(),
            concurrent_registrations: state.admission.concurrent_registrations(),
        },
    })
}

async fn update_admission(
    State(state): State<Arc<HealthState>>,
    Json(req): Json<UpdateAdmissionRequest>,
) -> Result<Json<AdmissionStatus>, (StatusCode, Json<AdminError>)> {
    let mut cfg = state.admission.config_snapshot();

    // Merge partial update into current config
    if let Some(v) = req.max_cps {
        cfg.max_cps = v;
    }
    if let Some(v) = req.max_concurrent_calls {
        cfg.max_concurrent_calls = v;
    }
    if let Some(v) = req.max_concurrent_registrations {
        cfg.max_concurrent_registrations = v;
    }
    if let Some(entries) = req.per_trunk_cps {
        cfg.per_trunk_cps = entries
            .into_iter()
            .map(|e| fcp_sbc_core::TrunkCpsLimit {
                trunk_name: e.trunk_name,
                max_cps: e.max_cps,
            })
            .collect();
    }
    if let Some(ref action_str) = req.overload_action {
        match action_str.to_lowercase().as_str() {
            "reject" => cfg.overload_action = fcp_sbc_core::AdmissionAction::Reject,
            "queue" => cfg.overload_action = fcp_sbc_core::AdmissionAction::Queue,
            _ => {
                return Err(admin_err(
                    StatusCode::BAD_REQUEST,
                    "invalid overload_action: must be 'reject' or 'queue'",
                ))
            }
        }
    }
    if let Some(v) = req.retry_after_secs {
        cfg.retry_after_secs = v;
    }

    state
        .admission
        .update_config(cfg)
        .map_err(|e| admin_err_detail(StatusCode::BAD_REQUEST, "invalid config", &e))?;

    // Return updated state
    let cfg = state.admission.config_snapshot();
    Ok(Json(AdmissionStatus {
        config: AdmissionConfigView {
            max_cps: cfg.max_cps,
            max_concurrent_calls: cfg.max_concurrent_calls,
            max_concurrent_registrations: cfg.max_concurrent_registrations,
            per_trunk_cps: cfg
                .per_trunk_cps
                .iter()
                .map(|e| TrunkCpsView {
                    trunk_name: e.trunk_name.clone(),
                    max_cps: e.max_cps,
                })
                .collect(),
            overload_action: format!("{:?}", cfg.overload_action),
            retry_after_secs: cfg.retry_after_secs,
        },
        counters: AdmissionCounters {
            current_cps: state.admission.current_cps(),
            concurrent_calls: state.admission.concurrent_calls(),
            concurrent_registrations: state.admission.concurrent_registrations(),
        },
    }))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{self, Request};
    use tower::ServiceExt as _;

    fn test_state() -> Arc<HealthState> {
        test_state_with_trunks(vec![])
    }

    fn test_state_with_trunks(trunks: Vec<fcp_sbc_core::TrunkConfig>) -> Arc<HealthState> {
        let admission = Arc::new(crate::admission::AdmissionController::new(Default::default()));
        let health_monitor = Arc::new(crate::trunk_health::TrunkHealthMonitor::new(&trunks));
        let metrics = Arc::new(crate::metrics::SbcRuntimeMetrics::new());
        let call_manager = Arc::new(fcp_b2bua_core::CallManager::new());

        Arc::new(HealthState {
            admission,
            health_monitor,
            metrics,
            call_manager,
            trunk_configs: trunks,
        })
    }

    fn app(state: Arc<HealthState>) -> Router {
        admin_router().with_state(state)
    }

    async fn get_json(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, json)
    }

    async fn post_empty(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(http::Method::POST)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, json)
    }

    async fn put_json(
        app: Router,
        path: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(http::Method::PUT)
            .uri(path)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn test_list_trunks_empty() {
        let state = test_state();
        let (status, json) = get_json(app(state), "/trunks").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_list_trunks_with_entries() {
        let trunks = vec![fcp_sbc_core::TrunkConfig {
            name: "carrier-a".into(),
            hosts: vec!["sip.carrier.com".into()],
            ..Default::default()
        }];
        let state = test_state_with_trunks(trunks);
        let (status, json) = get_json(app(state), "/trunks").await;
        assert_eq!(status, StatusCode::OK);
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "carrier-a");
        assert_eq!(arr[0]["admin_disabled"], false);
    }

    #[tokio::test]
    async fn test_get_trunk_not_found() {
        let state = test_state();
        let (status, json) = get_json(app(state), "/trunks/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(json["error"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_disable_enable_trunk() {
        let trunks = vec![fcp_sbc_core::TrunkConfig {
            name: "trunk-a".into(),
            hosts: vec!["h".into()],
            ..Default::default()
        }];
        let state = test_state_with_trunks(trunks);

        // Disable
        let (status, json) = post_empty(app(state.clone()), "/trunks/trunk-a/disable").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["action"], "disabled");
        assert_eq!(json["admin_disabled"], true);

        // Verify disabled
        let (_, json) = get_json(app(state.clone()), "/trunks/trunk-a").await;
        assert_eq!(json["admin_disabled"], true);

        // Enable
        let (status, json) = post_empty(app(state.clone()), "/trunks/trunk-a/enable").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["action"], "enabled");
        assert_eq!(json["admin_disabled"], false);
    }

    #[tokio::test]
    async fn test_list_calls_empty() {
        let state = test_state();
        let (status, json) = get_json(app(state), "/calls").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_call_stats() {
        let state = test_state();
        let (status, json) = get_json(app(state), "/calls/stats").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["total"], 0);
        assert_eq!(json["active"], 0);
    }

    #[tokio::test]
    async fn test_get_admission() {
        let state = test_state();
        let (status, json) = get_json(app(state), "/admission").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["config"]["max_cps"], 100);
        assert_eq!(json["config"]["max_concurrent_calls"], 1000);
        assert_eq!(json["counters"]["current_cps"], 0);
    }

    #[tokio::test]
    async fn test_update_admission_valid() {
        let state = test_state();
        let update = serde_json::json!({
            "max_cps": 200,
            "max_concurrent_calls": 5000
        });
        let (status, json) = put_json(app(state.clone()), "/admission", update).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["config"]["max_cps"], 200);
        assert_eq!(json["config"]["max_concurrent_calls"], 5000);
    }

    #[tokio::test]
    async fn test_update_admission_invalid() {
        let state = test_state();
        let update = serde_json::json!({
            "max_cps": 200000
        });
        let (status, json) = put_json(app(state), "/admission", update).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("invalid config"));
    }
}
