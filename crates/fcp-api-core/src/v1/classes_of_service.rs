//! A tenant's classes of service (`docs/ENTERPRISE_CALLING.md` §3.7): what
//! its users may dial, how many calls they may be on, how their caller ID
//! leaves. One class may be the default, the class of every user with
//! none of their own. `GET /classes-of-service/classify` says what a
//! number is to the tenant, so a page can show it before anyone dials.

use crate::auth::AuthIdentity;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::{
    CallerIdPolicy, CallingStore, ClassOfService, ClassOfServiceId, ClassOfServiceStore,
    DestinationClass, TenantStore,
};
use fcp_events::{PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ClassOfServiceResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub allowed: Vec<DestinationClass>,
    pub max_calls: Option<u32>,
    pub caller_id: CallerIdPolicy,
    pub is_default: bool,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: u64,
}

impl From<ClassOfService> for ClassOfServiceResponse {
    fn from(c: ClassOfService) -> Self {
        Self {
            id: c.id.to_string(),
            tenant_id: c.tenant_id.to_string(),
            updated_at: c
                .updated_at
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            name: c.name,
            allowed: c.allowed,
            max_calls: c.max_calls,
            caller_id: c.caller_id,
            is_default: c.is_default,
        }
    }
}

/// What a class is written as. `allowed` unset allows everything;
/// `max_calls` unset is two, `null` no limit.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ClassOfServiceRequest {
    pub name: String,
    #[serde(default)]
    pub allowed: Option<Vec<DestinationClass>>,
    #[serde(default, deserialize_with = "present")]
    pub max_calls: Option<Option<u32>>,
    #[serde(default)]
    pub caller_id: CallerIdPolicy,
    #[serde(default)]
    pub is_default: bool,
}

/// A field given, `null` included: absent is `None`, `null` `Some(None)`.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<u32>>, D::Error> {
    Option::<u32>::deserialize(d).map(Some)
}

impl ClassOfServiceRequest {
    fn apply(self, class: &mut ClassOfService) {
        class.name = self.name;
        class.allowed = self
            .allowed
            .unwrap_or_else(|| DestinationClass::ALL.to_vec());
        if !class.allowed.contains(&DestinationClass::Emergency) {
            // Always allowed; listed so a page reading it back says so.
            class.allowed.push(DestinationClass::Emergency);
        }
        let mut seen = std::collections::HashSet::new();
        class.allowed.retain(|c| seen.insert(*c));
        class.max_calls = match self.max_calls {
            None => Some(fcp_core::class_of_service::DEFAULT_MAX_CALLS),
            Some(max) => max,
        };
        class.caller_id = self.caller_id;
        class.is_default = self.is_default;
    }
}

fn parse_id(id: &str) -> Result<ClassOfServiceId, ApiError> {
    id.parse()
        .map_err(|_| ApiError::BadRequest(format!("Invalid class of service ID: {id}")))
}

async fn announce(state: &ApiState, class: &ClassOfService) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "class_of_service".to_string(),
            entity_id: class.id.to_string(),
            tenant_id: class.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}

async fn owned(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    id: &str,
) -> Result<ClassOfService, ApiError> {
    let storage = require_storage!(state);
    storage
        .get_class_of_service(&parse_id(id)?)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|c| c.tenant_id == *tenant)
        .ok_or_else(|| ApiError::NotFound(format!("Class of service {id} not found")))
}

/// Write a class; a new default takes the mark from the old one.
async fn store(state: &ApiState, class: &ClassOfService) -> Result<(), ApiError> {
    let storage = require_storage!(state);
    class.validate().map_err(ApiError::BadRequest)?;
    let others = storage
        .list_classes_of_service(&class.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if let Some(taken) = others
        .iter()
        .find(|c| c.id != class.id && c.name.eq_ignore_ascii_case(&class.name))
    {
        return Err(ApiError::Conflict(format!(
            "A class of service named `{}` exists",
            taken.name
        )));
    }
    if class.is_default {
        for mut other in others
            .into_iter()
            .filter(|c| c.id != class.id && c.is_default)
        {
            other.is_default = false;
            other.updated_at = SystemTime::now();
            storage
                .put_class_of_service(other.clone())
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            announce(state, &other).await;
        }
    }
    storage
        .put_class_of_service(class.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(state, class).await;
    Ok(())
}

/// GET /api/v1/classes-of-service
#[utoipa::path(
    get,
    path = "/api/v1/classes-of-service",
    tag = "classes_of_service",
    operation_id = "classes_of_service_list_classes",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<ClassOfServiceResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_classes(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleRead,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);
    let classes = storage
        .list_classes_of_service(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(crate::page::paginate(
        classes
            .into_iter()
            .map(ClassOfServiceResponse::from)
            .collect::<Vec<_>>(),
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.name, r.id),
    )?))
}

/// GET /api/v1/classes-of-service/:id
#[utoipa::path(
    get,
    path = "/api/v1/classes-of-service/{id}",
    tag = "classes_of_service",
    operation_id = "classes_of_service_get_class",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = ClassOfServiceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_class(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleRead,
        state.deployment_mode,
    )?;
    Ok(Json(ClassOfServiceResponse::from(
        owned(&state, &tenant.0, &id).await?,
    )))
}

/// POST /api/v1/classes-of-service
#[utoipa::path(
    post,
    path = "/api/v1/classes-of-service",
    tag = "classes_of_service",
    operation_id = "classes_of_service_create_class",
    request_body = ClassOfServiceRequest,
    responses(
        (status = 201, description = "OK", body = ClassOfServiceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_class(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<ClassOfServiceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleManage,
        state.deployment_mode,
    )?;
    let mut class = ClassOfService::new(tenant.0, req.name.clone());
    req.apply(&mut class);
    store(&state, &class).await?;
    Ok((
        StatusCode::CREATED,
        Json(ClassOfServiceResponse::from(class)),
    ))
}

/// PUT /api/v1/classes-of-service/:id — the whole class, replaced.
#[utoipa::path(
    put,
    path = "/api/v1/classes-of-service/{id}",
    tag = "classes_of_service",
    operation_id = "classes_of_service_update_class",
    params(("id" = String, Path)),
    request_body = ClassOfServiceRequest,
    responses(
        (status = 200, description = "OK", body = ClassOfServiceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_class(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<ClassOfServiceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleManage,
        state.deployment_mode,
    )?;
    let mut class = owned(&state, &tenant.0, &id).await?;
    req.apply(&mut class);
    class.updated_at = SystemTime::now();
    store(&state, &class).await?;
    Ok(Json(ClassOfServiceResponse::from(class)))
}

/// DELETE /api/v1/classes-of-service/:id — refused while a user has it.
#[utoipa::path(
    delete,
    path = "/api/v1/classes-of-service/{id}",
    tag = "classes_of_service",
    operation_id = "classes_of_service_delete_class",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_class(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleManage,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);
    let class = owned(&state, &tenant.0, &id).await?;
    let users = storage
        .list_calling_settings(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .filter(|s| s.class_of_service_id == Some(class.id))
        .count();
    if users > 0 {
        return Err(ApiError::Conflict(format!(
            "{users} user(s) have the class `{}`; give them another first",
            class.name
        )));
    }
    storage
        .delete_class_of_service(&class.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &class).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ClassifyQuery {
    pub dialed: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ClassifyResponse {
    pub dialed: String,
    pub class: DestinationClass,
    pub e164: Option<String>,
}

/// GET /api/v1/classes-of-service/classify?dialed=… — what a number is to
/// the tenant, by its country and overrides. Extensions are read as the
/// platform's default of six digits.
#[utoipa::path(
    get,
    path = "/api/v1/classes-of-service/classify",
    tag = "classes_of_service",
    operation_id = "classes_of_service_classify",
    params(ClassifyQuery),
    responses(
        (status = 200, description = "OK", body = ClassifyResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn classify(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Query(query): Query<ClassifyQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleRead,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);
    let calling = storage
        .get_tenant(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|t| t.calling)
        .unwrap_or_default();
    let classified = fcp_core::class_of_service::classify(
        &query.dialed,
        &fcp_core::class_of_service::Dialing {
            country: calling.country.as_deref(),
            max_extension_digits: 6,
            overrides: &calling.destination_overrides,
        },
    );
    Ok(Json(ClassifyResponse {
        dialed: query.dialed,
        class: classified.class,
        e164: classified.e164,
    }))
}

/// A class id given to a user's settings: it must be one of the tenant's.
pub async fn tenant_class(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    given: &ClassOfServiceId,
) -> Result<(), ApiError> {
    owned(state, tenant, &given.to_string()).await.map(|_| ())
}
