//! HTTP health/status/voicemail API for fcp-voicemail-server.

use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};
use fcp_b2bua_core::CallManager;
use fcp_core::DynStorage;
use fcp_sip::flow_handler::FlowCallHandler;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

/// Shared state for the voicemail HTTP API.
pub struct VoicemailState {
    pub call_manager: Arc<CallManager>,
    pub flow_handler: Arc<FlowCallHandler>,
    pub dyn_storage: Arc<DynStorage>,
    pub start_time: Instant,
}

/// Create the voicemail health/status router.
pub fn voicemail_router() -> Router<Arc<VoicemailState>> {
    Router::new()
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        .route("/voicemail/routes", get(list_routes_handler))
        .route("/voicemail/routes", post(add_route_handler))
        .route("/voicemail/routes/{did}", delete(remove_route_handler))
        .route("/voicemail/mailboxes", get(list_mailboxes_handler))
        .route("/voicemail/mailboxes/{id}", get(get_mailbox_handler))
        .route(
            "/voicemail/mailboxes/{id}/messages",
            get(list_messages_handler),
        )
        .route("/voicemail/mailboxes/{id}/mwi", get(get_mwi_handler))
}

/// Run the HTTP health/status/voicemail server.
pub async fn run_http_server(
    bind_addr: SocketAddr,
    state: Arc<VoicemailState>,
) -> anyhow::Result<()> {
    let app = voicemail_router().with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "HTTP API server started");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-voicemail"
    }))
}

async fn status_handler(State(state): State<Arc<VoicemailState>>) -> Json<serde_json::Value> {
    let uptime = state.start_time.elapsed();
    let call_stats = state.call_manager.stats();
    let routes = state.flow_handler.get_all_routes().await;

    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-voicemail",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime.as_secs(),
        "active_calls": call_stats.active,
        "total_calls": call_stats.total,
        "voicemail_routes": routes.len(),
    }))
}

async fn list_routes_handler(
    State(state): State<Arc<VoicemailState>>,
) -> Json<serde_json::Value> {
    let routes = state.flow_handler.get_all_routes().await;
    let route_list: Vec<serde_json::Value> = routes
        .iter()
        .map(|(did, flow_id)| {
            serde_json::json!({
                "did": did,
                "flow_id": flow_id,
            })
        })
        .collect();

    Json(serde_json::json!({ "routes": route_list }))
}

#[derive(Deserialize)]
struct AddRouteRequest {
    did: String,
    mailbox_id: String,
}

async fn add_route_handler(
    State(state): State<Arc<VoicemailState>>,
    Json(payload): Json<AddRouteRequest>,
) -> Json<serde_json::Value> {
    // Create a flow_id derived from the mailbox_id for route tracking
    let flow_id = format!("vm-flow-{}", payload.mailbox_id);
    state
        .flow_handler
        .add_route(payload.did.clone(), flow_id.clone())
        .await;

    Json(serde_json::json!({
        "status": "ok",
        "did": payload.did,
        "mailbox_id": payload.mailbox_id,
        "flow_id": flow_id,
    }))
}

async fn remove_route_handler(
    State(state): State<Arc<VoicemailState>>,
    Path(did): Path<String>,
) -> Json<serde_json::Value> {
    let existed = state.flow_handler.remove_route(&did).await;

    Json(serde_json::json!({
        "status": "ok",
        "did": did,
        "existed": existed,
    }))
}

async fn list_mailboxes_handler(
    State(state): State<Arc<VoicemailState>>,
) -> Json<serde_json::Value> {
    use fcp_core::storage::VoicemailStore;

    // Use a default tenant ID for listing; in production this would come from auth context
    let tenant_id = fcp_core::TenantId::new();
    match state.dyn_storage.list_mailboxes(&tenant_id).await {
        Ok(mailboxes) => {
            let mailbox_list: Vec<serde_json::Value> = mailboxes
                .iter()
                .map(|mb| {
                    serde_json::json!({
                        "id": mb.id,
                        "extension": mb.extension,
                        "enabled": mb.enabled,
                        "max_messages": mb.max_messages,
                        "max_message_duration": mb.max_message_duration,
                    })
                })
                .collect();
            Json(serde_json::json!({ "mailboxes": mailbox_list }))
        }
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to list mailboxes: {}", e)
        })),
    }
}

async fn get_mailbox_handler(
    State(state): State<Arc<VoicemailState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    use fcp_core::storage::VoicemailStore;

    match state.dyn_storage.get_mailbox(&id).await {
        Ok(Some(mailbox)) => Json(serde_json::json!({
            "id": mailbox.id,
            "extension": mailbox.extension,
            "enabled": mailbox.enabled,
            "max_messages": mailbox.max_messages,
            "max_message_duration": mailbox.max_message_duration,
        })),
        Ok(None) => Json(serde_json::json!({
            "error": format!("Mailbox not found: {}", id)
        })),
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to get mailbox: {}", e)
        })),
    }
}

async fn list_messages_handler(
    State(state): State<Arc<VoicemailState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    use fcp_core::storage::VoicemailStore;

    match state.dyn_storage.list_messages(&id).await {
        Ok(messages) => {
            let message_list: Vec<serde_json::Value> = messages
                .iter()
                .map(|msg| {
                    serde_json::json!({
                        "id": msg.id,
                        "caller": msg.caller,
                        "caller_name": msg.caller_name,
                        "status": format!("{:?}", msg.status),
                        "duration": msg.duration,
                        "created_at": msg.created_at
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                    })
                })
                .collect();
            Json(serde_json::json!({ "messages": message_list }))
        }
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to list messages: {}", e)
        })),
    }
}

async fn get_mwi_handler(
    State(state): State<Arc<VoicemailState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    use fcp_core::storage::VoicemailStore;

    match state.dyn_storage.get_mwi_status(&id).await {
        Ok(mwi) => Json(serde_json::json!({
            "mailbox_id": id,
            "has_messages": mwi.has_messages(),
            "new_messages": mwi.new_messages,
            "old_messages": mwi.old_messages,
            "urgent_new_messages": mwi.urgent_new_messages,
            "urgent_old_messages": mwi.urgent_old_messages,
        })),
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to get MWI status: {}", e)
        })),
    }
}
