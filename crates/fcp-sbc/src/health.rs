//! HTTP health, readiness, and metrics endpoints for the SBC.

use crate::admission::AdmissionController;
use crate::metrics::SbcRuntimeMetrics;
use crate::trunk_health::TrunkHealthMonitor;
use axum::{extract::State, routing::get, Json, Router};
use std::sync::Arc;
use tracing::info;

/// Shared state for the HTTP health server.
pub struct HealthState {
    pub admission: Arc<AdmissionController>,
    pub health_monitor: Arc<TrunkHealthMonitor>,
    pub metrics: Arc<SbcRuntimeMetrics>,
}

/// Build the health/readiness/metrics router.
pub fn health_router(state: Arc<HealthState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .route("/metrics", get(metrics_snapshot))
        .with_state(state)
}

/// Run the HTTP health server.
pub async fn run_health_server(
    bind_addr: std::net::SocketAddr,
    state: Arc<HealthState>,
) -> anyhow::Result<()> {
    let app = health_router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "SBC health server started");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-sbc"
    }))
}

async fn readiness_check(State(state): State<Arc<HealthState>>) -> Json<serde_json::Value> {
    let ready = state.health_monitor.any_healthy();
    let status = if ready { "ready" } else { "not_ready" };
    Json(serde_json::json!({
        "status": status,
        "service": "fcp-sbc",
        "trunks_healthy": ready,
    }))
}

async fn metrics_snapshot(State(state): State<Arc<HealthState>>) -> Json<serde_json::Value> {
    let snap = state.metrics.snapshot();
    Json(serde_json::to_value(&snap).unwrap_or_default())
}
