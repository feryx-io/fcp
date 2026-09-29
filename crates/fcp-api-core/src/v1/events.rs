//! Real-time event streaming via WebSocket.
//!
//! Provides a WebSocket endpoint that streams platform events in real-time.
//! Clients can subscribe to events filtered by tenant_id, call_id, or event types.
//!
//! ## Authentication
//!
//! When auth is enabled, the client must send an auth message as the very first
//! WebSocket frame after connection:
//!
//! ```json
//! {"type": "auth", "token": "<bearer-token>"}
//! ```
//!
//! The server validates the token and responds with either:
//! - `{"type": "auth_ok"}` — proceed to event streaming
//! - `{"type": "auth_error", "message": "..."}` — connection is closed
//!
//! This avoids putting the token in the URL (which leaks via access logs,
//! proxies, and monitoring tools).

use crate::auth::{self, AuthIdentity};
use crate::state::ApiState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use fcp_core::TenantId;
use fcp_events::{filter::EventType, EventFilter};
use futures_util::{stream::StreamExt, SinkExt};
use serde::Deserialize;
use std::str::FromStr;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Query parameters for event stream filtering.
#[derive(Debug, Deserialize)]
pub struct EventStreamQuery {
    /// Filter by tenant ID (optional)
    pub tenant_id: Option<String>,

    /// Filter by call ID (optional)
    pub call_id: Option<String>,

    /// Filter by event types (comma-separated: call,media,ivr,recording,system)
    pub event_types: Option<String>,
}

/// First-message auth payload sent by the client.
#[derive(Debug, Deserialize)]
struct WsAuthMessage {
    #[serde(rename = "type")]
    msg_type: String,
    token: Option<String>,
}

/// WebSocket handler for event streaming.
///
/// GET /api/v1/events
///
/// Query parameters:
/// - tenant_id: Filter events for a specific tenant (UUID)
/// - call_id: Filter events for a specific call (UUID)
/// - event_types: Comma-separated list of event types (call,media,ivr,recording,system)
///
/// Example: ws://localhost:8080/api/v1/events?event_types=call&tenant_id=123e4567-e89b-12d3-a456-426614174000
pub async fn event_stream_websocket(
    ws: WebSocketUpgrade,
    Query(params): Query<EventStreamQuery>,
    State(state): State<ApiState>,
) -> impl IntoResponse {
    info!(
        tenant_id = ?params.tenant_id,
        call_id = ?params.call_id,
        event_types = ?params.event_types,
        "New WebSocket connection for event streaming"
    );

    ws.on_upgrade(move |socket| handle_websocket(socket, params, state))
}

/// Handle WebSocket connection.
async fn handle_websocket(socket: WebSocket, params: EventStreamQuery, state: ApiState) {
    let (mut sender, mut receiver) = socket.split();
    let mut identity: Option<AuthIdentity> = None;

    // --- First-message authentication ---
    // When auth is configured, the client must send {"type":"auth","token":"..."}
    // as the first message. This keeps the token out of the URL entirely.
    if let Some(ref auth_config) = state.auth_config {
        let auth_result = tokio::time::timeout(Duration::from_secs(5), receiver.next()).await;

        identity = match auth_result {
            Ok(Some(Ok(Message::Text(text)))) => {
                match serde_json::from_str::<WsAuthMessage>(&text) {
                    Ok(msg) if msg.msg_type == "auth" => {
                        if let Some(ref token) = msg.token {
                            auth::validate_token(token, auth_config).await
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        };

        // Events WebSocket is admin-only — end users must not subscribe
        // to platform-wide event streams — and carries the tenant's calls,
        // so it takes `call_view` (a scoped API token reaches it only when
        // it names that).
        let is_admin = identity.as_ref().is_some_and(|id| {
            id.is_admin()
                && crate::auth::check_permission(
                    id,
                    fcp_core::permissions::Permission::CallView,
                    state.deployment_mode,
                )
                .is_ok()
        });
        if !is_admin {
            let msg = if identity.is_none() {
                "Invalid or missing authentication token"
            } else {
                "Admin access required"
            };
            let _ = sender
                .send(Message::Text(
                    serde_json::json!({
                        "type": "auth_error",
                        "message": msg
                    })
                    .to_string(),
                ))
                .await;
            debug!("WebSocket auth failed — closing connection");
            return;
        }

        // Auth succeeded — confirm to client
        if let Err(e) = sender
            .send(Message::Text(
                serde_json::json!({"type": "auth_ok"}).to_string(),
            ))
            .await
        {
            warn!(error = %e, "Failed to send auth_ok");
            return;
        }
        debug!("WebSocket auth succeeded");
    }

    // Parse filter parameters
    let filter = match build_event_filter(&params, identity.as_ref()) {
        Ok(filter) => filter,
        Err(e) => {
            error!(error = %e, "Failed to build event filter");
            let _ = sender.send(Message::Text(format!("Error: {}", e))).await;
            return;
        }
    };

    debug!(?filter, "Event filter configured");

    // Subscribe to event bus
    let mut event_stream = match state.event_bus.subscribe(filter).await {
        Ok(stream) => stream,
        Err(e) => {
            error!(error = %e, "Failed to subscribe to event bus");
            let _ = sender
                .send(Message::Text(format!(
                    "Error: Failed to subscribe to events: {}",
                    e
                )))
                .await;
            return;
        }
    };

    info!("Client subscribed to event stream");

    // Send confirmation message
    if let Err(e) = sender
        .send(Message::Text(
            serde_json::json!({
                "type": "connected",
                "message": "Successfully subscribed to event stream"
            })
            .to_string(),
        ))
        .await
    {
        warn!(error = %e, "Failed to send connection confirmation");
        return;
    }

    // Spawn task to handle incoming messages (for ping/pong)
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Close(_) => {
                    debug!("Client sent close frame");
                    break;
                }
                Message::Ping(_data) => {
                    debug!("Received ping from client");
                    // Pongs are automatically sent by axum
                }
                Message::Text(text) => {
                    debug!(text = %text, "Received text message from client (ignored)");
                }
                _ => {}
            }
        }
    });

    // Forward events to WebSocket
    let mut send_task = tokio::spawn(async move {
        while let Some(event) = event_stream.next().await {
            // Serialize event to JSON
            let json = match serde_json::to_string(&event) {
                Ok(json) => json,
                Err(e) => {
                    error!(error = %e, "Failed to serialize event");
                    continue;
                }
            };

            // Send to WebSocket
            if let Err(e) = sender.send(Message::Text(json)).await {
                warn!(error = %e, "Failed to send event to WebSocket");
                break;
            }
        }

        info!("Event stream ended");
    });

    // Wait for either task to finish
    tokio::select! {
        _ = &mut send_task => {
            recv_task.abort();
            debug!("Send task completed");
        }
        _ = &mut recv_task => {
            send_task.abort();
            debug!("Receive task completed");
        }
    }

    info!("WebSocket connection closed");
}

/// Build an EventFilter from query parameters.
fn build_event_filter(
    params: &EventStreamQuery,
    identity: Option<&AuthIdentity>,
) -> Result<EventFilter, String> {
    let mut filter = EventFilter::new();

    if let Some(scoped_tenant_id) = identity.and_then(AuthIdentity::tenant_id) {
        let requested_tenant_id = if let Some(ref tenant_id_str) = params.tenant_id {
            let tenant_id = TenantId::from_str(tenant_id_str)
                .map_err(|e| format!("Invalid tenant_id: {}", e))?;
            if tenant_id != *scoped_tenant_id {
                return Err("tenant_id outside authenticated tenant scope".to_string());
            }
            tenant_id
        } else {
            *scoped_tenant_id
        };
        filter = filter.tenant(requested_tenant_id);
    } else if let Some(ref tenant_id_str) = params.tenant_id {
        let tenant_id =
            TenantId::from_str(tenant_id_str).map_err(|e| format!("Invalid tenant_id: {}", e))?;
        filter = filter.tenant(tenant_id);
    }

    // Parse call_id if provided
    if let Some(ref call_id_str) = params.call_id {
        let call_id = fcp_core::CallId::from_str(call_id_str)
            .map_err(|e| format!("Invalid call_id: {}", e))?;
        filter = filter.call(call_id);
    }

    // Parse event_types if provided
    if let Some(ref types_str) = params.event_types {
        let types: Result<Vec<EventType>, _> = types_str
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(parse_event_type)
            .collect();

        filter = filter.types(types?);
    }

    Ok(filter)
}

/// Parse an event type from a string.
fn parse_event_type(s: &str) -> Result<EventType, String> {
    match s.to_lowercase().as_str() {
        "call" => Ok(EventType::Call),
        "media" => Ok(EventType::Media),
        "ivr" => Ok(EventType::Ivr),
        "recording" => Ok(EventType::Recording),
        "system" => Ok(EventType::System),
        "queue" => Ok(EventType::Queue),
        "voicemail" => Ok(EventType::Voicemail),
        "conference" => Ok(EventType::Conference),
        "user" => Ok(EventType::User),
        "registration" => Ok(EventType::Registration),
        "flow" => Ok(EventType::Flow),
        "sip" => Ok(EventType::Sip),
        "billing" => Ok(EventType::Billing),
        "cdr" => Ok(EventType::Cdr),
        "compliance" => Ok(EventType::Compliance),
        _ => Err(format!("Invalid event type: {}", s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_scoped_identity_forces_filter_to_authenticated_tenant() {
        let tenant_id = TenantId::new();
        let params = EventStreamQuery {
            tenant_id: None,
            call_id: None,
            event_types: None,
        };
        let identity = AuthIdentity::ApiToken {
            token_id: "token-test".to_string(),
            tenant_id,
            permissions: vec!["admin".to_string()],
        };

        let filter = build_event_filter(&params, Some(&identity)).expect("filter should build");

        assert_eq!(filter.tenant_id, Some(tenant_id));
    }

    #[test]
    fn tenant_scoped_identity_rejects_other_tenant() {
        let scoped_tenant = TenantId::new();
        let params = EventStreamQuery {
            tenant_id: Some(TenantId::new().to_string()),
            call_id: None,
            event_types: None,
        };
        let identity = AuthIdentity::ApiToken {
            token_id: "token-test".to_string(),
            tenant_id: scoped_tenant,
            permissions: vec!["admin".to_string()],
        };

        let err = build_event_filter(&params, Some(&identity)).expect_err("filter should fail");

        assert!(err.contains("tenant scope"));
    }
}
