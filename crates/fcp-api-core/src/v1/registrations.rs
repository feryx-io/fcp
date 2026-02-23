//! Registration viewing API endpoints.

use crate::{error::ApiError, state::ApiState, validation};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use fcp_core::{Registration, RegistrationStore};
use serde::Serialize;

/// Registration response.
#[derive(Debug, Serialize)]
pub struct RegistrationResponse {
    pub aor: String,
    pub contact: String,
    pub expires_at: std::time::SystemTime,
    pub call_id: String,
    pub cseq: u32,
    pub user_agent: Option<String>,
    pub q: Option<f32>,
    pub registered_at: std::time::SystemTime,
    pub expires_in_seconds: u64,
}

impl From<Registration> for RegistrationResponse {
    fn from(reg: Registration) -> Self {
        let expires_in_seconds = reg
            .expires_at
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default()
            .as_secs();

        Self {
            aor: reg.aor,
            contact: reg.contact,
            expires_at: reg.expires_at,
            call_id: reg.call_id,
            cseq: reg.cseq,
            user_agent: reg.user_agent,
            q: reg.q,
            registered_at: reg.registered_at,
            expires_in_seconds,
        }
    }
}

/// List all active registrations.
///
/// GET /api/v1/registrations
pub async fn list_registrations(
    State(state): State<ApiState>,
) -> Result<Json<Vec<RegistrationResponse>>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    let registrations = storage.get_all_registrations().await?;

    let now = std::time::SystemTime::now();
    let active: Vec<RegistrationResponse> = registrations
        .into_iter()
        .filter(|r| r.expires_at > now)
        .map(RegistrationResponse::from)
        .collect();

    Ok(Json(active))
}

/// List registrations for a specific AOR.
///
/// GET /api/v1/registrations/:aor
pub async fn get_registrations(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
) -> Result<Json<Vec<RegistrationResponse>>, ApiError> {
    validation::validate_aor(&aor)?;

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    let registrations = storage.get_registrations(&aor).await?;

    // Filter out expired registrations
    let now = std::time::SystemTime::now();
    let active: Vec<RegistrationResponse> = registrations
        .into_iter()
        .filter(|r| r.expires_at > now)
        .map(RegistrationResponse::from)
        .collect();

    Ok(Json(active))
}

/// Delete all registrations for a specific AOR.
///
/// DELETE /api/v1/registrations/:aor
pub async fn delete_registrations(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
) -> Result<StatusCode, ApiError> {
    validation::validate_aor(&aor)?;

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    storage.delete_all_registrations(&aor).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a specific registration.
///
/// DELETE /api/v1/registrations/:aor/:contact
pub async fn delete_registration(
    State(state): State<ApiState>,
    Path((aor, contact)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    validation::validate_aor(&aor)?;
    validation::validate_contact(&contact)?;

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    storage.delete_registration(&aor, &contact).await?;
    Ok(StatusCode::NO_CONTENT)
}
