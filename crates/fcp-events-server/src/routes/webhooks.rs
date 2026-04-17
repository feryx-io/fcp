//! REST API endpoints for webhook endpoint management.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use fcp_core::{TenantId, WebhookId};
use fcp_webhooks::WebhookDispatcher;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::warn;

use crate::AuthPrincipal;

/// Shared state for webhook routes.
#[derive(Clone)]
pub struct WebhookRouteState {
    pub dispatcher: Arc<WebhookDispatcher>,
}

/// Build the webhook router.
pub fn webhook_router(state: WebhookRouteState) -> Router {
    Router::new()
        .route("/v1/webhooks", post(create_endpoint).get(list_endpoints))
        .route(
            "/v1/webhooks/{id}",
            get(get_endpoint)
                .put(update_endpoint)
                .delete(delete_endpoint_handler),
        )
        .route("/v1/webhooks/{id}/test", post(test_endpoint))
        .route("/v1/webhooks/{id}/deliveries", get(list_deliveries))
        .route(
            "/v1/webhooks/deliveries/{delivery_id}/replay",
            post(replay_delivery),
        )
        .with_state(state)
}

/// Request body for creating a webhook endpoint.
#[derive(Debug, Deserialize)]
struct CreateEndpointRequest {
    tenant_id: String,
    url: String,
    secret: String,
    #[serde(default)]
    events: Vec<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    description: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Request body for updating a webhook endpoint.
#[derive(Debug, Deserialize)]
struct UpdateEndpointRequest {
    url: Option<String>,
    secret: Option<String>,
    events: Option<Vec<String>>,
    enabled: Option<bool>,
    description: Option<String>,
}

/// Query parameters for listing endpoints.
#[derive(Debug, Deserialize)]
struct ListEndpointsQuery {
    tenant_id: String,
}

/// Query parameters for listing deliveries.
#[derive(Debug, Deserialize)]
struct ListDeliveriesQuery {
    #[serde(default = "default_delivery_limit")]
    limit: usize,
}

fn default_delivery_limit() -> usize {
    50
}

/// API response for a webhook endpoint.
#[derive(Debug, Serialize)]
struct EndpointResponse {
    id: String,
    tenant_id: String,
    url: String,
    events: Vec<String>,
    enabled: bool,
    description: Option<String>,
    created_at: u64,
    updated_at: u64,
}

impl From<fcp_core::storage::WebhookEndpoint> for EndpointResponse {
    fn from(ep: fcp_core::storage::WebhookEndpoint) -> Self {
        Self {
            id: ep.id.to_string(),
            tenant_id: ep.tenant_id.to_string(),
            url: ep.url,
            events: ep.events,
            enabled: ep.enabled,
            description: ep.description,
            created_at: ep
                .created_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            updated_at: ep
                .updated_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

fn json_error(status: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn scoped_tenant(
    principal: &AuthPrincipal,
    requested_tenant: &str,
) -> Result<TenantId, axum::response::Response> {
    let tenant_id: TenantId = requested_tenant
        .parse()
        .map_err(|_| json_error(StatusCode::BAD_REQUEST, "Invalid tenant_id"))?;

    if let Some(scope) = principal.tenant_scope() {
        if scope != &tenant_id {
            return Err(json_error(
                StatusCode::FORBIDDEN,
                "Tenant scope does not match authenticated principal",
            ));
        }
    }

    Ok(tenant_id)
}

fn authorize_endpoint_access(
    principal: &AuthPrincipal,
    endpoint_tenant_id: &TenantId,
) -> Result<(), axum::response::Response> {
    match principal.tenant_scope() {
        None => Ok(()),
        Some(scope) if scope == endpoint_tenant_id => Ok(()),
        Some(_) => Err(json_error(StatusCode::NOT_FOUND, "Endpoint not found")),
    }
}

/// POST /v1/webhooks - Create a new webhook endpoint
async fn create_endpoint(
    State(state): State<WebhookRouteState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(req): Json<CreateEndpointRequest>,
) -> impl IntoResponse {
    let tenant_id = match scoped_tenant(&principal, &req.tenant_id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let now = SystemTime::now();
    let endpoint = fcp_core::storage::WebhookEndpoint {
        id: WebhookId::new(),
        tenant_id,
        url: req.url,
        secret: req.secret,
        events: req.events,
        enabled: req.enabled,
        description: req.description,
        created_at: now,
        updated_at: now,
    };

    let response = EndpointResponse::from(endpoint.clone());

    match state.dispatcher.store().create_endpoint(endpoint).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(response).unwrap()),
        )
            .into_response(),
        Err(e) => {
            warn!(error = %e, "Failed to create webhook endpoint");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// GET /v1/webhooks?tenant_id=... - List webhook endpoints for a tenant
async fn list_endpoints(
    State(state): State<WebhookRouteState>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(query): Query<ListEndpointsQuery>,
) -> impl IntoResponse {
    let tenant_id = match scoped_tenant(&principal, &query.tenant_id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    match state.dispatcher.store().list_endpoints(&tenant_id).await {
        Ok(endpoints) => {
            let responses: Vec<EndpointResponse> =
                endpoints.into_iter().map(EndpointResponse::from).collect();
            Json(serde_json::json!({
                "endpoints": responses,
                "count": responses.len(),
            }))
            .into_response()
        }
        Err(e) => {
            warn!(error = %e, "Failed to list webhook endpoints");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// GET /v1/webhooks/:id - Get a webhook endpoint by ID
async fn get_endpoint(
    State(state): State<WebhookRouteState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let webhook_id: WebhookId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid webhook ID"})),
            )
                .into_response()
        }
    };

    match state.dispatcher.store().get_endpoint(&webhook_id).await {
        Ok(Some(ep)) => {
            if let Err(response) = authorize_endpoint_access(&principal, &ep.tenant_id) {
                return response;
            }
            Json(serde_json::to_value(EndpointResponse::from(ep)).unwrap()).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Endpoint not found"})),
        )
            .into_response(),
        Err(e) => {
            warn!(error = %e, "Failed to get webhook endpoint");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// PUT /v1/webhooks/:id - Update a webhook endpoint
async fn update_endpoint(
    State(state): State<WebhookRouteState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
    Json(req): Json<UpdateEndpointRequest>,
) -> impl IntoResponse {
    let webhook_id: WebhookId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid webhook ID"})),
            )
                .into_response()
        }
    };

    let existing = match state.dispatcher.store().get_endpoint(&webhook_id).await {
        Ok(Some(ep)) => ep,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Endpoint not found"})),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };

    if let Err(response) = authorize_endpoint_access(&principal, &existing.tenant_id) {
        return response;
    }

    let updated = fcp_core::storage::WebhookEndpoint {
        url: req.url.unwrap_or(existing.url),
        secret: req.secret.unwrap_or(existing.secret),
        events: req.events.unwrap_or(existing.events),
        enabled: req.enabled.unwrap_or(existing.enabled),
        description: req.description.or(existing.description),
        updated_at: SystemTime::now(),
        ..existing
    };

    let response = EndpointResponse::from(updated.clone());

    match state.dispatcher.store().update_endpoint(updated).await {
        Ok(()) => Json(serde_json::to_value(response).unwrap()).into_response(),
        Err(e) => {
            warn!(error = %e, "Failed to update webhook endpoint");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// DELETE /v1/webhooks/:id - Delete a webhook endpoint
async fn delete_endpoint_handler(
    State(state): State<WebhookRouteState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let webhook_id: WebhookId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid webhook ID"})),
            )
                .into_response()
        }
    };

    let existing = match state.dispatcher.store().get_endpoint(&webhook_id).await {
        Ok(Some(ep)) => ep,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "Endpoint not found"),
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    if let Err(response) = authorize_endpoint_access(&principal, &existing.tenant_id) {
        return response;
    }

    match state.dispatcher.store().delete_endpoint(&webhook_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            warn!(error = %e, "Failed to delete webhook endpoint");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// POST /v1/webhooks/:id/test - Send a test event to a webhook endpoint
async fn test_endpoint(
    State(state): State<WebhookRouteState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let webhook_id: WebhookId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid webhook ID"})),
            )
                .into_response()
        }
    };

    let endpoint = match state.dispatcher.store().get_endpoint(&webhook_id).await {
        Ok(Some(ep)) => ep,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Endpoint not found"})),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };

    if let Err(response) = authorize_endpoint_access(&principal, &endpoint.tenant_id) {
        return response;
    }

    // Build a test payload
    let payload = fcp_webhooks::WebhookPayload {
        event_id: uuid::Uuid::new_v4().to_string(),
        event_type: "webhook.test".to_string(),
        timestamp: {
            let now = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("{}Z", now)
        },
        tenant_id: Some(endpoint.tenant_id.to_string()),
        data: serde_json::json!({
            "message": "This is a test webhook delivery",
            "endpoint_id": endpoint.id.to_string(),
        }),
    };

    let payload_json = serde_json::to_string(&payload).unwrap_or_default();
    let signature = fcp_webhooks::compute_signature(&endpoint.secret, &payload_json);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    match client
        .post(&endpoint.url)
        .header("Content-Type", "application/json")
        .header("X-FCP-Event", "webhook.test")
        .header("X-FCP-Signature", &signature)
        .body(payload_json)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            Json(serde_json::json!({
                "success": resp.status().is_success(),
                "status_code": status,
            }))
            .into_response()
        }
        Err(e) => Json(serde_json::json!({
            "success": false,
            "error": e.to_string(),
        }))
        .into_response(),
    }
}

/// GET /v1/webhooks/:id/deliveries - List recent delivery logs
async fn list_deliveries(
    State(state): State<WebhookRouteState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
    Query(query): Query<ListDeliveriesQuery>,
) -> impl IntoResponse {
    let webhook_id: WebhookId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid webhook ID"})),
            )
                .into_response()
        }
    };

    let endpoint = match state.dispatcher.store().get_endpoint(&webhook_id).await {
        Ok(Some(ep)) => ep,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "Endpoint not found"),
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    if let Err(response) = authorize_endpoint_access(&principal, &endpoint.tenant_id) {
        return response;
    }

    match state
        .dispatcher
        .store()
        .get_delivery_logs(&webhook_id, query.limit)
        .await
    {
        Ok(logs) => {
            let count = logs.len();
            Json(serde_json::json!({
                "deliveries": logs,
                "count": count,
            }))
            .into_response()
        }
        Err(e) => {
            warn!(error = %e, "Failed to get delivery logs");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// Replay a failed webhook delivery.
///
/// POST /v1/webhooks/deliveries/:delivery_id/replay
async fn replay_delivery(
    State(state): State<WebhookRouteState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(delivery_id): Path<String>,
) -> impl IntoResponse {
    // Look up the delivery log
    let log = match state.dispatcher.store().get_delivery_log(&delivery_id).await {
        Ok(Some(log)) => log,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "Delivery log not found"),
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    // Look up the endpoint to verify it still exists and authorize access
    let endpoint = match state.dispatcher.store().get_endpoint(&log.endpoint_id).await {
        Ok(Some(ep)) => ep,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "Webhook endpoint no longer exists"),
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    if let Err(response) = authorize_endpoint_access(&principal, &endpoint.tenant_id) {
        return response;
    }

    // Re-deliver the stored payload with HMAC signature (matching normal delivery)
    let signature = fcp_webhooks::compute_signature(&endpoint.secret, &log.payload);
    let client = reqwest::Client::new();
    let now = SystemTime::now();
    let result = client
        .post(&endpoint.url)
        .header("Content-Type", "application/json")
        .header("X-FCP-Event", &log.event_type)
        .header("X-FCP-Signature", &signature)
        .header("X-FCP-Replay", "true")
        .body(log.payload.clone())
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    use fcp_core::storage::WebhookDeliveryLog;

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let delivered = resp.status().is_success();
            let resp_body = resp.text().await.ok().map(|b| {
                if b.len() > 65536 { b[..65536].to_string() } else { b }
            });

            let replay_log = WebhookDeliveryLog {
                id: uuid::Uuid::new_v4().to_string(),
                endpoint_id: log.endpoint_id,
                event_type: log.event_type,
                payload: log.payload,
                status_code: Some(status),
                attempts: 1,
                last_attempt_at: Some(now),
                next_retry_at: None,
                delivered,
                error: if delivered { None } else { Some(format!("HTTP {}", status)) },
                response_body: resp_body,
                response_headers: None,
                created_at: now,
            };

            let _ = state.dispatcher.store().log_delivery(replay_log.clone()).await;

            Json(serde_json::json!({
                "replayed": true,
                "delivery_id": replay_log.id,
                "status_code": status,
                "delivered": delivered,
            }))
            .into_response()
        }
        Err(e) => {
            let replay_log = WebhookDeliveryLog {
                id: uuid::Uuid::new_v4().to_string(),
                endpoint_id: log.endpoint_id,
                event_type: log.event_type,
                payload: log.payload,
                status_code: None,
                attempts: 1,
                last_attempt_at: Some(now),
                next_retry_at: None,
                delivered: false,
                error: Some(e.to_string()),
                response_body: None,
                response_headers: None,
                created_at: now,
            };

            let _ = state.dispatcher.store().log_delivery(replay_log).await;

            json_error(
                StatusCode::BAD_GATEWAY,
                format!("Replay delivery failed: {}", e),
            )
        }
    }
}
