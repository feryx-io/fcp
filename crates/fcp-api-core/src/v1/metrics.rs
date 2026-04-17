//! Prometheus metrics endpoint

use crate::{
    auth::{can_access_all_tenants, AuthIdentity},
    error::ApiError,
    state::ApiState,
};
use fcp_core::storage::RegistrationStore;
use axum::{
    extract::{Extension, State},
    http::header,
    response::IntoResponse,
};

/// GET /metrics
/// Prometheus metrics endpoint
pub async fn prometheus_metrics(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<impl IntoResponse, ApiError> {
    if identity.is_some() && !can_access_all_tenants(identity.as_ref().map(|ext| &ext.0)) {
        return Err(ApiError::Forbidden(
            "Metrics endpoint requires platform-admin scope".to_string(),
        ));
    }

    // Collect metrics
    let mut buffer = String::new();

    // System metrics
    buffer.push_str("# HELP fcp_up FCP server status (1 = up)\n");
    buffer.push_str("# TYPE fcp_up gauge\n");
    buffer.push_str("fcp_up 1\n");

    // Call metrics
    let active_calls = state.call_manager.active_call_count();
    let total_calls = state.call_manager.total_call_count();

    buffer.push_str("# HELP fcp_calls_active Number of active calls\n");
    buffer.push_str("# TYPE fcp_calls_active gauge\n");
    buffer.push_str(&format!("fcp_calls_active {}\n", active_calls));

    buffer.push_str("# HELP fcp_calls_total Total number of calls (including terminated)\n");
    buffer.push_str("# TYPE fcp_calls_total gauge\n");
    buffer.push_str(&format!("fcp_calls_total {}\n", total_calls));

    // Dialog metrics
    let active_dialogs = state.dialog_manager.count();

    buffer.push_str("# HELP fcp_dialogs_active Number of active SIP dialogs\n");
    buffer.push_str("# TYPE fcp_dialogs_active gauge\n");
    buffer.push_str(&format!("fcp_dialogs_active {}\n", active_dialogs));

    // Subscription metrics
    let active_subscriptions = state.subscription_manager.all().len();

    buffer.push_str("# HELP fcp_subscriptions_active Number of active subscriptions\n");
    buffer.push_str("# TYPE fcp_subscriptions_active gauge\n");
    buffer.push_str(&format!(
        "fcp_subscriptions_active {}\n",
        active_subscriptions
    ));

    // Uptime metric
    let uptime = state.start_time.elapsed().as_secs();
    buffer.push_str("# HELP fcp_uptime_seconds Server uptime in seconds\n");
    buffer.push_str("# TYPE fcp_uptime_seconds gauge\n");
    buffer.push_str(&format!("fcp_uptime_seconds {}\n", uptime));

    // Rate limit entries
    let rate_limit_entries = state.handler.rate_limit_count();
    buffer.push_str("# HELP fcp_rate_limit_entries Number of active rate limit entries\n");
    buffer.push_str("# TYPE fcp_rate_limit_entries gauge\n");
    buffer.push_str(&format!("fcp_rate_limit_entries {}\n", rate_limit_entries));

    // Registration count
    let active_registrations = if let Some(ref storage) = state.storage {
        storage.registration_count().await.unwrap_or(0)
    } else {
        0
    };
    buffer.push_str("# HELP fcp_active_registrations Number of active SIP registrations\n");
    buffer.push_str("# TYPE fcp_active_registrations gauge\n");
    buffer.push_str(&format!(
        "fcp_active_registrations {}\n",
        active_registrations
    ));

    // Call statistics
    let call_stats = state.call_manager.stats();
    buffer.push_str("# HELP fcp_calls_established Total established calls\n");
    buffer.push_str("# TYPE fcp_calls_established counter\n");
    buffer.push_str(&format!(
        "fcp_calls_established {}\n",
        call_stats.established
    ));

    buffer.push_str("# HELP fcp_calls_terminated Total terminated calls\n");
    buffer.push_str("# TYPE fcp_calls_terminated counter\n");
    buffer.push_str(&format!("fcp_calls_terminated {}\n", call_stats.terminated));

    buffer.push_str("# HELP fcp_calls_failed Total failed calls\n");
    buffer.push_str("# TYPE fcp_calls_failed counter\n");
    buffer.push_str(&format!("fcp_calls_failed {}\n", call_stats.failed));

    Ok((
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        buffer,
    ))
}
