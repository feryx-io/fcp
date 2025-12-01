//! System status API endpoints.

use crate::api::{error::ApiError, state::ApiState};
use axum::{extract::State, Json};
use fcp_core::CredentialStore;
use serde::Serialize;

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// System status response.
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub users_count: usize,
    pub active_registrations_count: usize,
}

/// Health check endpoint.
///
/// GET /api/v1/health
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// System status endpoint.
///
/// GET /api/v1/status
pub async fn status(State(state): State<ApiState>) -> Result<Json<StatusResponse>, ApiError> {
    // Count users
    let users = state.storage.list_users().await?;
    let users_count = users.len();

    // Count active registrations across all AORs
    // Note: This is a simplified count. In production, you might want to
    // maintain a global registrations index.
    let active_registrations_count = 0; // TODO: Implement global count

    Ok(Json(StatusResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: 0, // TODO: Track uptime
        users_count,
        active_registrations_count,
    }))
}
