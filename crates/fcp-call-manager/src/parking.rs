//! The node's parked calls (`docs/PARKING.md` §4): what is parked here,
//! sent on to someone, or hung up. The admin API asks every call manager
//! and puts their answers together. Behind the node's API token, scoped to
//! the tenant the caller names (the node's default tenant otherwise).

use crate::health::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use fcp_core::TenantId;
use serde::Deserialize;
use std::sync::Arc;

/// The routes, for the protected router.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/parking/calls", get(list))
        .route("/parking/calls/:slot/retrieve", post(retrieve))
        .route("/parking/calls/:slot", delete(hang_up))
}

#[derive(Debug, Default, Deserialize)]
struct Tenant {
    #[serde(default)]
    tenant_id: Option<TenantId>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrieveBody {
    /// An extension or a user, dialed through this node.
    target: String,
    #[serde(default)]
    tenant_id: Option<TenantId>,
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({"error": "no SIP handler on this node"})),
    )
        .into_response()
}

fn failed((status, message): (u16, String)) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// GET /parking/calls?tenant_id= — the tenant's calls parked on this node.
async fn list(State(state): State<Arc<AppState>>, Query(q): Query<Tenant>) -> Response {
    let Some(handler) = state.sip_handler.as_ref() else {
        return unavailable();
    };
    let tenant = q.tenant_id.unwrap_or(state.default_tenant_id);
    Json(serde_json::json!({ "calls": handler.parked_calls(&tenant) })).into_response()
}

/// POST /parking/calls/:slot/retrieve — ring `target` with the call; 404
/// when nothing is parked there on this node.
async fn retrieve(
    State(state): State<Arc<AppState>>,
    Path(slot): Path<String>,
    Json(body): Json<RetrieveBody>,
) -> Response {
    let Some(handler) = state.sip_handler.as_ref() else {
        return unavailable();
    };
    let target = body.target.trim();
    if target.is_empty() {
        return failed((400, "a retrieve needs a `target`".to_string()));
    }
    let tenant = body.tenant_id.unwrap_or(state.default_tenant_id);
    match handler.retrieve_parked_to(&tenant, &slot, target).await {
        Ok(call_id) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({"slot": slot, "call_id": call_id, "target": target})),
        )
            .into_response(),
        Err(e) => failed(e),
    }
}

/// DELETE /parking/calls/:slot?tenant_id= — hang up the parked call.
async fn hang_up(
    State(state): State<Arc<AppState>>,
    Path(slot): Path<String>,
    Query(q): Query<Tenant>,
) -> Response {
    let Some(handler) = state.sip_handler.as_ref() else {
        return unavailable();
    };
    let tenant = q.tenant_id.unwrap_or(state.default_tenant_id);
    match handler.hang_up_parked(&tenant, &slot).await {
        Ok(call_id) => {
            Json(serde_json::json!({"slot": slot, "call_id": call_id, "status": "ended"}))
                .into_response()
        }
        Err(e) => failed(e),
    }
}
