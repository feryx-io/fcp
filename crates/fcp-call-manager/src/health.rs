//! HTTP health/status/calls API for fcp-call-manager.

use axum::{extract::Query, extract::State, routing::get, Json, Router};
use fcp_b2bua_core::CallManager;
use fcp_core::DynStorage;
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
pub async fn run_http_server(
    bind_addr: SocketAddr,
    state: Arc<AppState>,
) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        .route("/calls", get(calls_handler))
        .route("/registrations", get(registrations_handler))
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
            let elapsed = call
                .created_at
                .elapsed()
                .map(|d| d.as_secs())
                .unwrap_or(0);
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
