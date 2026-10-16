//! Dashboard summary endpoint.
//!
//! GET /api/v1/dashboard/summary — returns aggregated stats for the admin UI.

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{extract::State, response::IntoResponse, Json};
use fcp_core::storage::{DeviceStore, ExtensionStore, PhoneNumberStore};
use fcp_core::{CdrStore, CredentialStore, RegistrationStore};
use serde::Serialize;
use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DashboardSummary {
    pub active_calls: usize,
    pub registered_devices: usize,
    pub total_users: usize,
    pub total_extensions: usize,
    pub total_phone_numbers: usize,
    pub calls_today: usize,
    pub avg_call_duration_secs: f64,
}

/// A count that could not be read is an error, not a zero.
fn failed<E: std::fmt::Display>(what: &'static str) -> impl Fn(E) -> ApiError {
    move |e| ApiError::Internal(format!("Could not count {what}: {e}"))
}

/// GET /api/v1/dashboard/summary
#[utoipa::path(
    get,
    path = "/api/v1/dashboard/summary",
    tag = "dashboard",
    operation_id = "dashboard_summary",
    responses(
        (status = 200, description = "OK", body = DashboardSummary),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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

    // The calls up now, where the call list finds them: the cluster's.
    summary.active_calls = crate::v1::media::active_calls(&state, &tenant.0, false)
        .await
        .len();

    if let Some(storage) = &state.storage {
        let tenant_id = &tenant.0;
        summary.total_users = storage
            .count_users_in_tenant(tenant_id)
            .await
            .map_err(failed("users"))? as usize;
        summary.total_extensions = storage
            .list_extensions(tenant_id)
            .await
            .map_err(failed("extensions"))?
            .len();
        summary.total_phone_numbers = storage
            .list_phone_numbers(tenant_id)
            .await
            .map_err(failed("phone numbers"))?
            .len();

        // Registered devices: registrations at a device's address.
        let devices = storage
            .list_devices(tenant_id)
            .await
            .map_err(failed("devices"))?;
        let registrations = storage
            .get_all_registrations_in_tenant(tenant_id)
            .await
            .map_err(failed("registrations"))?;
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

        // Today's calls (since midnight UTC), counted in the database
        // rather than over a page of records.
        let now = SystemTime::now();
        let since_midnight = now
            .duration_since(UNIX_EPOCH)
            .map(|d| Duration::from_secs(d.as_secs() % 86_400))
            .unwrap_or_default();
        let usage = storage
            .aggregate_usage(tenant_id, now - since_midnight, now)
            .await
            .map_err(failed("today's calls"))?;
        summary.calls_today = usage.total_calls as usize;
        if usage.answered_calls > 0 {
            summary.avg_call_duration_secs =
                (usage.total_duration_ms as f64 / 1000.0) / usage.answered_calls as f64;
        }
    }

    Ok(Json(summary))
}
