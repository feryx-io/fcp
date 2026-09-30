//! A tenant's webhooks (docs/DEVELOPER_SURFACE.md §3, phase 5).
//!
//! A webhook is a URL the platform POSTs the tenant's events to, signed
//! with a secret the server makes and shows once: `X-FCP-Signature:
//! t=<unix>,v1=<hex HMAC-SHA256 of "<t>.<body>">`. Rotating makes a new
//! secret and keeps signing with the old one beside it for an overlap, so a
//! receiver moves across without refusing a genuine delivery.
//!
//! A URL must be HTTPS and reach only public addresses, checked here when
//! it is written and by the dispatcher's resolver when it is called
//! (`fcp_webhooks::target`), unless the platform allows private targets
//! (`[webhooks] allow_private_targets`, for a lab). The events server
//! delivers; this API manages, tests and replays.

use crate::auth::{AuthIdentity, TenantContext};
use crate::error::ApiError;
use crate::page::{Order, Page, PageQuery};
use crate::state::ApiState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::storage::{WebhookDeliveryLog, WebhookEndpoint, WebhookStore};
use fcp_core::{TenantId, WebhookId};
use fcp_events::PlatformEvent;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// How long a rotated-out secret still signs, unless asked.
const DEFAULT_ROTATION_OVERLAP: Duration = Duration::from_secs(24 * 3600);
/// The longest overlap a rotation may give the old secret.
const MAX_ROTATION_OVERLAP: Duration = Duration::from_secs(7 * 24 * 3600);
/// The event type a test delivery carries.
pub const TEST_EVENT_TYPE: &str = "webhook.test";
/// The most deliveries a webhook's log is read back.
const DELIVERY_WINDOW: usize = crate::page::MAX_LIMIT;

/// A webhook, without its secret.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WebhookResponse {
    pub id: String,
    pub tenant_id: String,
    pub url: String,
    /// Event types delivered: names from `/webhooks/event-types`, a
    /// category's `call.*`, or `*`. Empty is every event.
    pub events: Vec<String>,
    pub enabled: bool,
    pub description: Option<String>,
    /// The first characters of the secret, to tell secrets apart.
    pub secret_prefix: String,
    /// Until when the secret a rotation replaced also signs deliveries.
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub previous_secret_valid_until: Option<SystemTime>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: SystemTime,
}

impl From<&WebhookEndpoint> for WebhookResponse {
    fn from(e: &WebhookEndpoint) -> Self {
        let now = SystemTime::now();
        Self {
            id: e.id.to_string(),
            tenant_id: e.tenant_id.to_string(),
            url: e.url.clone(),
            events: e.events.clone(),
            enabled: e.enabled,
            description: e.description.clone(),
            secret_prefix: secret_prefix(&e.secret),
            previous_secret_valid_until: e
                .previous_secret
                .as_ref()
                .and(e.previous_secret_expires_at)
                .filter(|until| *until > now),
            created_at: e.created_at,
            updated_at: e.updated_at,
        }
    }
}

/// A webhook just made or rotated, with its secret: the only time the
/// secret is shown.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WebhookWithSecret {
    pub webhook: WebhookResponse,
    /// `whsec_…`: verify each delivery's `X-FCP-Signature` with it.
    pub secret: String,
}

/// What a webhook is made with.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateWebhookRequest {
    pub url: String,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_true() -> bool {
    true
}

/// A change to a webhook; what is left out stays.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct UpdateWebhookRequest {
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub enabled: Option<bool>,
    /// Empty clears it.
    pub description: Option<String>,
}

/// A rotation.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct RotateWebhookSecretRequest {
    /// How long the old secret keeps signing beside the new one, in
    /// seconds (default a day, at most a week; 0 retires it at once).
    pub overlap_seconds: Option<u64>,
}

/// A test delivery.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct TestWebhookRequest {
    /// The event type the test carries; `webhook.test` unless given.
    pub event_type: Option<String>,
}

/// What one delivery attempt came to.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WebhookDeliveryOutcome {
    /// The receiver answered 2xx.
    pub delivered: bool,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    /// The start of what the receiver answered.
    pub response_body: Option<String>,
}

impl From<fcp_webhooks::DeliveryOutcome> for WebhookDeliveryOutcome {
    fn from(o: fcp_webhooks::DeliveryOutcome) -> Self {
        Self {
            delivered: o.delivered,
            status_code: o.status_code,
            error: o.error,
            response_body: o.response_body,
        }
    }
}

/// A logged delivery.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WebhookDeliveryResponse {
    pub id: String,
    pub webhook_id: String,
    pub event_type: String,
    /// The body delivered.
    #[schema(value_type = Object)]
    pub payload: serde_json::Value,
    pub delivered: bool,
    pub status_code: Option<u16>,
    pub attempts: u32,
    pub error: Option<String>,
    pub response_body: Option<String>,
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_attempt_at: Option<SystemTime>,
    /// When the dispatcher tries again; none once delivered or given up.
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub next_retry_at: Option<SystemTime>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
}

impl From<WebhookDeliveryLog> for WebhookDeliveryResponse {
    fn from(log: WebhookDeliveryLog) -> Self {
        Self {
            payload: serde_json::from_str(&log.payload)
                .unwrap_or(serde_json::Value::String(log.payload)),
            id: log.id,
            webhook_id: log.endpoint_id.to_string(),
            event_type: log.event_type,
            delivered: log.delivered,
            status_code: log.status_code,
            attempts: log.attempts,
            error: log.error,
            response_body: log.response_body,
            last_attempt_at: log.last_attempt_at,
            next_retry_at: log.next_retry_at,
            created_at: log.created_at,
        }
    }
}

/// An event type a webhook may subscribe to.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WebhookEventType {
    /// `call.answered`.
    pub name: &'static str,
    /// `call`: subscribe to all of them with `call.*`.
    pub category: &'static str,
}

/// The body of every delivery (the spec's `webhooks` section).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WebhookEvent {
    /// Unique per delivery: a receiver may drop one it has seen.
    pub event_id: String,
    /// A name from `/webhooks/event-types`, or `webhook.test`.
    pub event_type: String,
    /// When the event happened, RFC 3339.
    #[schema(format = DateTime)]
    pub timestamp: String,
    pub tenant_id: Option<String>,
    /// The event itself.
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

fn store(state: &ApiState) -> Result<&Arc<dyn WebhookStore>, ApiError> {
    Ok(state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Storage is not configured".into()))?
        .webhook_store())
}

fn dispatcher(state: &ApiState) -> Result<&Arc<fcp_webhooks::WebhookDispatcher>, ApiError> {
    state
        .webhook_dispatcher
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Webhook delivery is not configured".into()))
}

fn allow_private(state: &ApiState) -> bool {
    state
        .webhook_dispatcher
        .as_ref()
        .is_some_and(|d| d.config().allow_private_targets)
}

fn identity_of(identity: Option<Extension<AuthIdentity>>) -> AuthIdentity {
    identity
        .map(|Extension(id)| id)
        .unwrap_or(AuthIdentity::StaticToken)
}

fn may_manage(state: &ApiState, identity: Option<Extension<AuthIdentity>>) -> Result<(), ApiError> {
    crate::auth::check_permission(
        &identity_of(identity),
        Permission::WebhookManage,
        state.deployment_mode,
    )
}

/// A fresh secret: `whsec_` and 32 random bytes, base64url.
fn new_secret() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!(
        "whsec_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

fn secret_prefix(secret: &str) -> String {
    secret.chars().take(10).collect()
}

async fn check_target(state: &ApiState, url: &str) -> Result<String, ApiError> {
    fcp_webhooks::target::check_resolved(url, allow_private(state))
        .await
        .map(|u| u.to_string())
        .map_err(|e| ApiError::BadRequest(format!("Webhook URL refused: {e}")))
}

/// Each filter a known event type, a category's `<category>.*`, or `*`.
fn check_events(events: &[String]) -> Result<Vec<String>, ApiError> {
    let mut out: Vec<String> = Vec::new();
    for raw in events {
        let name = raw.trim();
        let known = name == "*"
            || PlatformEvent::EVENT_TYPES.contains(&name)
            || name.strip_suffix(".*").is_some_and(|category| {
                PlatformEvent::EVENT_TYPES
                    .iter()
                    .any(|t| t.split('.').next() == Some(category))
            });
        if !known {
            return Err(ApiError::BadRequest(format!(
                "Unknown event type '{name}': see /api/v1/webhooks/event-types"
            )));
        }
        if !out.iter().any(|e| e == name) {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

fn check_description(description: Option<String>) -> Result<Option<String>, ApiError> {
    let description = description.map(|d| d.trim().to_string());
    if description.as_ref().is_some_and(|d| d.len() > 256) {
        return Err(ApiError::BadRequest(
            "A description may be at most 256 characters".into(),
        ));
    }
    Ok(description.filter(|d| !d.is_empty()))
}

async fn owned(state: &ApiState, tenant: &TenantId, id: &str) -> Result<WebhookEndpoint, ApiError> {
    let webhook_id: WebhookId = id
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("Invalid webhook ID: {id}")))?;
    store(state)?
        .get_endpoint(&webhook_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read the webhook: {e}")))?
        .filter(|e| e.tenant_id == *tenant)
        .ok_or_else(|| ApiError::NotFound(format!("Webhook {id} not found")))
}

/// GET /api/v1/webhooks
#[utoipa::path(
    get,
    path = "/api/v1/webhooks",
    tag = "webhooks",
    operation_id = "webhooks_list_webhooks",
    params(PageQuery),
    responses(
        (status = 200, description = "OK", body = Page<WebhookResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_webhooks(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    tenant: TenantContext,
    Query(page): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let endpoints = store(&state)?
        .list_endpoints(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list webhooks: {e}")))?;
    Ok(Json(crate::page::paginate(
        endpoints.iter().map(WebhookResponse::from).collect(),
        &page,
        Order::Ascending,
        |w| crate::page::system_time_key(w.created_at, &w.id),
    )?))
}

/// POST /api/v1/webhooks
#[utoipa::path(
    post,
    path = "/api/v1/webhooks",
    tag = "webhooks",
    operation_id = "webhooks_create_webhook",
    request_body = CreateWebhookRequest,
    responses(
        (status = 201, description = "Made; the secret is shown this once", body = WebhookWithSecret),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_webhook(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    tenant: TenantContext,
    Json(req): Json<CreateWebhookRequest>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let url = check_target(&state, &req.url).await?;
    let events = check_events(&req.events)?;
    let description = check_description(req.description)?;
    let secret = new_secret();
    let now = SystemTime::now();
    let endpoint = WebhookEndpoint {
        id: WebhookId::new(),
        tenant_id: tenant.0,
        url,
        secret: secret.clone(),
        events,
        enabled: req.enabled,
        description,
        created_at: now,
        updated_at: now,
        previous_secret: None,
        previous_secret_expires_at: None,
    };
    store(&state)?
        .create_endpoint(endpoint.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create the webhook: {e}")))?;
    Ok((
        StatusCode::CREATED,
        Json(WebhookWithSecret {
            webhook: WebhookResponse::from(&endpoint),
            secret,
        }),
    ))
}

/// GET /api/v1/webhooks/event-types
#[utoipa::path(
    get,
    path = "/api/v1/webhooks/event-types",
    tag = "webhooks",
    operation_id = "webhooks_list_event_types",
    params(PageQuery),
    responses(
        (status = 200, description = "OK", body = Page<WebhookEventType>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_event_types(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    Query(page): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let types: Vec<WebhookEventType> = PlatformEvent::EVENT_TYPES
        .iter()
        .map(|name| WebhookEventType {
            name,
            category: name.split('.').next().unwrap_or(name),
        })
        .collect();
    Ok(Json(crate::page::paginate(
        types,
        &page,
        Order::Ascending,
        |t| t.name.to_string(),
    )?))
}

/// GET /api/v1/webhooks/:id
#[utoipa::path(
    get,
    path = "/api/v1/webhooks/{id}",
    tag = "webhooks",
    operation_id = "webhooks_get_webhook",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = WebhookResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_webhook(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let endpoint = owned(&state, &tenant.0, &id).await?;
    Ok(Json(WebhookResponse::from(&endpoint)))
}

/// PATCH /api/v1/webhooks/:id
#[utoipa::path(
    patch,
    path = "/api/v1/webhooks/{id}",
    tag = "webhooks",
    operation_id = "webhooks_update_webhook",
    params(("id" = String, Path)),
    request_body = UpdateWebhookRequest,
    responses(
        (status = 200, description = "OK", body = WebhookResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_webhook(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateWebhookRequest>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let mut endpoint = owned(&state, &tenant.0, &id).await?;
    if let Some(url) = req.url {
        endpoint.url = check_target(&state, &url).await?;
    }
    if let Some(events) = req.events {
        endpoint.events = check_events(&events)?;
    }
    if let Some(enabled) = req.enabled {
        endpoint.enabled = enabled;
    }
    if req.description.is_some() {
        endpoint.description = check_description(req.description)?;
    }
    endpoint.updated_at = SystemTime::now();
    store(&state)?
        .update_endpoint(endpoint.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update the webhook: {e}")))?;
    Ok(Json(WebhookResponse::from(&endpoint)))
}

/// DELETE /api/v1/webhooks/:id
#[utoipa::path(
    delete,
    path = "/api/v1/webhooks/{id}",
    tag = "webhooks",
    operation_id = "webhooks_delete_webhook",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Deleted, with its deliveries"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_webhook(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let endpoint = owned(&state, &tenant.0, &id).await?;
    store(&state)?
        .delete_endpoint(&endpoint.id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete the webhook: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/webhooks/:id/rotate-secret
#[utoipa::path(
    post,
    path = "/api/v1/webhooks/{id}/rotate-secret",
    tag = "webhooks",
    operation_id = "webhooks_rotate_webhook_secret",
    params(("id" = String, Path)),
    request_body = RotateWebhookSecretRequest,
    responses(
        (status = 200, description = "The new secret, shown this once", body = WebhookWithSecret),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn rotate_webhook_secret(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    body: Option<Json<RotateWebhookSecretRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let overlap = req
        .overlap_seconds
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_ROTATION_OVERLAP);
    if overlap > MAX_ROTATION_OVERLAP {
        return Err(ApiError::BadRequest(format!(
            "overlap_seconds may be at most {}",
            MAX_ROTATION_OVERLAP.as_secs()
        )));
    }
    let mut endpoint = owned(&state, &tenant.0, &id).await?;
    let secret = new_secret();
    let now = SystemTime::now();
    let old = std::mem::replace(&mut endpoint.secret, secret.clone());
    if overlap.is_zero() {
        endpoint.previous_secret = None;
        endpoint.previous_secret_expires_at = None;
    } else {
        endpoint.previous_secret = Some(old);
        endpoint.previous_secret_expires_at = Some(now + overlap);
    }
    endpoint.updated_at = now;
    store(&state)?
        .update_endpoint(endpoint.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to rotate the secret: {e}")))?;
    Ok(Json(WebhookWithSecret {
        webhook: WebhookResponse::from(&endpoint),
        secret,
    }))
}

/// POST /api/v1/webhooks/:id/test
#[utoipa::path(
    post,
    path = "/api/v1/webhooks/{id}/test",
    tag = "webhooks",
    operation_id = "webhooks_test_webhook",
    params(("id" = String, Path)),
    request_body = TestWebhookRequest,
    responses(
        (status = 200, description = "What the receiver answered, delivered or not", body = WebhookDeliveryOutcome),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn test_webhook(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    body: Option<Json<TestWebhookRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let endpoint = owned(&state, &tenant.0, &id).await?;
    if !endpoint.enabled {
        return Err(ApiError::Conflict(format!(
            "Webhook {id} is disabled: enable it to test it"
        )));
    }
    let event_type = body
        .and_then(|Json(r)| r.event_type)
        .unwrap_or_else(|| TEST_EVENT_TYPE.to_string());
    if event_type != TEST_EVENT_TYPE && !PlatformEvent::EVENT_TYPES.contains(&event_type.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "Unknown event type '{event_type}'"
        )));
    }
    let payload = serde_json::json!({
        "test": true,
        "webhook_id": endpoint.id.to_string(),
        "message": "A test delivery from FCP",
    });
    let outcome = dispatcher(&state)?
        .deliver_now(&endpoint.id, &tenant.0, &event_type, &payload)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to send the test: {e}")))?;
    Ok(Json(WebhookDeliveryOutcome::from(outcome)))
}

/// GET /api/v1/webhooks/:id/deliveries
#[utoipa::path(
    get,
    path = "/api/v1/webhooks/{id}/deliveries",
    tag = "webhooks",
    operation_id = "webhooks_list_deliveries",
    params(("id" = String, Path), PageQuery),
    responses(
        (status = 200, description = "The latest deliveries, newest first", body = Page<WebhookDeliveryResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_deliveries(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Query(page): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let endpoint = owned(&state, &tenant.0, &id).await?;
    let logs = store(&state)?
        .get_delivery_logs(&endpoint.id, DELIVERY_WINDOW)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read deliveries: {e}")))?;
    Ok(Json(crate::page::paginate(
        logs.into_iter()
            .map(WebhookDeliveryResponse::from)
            .collect(),
        &page,
        Order::Descending,
        |d| crate::page::system_time_key(d.created_at, &d.id),
    )?))
}

/// POST /api/v1/webhooks/:id/deliveries/:delivery_id/replay
#[utoipa::path(
    post,
    path = "/api/v1/webhooks/{id}/deliveries/{delivery_id}/replay",
    tag = "webhooks",
    operation_id = "webhooks_replay_delivery",
    params(("id" = String, Path), ("delivery_id" = String, Path)),
    responses(
        (status = 200, description = "Sent again now, signed now", body = WebhookDeliveryOutcome),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn replay_delivery(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    tenant: TenantContext,
    Path((id, delivery_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    may_manage(&state, identity)?;
    let endpoint = owned(&state, &tenant.0, &id).await?;
    let log = store(&state)?
        .get_delivery_log(&delivery_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read the delivery: {e}")))?
        .filter(|l| l.endpoint_id == endpoint.id)
        .ok_or_else(|| ApiError::NotFound(format!("Delivery {delivery_id} not found")))?;
    if !endpoint.enabled {
        return Err(ApiError::Conflict(format!(
            "Webhook {id} is disabled: enable it to replay to it"
        )));
    }
    let outcome = dispatcher(&state)?
        .replay(&log.id, &tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to replay: {e}")))?;
    Ok(Json(WebhookDeliveryOutcome::from(outcome)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_filters_are_names_categories_or_everything() {
        assert!(check_events(&["call.answered".into()]).is_ok());
        assert!(check_events(&["call.*".into(), "*".into()]).is_ok());
        assert!(check_events(&["call.nonsense".into()]).is_err());
        assert!(check_events(&["nonsense.*".into()]).is_err());
        assert_eq!(
            check_events(&["call.*".into(), "call.*".into()]).unwrap(),
            vec!["call.*".to_string()]
        );
    }

    #[test]
    fn a_secret_is_long_and_marked() {
        let s = new_secret();
        assert!(s.starts_with("whsec_"));
        assert!(s.len() > 40);
        assert_ne!(s, new_secret());
    }
}
