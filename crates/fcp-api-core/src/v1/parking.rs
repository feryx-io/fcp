//! A tenant's parking lots (`docs/PARKING.md`): ranges of slots the tenant
//! numbers, each a number in its dial plan, so a lot is refused when one of
//! its numbers is already something else's.

use crate::auth::AuthIdentity;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::parking::{ParkingLot, ParkingLotStore};
use fcp_core::permissions::Permission;
use fcp_core::storage::{ConferenceStore, ExtensionStore, QueueConfigStore};
use fcp_core::{CallTarget, TenantId};
use fcp_events::{PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ParkingLotResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    /// The first and last slot: digit strings of one width.
    pub first: String,
    pub last: String,
    pub slots: u64,
    pub park_number: Option<String>,
    pub timeout_secs: u32,
    /// Where a call goes when its parker does not take it back
    /// (`ext:1001`, `queue:support`, `voicemail`, …).
    pub fallback: Option<String>,
    pub music: Option<String>,
    pub location_id: Option<String>,
    pub enabled: bool,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: u64,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: u64,
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl From<ParkingLot> for ParkingLotResponse {
    fn from(l: ParkingLot) -> Self {
        Self {
            slots: l.slot_count(),
            id: l.id,
            tenant_id: l.tenant_id.to_string(),
            name: l.name,
            first: l.first,
            last: l.last,
            park_number: l.park_number,
            timeout_secs: l.timeout_secs,
            fallback: l.fallback.as_ref().map(CallTarget::as_string),
            music: l.music,
            location_id: l.location_id,
            enabled: l.enabled,
            created_at: secs(l.created_at),
            updated_at: secs(l.updated_at),
        }
    }
}

/// What a lot is written as; a `PUT` replaces it whole.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ParkingLotRequest {
    pub name: String,
    pub first: String,
    pub last: String,
    #[serde(default)]
    pub park_number: Option<String>,
    /// Seconds; 60 when unset.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub music: Option<String>,
    #[serde(default)]
    pub location_id: Option<String>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

fn store(state: &ApiState) -> Result<&Arc<dyn ParkingLotStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.parking_lot_store())
        .ok_or_else(|| ApiError::Internal("Parking lot store not configured".to_string()))
}

fn blank(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// `req` laid onto `lot`, checked.
fn apply(lot: &mut ParkingLot, req: ParkingLotRequest) -> Result<(), ApiError> {
    lot.name = req.name.trim().to_string();
    lot.first = req.first.trim().to_string();
    lot.last = req.last.trim().to_string();
    lot.park_number = blank(req.park_number);
    lot.timeout_secs = req.timeout_secs.unwrap_or(60);
    lot.fallback = blank(req.fallback)
        .map(|f| CallTarget::parse(&f))
        .transpose()
        .map_err(|e| ApiError::BadRequest(format!("fallback: {e}")))?;
    lot.music = blank(req.music);
    lot.location_id = blank(req.location_id);
    lot.enabled = req.enabled;
    lot.validate().map_err(ApiError::BadRequest)
}

/// The first of `lot`'s numbers that is already the tenant's for something
/// else: another lot's, an extension, a queue's or a meeting room's.
async fn clash(state: &ApiState, lot: &ParkingLot) -> Result<Option<String>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let internal = |e: anyhow::Error| ApiError::Internal(e.to_string());
    for other in store(state)?
        .list_parking_lots(&lot.tenant_id)
        .await
        .map_err(internal)?
    {
        if other.id != lot.id {
            if let Some(n) = lot.overlap(&other) {
                return Ok(Some(format!("{n} is in the parking lot {:?}", other.name)));
            }
        }
    }
    let mine: HashSet<String> = lot.numbers().collect();
    let taken = |what: &str, n: &str| Some(format!("{n} is {what}"));
    for ext in storage
        .list_extensions(&lot.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        if mine.contains(&ext.number) {
            return Ok(taken("an extension", &ext.number));
        }
    }
    for queue in storage
        .list_queue_configs(&lot.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        if mine.contains(&queue.extension) {
            return Ok(taken("a queue's number", &queue.extension));
        }
    }
    for meeting in storage
        .list_meetings(&lot.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        if mine.contains(&meeting.room_number) {
            return Ok(taken("a meeting room", &meeting.room_number));
        }
    }
    Ok(None)
}

async fn owned(state: &ApiState, tenant: &TenantId, id: &str) -> Result<ParkingLot, ApiError> {
    store(state)?
        .get_parking_lot(tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Parking lot {id} not found")))
}

async fn announce(state: &ApiState, lot: &ParkingLot) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "parking_lot".to_string(),
            entity_id: lot.id.clone(),
            tenant_id: lot.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}

/// GET /api/v1/parking/lots — none listed means the node's own range
/// (`[pbx.parking]`) is the tenant's.
#[utoipa::path(
    get,
    path = "/api/v1/parking/lots",
    tag = "parking",
    operation_id = "parking_list_lots",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<ParkingLotResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_lots(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::ParkingRead, state.deployment_mode)?;
    let lots = store(&state)?
        .list_parking_lots(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(crate::page::paginate(
        lots.into_iter()
            .map(ParkingLotResponse::from)
            .collect::<Vec<_>>(),
        &page,
        crate::page::Order::Ascending,
        |r| format!("{:08}/{}/{}", r.first.len(), r.first, r.id),
    )?))
}

/// GET /api/v1/parking/lots/:id
#[utoipa::path(
    get,
    path = "/api/v1/parking/lots/{id}",
    tag = "parking",
    operation_id = "parking_get_lot",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = ParkingLotResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_lot(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::ParkingRead, state.deployment_mode)?;
    Ok(Json(ParkingLotResponse::from(
        owned(&state, &tenant.0, &id).await?,
    )))
}

/// POST /api/v1/parking/lots — 409 when one of its numbers is taken.
#[utoipa::path(
    post,
    path = "/api/v1/parking/lots",
    tag = "parking",
    operation_id = "parking_create_lot",
    request_body = ParkingLotRequest,
    responses(
        (status = 201, description = "OK", body = ParkingLotResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_lot(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<ParkingLotRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::ParkingManage, state.deployment_mode)?;
    let mut lot = ParkingLot::new(tenant.0, "", "", "");
    apply(&mut lot, req)?;
    if let Some(why) = clash(&state, &lot).await? {
        return Err(ApiError::Conflict(why));
    }
    store(&state)?
        .put_parking_lot(&lot)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &lot).await;
    Ok((StatusCode::CREATED, Json(ParkingLotResponse::from(lot))))
}

/// PUT /api/v1/parking/lots/:id — the whole lot, replaced.
#[utoipa::path(
    put,
    path = "/api/v1/parking/lots/{id}",
    tag = "parking",
    operation_id = "parking_update_lot",
    params(("id" = String, Path)),
    request_body = ParkingLotRequest,
    responses(
        (status = 200, description = "OK", body = ParkingLotResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_lot(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<ParkingLotRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::ParkingManage, state.deployment_mode)?;
    let mut lot = owned(&state, &tenant.0, &id).await?;
    apply(&mut lot, req)?;
    lot.updated_at = SystemTime::now();
    if let Some(why) = clash(&state, &lot).await? {
        return Err(ApiError::Conflict(why));
    }
    store(&state)?
        .put_parking_lot(&lot)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &lot).await;
    Ok(Json(ParkingLotResponse::from(lot)))
}

/// DELETE /api/v1/parking/lots/:id — calls parked in it stay until taken
/// or timed out.
#[utoipa::path(
    delete,
    path = "/api/v1/parking/lots/{id}",
    tag = "parking",
    operation_id = "parking_delete_lot",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_lot(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::ParkingManage, state.deployment_mode)?;
    let lot = owned(&state, &tenant.0, &id).await?;
    store(&state)?
        .delete_parking_lot(&tenant.0, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &lot).await;
    Ok(StatusCode::NO_CONTENT)
}

// ─── Parked calls (`docs/PARKING.md` §4) ─────────────────────────────────

/// Who a parked call is from.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ParkedCaller {
    pub uri: String,
    pub name: Option<String>,
}

/// A call waiting in a slot, on whichever call manager holds it.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ParkedCallResponse {
    pub slot: String,
    /// The lot; `default` for the node's own range.
    pub lot_id: String,
    pub call_id: String,
    pub caller: Option<ParkedCaller>,
    pub parker_extension: String,
    /// How long it has waited, and how long it may.
    pub parked_secs: u64,
    pub timeout_secs: u64,
    /// The call manager holding it.
    pub node_id: String,
}

/// Where a parked call is sent.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RetrieveParkedRequest {
    /// An extension or a user: their phones ring, and answering takes the
    /// call. Unanswered, their own plan decides (their voicemail).
    pub target: String,
}

fn supervisor(state: &ApiState) -> Result<&Arc<dyn crate::CallSupervisor>, ApiError> {
    state
        .call_supervisor
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("no call manager can be reached".to_string()))
}

/// What a node answered, in the API's shape.
fn node_answer(status: u16, body: serde_json::Value) -> axum::response::Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    if !status.is_success() {
        let error = crate::error::ErrorResponse::from_upstream(status, &body);
        return (status, Json(error)).into_response();
    }
    (status, Json(body)).into_response()
}

/// GET /api/v1/parking/calls — the tenant's parked calls on every call
/// manager, by slot.
#[utoipa::path(
    get,
    path = "/api/v1/parking/calls",
    tag = "parking",
    operation_id = "parking_list_calls",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<ParkedCallResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_calls(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::ParkingRead, state.deployment_mode)?;
    let calls = supervisor(&state)?
        .parked_calls(tenant.0)
        .await
        .map_err(ApiError::ServiceUnavailable)?;
    let calls: Vec<ParkedCallResponse> = calls
        .into_iter()
        .filter_map(|c| serde_json::from_value(c).ok())
        .collect();
    Ok(Json(crate::page::paginate(
        calls,
        &page,
        crate::page::Order::Ascending,
        |c| format!("{:08}/{}/{}", c.slot.len(), c.slot, c.call_id),
    )?))
}

/// POST /api/v1/parking/calls/:slot/retrieve — ring `target` with the
/// call in the slot (202); 404 when the slot is empty.
#[utoipa::path(
    post,
    path = "/api/v1/parking/calls/{slot}/retrieve",
    tag = "parking",
    operation_id = "parking_retrieve_call",
    params(("slot" = String, Path)),
    request_body = RetrieveParkedRequest,
    responses(
        (status = 202, description = "Ringing the target"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn retrieve_call(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(slot): Path<String>,
    Json(req): Json<RetrieveParkedRequest>,
) -> Result<axum::response::Response, ApiError> {
    crate::auth::check_permission(&identity, Permission::ParkingManage, state.deployment_mode)?;
    if !fcp_core::parking::is_number(&slot) {
        return Err(ApiError::BadRequest(format!("{slot:?} is not a slot")));
    }
    let target = req.target.trim();
    if target.is_empty() {
        return Err(ApiError::BadRequest("target is required".to_string()));
    }
    let (status, body) = supervisor(&state)?
        .parked_call(tenant.0, &slot, Some(target))
        .await
        .map_err(ApiError::ServiceUnavailable)?;
    Ok(node_answer(status, body))
}

/// DELETE /api/v1/parking/calls/:slot — hang up the call in the slot.
#[utoipa::path(
    delete,
    path = "/api/v1/parking/calls/{slot}",
    tag = "parking",
    operation_id = "parking_hang_up_call",
    params(("slot" = String, Path)),
    responses(
        (status = 200, description = "Ended"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn hang_up_call(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(slot): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    crate::auth::check_permission(&identity, Permission::ParkingManage, state.deployment_mode)?;
    if !fcp_core::parking::is_number(&slot) {
        return Err(ApiError::BadRequest(format!("{slot:?} is not a slot")));
    }
    let (status, body) = supervisor(&state)?
        .parked_call(tenant.0, &slot, None)
        .await
        .map_err(ApiError::ServiceUnavailable)?;
    Ok(node_answer(status, body))
}
