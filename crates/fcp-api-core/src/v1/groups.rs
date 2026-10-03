//! A tenant's groups (`docs/USERS.md` §5): who is in them, the role they
//! give their members, and the identity-provider group they follow.

use crate::auth::AuthIdentity;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::group::{Group, GroupId, GroupMember};
use fcp_core::permissions::Permission;
use fcp_core::{CredentialStore, TenantId, User, UserRole};
use fcp_events::{PlatformEvent, SystemEvent, UserEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::SystemTime;

/// A group as the API answers it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GroupResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    /// The role the group's members hold besides their own.
    pub role: Option<String>,
    /// The group of this name at the tenant's identity providers.
    pub idp_group: Option<String>,
    pub member_count: usize,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: SystemTime,
}

fn response(group: Group, member_count: usize) -> GroupResponse {
    GroupResponse {
        id: group.id.to_string(),
        tenant_id: group.tenant_id.to_string(),
        role: group
            .role
            .map(|r| crate::v1::auth_session::role_to_string(&r)),
        name: group.name,
        description: group.description,
        idp_group: group.idp_group,
        member_count,
        created_at: group.created_at,
        updated_at: group.updated_at,
    }
}

/// What a group is written as.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct GroupRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// A role its members hold besides their own: not `super_admin`, nor
    /// `department_admin` (which needs a department).
    #[serde(default)]
    pub role: Option<String>,
    /// A group at the tenant's identity providers: a sign-in through one
    /// sets the user's membership from the groups the provider names.
    #[serde(default)]
    pub idp_group: Option<String>,
}

/// A member as the API answers it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GroupMemberResponse {
    pub user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub account_status: String,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub added_at: SystemTime,
}

/// Who to add.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AddMemberRequest {
    pub username: String,
}

fn storage(state: &ApiState) -> Result<&fcp_core::DynStorage, ApiError> {
    state
        .storage
        .as_deref()
        .ok_or_else(|| ApiError::NotFound("Storage is required but not configured".to_string()))
}

fn groups(state: &ApiState) -> Result<&std::sync::Arc<dyn fcp_core::group::GroupStore>, ApiError> {
    storage(state)?
        .group_store()
        .ok_or_else(|| ApiError::NotFound("Groups are not available here".to_string()))
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

async fn owned(state: &ApiState, tenant: &TenantId, id: &str) -> Result<Group, ApiError> {
    let id: GroupId = id
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("Invalid group ID: {id}")))?;
    groups(state)?
        .get_group(&id)
        .await
        .map_err(internal)?
        .filter(|g| g.tenant_id == *tenant)
        .ok_or_else(|| ApiError::NotFound(format!("Group {id} not found")))
}

/// The group a request describes, checked: a role a group may give, and
/// a name no other group of the tenant has.
async fn written(state: &ApiState, mut group: Group, req: GroupRequest) -> Result<Group, ApiError> {
    group.name = req.name;
    group.description = req.description;
    group.idp_group = req.idp_group;
    group.role = match req.role.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        Some(role) => Some(
            UserRole::from_str(role)
                .map_err(|_| ApiError::BadRequest(format!("Unknown role '{role}'")))?,
        ),
        None => None,
    };
    group.updated_at = SystemTime::now();
    let group = group.normalized().map_err(ApiError::BadRequest)?;
    let taken = groups(state)?
        .list_groups(&group.tenant_id)
        .await
        .map_err(internal)?
        .into_iter()
        .any(|g| g.id != group.id && g.name.eq_ignore_ascii_case(&group.name));
    if taken {
        return Err(ApiError::Conflict(format!(
            "The tenant already has a group named '{}'",
            group.name
        )));
    }
    Ok(group)
}

async fn announce(state: &ApiState, group: &Group) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "group".to_string(),
            entity_id: group.id.to_string(),
            tenant_id: group.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}

/// A user's groups changed: what they may do changed with them.
pub(crate) async fn membership_changed(state: &ApiState, user: &User) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Updated {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            changed_fields: vec!["groups".to_string()],
            timestamp: SystemTime::now(),
        }))
        .await;
}

/// GET /api/v1/groups
#[utoipa::path(
    get,
    path = "/api/v1/groups",
    tag = "groups",
    operation_id = "groups_list_groups",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<GroupResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_groups(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::GroupRead, state.deployment_mode)?;
    let store = groups(&state)?;
    let mut counts: HashMap<GroupId, usize> = HashMap::new();
    for m in store
        .list_tenant_members(&tenant.0)
        .await
        .map_err(internal)?
    {
        *counts.entry(m.group_id).or_default() += 1;
    }
    let listed: Vec<GroupResponse> = store
        .list_groups(&tenant.0)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|g| {
            let n = counts.get(&g.id).copied().unwrap_or(0);
            response(g, n)
        })
        .collect();
    Ok(Json(crate::page::paginate(
        listed,
        &page,
        crate::page::Order::Ascending,
        |g| format!("{}/{}", g.name.to_lowercase(), g.id),
    )?))
}

/// GET /api/v1/groups/:id
#[utoipa::path(
    get,
    path = "/api/v1/groups/{id}",
    tag = "groups",
    operation_id = "groups_get_group",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = GroupResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_group(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<GroupResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::GroupRead, state.deployment_mode)?;
    let group = owned(&state, &tenant.0, &id).await?;
    let n = groups(&state)?
        .list_members(&group.id)
        .await
        .map_err(internal)?
        .len();
    Ok(Json(response(group, n)))
}

/// POST /api/v1/groups
#[utoipa::path(
    post,
    path = "/api/v1/groups",
    tag = "groups",
    operation_id = "groups_create_group",
    request_body = GroupRequest,
    responses(
        (status = 201, description = "Created", body = GroupResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_group(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<GroupRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::GroupManage, state.deployment_mode)?;
    let group = written(&state, Group::new(tenant.0, String::new()), req).await?;
    groups(&state)?.put_group(&group).await.map_err(internal)?;
    announce(&state, &group).await;
    Ok((StatusCode::CREATED, Json(response(group, 0))))
}

/// PUT /api/v1/groups/:id
#[utoipa::path(
    put,
    path = "/api/v1/groups/{id}",
    tag = "groups",
    operation_id = "groups_update_group",
    params(("id" = String, Path)),
    request_body = GroupRequest,
    responses(
        (status = 200, description = "OK", body = GroupResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_group(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<GroupRequest>,
) -> Result<Json<GroupResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::GroupManage, state.deployment_mode)?;
    let before = owned(&state, &tenant.0, &id).await?;
    let group = written(&state, before, req).await?;
    let store = groups(&state)?;
    store.put_group(&group).await.map_err(internal)?;
    announce(&state, &group).await;
    let n = store.list_members(&group.id).await.map_err(internal)?.len();
    Ok(Json(response(group, n)))
}

/// DELETE /api/v1/groups/:id
#[utoipa::path(
    delete,
    path = "/api/v1/groups/{id}",
    tag = "groups",
    operation_id = "groups_delete_group",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Deleted; queues that took agents from it take none"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_group(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::GroupManage, state.deployment_mode)?;
    let group = owned(&state, &tenant.0, &id).await?;
    groups(&state)?
        .delete_group(&group.id)
        .await
        .map_err(internal)?;
    announce(&state, &group).await;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/groups/:id/members
#[utoipa::path(
    get,
    path = "/api/v1/groups/{id}/members",
    tag = "groups",
    operation_id = "groups_list_members",
    params(("id" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<GroupMemberResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_members(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::GroupRead, state.deployment_mode)?;
    let group = owned(&state, &tenant.0, &id).await?;
    let members = groups(&state)?
        .list_members(&group.id)
        .await
        .map_err(internal)?;
    let users: HashMap<fcp_core::UserId, User> = storage(&state)?
        .list_users_in_tenant(&tenant.0, &Default::default())
        .await?
        .into_iter()
        .map(|u| (u.id, u))
        .collect();
    let listed: Vec<GroupMemberResponse> = members
        .into_iter()
        .filter_map(|m| {
            let user = users.get(&m.user_id)?;
            Some(GroupMemberResponse {
                user_id: user.id.to_string(),
                username: user.username.clone(),
                display_name: user.display_name.clone(),
                account_status: user.account_status.to_string(),
                added_at: m.added_at,
            })
        })
        .collect();
    Ok(Json(crate::page::paginate(
        listed,
        &page,
        crate::page::Order::Ascending,
        |m| format!("{}/{}", m.username.to_lowercase(), m.user_id),
    )?))
}

async fn member_user(
    state: &ApiState,
    tenant: &TenantId,
    username: &str,
) -> Result<User, ApiError> {
    storage(state)?
        .get_user_in_tenant(username, tenant)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{username}' not found")))
}

/// POST /api/v1/groups/:id/members
#[utoipa::path(
    post,
    path = "/api/v1/groups/{id}/members",
    tag = "groups",
    operation_id = "groups_add_member",
    params(("id" = String, Path)),
    request_body = AddMemberRequest,
    responses(
        (status = 204, description = "A member (already, or now)"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn add_member(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<AddMemberRequest>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::GroupManage, state.deployment_mode)?;
    let group = owned(&state, &tenant.0, &id).await?;
    let user = member_user(&state, &tenant.0, &req.username).await?;
    crate::v1::users::refuse_deprovisioned(&user)?;
    let added = groups(&state)?
        .add_member(&GroupMember {
            group_id: group.id,
            user_id: user.id,
            tenant_id: tenant.0,
            added_at: SystemTime::now(),
        })
        .await
        .map_err(internal)?;
    if added {
        membership_changed(&state, &user).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/groups/:id/members/:username
#[utoipa::path(
    delete,
    path = "/api/v1/groups/{id}/members/{username}",
    tag = "groups",
    operation_id = "groups_remove_member",
    params(("id" = String, Path), ("username" = String, Path)),
    responses(
        (status = 204, description = "Not a member (now)"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn remove_member(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path((id, username)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::GroupManage, state.deployment_mode)?;
    let group = owned(&state, &tenant.0, &id).await?;
    let user = member_user(&state, &tenant.0, &username).await?;
    if groups(&state)?
        .remove_member(&group.id, &user.id)
        .await
        .map_err(internal)?
    {
        membership_changed(&state, &user).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/users/:username/groups
#[utoipa::path(
    get,
    path = "/api/v1/users/{username}/groups",
    tag = "groups",
    operation_id = "groups_list_user_groups",
    params(("username" = String, Path)),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<GroupResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_user_groups(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::GroupRead, state.deployment_mode)?;
    let user = member_user(&state, &tenant.0, &username).await?;
    let store = groups(&state)?;
    let mut listed = Vec::new();
    for g in store.groups_of_user(&user.id).await.map_err(internal)? {
        let n = store.list_members(&g.id).await.map_err(internal)?.len();
        listed.push(response(g, n));
    }
    Ok(Json(crate::page::paginate(
        listed,
        &crate::page::PageQuery::default(),
        crate::page::Order::Ascending,
        |g| format!("{}/{}", g.name.to_lowercase(), g.id),
    )?))
}

/// Set a user's membership of the groups that follow an identity
/// provider's (`idp_group`) from the groups a sign-in through it names
/// (`docs/USERS.md` §5): added to each named, taken out of the rest.
/// Groups no provider names are the tenant's own, and untouched. Whether
/// anything changed.
pub(crate) async fn follow_idp_groups(
    state: &ApiState,
    user: &User,
    claimed: &[String],
) -> Result<bool, ApiError> {
    let Ok(store) = groups(state) else {
        return Ok(false);
    };
    let following: Vec<Group> = store
        .list_groups(&user.tenant_id)
        .await
        .map_err(internal)?
        .into_iter()
        .filter(|g| g.idp_group.is_some())
        .collect();
    let mut changed = false;
    for group in following {
        let wanted = group
            .idp_group
            .as_deref()
            .is_some_and(|name| claimed.iter().any(|c| c == name));
        changed |= if wanted {
            store
                .add_member(&GroupMember {
                    group_id: group.id,
                    user_id: user.id,
                    tenant_id: user.tenant_id,
                    added_at: SystemTime::now(),
                })
                .await
                .map_err(internal)?
        } else {
            store
                .remove_member(&group.id, &user.id)
                .await
                .map_err(internal)?
        };
    }
    if changed {
        membership_changed(state, user).await;
    }
    Ok(changed)
}
