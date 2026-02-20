//! HTTP REST API for the standalone presence server.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, put},
    Json, Router,
};
use fcp_presence::PresenceManager;
use fcp_sip::presence::SipPresenceHandler;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tracing::warn;

/// Shared application state for HTTP handlers.
#[derive(Clone)]
pub struct AppState {
    pub manager: PresenceManager,
    pub sip_handler: Option<Arc<SipPresenceHandler>>,
    pub start_time: Instant,
    pub resource_list_store: Arc<dyn fcp_core::storage::ResourceListStore>,
    pub default_tenant_id: fcp_core::TenantId,
}

/// Build the HTTP router.
///
/// If `xcap_root` is provided, the XCAP server routes are nested under that path.
pub fn router(state: Arc<AppState>, xcap_root: Option<&str>) -> Router {
    let mut app = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/metrics", get(metrics))
        .route("/v1/presence/{aor}", get(get_presence))
        .route("/v1/presence/{aor}", put(update_presence))
        .route("/v1/presence/{aor}", delete(remove_presence))
        .route("/v1/presence/{aor}/watchers", get(get_watchers))
        .route(
            "/v1/presence/{aor}/watchers/{subscription_id}",
            delete(remove_watcher),
        );

    if let Some(root) = xcap_root {
        app = app.nest(root, crate::xcap_server::xcap_router());
    }

    app.with_state(state)
}

// ============================================================================
// Health & Metrics
// ============================================================================

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();
    Json(serde_json::json!({
        "status": "ok",
        "uptime_secs": uptime,
    }))
}

async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let presences = state.manager.get_all_presences();
    let watchers = state.manager.get_all_watchers();
    let rls_subs = state.manager.rls_subscription_count();

    Json(serde_json::json!({
        "presentities": presences.len(),
        "watchers": watchers.len(),
        "rls_subscriptions": rls_subs,
        "uptime_secs": state.start_time.elapsed().as_secs(),
    }))
}

// ============================================================================
// Presence CRUD
// ============================================================================

#[derive(Serialize)]
struct PresenceResponse {
    aor: String,
    state: String,
    note: Option<String>,
}

async fn get_presence(
    State(state): State<Arc<AppState>>,
    Path(aor): Path<String>,
) -> impl IntoResponse {
    match state.manager.get_presence(&aor) {
        Some(info) => (
            StatusCode::OK,
            Json(serde_json::json!(PresenceResponse {
                aor: info.aor,
                state: info.state.to_string(),
                note: info.note,
            })),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct UpdatePresenceRequest {
    state: String,
    note: Option<String>,
    #[serde(default = "default_tenant")]
    tenant_id: String,
}

fn default_tenant() -> String {
    "tenant-00000000-0000-0000-0000-000000000000".to_string()
}

async fn update_presence(
    State(state): State<Arc<AppState>>,
    Path(aor): Path<String>,
    Json(body): Json<UpdatePresenceRequest>,
) -> impl IntoResponse {
    let presence_state = match body.state.as_str() {
        "open" => fcp_presence::PresenceState::Open,
        "away" => fcp_presence::PresenceState::Away,
        "busy" | "dnd" => fcp_presence::PresenceState::Busy,
        "on-the-phone" => fcp_presence::PresenceState::OnThePhone,
        "closed" | "offline" => fcp_presence::PresenceState::Closed,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid state"})),
            )
                .into_response()
        }
    };

    let tenant_id: fcp_core::TenantId = body.tenant_id.parse().unwrap_or_default();
    let update = fcp_presence::PresenceUpdate {
        aor: aor.clone(),
        state: presence_state,
        note: body.note,
    };

    match state.manager.update_presence(update, &tenant_id).await {
        Ok(info) => (
            StatusCode::OK,
            Json(serde_json::json!(PresenceResponse {
                aor: info.aor,
                state: info.state.to_string(),
                note: info.note,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /v1/presence/{aor}
///
/// If a SIP handler is available, sends `terminated;reason=noresource` NOTIFYs
/// to all watchers before removing the presentity. Otherwise falls back to a
/// simple remove without NOTIFY.
async fn remove_presence(
    State(state): State<Arc<AppState>>,
    Path(aor): Path<String>,
) -> impl IntoResponse {
    let tenant_id = fcp_core::TenantId::default();

    if let Some(ref sip) = state.sip_handler {
        if let Err(e) = sip.terminate_watchers_noresource(&aor, &tenant_id).await {
            warn!(aor = %aor, error = %e, "Failed to send noresource NOTIFYs");
            // Fall back to plain remove
            state.manager.remove_presence(&aor, &tenant_id).await;
        }
    } else {
        state.manager.remove_presence(&aor, &tenant_id).await;
    }

    StatusCode::NO_CONTENT
}

// ============================================================================
// Watchers
// ============================================================================

async fn get_watchers(
    State(state): State<Arc<AppState>>,
    Path(aor): Path<String>,
) -> impl IntoResponse {
    let watchers = state.manager.get_watchers(&aor);
    let watcher_list: Vec<serde_json::Value> = watchers
        .iter()
        .map(|w| {
            serde_json::json!({
                "watcher_aor": w.watcher_aor,
                "subscription_id": w.subscription_id,
                "contact_uri": w.contact_uri,
            })
        })
        .collect();

    Json(serde_json::json!({
        "aor": aor,
        "watchers": watcher_list,
    }))
}

/// DELETE /v1/presence/{aor}/watchers/{subscription_id}
///
/// Force-terminates a specific watcher subscription. Sends a NOTIFY with
/// `Subscription-State: terminated;reason=forced` if a SIP handler is available.
/// Returns 503 if no SIP handler is configured.
async fn remove_watcher(
    State(state): State<Arc<AppState>>,
    Path((aor, subscription_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let Some(ref sip) = state.sip_handler else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "SIP handler not configured"})),
        )
            .into_response();
    };

    match sip.terminate_watcher_forced(&aor, &subscription_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
