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
    pub history_store: Option<Arc<dyn fcp_core::storage::PresenceHistoryStore>>,
    #[cfg(feature = "webhook")]
    pub webhook_dispatcher: Option<Arc<fcp_presence::webhook::WebhookDispatcher>>,
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
        )
        .route(
            "/v1/analytics/presence/{aor}/history",
            get(get_presence_history),
        )
        .route("/v1/analytics/presence/summary", get(get_presence_summary));

    #[cfg(feature = "webhook")]
    {
        app = app
            .route("/v1/webhooks", axum::routing::post(create_webhook))
            .route("/v1/webhooks", get(list_webhooks))
            .route("/v1/webhooks/{id}", delete(delete_webhook));
    }

    let _ = &app; // suppress unused warning when webhook feature is off

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

// ============================================================================
// Analytics
// ============================================================================

#[derive(Deserialize)]
struct HistoryQuery {
    from: Option<u64>,
    to: Option<u64>,
    #[serde(default = "default_history_limit")]
    limit: usize,
}

fn default_history_limit() -> usize {
    100
}

#[derive(Deserialize)]
struct SummaryQuery {
    from: Option<u64>,
    to: Option<u64>,
}

async fn get_presence_history(
    State(state): State<Arc<AppState>>,
    Path(aor): Path<String>,
    axum::extract::Query(query): axum::extract::Query<HistoryQuery>,
) -> impl IntoResponse {
    let Some(ref store) = state.history_store else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Analytics not enabled"})),
        )
            .into_response();
    };

    match store
        .query_history(&aor, query.from, query.to, query.limit)
        .await
    {
        Ok(history) => {
            let items: Vec<serde_json::Value> = history
                .iter()
                .map(|h| {
                    serde_json::json!({
                        "id": h.id,
                        "aor": h.aor,
                        "old_state": h.old_state,
                        "new_state": h.new_state,
                        "timestamp": h.timestamp_epoch,
                    })
                })
                .collect();

            Json(serde_json::json!({
                "aor": aor,
                "count": items.len(),
                "history": items,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_presence_summary(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<SummaryQuery>,
) -> impl IntoResponse {
    let Some(ref store) = state.history_store else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Analytics not enabled"})),
        )
            .into_response();
    };

    match store.query_summary(query.from, query.to).await {
        Ok(summaries) => {
            let items: Vec<serde_json::Value> = summaries
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "aor": s.aor,
                        "total_changes": s.total_changes,
                        "time_in_open_secs": s.time_in_open_secs,
                        "time_in_busy_secs": s.time_in_busy_secs,
                        "time_in_away_secs": s.time_in_away_secs,
                        "time_in_closed_secs": s.time_in_closed_secs,
                    })
                })
                .collect();

            Json(serde_json::json!({
                "count": items.len(),
                "summaries": items,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ============================================================================
// Webhooks (feature-gated)
// ============================================================================

#[cfg(feature = "webhook")]
async fn create_webhook(
    State(state): State<Arc<AppState>>,
    Json(body): Json<fcp_presence::webhook::WebhookEndpoint>,
) -> impl IntoResponse {
    let Some(ref dispatcher) = state.webhook_dispatcher else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Webhooks not enabled"})),
        )
            .into_response();
    };

    let id = body.id.clone();
    dispatcher.add_endpoint(body).await;

    (
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id, "status": "created"})),
    )
        .into_response()
}

#[cfg(feature = "webhook")]
async fn list_webhooks(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let Some(ref dispatcher) = state.webhook_dispatcher else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Webhooks not enabled"})),
        )
            .into_response();
    };

    let endpoints = dispatcher.list_endpoints().await;
    let items: Vec<serde_json::Value> = endpoints
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "url": e.url,
                "events": e.events,
                "enabled": e.enabled,
            })
        })
        .collect();

    Json(serde_json::json!({
        "count": items.len(),
        "webhooks": items,
    }))
    .into_response()
}

#[cfg(feature = "webhook")]
async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(ref dispatcher) = state.webhook_dispatcher else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Webhooks not enabled"})),
        )
            .into_response();
    };

    if dispatcher.remove_endpoint(&id).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}
