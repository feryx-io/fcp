//! Debug API endpoints for inspecting live system state.
//!
//! These endpoints are intended for development and debugging purposes.
//! In production, these should be restricted via authentication/authorization.

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

fn visible_call_ids(
    state: &ApiState,
    tenant: &TenantContext,
    identity: Option<&AuthIdentity>,
) -> Option<HashSet<String>> {
    if can_access_all_tenants(identity) {
        None
    } else {
        Some(
            state
                .call_manager
                .all_calls()
                .into_iter()
                .filter(|call| call.tenant_id == tenant.0)
                .map(|call| call.id.to_string())
                .collect(),
        )
    }
}

/// Dialog information response.
#[derive(Debug, Serialize)]
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
#[derive(Debug, Serialize)]
pub struct SubscriptionInfo {
    pub call_id: String,
    pub from_tag: String,
    pub to_tag: String,
    pub event: String,
    pub state: String,
    pub contact: String,
    pub expires_at: String,
}

/// Rate limit state response.
#[derive(Debug, Serialize)]
pub struct RateLimitInfo {
    pub dialog_id: String,
    pub window_start: String,
    pub counts: HashMap<String, u32>,
}

/// Active calls information.
#[derive(Debug, Serialize)]
pub struct CallInfo {
    pub call_id: String,
    pub media_session_id: Option<String>,
    pub a_leg_dialog_id: Option<String>,
    pub b_leg_dialog_id: Option<String>,
    pub state: String,
}

/// System debug summary.
#[derive(Debug, Serialize)]
pub struct DebugSummary {
    pub active_dialogs: usize,
    pub active_subscriptions: usize,
    pub active_calls: usize,
    pub rate_limit_entries: usize,
}

/// List active dialogs.
///
/// GET /api/v1/debug/dialogs
pub async fn list_dialogs(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Json<Vec<DialogInfo>> {
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

    Json(dialog_infos)
}

/// List active subscriptions.
///
/// GET /api/v1/debug/subscriptions
pub async fn list_subscriptions(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Json<Vec<SubscriptionInfo>> {
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
            expires_at: format!("{:?}", s.expires()),
        })
        .collect();

    Json(sub_infos)
}

/// List rate limit state.
///
/// GET /api/v1/debug/rate-limits
pub async fn list_rate_limits(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Json<Vec<RateLimitInfo>> {
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

    Json(rate_limit_infos)
}

/// List active calls.
///
/// GET /api/v1/debug/calls
pub async fn list_calls(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Json<Vec<CallInfo>> {
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

    Json(call_infos)
}

/// Get debug summary.
///
/// GET /api/v1/debug/summary
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
