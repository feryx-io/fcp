//! Encryption-at-rest management (platform admin only).
//!
//! The storage backend seals secret columns with data encryption keys
//! wrapped by the deployment's key-encryption key (see `docs/ENCRYPTION.md`).
//! These endpoints expose the key ring, rotate the active DEK, and re-seal
//! stored secrets after enabling encryption or rotating.

use crate::auth::AuthIdentity;
use crate::{error::ApiError, state::ApiState};
use axum::{extract::State, response::IntoResponse, Extension, Json};
use fcp_core::encryption::EncryptionStatus;
use tracing::info;

fn require_platform(identity: Option<&AuthIdentity>) -> Result<(), ApiError> {
    match identity {
        Some(id) if id.is_platform_admin() => Ok(()),
        Some(_) => Err(ApiError::Forbidden(
            "Platform admin access required".to_string(),
        )),
        // No identity means auth is disabled for this deployment.
        None => Ok(()),
    }
}

/// GET /api/v1/system/encryption — Key ring status.
#[utoipa::path(
    get,
    path = "/api/v1/system/encryption",
    tag = "encryption",
    operation_id = "encryption_status",
    responses(
        (status = 200, description = "OK", body = fcp_core::EncryptionStatus),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn status(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<impl IntoResponse, ApiError> {
    require_platform(identity.as_ref().map(|e| &e.0))?;
    let status = match state.storage.as_ref().and_then(|s| s.encryption_store()) {
        Some(store) => store
            .encryption_status()
            .await
            .map_err(|e| ApiError::Internal(format!("Encryption status failed: {e}")))?,
        None => EncryptionStatus::disabled(),
    };
    Ok(Json(status))
}

/// POST /api/v1/system/encryption/rotate — Create and activate a new DEK.
#[utoipa::path(
    post,
    path = "/api/v1/system/encryption/rotate",
    tag = "encryption",
    operation_id = "encryption_rotate",
    responses(
        (status = 200, description = "OK", body = fcp_core::EncryptionStatus),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn rotate(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<impl IntoResponse, ApiError> {
    require_platform(identity.as_ref().map(|e| &e.0))?;
    let store = state
        .storage
        .as_ref()
        .and_then(|s| s.encryption_store())
        .ok_or_else(|| {
            ApiError::Conflict("Encryption at rest is not enabled on this deployment".to_string())
        })?;
    let status = store
        .rotate_encryption_key()
        .await
        .map_err(|e| ApiError::Conflict(format!("Key rotation failed: {e}")))?;
    info!(active_key = ?status.active_key_id, "Data encryption key rotated via API");
    Ok(Json(status))
}

/// POST /api/v1/system/encryption/reencrypt — Re-seal every stored secret
/// with the active DEK (plaintext rows and rows sealed with older keys).
#[utoipa::path(
    post,
    path = "/api/v1/system/encryption/reencrypt",
    tag = "encryption",
    operation_id = "encryption_reencrypt",
    responses(
        (status = 200, description = "OK", body = fcp_core::ReencryptReport),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn reencrypt(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<impl IntoResponse, ApiError> {
    require_platform(identity.as_ref().map(|e| &e.0))?;
    let store = state
        .storage
        .as_ref()
        .and_then(|s| s.encryption_store())
        .ok_or_else(|| {
            ApiError::Conflict("Encryption at rest is not enabled on this deployment".to_string())
        })?;
    let report = store
        .reencrypt_secrets()
        .await
        .map_err(|e| ApiError::Internal(format!("Re-encryption failed: {e}")))?;
    info!(
        rewritten = report.rewritten,
        already_current = report.already_current,
        "Secrets re-encrypted via API"
    );
    Ok(Json(report))
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct SealRequest {
    /// The credential to seal, in plaintext.
    pub value: String,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct SealResponse {
    /// The sealed form (`enc:v1:…`) to put in a configuration file.
    pub sealed: String,
}

/// POST /api/v1/system/encryption/seal — Seal one credential with the
/// active DEK, for secrets that live in a configuration file rather than a
/// column: the E911 validator's bearer token (`docs/E911_BLOCK_2.md` §7).
/// The value is never stored or logged.
#[utoipa::path(
    post,
    path = "/api/v1/system/encryption/seal",
    tag = "encryption",
    operation_id = "encryption_seal",
    request_body = SealRequest,
    responses(
        (status = 200, description = "OK", body = SealResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn seal(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    Json(req): Json<SealRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_platform(identity.as_ref().map(|e| &e.0))?;
    if req.value.is_empty() {
        return Err(ApiError::BadRequest("value must not be empty".to_string()));
    }
    let store = state
        .storage
        .as_ref()
        .and_then(|s| s.encryption_store())
        .ok_or_else(|| {
            ApiError::Conflict("Encryption at rest is not enabled on this deployment".to_string())
        })?;
    let sealed = store
        .seal_secret(&req.value)
        .await
        .map_err(|e| ApiError::Conflict(format!("Sealing failed: {e}")))?;
    info!("Configuration secret sealed via API");
    Ok(Json(SealResponse { sealed }))
}
