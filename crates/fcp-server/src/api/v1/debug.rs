//! Debug API endpoints for inspecting live system state.
//!
//! These endpoints are intended for development and debugging purposes.
//! In production, these should be restricted via authentication/authorization.

use crate::api::state::ApiState;
use axum::{extract::State, Json};
use serde::Serialize;
use std::collections::HashMap;

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
pub async fn list_dialogs(State(state): State<ApiState>) -> Json<Vec<DialogInfo>> {
    let dialog_ids = state.dialog_manager.all_ids();

    let dialog_infos: Vec<DialogInfo> = dialog_ids
        .into_iter()
        .filter_map(|id| {
            state.dialog_manager.get(&id).map(|d| DialogInfo {
                call_id: d.id().call_id().to_string(),
                local_tag: d.id().local_tag().to_string(),
                remote_tag: d.id().remote_tag().to_string(),
                state: format!("{:?}", d.state()),
                local_uri: format!("{:?}", d.local_uri()),
                remote_uri: format!("{:?}", d.remote_uri()),
                remote_target: format!("{:?}", d.remote_target()),
                local_cseq: d.local_cseq(),
                remote_cseq: d.remote_cseq(),
                route_set: d.route_set().iter().map(|uri| format!("{:?}", uri)).collect(),
            })
        })
        .collect();

    Json(dialog_infos)
}

/// List active subscriptions.
///
/// GET /api/v1/debug/subscriptions
pub async fn list_subscriptions(State(state): State<ApiState>) -> Json<Vec<SubscriptionInfo>> {
    let subscriptions = state.subscription_manager.all();

    let sub_infos: Vec<SubscriptionInfo> = subscriptions
        .into_iter()
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
pub async fn list_rate_limits(State(state): State<ApiState>) -> Json<Vec<RateLimitInfo>> {
    let rate_limits = state.handler.rate_limits();

    let rate_limit_infos: Vec<RateLimitInfo> = rate_limits
        .iter()
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
                counts: window.counts.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
                window_start: format!("{:?}", window.window_start),
            }
        })
        .collect();

    Json(rate_limit_infos)
}

/// List active calls.
///
/// GET /api/v1/debug/calls
pub async fn list_calls(State(state): State<ApiState>) -> Json<Vec<CallInfo>> {
    let calls = state.call_manager.all_calls();

    let call_infos: Vec<CallInfo> = calls
        .into_iter()
        .map(|c| CallInfo {
            call_id: c.id.to_string(),
            a_leg_dialog_id: c.a_leg.dialog.as_ref().map(|d| format!(
                "{}:{}:{}",
                d.id().call_id(), d.id().local_tag(), d.id().remote_tag()
            )),
            b_leg_dialog_id: c.b_leg.as_ref().and_then(|b| {
                b.dialog.as_ref().map(|d| {
                    format!("{}:{}:{}", d.id().call_id(), d.id().local_tag(), d.id().remote_tag())
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
pub async fn debug_summary(State(state): State<ApiState>) -> Json<DebugSummary> {
    let summary = DebugSummary {
        active_dialogs: state.dialog_manager.count(),
        active_subscriptions: state.subscription_manager.all().len(),
        active_calls: state.call_manager.active_call_count(),
        rate_limit_entries: state.handler.rate_limit_count(),
    };

    Json(summary)
}
