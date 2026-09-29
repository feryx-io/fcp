//! Debug API endpoints for inspecting live system state.
//!
//! These endpoints are intended for development and debugging purposes.
//! In production, these should be restricted via authentication/authorization.

use crate::error::ApiError;
use crate::{
    auth::{can_access_all_tenants, AuthIdentity, TenantContext},
    state::ApiState,
};
use axum::{
    extract::{Extension, State},
    Json,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// SIP Call-IDs a tenant-scoped caller may see (`None` = everything).
///
/// Dialogs and subscriptions are keyed by the SIP `Call-ID` header, not by
/// FCP's internal `call-…` id, so the visible set is built from the SIP
/// dialogs attached to the tenant's B2BUA call legs. The internal ids are
/// included as well for callers that key on them.
fn visible_call_ids(
    state: &ApiState,
    tenant: &TenantContext,
    identity: Option<&AuthIdentity>,
) -> Option<HashSet<String>> {
    if can_access_all_tenants(identity) {
        return None;
    }
    let mut visible = HashSet::new();
    for call in state
        .call_manager
        .all_calls()
        .into_iter()
        .filter(|call| call.tenant_id == tenant.0)
    {
        visible.insert(call.id.to_string());
        if let Some(dialog) = &call.a_leg.dialog {
            visible.insert(dialog.id().call_id().to_string());
        }
        if let Some(dialog) = call.b_leg.as_ref().and_then(|leg| leg.dialog.as_ref()) {
            visible.insert(dialog.id().call_id().to_string());
        }
    }
    Some(visible)
}

/// Dialog information response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DialogInfo {
    pub call_id: String,
    pub local_tag: String,
    pub remote_tag: String,
    pub state: String,
    pub local_uri: String,
    pub remote_uri: String,
    pub remote_target: String,
    pub local_cseq: u32,
    pub remote_cseq: u32,
    pub route_set: Vec<String>,
}

/// Subscription information response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SubscriptionInfo {
    pub call_id: String,
    pub from_tag: String,
    pub to_tag: String,
    pub event: String,
    pub state: String,
    pub contact: String,
    /// The subscription's lifetime, as its `Expires` gave it.
    pub expires_in_secs: u64,
}

/// Rate limit state response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RateLimitInfo {
    pub dialog_id: String,
    pub window_start: String,
    pub counts: HashMap<String, u32>,
}

/// Active calls information.
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(as = debug::CallInfo)]
pub struct CallInfo {
    pub call_id: String,
    pub media_session_id: Option<String>,
    pub a_leg_dialog_id: Option<String>,
    pub b_leg_dialog_id: Option<String>,
    pub state: String,
}

/// System debug summary.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DebugSummary {
    pub active_dialogs: usize,
    pub active_subscriptions: usize,
    pub active_calls: usize,
    pub rate_limit_entries: usize,
}

/// List active dialogs.
///
/// GET /api/v1/debug/dialogs
#[utoipa::path(
    get,
    path = "/api/v1/debug/dialogs",
    tag = "debug",
    operation_id = "debug_list_dialogs",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<DialogInfo>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_dialogs(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<DialogInfo>>, ApiError> {
    let dialog_ids = state.dialog_manager.all_ids();
    let visible_call_ids = visible_call_ids(&state, &tenant, identity.as_ref().map(|ext| &ext.0));

    let dialog_infos: Vec<DialogInfo> = dialog_ids
        .into_iter()
        .filter_map(|id| {
            state.dialog_manager.get(&id).and_then(|d| {
                let call_id = d.id().call_id().to_string();
                if visible_call_ids
                    .as_ref()
                    .map(|visible| visible.contains(&call_id))
                    .unwrap_or(true)
                {
                    Some(DialogInfo {
                        call_id,
                        local_tag: d.id().local_tag().to_string(),
                        remote_tag: d.id().remote_tag().to_string(),
                        state: format!("{:?}", d.state()),
                        local_uri: format!("{:?}", d.local_uri()),
                        remote_uri: format!("{:?}", d.remote_uri()),
                        remote_target: format!("{:?}", d.remote_target()),
                        local_cseq: d.local_cseq(),
                        remote_cseq: d.remote_cseq(),
                        route_set: d
                            .route_set()
                            .iter()
                            .map(|uri| format!("{:?}", uri))
                            .collect(),
                    })
                } else {
                    None
                }
            })
        })
        .collect();

    Ok(Json(crate::page::paginate(
        dialog_infos,
        &page,
        crate::page::Order::Ascending,
        |r| r.call_id.clone(),
    )?))
}

/// List active subscriptions.
///
/// GET /api/v1/debug/subscriptions
#[utoipa::path(
    get,
    path = "/api/v1/debug/subscriptions",
    tag = "debug",
    operation_id = "debug_list_subscriptions",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<SubscriptionInfo>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_subscriptions(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<SubscriptionInfo>>, ApiError> {
    let subscriptions = state.subscription_manager.all();
    let visible_call_ids = visible_call_ids(&state, &tenant, identity.as_ref().map(|ext| &ext.0));

    let sub_infos: Vec<SubscriptionInfo> = subscriptions
        .into_iter()
        .filter(|s| {
            visible_call_ids
                .as_ref()
                .map(|visible| visible.contains(&s.id().call_id().to_string()))
                .unwrap_or(true)
        })
        .map(|s| SubscriptionInfo {
            call_id: s.id().call_id().to_string(),
            from_tag: s.id().from_tag().to_string(),
            to_tag: s.id().to_tag().to_string(),
            event: s.id().event().to_string(),
            state: format!("{:?}", s.state()),
            contact: format!("{:?}", s.contact()),
            expires_in_secs: s.expires().as_secs(),
        })
        .collect();

    Ok(Json(crate::page::paginate(
        sub_infos,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.call_id, r.event),
    )?))
}

/// List rate limit state.
///
/// GET /api/v1/debug/rate-limits
#[utoipa::path(
    get,
    path = "/api/v1/debug/rate-limits",
    tag = "debug",
    operation_id = "debug_list_rate_limits",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<RateLimitInfo>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_rate_limits(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<RateLimitInfo>>, ApiError> {
    let rate_limits = state.handler.rate_limits();
    let visible_call_ids = visible_call_ids(&state, &tenant, identity.as_ref().map(|ext| &ext.0));

    let rate_limit_infos: Vec<RateLimitInfo> = rate_limits
        .iter()
        .filter(|entry| {
            visible_call_ids
                .as_ref()
                .map(|visible| visible.contains(&entry.key().call_id().to_string()))
                .unwrap_or(true)
        })
        .map(|entry| {
            let dialog_id = entry.key();
            let window = entry.value();
            RateLimitInfo {
                dialog_id: format!(
                    "{}:{}:{}",
                    dialog_id.call_id(),
                    dialog_id.local_tag(),
                    dialog_id.remote_tag()
                ),
                counts: window
                    .counts
                    .iter()
                    .map(|(k, v)| (k.to_string(), *v))
                    .collect(),
                window_start: format!("{:?}", window.window_start),
            }
        })
        .collect();

    Ok(Json(crate::page::paginate(
        rate_limit_infos,
        &page,
        crate::page::Order::Ascending,
        |r| r.dialog_id.clone(),
    )?))
}

/// List active calls.
///
/// GET /api/v1/debug/calls
#[utoipa::path(
    get,
    path = "/api/v1/debug/calls",
    tag = "debug",
    operation_id = "debug_list_calls",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<CallInfo>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_calls(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<CallInfo>>, ApiError> {
    let allow_all_tenants = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0));
    let calls: Vec<_> = state
        .call_manager
        .all_calls()
        .into_iter()
        .filter(|call| allow_all_tenants || call.tenant_id == tenant.0)
        .collect();

    let call_infos: Vec<CallInfo> = calls
        .into_iter()
        .map(|c| CallInfo {
            call_id: c.id.to_string(),
            media_session_id: c.media_session_id.as_ref().map(|id| id.to_string()),
            a_leg_dialog_id: c.a_leg.dialog.as_ref().map(|d| {
                format!(
                    "{}:{}:{}",
                    d.id().call_id(),
                    d.id().local_tag(),
                    d.id().remote_tag()
                )
            }),
            b_leg_dialog_id: c.b_leg.as_ref().and_then(|b| {
                b.dialog.as_ref().map(|d| {
                    format!(
                        "{}:{}:{}",
                        d.id().call_id(),
                        d.id().local_tag(),
                        d.id().remote_tag()
                    )
                })
            }),
            state: format!("{:?}", c.state),
        })
        .collect();

    Ok(Json(crate::page::paginate(
        call_infos,
        &page,
        crate::page::Order::Ascending,
        |r| r.call_id.clone(),
    )?))
}

/// Get debug summary.
///
/// GET /api/v1/debug/summary
#[utoipa::path(
    get,
    path = "/api/v1/debug/summary",
    tag = "debug",
    operation_id = "debug_debug_summary",
    responses(
        (status = 200, description = "OK", body = DebugSummary),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn debug_summary(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Json<DebugSummary> {
    let visible_call_ids = visible_call_ids(&state, &tenant, identity.as_ref().map(|ext| &ext.0));
    let summary = DebugSummary {
        active_dialogs: state
            .dialog_manager
            .all_ids()
            .into_iter()
            .filter(|id| {
                visible_call_ids
                    .as_ref()
                    .map(|visible| visible.contains(&id.call_id().to_string()))
                    .unwrap_or(true)
            })
            .count(),
        active_subscriptions: state
            .subscription_manager
            .all()
            .into_iter()
            .filter(|subscription| {
                visible_call_ids
                    .as_ref()
                    .map(|visible| visible.contains(&subscription.id().call_id().to_string()))
                    .unwrap_or(true)
            })
            .count(),
        active_calls: state
            .call_manager
            .all_calls()
            .into_iter()
            .filter(|call| {
                visible_call_ids
                    .as_ref()
                    .map(|visible| visible.contains(&call.id.to_string()))
                    .unwrap_or(true)
            })
            .count(),
        rate_limit_entries: state
            .handler
            .rate_limits()
            .iter()
            .filter(|entry| {
                visible_call_ids
                    .as_ref()
                    .map(|visible| visible.contains(&entry.key().call_id().to_string()))
                    .unwrap_or(true)
            })
            .count(),
    };

    Json(summary)
}
