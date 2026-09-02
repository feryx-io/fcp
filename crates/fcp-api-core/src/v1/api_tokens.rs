//! API Token management endpoints.
//!
//! Provides create/list/delete for API tokens used for programmatic access.

use crate::{auth::TenantContext, error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Response model for an API token (never includes the hash).
#[derive(Debug, Serialize)]
pub struct ApiTokenResponse {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub user_id: String,
    pub tenant_id: String,
    pub permissions: Vec<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

impl From<fcp_core::ApiToken> for ApiTokenResponse {
    fn from(t: fcp_core::ApiToken) -> Self {
        Self {
            id: t.id.to_string(),
            name: t.name,
            prefix: t.prefix,
            user_id: t.user_id,
            tenant_id: t.tenant_id.to_string(),
            permissions: t.permissions,
            expires_at: t.expires_at.map(|ts| {
                let d = ts.duration_since(UNIX_EPOCH).unwrap_or_default();
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default()
            }),
            created_at: {
                let d = t.created_at.duration_since(UNIX_EPOCH).unwrap_or_default();
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default()
            },
            last_used_at: t.last_used_at.map(|ts| {
                let d = ts.duration_since(UNIX_EPOCH).unwrap_or_default();
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default()
            }),
        }
    }
}

/// Request body for creating a new API token.
#[derive(Debug, Deserialize)]
pub struct CreateApiTokenRequest {
    pub name: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    pub expires_at: Option<String>,
}

/// Response body after creating a new API token (includes the one-time secret).
#[derive(Debug, Serialize)]
pub struct CreateApiTokenResponse {
    pub token: ApiTokenResponse,
    pub secret: String,
}

/// List all API tokens for the default tenant.
///
/// GET /api/v1/api-tokens
pub async fn list_api_tokens(
    State(state): State<ApiState>,
    tenant: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;

    let store = storage
        .api_token_store()
        .ok_or_else(|| ApiError::Internal("API token store not configured".to_string()))?;

    let tokens = store
        .list_api_tokens(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list API tokens: {}", e)))?;

    let response: Vec<ApiTokenResponse> = tokens.into_iter().map(ApiTokenResponse::from).collect();
    Ok(Json(response))
}

/// Create a new API token.
///
/// POST /api/v1/api-tokens
pub async fn create_api_token(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<CreateApiTokenRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.is_empty() || req.name.len() > 64 {
        return Err(ApiError::BadRequest(
            "Name must be between 1 and 64 characters".to_string(),
        ));
    }

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;

    let store = storage
        .api_token_store()
        .ok_or_else(|| ApiError::Internal("API token store not configured".to_string()))?;

    // Generate 32 random bytes and encode as base64url
    let mut secret_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret_bytes);
    use base64::Engine;
    let secret_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret_bytes);
    let secret = format!("fcp_{}", secret_b64);

    // Hash the secret with SHA-256
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let token_hash = format!("{:x}", hasher.finalize());

    // First 8 chars for display prefix
    let prefix = secret.chars().take(8).collect::<String>();

    // Parse optional expiry — accept both RFC3339 ("2026-03-15T00:00:00Z")
    // and plain date ("2026-03-15") from HTML date inputs.
    let expires_at = if let Some(ref exp_str) = req.expires_at {
        if exp_str.is_empty() {
            None
        } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(exp_str) {
            let secs = dt.timestamp() as u64;
            Some(UNIX_EPOCH + std::time::Duration::from_secs(secs))
        } else if let Ok(date) = chrono::NaiveDate::parse_from_str(exp_str, "%Y-%m-%d") {
            let dt = date.and_hms_opt(23, 59, 59).unwrap().and_utc();
            let secs = dt.timestamp() as u64;
            Some(UNIX_EPOCH + std::time::Duration::from_secs(secs))
        } else {
            return Err(ApiError::BadRequest(format!(
                "Invalid expires_at: expected YYYY-MM-DD or RFC3339, got '{}'",
                exp_str
            )));
        }
    } else {
        None
    };

    let token_id = fcp_core::ApiTokenId::new();
    let now = SystemTime::now();

    let api_token = fcp_core::ApiToken {
        id: token_id,
        name: req.name,
        token_hash,
        prefix: prefix.clone(),
        user_id: String::new(),
        tenant_id: tenant.0,
        permissions: req.permissions,
        expires_at,
        created_at: now,
        last_used_at: None,
    };

    store
        .create_api_token(api_token.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create API token: {}", e)))?;

    let response = CreateApiTokenResponse {
        token: ApiTokenResponse::from(api_token),
        secret,
    };

    Ok((axum::http::StatusCode::CREATED, Json(response)))
}

/// Delete an API token by ID.
///
/// DELETE /api/v1/api-tokens/:id
pub async fn delete_api_token(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;

    let store = storage
        .api_token_store()
        .ok_or_else(|| ApiError::Internal("API token store not configured".to_string()))?;

    let token_id: fcp_core::ApiTokenId = id
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid token ID".to_string()))?;

    let token = store
        .get_api_token(&token_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to look up API token: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("API token not found".to_string()))?;

    if token.tenant_id != tenant.0 {
        return Err(ApiError::NotFound("API token not found".to_string()));
    }

    store
        .delete_api_token(&token_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete API token: {}", e)))?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}
