//! HTTP health/status/flow-routes API for fcp-ivr.

use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};
use fcp_b2bua_core::CallManager;
use fcp_sip::flow_handler::FlowCallHandler;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

/// Shared state for the IVR HTTP API.
pub struct IvrState {
    pub call_manager: Arc<CallManager>,
    pub flow_handler: Arc<FlowCallHandler>,
    pub start_time: Instant,
}

/// Create the IVR health/status router.
pub fn ivr_router() -> Router<Arc<IvrState>> {
    Router::new()
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        .route("/flows/routes", get(list_routes_handler))
        .route("/flows/routes", post(add_route_handler))
        .route("/flows/routes/{did}", delete(remove_route_handler))
        .route("/flows/active", get(active_flows_handler))
}

/// Run the HTTP health/status/flows server.
pub async fn run_http_server(bind_addr: SocketAddr, state: Arc<IvrState>) -> anyhow::Result<()> {
    let app = ivr_router().with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "HTTP API server started");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Merge the IVR health router with another router (e.g. fcp-api-core).
pub fn merge_with_api(api_router: Router<Arc<IvrState>>) -> Router<Arc<IvrState>> {
    ivr_router().merge(api_router)
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-ivr"
    }))
}

async fn status_handler(State(state): State<Arc<IvrState>>) -> Json<serde_json::Value> {
    let uptime = state.start_time.elapsed();
    let call_stats = state.call_manager.stats();
    let routes = state.flow_handler.get_all_routes().await;

    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-ivr",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime.as_secs(),
        "active_calls": call_stats.active,
        "total_calls": call_stats.total,
        "flow_routes": routes.len(),
    }))
}

async fn list_routes_handler(
    State(state): State<Arc<IvrState>>,
) -> Json<serde_json::Value> {
    let routes = state.flow_handler.get_all_routes().await;
    let route_list: Vec<serde_json::Value> = routes
        .iter()
        .map(|(did, flow_id)| {
            serde_json::json!({
                "did": did,
                "flow_id": flow_id,
            })
        })
        .collect();

    Json(serde_json::json!({ "routes": route_list }))
}

#[derive(Deserialize)]
struct AddRouteRequest {
    did: String,
    flow_id: String,
}

async fn add_route_handler(
    State(state): State<Arc<IvrState>>,
    Json(payload): Json<AddRouteRequest>,
) -> Json<serde_json::Value> {
    state
        .flow_handler
        .add_route(payload.did.clone(), payload.flow_id.clone())
        .await;

    Json(serde_json::json!({
        "status": "ok",
        "did": payload.did,
        "flow_id": payload.flow_id,
    }))
}

async fn remove_route_handler(
    State(state): State<Arc<IvrState>>,
    Path(did): Path<String>,
) -> Json<serde_json::Value> {
    let existed = state.flow_handler.remove_route(&did).await;

    Json(serde_json::json!({
        "status": "ok",
        "did": did,
        "existed": existed,
    }))
}

async fn active_flows_handler(
    State(state): State<Arc<IvrState>>,
) -> Json<serde_json::Value> {
    let calls = state.call_manager.all_calls();
    let active_flows: Vec<serde_json::Value> = calls
        .iter()
        .filter(|call| call.flow_id.is_some())
        .map(|call| {
            let elapsed = call.created_at.elapsed().map(|d| d.as_secs()).unwrap_or(0);
            serde_json::json!({
                "call_id": call.id.to_string(),
                "flow_id": call.flow_id.as_deref().unwrap_or(""),
                "current_node": call.flow_current_node.as_deref().unwrap_or(""),
                "started_seconds_ago": elapsed,
            })
        })
        .collect();

    Json(serde_json::json!({ "active_flows": active_flows }))
}
