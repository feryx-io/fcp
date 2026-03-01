//! HTTP health/status/voicemail API for fcp-voicemail-server.

use axum::{
    extract::{Multipart, Path, State},
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
use tracing::{info, warn};

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
        .route(
            "/voicemail/mailboxes/{id}/greetings",
            get(list_greetings_handler),
        )
        .route(
            "/voicemail/mailboxes/{id}/greetings",
            post(upload_greeting_handler),
        )
        .route(
            "/voicemail/mailboxes/{id}/greetings/{greeting_type}",
            delete(delete_greeting_handler),
        )
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

// =========================================================================
// Greeting Management
// =========================================================================

async fn list_greetings_handler(
    State(state): State<Arc<VoicemailState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    use fcp_core::storage::VoicemailStore;

    match state.dyn_storage.get_mailbox(&id).await {
        Ok(Some(mailbox)) => {
            let greetings: Vec<serde_json::Value> = mailbox
                .greetings
                .iter()
                .map(|g| {
                    serde_json::json!({
                        "type": g.greeting_type.to_string(),
                        "audio_path": g.audio_path.display().to_string(),
                        "created_at": g.created_at
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                        "expired": g.is_expired(),
                        "expires_at": g.expires_at.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()),
                    })
                })
                .collect();
            Json(serde_json::json!({ "mailbox_id": id, "greetings": greetings }))
        }
        Ok(None) => Json(serde_json::json!({
            "error": format!("Mailbox not found: {}", id)
        })),
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to get mailbox: {}", e)
        })),
    }
}

fn parse_greeting_type(s: &str) -> Option<fcp_core::GreetingType> {
    match s.to_lowercase().as_str() {
        "unavailable" => Some(fcp_core::GreetingType::Unavailable),
        "busy" => Some(fcp_core::GreetingType::Busy),
        "name" => Some(fcp_core::GreetingType::Name),
        "temporary" => Some(fcp_core::GreetingType::Temporary),
        _ => None,
    }
}

async fn upload_greeting_handler(
    State(state): State<Arc<VoicemailState>>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> Json<serde_json::Value> {
    use fcp_core::storage::VoicemailStore;

    // Parse multipart fields: "type" (text), "audio" (file), optional "expires_in" (hours)
    let mut greeting_type_str = None;
    let mut audio_data: Option<Vec<u8>> = None;
    let mut audio_filename = None;
    let mut expires_in_hours: Option<u64> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        if name == "type" {
            if let Ok(text) = field.text().await {
                greeting_type_str = Some(text);
            }
        } else if name == "audio" {
            audio_filename = field.file_name().map(str::to_string);
            if let Ok(data) = field.bytes().await {
                audio_data = Some(data.to_vec());
            }
        } else if name == "expires_in" {
            if let Ok(text) = field.text().await {
                expires_in_hours = text.parse::<u64>().ok();
            }
        }
    }

    let greeting_type = match greeting_type_str.as_deref().and_then(parse_greeting_type) {
        Some(gt) => gt,
        None => {
            return Json(serde_json::json!({
                "error": "Missing or invalid 'type' field. Must be: unavailable, busy, name, or temporary"
            }));
        }
    };

    let audio_data = match audio_data {
        Some(data) if !data.is_empty() => data,
        _ => {
            return Json(serde_json::json!({
                "error": "Missing or empty 'audio' file field"
            }));
        }
    };

    // Get the mailbox
    let mut mailbox = match state.dyn_storage.get_mailbox(&id).await {
        Ok(Some(mb)) => mb,
        Ok(None) => {
            return Json(serde_json::json!({
                "error": format!("Mailbox not found: {}", id)
            }));
        }
        Err(e) => {
            return Json(serde_json::json!({
                "error": format!("Failed to get mailbox: {}", e)
            }));
        }
    };

    // Save greeting audio file
    let ext = audio_filename
        .as_deref()
        .and_then(|f: &str| f.rsplit('.').next())
        .unwrap_or("wav");
    let greeting_dir = std::path::PathBuf::from("/var/spool/voicemail/greetings");
    let audio_path = greeting_dir.join(format!("{}-{}.{}", id, greeting_type, ext));

    // Ensure directory exists
    if let Err(e) = tokio::fs::create_dir_all(&greeting_dir).await {
        warn!(error = %e, "Failed to create greeting directory");
        return Json(serde_json::json!({
            "error": format!("Failed to create greeting directory: {}", e)
        }));
    }

    if let Err(e) = tokio::fs::write(&audio_path, &audio_data).await {
        return Json(serde_json::json!({
            "error": format!("Failed to write greeting audio: {}", e)
        }));
    }

    // Update mailbox with new greeting
    let greeting = if greeting_type == fcp_core::GreetingType::Temporary {
        if let Some(hours) = expires_in_hours {
            let expires_at = std::time::SystemTime::now()
                + std::time::Duration::from_secs(hours * 3600);
            fcp_core::VoicemailGreeting::with_expiration(greeting_type, audio_path.clone(), expires_at)
        } else {
            fcp_core::VoicemailGreeting::new(greeting_type, audio_path.clone())
        }
    } else {
        fcp_core::VoicemailGreeting::new(greeting_type, audio_path.clone())
    };
    mailbox.add_greeting(greeting);

    if let Err(e) = state.dyn_storage.put_mailbox(mailbox).await {
        return Json(serde_json::json!({
            "error": format!("Failed to update mailbox: {}", e)
        }));
    }

    info!(
        mailbox_id = %id,
        greeting_type = %greeting_type,
        audio_path = %audio_path.display(),
        audio_size = audio_data.len(),
        "Greeting uploaded"
    );

    Json(serde_json::json!({
        "status": "ok",
        "mailbox_id": id,
        "greeting_type": greeting_type.to_string(),
        "audio_path": audio_path.display().to_string(),
    }))
}

async fn delete_greeting_handler(
    State(state): State<Arc<VoicemailState>>,
    Path((id, greeting_type_str)): Path<(String, String)>,
) -> Json<serde_json::Value> {
    use fcp_core::storage::VoicemailStore;

    let greeting_type = match parse_greeting_type(&greeting_type_str) {
        Some(gt) => gt,
        None => {
            return Json(serde_json::json!({
                "error": format!("Invalid greeting type: {}. Must be: unavailable, busy, name, or temporary", greeting_type_str)
            }));
        }
    };

    let mut mailbox = match state.dyn_storage.get_mailbox(&id).await {
        Ok(Some(mb)) => mb,
        Ok(None) => {
            return Json(serde_json::json!({
                "error": format!("Mailbox not found: {}", id)
            }));
        }
        Err(e) => {
            return Json(serde_json::json!({
                "error": format!("Failed to get mailbox: {}", e)
            }));
        }
    };

    // Find and remove the greeting, also delete the audio file
    let removed = mailbox
        .greetings
        .iter()
        .find(|g| g.greeting_type == greeting_type)
        .map(|g| g.audio_path.clone());

    mailbox
        .greetings
        .retain(|g| g.greeting_type != greeting_type);
    mailbox.updated_at = std::time::SystemTime::now();

    if let Some(audio_path) = &removed {
        let _ = tokio::fs::remove_file(audio_path).await;
    }

    if let Err(e) = state.dyn_storage.put_mailbox(mailbox).await {
        return Json(serde_json::json!({
            "error": format!("Failed to update mailbox: {}", e)
        }));
    }

    Json(serde_json::json!({
        "status": "ok",
        "mailbox_id": id,
        "greeting_type": greeting_type_str,
        "existed": removed.is_some(),
    }))
}
