//! System status API endpoints.

use crate::{error::ApiError, state::ApiState};
use axum::{extract::State, Json};
use fcp_core::{CredentialStore, RegistrationStore};
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
    // Count users if storage is available
    let users_count = if let Some(storage) = &state.storage {
        storage.list_users().await?.len()
    } else {
        0
    };

    // Count active registrations across all AORs
    let active_registrations_count = if let Some(storage) = &state.storage {
        storage.registration_count().await.unwrap_or(0)
    } else {
        0
    };

    Ok(Json(StatusResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        users_count,
        active_registrations_count,
    }))
}
