//! Emergency calling (E911) administration.
//!
//! - `GET/PUT /emergency/config`: the tenant's emergency numbers, test
//!   numbers, dial prefixes, and on-site notification contacts.
//! - `GET /emergency/readiness`: which users have a dispatchable location
//!   (and callback number) today, and which do not.
//! - `GET /emergency/calls`: the audit trail of emergency calls placed.
//! - `POST /emergency/validate-address`: run the configured validator on
//!   an address without storing anything.
//! - `POST /phone-numbers/:id/e911/validate`: validate the number's E911
//!   address and mark it validated when it passes.

use crate::auth::TenantContext;
use crate::v1::phone_numbers::E911AddressResponse;
use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::e911::{
    resolve_for_user, AddressValidation, DispatchableLocation, EmergencyCallRecord,
    EmergencyConfig, EmergencyStore,
};
use fcp_core::storage::{CredentialStore, PhoneNumberStore};
use fcp_core::{E911Address, PhoneNumberId};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

fn require_store(state: &ApiState) -> Result<&Arc<dyn EmergencyStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.emergency_store())
        .ok_or_else(|| ApiError::Internal("Emergency store not configured".to_string()))
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

// ─── Config ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct EmergencyConfigResponse {
    pub tenant_id: String,
    pub emergency_numbers: Vec<String>,
    pub test_numbers: Vec<String>,
    pub dial_prefixes: Vec<String>,
    pub notify_contacts: Vec<String>,
    pub require_dispatchable_location: bool,
    /// Device types whose own location (a PIDF-LO they send with the call)
    /// this tenant trusts.
    pub trust_device_locations: Vec<String>,
    /// How long a user's declared location stands, in seconds.
    pub declared_location_ttl_secs: u64,
    /// False when the tenant has never saved a config (defaults shown).
    pub configured: bool,
    pub updated_at: u64,
}

impl EmergencyConfigResponse {
    fn from_config(c: EmergencyConfig, configured: bool) -> Self {
        Self {
            tenant_id: c.tenant_id.to_string(),
            emergency_numbers: c.emergency_numbers,
            test_numbers: c.test_numbers,
            dial_prefixes: c.dial_prefixes,
            notify_contacts: c.notify_contacts,
            require_dispatchable_location: c.require_dispatchable_location,
            trust_device_locations: c
                .trust_device_locations
                .iter()
                .map(|t| t.to_string())
                .collect(),
            declared_location_ttl_secs: c.declared_location_ttl.as_secs(),
            configured,
            updated_at: unix_secs(c.updated_at),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateEmergencyConfigRequest {
    pub emergency_numbers: Option<Vec<String>>,
    pub test_numbers: Option<Vec<String>>,
    pub dial_prefixes: Option<Vec<String>>,
    pub notify_contacts: Option<Vec<String>>,
    pub require_dispatchable_location: Option<bool>,
    /// Device types trusted to send their own location (`softphone`,
    /// `desk_phone`, `webrtc`, `mobile`, …); replaces the list.
    pub trust_device_locations: Option<Vec<String>>,
    /// How long a declared location stands, in seconds (60 to 30 days).
    pub declared_location_ttl_secs: Option<u64>,
}

pub async fn get_config(
    State(state): State<ApiState>,
    tenant: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    let stored = store
        .get_emergency_config(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let configured = stored.is_some();
    let config = stored.unwrap_or_else(|| EmergencyConfig::defaults(tenant.0));
    Ok(Json(EmergencyConfigResponse::from_config(
        config, configured,
    )))
}

fn clean_numbers(field: &str, values: Vec<String>) -> Result<Vec<String>, ApiError> {
    let mut out = Vec::new();
    for v in values {
        let v = v.trim().to_string();
        if v.is_empty() {
            continue;
        }
        if !v
            .chars()
            .all(|c| c.is_ascii_digit() || c == '*' || c == '#')
        {
            return Err(ApiError::BadRequest(format!(
                "{field}: '{v}' must be a dialable digit string"
            )));
        }
        if !out.contains(&v) {
            out.push(v);
        }
    }
    Ok(out)
}

pub async fn update_config(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<UpdateEmergencyConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    let mut config = store
        .get_emergency_config(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| EmergencyConfig::defaults(tenant.0));

    if let Some(numbers) = req.emergency_numbers {
        let numbers = clean_numbers("emergency_numbers", numbers)?;
        if numbers.is_empty() {
            return Err(ApiError::BadRequest(
                "emergency_numbers must contain at least one number".to_string(),
            ));
        }
        config.emergency_numbers = numbers;
    }
    if let Some(numbers) = req.test_numbers {
        config.test_numbers = clean_numbers("test_numbers", numbers)?;
    }
    if let Some(prefixes) = req.dial_prefixes {
        config.dial_prefixes = clean_numbers("dial_prefixes", prefixes)?;
    }
    if let Some(contacts) = req.notify_contacts {
        for c in &contacts {
            crate::validation::reject_crlf("notify_contacts", c)?;
        }
        config.notify_contacts = contacts
            .into_iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();
    }
    if let Some(require) = req.require_dispatchable_location {
        config.require_dispatchable_location = require;
    }
    if let Some(types) = req.trust_device_locations {
        let mut trusted = Vec::new();
        for name in types {
            let parsed: fcp_core::DeviceType =
                serde_json::from_value(serde_json::Value::String(name.trim().to_string()))
                    .map_err(|_| {
                        ApiError::BadRequest(format!(
                            "trust_device_locations: unknown device type '{name}'"
                        ))
                    })?;
            if !trusted.contains(&parsed) {
                trusted.push(parsed);
            }
        }
        config.trust_device_locations = trusted;
    }
    if let Some(secs) = req.declared_location_ttl_secs {
        if !(60..=30 * 24 * 60 * 60).contains(&secs) {
            return Err(ApiError::BadRequest(
                "declared_location_ttl_secs must be between 60 and 2592000 (30 days)".to_string(),
            ));
        }
        config.declared_location_ttl = std::time::Duration::from_secs(secs);
    }
    if config
        .emergency_numbers
        .iter()
        .any(|n| config.test_numbers.contains(n))
    {
        return Err(ApiError::BadRequest(
            "a number cannot be both an emergency number and a test number".to_string(),
        ));
    }
    config.updated_at = SystemTime::now();
    store
        .put_emergency_config(&config)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    info!(tenant_id = %tenant.0, numbers = ?config.emergency_numbers, "Emergency config updated");
    Ok(Json(EmergencyConfigResponse::from_config(config, true)))
}

// ─── Readiness ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct LocationResponse {
    pub address: E911AddressResponse,
    pub callback_number: Option<String>,
    pub source: String,
    pub location_id: Option<String>,
    pub location_name: Option<String>,
    pub summary: String,
    /// The address the caller was seen at, when one placed them.
    pub client_address: Option<String>,
    /// For a declared location, how long ago the user declared it.
    pub declared_age_secs: Option<u64>,
}

impl From<DispatchableLocation> for LocationResponse {
    fn from(l: DispatchableLocation) -> Self {
        Self {
            summary: l.summary(),
            address: l.address.clone().into(),
            callback_number: l.callback_number,
            source: l.source.to_string(),
            location_id: l.location_id.map(|id| id.to_string()),
            location_name: l.location_name,
            client_address: l.client_address,
            declared_age_secs: l.declared_age.map(|d| d.as_secs()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UserReadiness {
    pub user_id: String,
    pub username: String,
    pub ready: bool,
    pub location: Option<LocationResponse>,
    /// Whether every device of this user's moves (a softphone, a mobile or
    /// a browser), so where it was provisioned may not be where it is.
    pub nomadic: bool,
    /// Why the user is not (fully) ready; empty when ready.
    pub issues: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ReadinessSummary {
    pub users: usize,
    pub ready: usize,
    pub missing_location: usize,
    pub missing_callback: usize,
    pub unvalidated_address: usize,
    /// Users whose devices all move but whom nothing places where they are.
    pub unplaced_nomadic: usize,
}

#[derive(Debug, Serialize)]
pub struct ReadinessResponse {
    pub summary: ReadinessSummary,
    pub users: Vec<UserReadiness>,
}

pub async fn readiness(
    State(state): State<ApiState>,
    tenant: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let users = storage
        .list_users()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .filter(|u| u.tenant_id == tenant.0 && u.enabled);

    let config = match storage.emergency_store() {
        Some(store) => store
            .get_emergency_config(&tenant.0)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| fcp_core::e911::EmergencyConfig::defaults(tenant.0)),
        None => fcp_core::e911::EmergencyConfig::defaults(tenant.0),
    };
    let mut summary = ReadinessSummary {
        users: 0,
        ready: 0,
        missing_location: 0,
        missing_callback: 0,
        unvalidated_address: 0,
        unplaced_nomadic: 0,
    };
    let mut rows = Vec::new();
    for user in users {
        summary.users += 1;
        let location = resolve_for_user(
            storage,
            &user,
            fcp_core::e911::CallerContext {
                config: Some(&config),
                ..Default::default()
            },
        )
        .await;
        let mut issues = Vec::new();
        match &location {
            None => {
                summary.missing_location += 1;
                issues.push("no dispatchable location: assign a site with a full address, set a device E911 override, or register an E911 address on the user's number".to_string());
            }
            Some(l) => {
                if l.callback_number.is_none() {
                    summary.missing_callback += 1;
                    issues.push("no callback number: set e911_callback_number on the site or e911_callback on the number".to_string());
                }
                if !l.address.validated {
                    summary.unvalidated_address += 1;
                    issues.push("address has not been validated".to_string());
                }
            }
        }
        // A device that moves, placed only by where it was provisioned, is
        // a location nobody has checked since the device last moved (§6.4).
        let devices =
            fcp_core::storage::DeviceStore::list_devices_by_user(storage.as_ref(), &user.id)
                .await
                .unwrap_or_default();
        let nomadic = !devices.is_empty()
            && devices
                .iter()
                .all(|d| fcp_core::e911::is_nomadic(d.device_type));
        if nomadic
            && location.as_ref().is_some_and(|l| {
                matches!(
                    l.source,
                    fcp_core::e911::LocationSource::DeviceLocation
                        | fcp_core::e911::LocationSource::UserLocation
                        | fcp_core::e911::LocationSource::PhoneNumber
                )
            })
        {
            summary.unplaced_nomadic += 1;
            issues.push(
                "this user's devices move, and nothing places them where they are: map the \
                 networks they use (/v1/emergency/networks) or have the user declare a location \
                 (/v1/me/location)"
                    .to_string(),
            );
        }

        let ready = location.is_some() && issues.is_empty();
        if ready {
            summary.ready += 1;
        }
        rows.push(UserReadiness {
            user_id: user.id.to_string(),
            username: user.username,
            ready,
            location: location.map(Into::into),
            nomadic,
            issues,
        });
    }
    rows.sort_by(|a, b| a.ready.cmp(&b.ready).then(a.username.cmp(&b.username)));
    Ok(Json(ReadinessResponse {
        summary,
        users: rows,
    }))
}

// ─── Audit trail ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListCallsQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    100
}

#[derive(Debug, Serialize)]
pub struct EmergencyCallResponse {
    pub id: i64,
    pub tenant_id: String,
    pub call_id: String,
    pub sip_call_id: Option<String>,
    pub caller_uri: String,
    pub caller_username: Option<String>,
    pub dialed_number: String,
    pub kind: String,
    pub callback_number: Option<String>,
    pub location: Option<LocationResponse>,
    pub trunk_name: Option<String>,
    pub outcome: String,
    pub placed_at: u64,
    pub notified: bool,
    /// The final SIP status, once the call has ended.
    pub sip_status: Option<u16>,
    pub answered_at: Option<u64>,
    pub ended_at: Option<u64>,
    /// Seconds from answer to end, for an answered call.
    pub duration_secs: Option<u64>,
    /// How the location travelled: `elin`, `pidf_lo`, `elin+pidf_lo`, or none.
    pub delivery: Option<String>,
    /// Whether the location's street went out in civic elements.
    pub location_structured: Option<bool>,
    /// The `Geolocation-Error` a trunk answered the location with.
    pub geolocation_error: Option<String>,
}

impl From<EmergencyCallRecord> for EmergencyCallResponse {
    fn from(r: EmergencyCallRecord) -> Self {
        Self {
            id: r.id,
            tenant_id: r.tenant_id.to_string(),
            call_id: r.call_id,
            sip_call_id: r.sip_call_id,
            caller_uri: r.caller_uri,
            caller_username: r.caller_username,
            dialed_number: r.dialed_number,
            kind: r.kind.to_string(),
            callback_number: r.callback_number,
            location: r.location.map(Into::into),
            trunk_name: r.trunk_name,
            outcome: r.outcome,
            placed_at: unix_secs(r.placed_at),
            notified: r.notified,
            sip_status: r.sip_status,
            answered_at: r.answered_at.map(unix_secs),
            ended_at: r.ended_at.map(unix_secs),
            duration_secs: r
                .answered_at
                .zip(r.ended_at)
                .and_then(|(a, e)| e.duration_since(a).ok())
                .map(|d| d.as_secs()),
            delivery: r.delivery,
            location_structured: r.location_structured,
            geolocation_error: r.geolocation_error,
        }
    }
}

pub async fn list_calls(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(params): Query<ListCallsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    let calls = store
        .list_emergency_calls(&tenant.0, params.limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let out: Vec<EmergencyCallResponse> = calls.into_iter().map(Into::into).collect();
    Ok(Json(out))
}

// ─── Validation ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ValidateAddressRequest {
    pub address: E911AddressResponse,
}

#[derive(Debug, Serialize)]
pub struct ValidationResponse {
    pub valid: bool,
    pub normalized: E911AddressResponse,
    pub issues: Vec<String>,
    pub validator: String,
}

impl From<AddressValidation> for ValidationResponse {
    fn from(v: AddressValidation) -> Self {
        Self {
            valid: v.valid,
            normalized: v.normalized.into(),
            issues: v.issues,
            validator: v.validator,
        }
    }
}

pub async fn validate_address(
    State(state): State<ApiState>,
    _tenant: TenantContext,
    Json(req): Json<ValidateAddressRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let address: E911Address = req.address.into();
    let result = state
        .e911_validator
        .validate(&address)
        .await
        .map_err(|e| ApiError::Internal(format!("Address validation failed: {e}")))?;
    Ok(Json(ValidationResponse::from(result)))
}

#[derive(Debug, Serialize)]
pub struct NumberValidationResponse {
    pub validation: ValidationResponse,
    pub phone_number_id: String,
    pub e164: String,
    pub validated: bool,
    pub validated_at: Option<u64>,
}

/// Validate a phone number's E911 address and mark it validated on success.
pub async fn validate_phone_number(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {e}")))?;
    let mut number = storage
        .get_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|n| n.tenant_id == tenant.0)
        .ok_or_else(|| ApiError::NotFound(format!("Phone number {id} not found")))?;
    let address = number.e911_address.clone().ok_or_else(|| {
        ApiError::BadRequest("Phone number has no E911 address to validate".to_string())
    })?;

    let result = state
        .e911_validator
        .validate(&address)
        .await
        .map_err(|e| ApiError::Internal(format!("Address validation failed: {e}")))?;
    if result.valid {
        let mut validated = result.normalized.clone();
        validated.validated = true;
        validated.validated_at = Some(SystemTime::now());
        number.e911_address = Some(validated);
        number.updated_at = SystemTime::now();
        storage
            .put_phone_number(number.clone())
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        info!(number = %number.e164, validator = %result.validator, "E911 address validated");
    }
    let stored = number.e911_address.as_ref();
    Ok((
        if result.valid {
            StatusCode::OK
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        },
        Json(NumberValidationResponse {
            validation: result.into(),
            phone_number_id: number.id.to_string(),
            e164: number.e164.clone(),
            validated: stored.map(|a| a.validated).unwrap_or(false),
            validated_at: stored.and_then(|a| a.validated_at).map(unix_secs),
        }),
    ))
}
