//! Prometheus metrics endpoint

use crate::{error::ApiError, state::ApiState};
use axum::{extract::State, http::header, response::IntoResponse};

/// GET /metrics
/// Prometheus metrics endpoint
pub async fn prometheus_metrics(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
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
    buffer.push_str(&format!("fcp_subscriptions_active {}\n", active_subscriptions));

    // TODO: Add more metrics:
    // - Call duration histograms
    // - SIP message counters (INVITE, BYE, etc.)
    // - Error rates
    // - Media session metrics
    // - Registration counts

    Ok((
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        buffer,
    ))
}
