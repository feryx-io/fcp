//! Presence management API endpoints.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_presence::{PresenceAuthPolicy, PresenceState, PresenceUpdate};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::info;

/// Helper macro to get presence_manager or return 404.
macro_rules! require_presence_manager {
    ($state:expr) => {
        $state.presence_manager.as_ref().ok_or_else(|| {
            ApiError::NotFound("Presence service is not configured".to_string())
        })?
    };
}

// ---- Response types ----

/// Presence info response for a single presentity.
#[derive(Debug, Serialize)]
pub struct PresenceResponse {
    pub aor: String,
    pub state: String,
    pub note: Option<String>,
    pub watcher_count: usize,
}

/// Watcher info response.
#[derive(Debug, Serialize)]
pub struct WatcherResponse {
    pub watcher_aor: String,
    pub subscription_id: String,
    pub event_package: String,
    pub expires_in_secs: u64,
}

/// Presence stats response.
#[derive(Debug, Serialize)]
pub struct PresenceStatsResponse {
    pub total_presentities: usize,
    pub total_watchers: usize,
}

// ---- Request types ----

/// Update presence request body.
#[derive(Debug, Deserialize)]
pub struct UpdatePresenceRequest {
    pub state: String,
    pub note: Option<String>,
}

// ---- Handlers ----

/// GET /presence — List all tracked presentities (filtered by tenant).
pub async fn list_presences(
    State(state): State<ApiState>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    let presences = pm.get_presences_by_tenant(&state.default_tenant_id);
    let responses: Vec<PresenceResponse> = presences
        .into_iter()
        .map(|p| {
            let watcher_count = pm.get_watchers(&p.aor).len();
            PresenceResponse {
                aor: p.aor,
                state: p.state.to_string(),
                note: p.note,
                watcher_count,
            }
        })
        .collect();

    Ok(Json(responses))
}

/// GET /presence/stats — Presentity and watcher counts.
pub async fn get_stats(
    State(state): State<ApiState>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    Ok(Json(PresenceStatsResponse {
        total_presentities: pm.presentity_count(),
        total_watchers: pm.total_watcher_count(),
    }))
}

/// GET /presence/:aor — Get a single presentity with its watchers.
pub async fn get_presence(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    let info = pm
        .get_presence(&aor)
        .ok_or_else(|| ApiError::NotFound(format!("Presentity '{}' not found", aor)))?;

    let now = Instant::now();
    let watchers: Vec<WatcherResponse> = pm
        .get_watchers(&aor)
        .into_iter()
        .map(|w| WatcherResponse {
            watcher_aor: w.watcher_aor,
            subscription_id: w.subscription_id,
            event_package: w.event_package,
            expires_in_secs: w.expires_at.saturating_duration_since(now).as_secs(),
        })
        .collect();

    #[derive(Serialize)]
    struct PresenceDetailResponse {
        aor: String,
        state: String,
        note: Option<String>,
        watcher_count: usize,
        watchers: Vec<WatcherResponse>,
    }

    Ok(Json(PresenceDetailResponse {
        aor: info.aor,
        state: info.state.to_string(),
        note: info.note,
        watcher_count: watchers.len(),
        watchers,
    }))
}

/// PUT /presence/:aor — Set presence state via API.
pub async fn update_presence(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
    Json(req): Json<UpdatePresenceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    let presence_state = parse_presence_state(&req.state)?;

    let update = PresenceUpdate {
        aor: aor.clone(),
        state: presence_state,
        note: req.note,
    };

    let tenant_id = state.default_tenant_id.clone();
    let info = pm
        .update_presence(update, &tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(aor = %aor, state = %info.state, "Presence updated via API");

    Ok(Json(PresenceResponse {
        aor: info.aor,
        state: info.state.to_string(),
        note: info.note,
        watcher_count: pm.get_watchers(&aor).len(),
    }))
}

/// DELETE /presence/:aor — Remove a presentity.
pub async fn delete_presence(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    if pm.get_presence(&aor).is_none() {
        return Err(ApiError::NotFound(format!(
            "Presentity '{}' not found",
            aor
        )));
    }

    let tenant_id = state.default_tenant_id.clone();
    pm.remove_presence(&aor, &tenant_id).await;

    info!(aor = %aor, "Presentity removed via API");

    Ok(StatusCode::NO_CONTENT)
}

/// GET /presence/:aor/watchers — List SIP watchers for an AOR.
pub async fn list_watchers(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    // Verify presentity exists
    if pm.get_presence(&aor).is_none() {
        return Err(ApiError::NotFound(format!(
            "Presentity '{}' not found",
            aor
        )));
    }

    let now = Instant::now();
    let watchers: Vec<WatcherResponse> = pm
        .get_watchers(&aor)
        .into_iter()
        .map(|w| WatcherResponse {
            watcher_aor: w.watcher_aor,
            subscription_id: w.subscription_id,
            event_package: w.event_package,
            expires_in_secs: w.expires_at.saturating_duration_since(now).as_secs(),
        })
        .collect();

    Ok(Json(watchers))
}

/// GET /presence/:aor/policy — Get authorization policy for a presentity.
pub async fn get_policy(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    match pm.get_auth_policy(&aor) {
        Some(policy) => Ok(Json(policy)),
        None => {
            // Return default open policy
            Ok(Json(PresenceAuthPolicy {
                default_allow: true,
                ..Default::default()
            }))
        }
    }
}

/// PUT /presence/:aor/policy — Set authorization policy for a presentity.
pub async fn set_policy(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
    Json(policy): Json<PresenceAuthPolicy>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    pm.set_auth_policy(&aor, policy.clone(), &state.default_tenant_id);

    info!(aor = %aor, default_allow = policy.default_allow, "Presence auth policy updated via API");

    Ok(Json(policy))
}

/// DELETE /presence/:aor/policy — Remove authorization policy (revert to default-allow).
pub async fn delete_policy(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    pm.remove_auth_policy(&aor);

    info!(aor = %aor, "Presence auth policy removed via API");

    Ok(StatusCode::NO_CONTENT)
}

/// GET /presence/:aor/pres-rules — Get authorization policy as XCAP pres-rules XML (RFC 5025).
pub async fn get_pres_rules(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    let policy = pm.get_auth_policy(&aor).unwrap_or(PresenceAuthPolicy {
        default_allow: true,
        ..Default::default()
    });

    let xml = fcp_presence::xcap::generate_pres_rules(&policy);

    Ok((
        StatusCode::OK,
        [("content-type", "application/auth-policy+xml")],
        xml,
    ))
}

/// PUT /presence/:aor/pres-rules — Set authorization policy from XCAP pres-rules XML (RFC 5025).
pub async fn set_pres_rules(
    State(state): State<ApiState>,
    Path(aor): Path<String>,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    let policy = fcp_presence::xcap::parse_pres_rules(&body)
        .map_err(|e| ApiError::BadRequest(format!("Invalid pres-rules XML: {e}")))?;

    pm.set_auth_policy(&aor, policy.clone(), &state.default_tenant_id);

    info!(aor = %aor, default_allow = policy.default_allow, "Presence auth policy updated via XCAP pres-rules");

    Ok((
        StatusCode::OK,
        [("content-type", "application/auth-policy+xml")],
        fcp_presence::xcap::generate_pres_rules(&policy),
    ))
}

/// DELETE /presence/:aor/watchers/:subscription_id — Force-terminate a specific watcher subscription.
///
/// Removes the watcher from the manager. The presence NOTIFY dispatcher will send
/// `terminated;reason=forced` if the SipPresenceHandler is wired up.
pub async fn delete_subscription(
    State(state): State<ApiState>,
    Path((aor, subscription_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    if pm.get_presence(&aor).is_none() {
        return Err(ApiError::NotFound(format!(
            "Presentity '{}' not found",
            aor
        )));
    }

    // Force-terminate the watcher (returns the removed watcher for potential NOTIFY)
    let removed = pm.force_terminate_watcher(&aor, &subscription_id);
    if removed.is_none() {
        return Err(ApiError::NotFound(format!(
            "Subscription '{}' not found for '{}'",
            subscription_id, aor
        )));
    }

    // Clean up from persistent store
    pm.remove_watcher(&aor, &subscription_id).await;

    info!(
        aor = %aor,
        subscription_id = %subscription_id,
        "Watcher force-terminated via API"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Parse a presence state string into a `PresenceState` enum.
fn parse_presence_state(s: &str) -> Result<PresenceState, ApiError> {
    match s {
        "open" => Ok(PresenceState::Open),
        "closed" => Ok(PresenceState::Closed),
        "away" => Ok(PresenceState::Away),
        "busy" => Ok(PresenceState::Busy),
        "on_the_phone" => Ok(PresenceState::OnThePhone),
        "ringing" => Ok(PresenceState::Ringing),
        other => Err(ApiError::BadRequest(format!(
            "Invalid presence state '{}'. Must be one of: open, closed, away, busy, on_the_phone, ringing",
            other
        ))),
    }
}
