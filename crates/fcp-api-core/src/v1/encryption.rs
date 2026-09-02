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
