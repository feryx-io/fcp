//! Dashboard summary endpoint.
//!
//! GET /api/v1/dashboard/summary — returns aggregated stats for the admin UI.

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{extract::State, response::IntoResponse, Json};
use fcp_core::storage::{DeviceStore, ExtensionStore, PhoneNumberStore};
use fcp_core::{CdrQuery, CdrStore, CredentialStore, RegistrationStore};
use serde::Serialize;
use std::collections::HashSet;

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
pub async fn summary(
    State(state): State<ApiState>,
    tenant: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let mut summary = DashboardSummary {
        active_calls: 0,
        registered_devices: 0,
        total_users: 0,
        total_extensions: 0,
        total_phone_numbers: 0,
        calls_today: 0,
        avg_call_duration_secs: 0.0,
    };

    summary.active_calls = state
        .call_manager
        .active_calls()
        .into_iter()
        .filter(|call| call.tenant_id == tenant.0)
        .count();

    // Get counts from storage
    if let Some(storage) = &state.storage {
        let tenant_id = &tenant.0;

        // Users
        if let Ok(users) = storage.list_users().await {
            summary.total_users = users
                .into_iter()
                .filter(|user| user.tenant_id == *tenant_id)
                .count();
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
        if let (Ok(devices), Ok(registrations)) = (
            storage.list_devices(tenant_id).await,
            storage.get_all_registrations_in_tenant(tenant_id).await,
        ) {
            let device_aors: HashSet<String> = devices
                .into_iter()
                .filter_map(|device| device.sip_uri)
                .filter_map(|sip_uri| fcp_sip::presence::extract_aor(&sip_uri))
                .collect();

            summary.registered_devices = registrations
                .into_iter()
                .filter(|registration| {
                    let aor = registration
                        .aor
                        .trim_start_matches("sip:")
                        .trim_start_matches("sips:");
                    device_aors.contains(aor)
                })
                .count();
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
            tenant_id: Some(*tenant_id),
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
