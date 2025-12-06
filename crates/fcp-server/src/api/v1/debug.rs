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
pub async fn list_dialogs(State(_state): State<ApiState>) -> Json<Vec<DialogInfo>> {
    // TODO: DialogManager doesn't expose an all() method for listing all dialogs
    // This would require adding a public method to DialogManager
    // For now, return empty list
    Json(vec![])
}

/// List active subscriptions.
///
/// GET /api/v1/debug/subscriptions
pub async fn list_subscriptions(State(state): State<ApiState>) -> Json<Vec<SubscriptionInfo>> {
    let subscriptions = state.subscription_manager.all();

    let sub_infos: Vec<SubscriptionInfo> = subscriptions
        .into_iter()
        .map(|s| SubscriptionInfo {
            call_id: s.id.call_id.to_string(),
            from_tag: s.id.from_tag.to_string(),
            to_tag: s.id.to_tag.to_string(),
            event: s.id.event.to_string(),
            state: format!("{:?}", s.state),
            contact: format!("{:?}", s.contact),
            expires_at: format!("{:?}", s.expires),
        })
        .collect();

    Json(sub_infos)
}

/// List rate limit state.
///
/// GET /api/v1/debug/rate-limits
pub async fn list_rate_limits(State(_state): State<ApiState>) -> Json<Vec<RateLimitInfo>> {
    // Access rate_limits from handler
    // Note: This requires exposing rate_limits as pub in SipMessageHandler
    // For now, return empty array
    // TODO: Add public accessor method to SipMessageHandler
    Json(vec![])
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
                d.id.call_id, d.id.local_tag, d.id.remote_tag
            )),
            b_leg_dialog_id: c.b_leg.as_ref().and_then(|b| {
                b.dialog.as_ref().map(|d| {
                    format!("{}:{}:{}", d.id.call_id, d.id.local_tag, d.id.remote_tag)
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
        rate_limit_entries: 0, // TODO: add accessor
    };

    Json(summary)
}
