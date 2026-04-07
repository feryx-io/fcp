//! Dashboard summary endpoint.
//!
//! GET /api/v1/dashboard/summary — returns aggregated stats for the admin UI.

use crate::error::ApiError;
use crate::state::ApiState;
use axum::{extract::State, response::IntoResponse, Json};
use fcp_core::storage::{ExtensionStore, PhoneNumberStore};
use fcp_core::{CdrQuery, CdrStore, CredentialStore, RegistrationStore};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DashboardSummary {
    pub active_calls: usize,
    pub registered_devices: usize,
    pub total_users: usize,
    pub total_extensions: usize,
    pub total_phone_numbers: usize,
    pub calls_today: usize,
    pub avg_call_duration_secs: f64,
}

/// GET /api/v1/dashboard/summary
pub async fn summary(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    let mut summary = DashboardSummary {
        active_calls: 0,
        registered_devices: 0,
        total_users: 0,
        total_extensions: 0,
        total_phone_numbers: 0,
        calls_today: 0,
        avg_call_duration_secs: 0.0,
    };

    // Active calls: try events server first, fall back to local call manager
    if let Some(ref events_url) = state.events_server_url {
        let url = format!("{}/stats", events_url);
        let client = reqwest::Client::new();
        let mut request = client.get(&url);
        if let Some(token) = state.events_server_auth_token.as_deref() {
            request = request.bearer_auth(token);
        }

        if let Ok(resp) = request.send().await {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(count) = body.pointer("/calls/active").and_then(|v| v.as_u64()) {
                    summary.active_calls = count as usize;
                }
            }
        }
    }
    if summary.active_calls == 0 {
        summary.active_calls = state.call_manager.active_call_count();
    }

    // Get counts from storage
    if let Some(storage) = &state.storage {
        let tenant_id = &state.default_tenant_id;

        // Users
        if let Ok(users) = storage.list_users().await {
            summary.total_users = users.len();
        }

        // Extensions
        if let Ok(extensions) = storage.list_extensions(tenant_id).await {
            summary.total_extensions = extensions.len();
        }

        // Phone numbers
        if let Ok(numbers) = storage.list_phone_numbers(tenant_id).await {
            summary.total_phone_numbers = numbers.len();
        }

        // Registered devices (from registrations)
        if let Ok(registrations) = storage.get_all_registrations().await {
            summary.registered_devices = registrations.len();
        }

        // CDR stats - calls today
        let today_start = {
            use std::time::{Duration, SystemTime};
            let now = SystemTime::now();
            let since_epoch = now.duration_since(std::time::UNIX_EPOCH).unwrap();
            let day_secs = since_epoch.as_secs() % 86400;
            now - Duration::from_secs(day_secs)
        };
        let query = CdrQuery {
            start_time_from: Some(today_start),
            ..Default::default()
        };
        if let Ok(cdrs) = storage.query_cdrs(query).await {
            summary.calls_today = cdrs.len();
            if !cdrs.is_empty() {
                let total_duration_ms: u64 =
                    cdrs.iter().filter_map(|cdr| cdr.call_duration_ms).sum();
                let calls_with_duration = cdrs
                    .iter()
                    .filter(|cdr| cdr.call_duration_ms.is_some())
                    .count();
                if calls_with_duration > 0 {
                    summary.avg_call_duration_secs =
                        (total_duration_ms as f64 / 1000.0) / calls_with_duration as f64;
                }
            }
        }
    }

    Ok(Json(summary))
}
