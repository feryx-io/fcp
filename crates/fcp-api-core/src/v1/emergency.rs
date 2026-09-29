//! Emergency calling (E911) administration.
//!
//! - `GET/PUT /emergency/config`: the tenant's emergency numbers, test
//!   numbers, dial prefixes, and on-site notification contacts.
//! - `GET /emergency/readiness`: which users have a dispatchable location
//!   (and callback number) today, and which do not.
//! - `GET /emergency/calls`: the audit trail of emergency calls placed.
//! - `POST /emergency/validate-address`: run the configured validator on
//!   an address without storing anything.
//! - `POST /phone-numbers/:id/e911/validate`, `POST /locations/:id/e911/validate`
//!   and `POST /devices/:id/e911/validate`: validate that entity's E911
//!   address and mark it validated when it passes. Editing an address
//!   clears its validation again.

use crate::auth::TenantContext;
use crate::v1::phone_numbers::E911AddressResponse;
use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::e911::notify::{EmergencyNotification, NotifyContact};
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EmergencyConfigResponse {
    pub tenant_id: String,
    pub emergency_numbers: Vec<String>,
    pub test_numbers: Vec<String>,
    pub dial_prefixes: Vec<String>,
    /// Who is told when an emergency call is placed, typed by channel
    /// (`docs/E911_BLOCK_2.md` §8.1).
    pub contacts: Vec<ContactResponse>,
    /// The same contacts as the plain strings this field has always
    /// carried, for clients written before block 2.
    pub notify_contacts: Vec<String>,
    pub require_dispatchable_location: bool,
    /// Device types whose own location (a PIDF-LO they send with the call)
    /// this tenant trusts.
    pub trust_device_locations: Vec<String>,
    /// How long a user's declared location stands, in seconds.
    pub declared_location_ttl_secs: u64,
    /// False when the tenant has never saved a config (defaults shown).
    pub configured: bool,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: u64,
}

impl EmergencyConfigResponse {
    fn from_config(c: EmergencyConfig, configured: bool) -> Self {
        let notify_contacts = c.contact_targets();
        let contacts = c.contacts.iter().map(ContactResponse::from).collect();
        Self {
            tenant_id: c.tenant_id.to_string(),
            emergency_numbers: c.emergency_numbers,
            test_numbers: c.test_numbers,
            dial_prefixes: c.dial_prefixes,
            notify_contacts,
            contacts,
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

/// One notification contact as the API shows it.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ContactResponse {
    /// `email`, `sms`, `call` or `webhook`.
    pub kind: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What keeps this contact from being delivered to, empty when it can.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<String>,
}

impl From<&NotifyContact> for ContactResponse {
    fn from(c: &NotifyContact) -> Self {
        Self {
            kind: c.kind.to_string(),
            target: c.target.clone(),
            name: c.name.clone(),
            issues: c.problems(),
        }
    }
}

/// A contact on the way in: either the plain string the API took before
/// block 2, read by its shape, or a typed object.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum ContactInput {
    Text(String),
    Typed {
        kind: String,
        target: String,
        #[serde(default)]
        name: Option<String>,
    },
}

impl ContactInput {
    fn into_contact(self, field: &str) -> Result<Option<NotifyContact>, ApiError> {
        let contact = match self {
            ContactInput::Text(text) => {
                crate::validation::reject_crlf(field, &text)?;
                if text.trim().is_empty() {
                    return Ok(None);
                }
                NotifyContact::parse(&text).ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "{field}: '{text}' is not an email address, phone number, extension, \
                         SIP URI or webhook endpoint id; give {{\"kind\": …, \"target\": …}} to say \
                         which it is"
                    ))
                })?
            }
            ContactInput::Typed { kind, target, name } => {
                crate::validation::reject_crlf(field, &target)?;
                let kind = kind
                    .parse::<fcp_core::e911::notify::ContactKind>()
                    .map_err(|e| ApiError::BadRequest(format!("{field}: {e}")))?;
                NotifyContact {
                    kind,
                    target: target.trim().to_string(),
                    name: name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
                }
            }
        };
        let problems = contact.problems();
        if !problems.is_empty() {
            return Err(ApiError::BadRequest(format!(
                "{field}: {}",
                problems.join("; ")
            )));
        }
        Ok(Some(contact))
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateEmergencyConfigRequest {
    pub emergency_numbers: Option<Vec<String>>,
    pub test_numbers: Option<Vec<String>>,
    pub dial_prefixes: Option<Vec<String>>,
    /// Replaces the notification contacts. Strings are read by shape.
    pub contacts: Option<Vec<ContactInput>>,
    /// The field this took before block 2; `contacts` wins when both are
    /// given.
    pub notify_contacts: Option<Vec<ContactInput>>,
    pub require_dispatchable_location: Option<bool>,
    /// Device types trusted to send their own location (`softphone`,
    /// `desk_phone`, `webrtc`, `mobile`, …); replaces the list.
    pub trust_device_locations: Option<Vec<String>>,
    /// How long a declared location stands, in seconds (60 to 30 days).
    pub declared_location_ttl_secs: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/emergency/config",
    tag = "emergency",
    operation_id = "emergency_get_config",
    responses(
        (status = 200, description = "OK", body = EmergencyConfigResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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

#[utoipa::path(
    put,
    path = "/api/v1/emergency/config",
    tag = "emergency",
    operation_id = "emergency_update_config",
    request_body = UpdateEmergencyConfigRequest,
    responses(
        (status = 200, description = "OK", body = EmergencyConfigResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
    let (field, given) = match (req.contacts, req.notify_contacts) {
        (Some(contacts), _) => ("contacts", Some(contacts)),
        (None, Some(contacts)) => ("notify_contacts", Some(contacts)),
        (None, None) => ("contacts", None),
    };
    if let Some(given) = given {
        let mut contacts: Vec<NotifyContact> = Vec::new();
        for input in given {
            if let Some(contact) = input.into_contact(field)? {
                if !contacts.contains(&contact) {
                    contacts.push(contact);
                }
            }
        }
        config.contacts = contacts;
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(as = emergency::LocationResponse)]
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ReadinessSummary {
    pub users: usize,
    pub ready: usize,
    pub missing_location: usize,
    pub missing_callback: usize,
    pub unvalidated_address: usize,
    /// Users whose devices all move but whom nothing places where they are.
    pub unplaced_nomadic: usize,
}

/// One contact and how it has fared (§8.4): a tenant should be able to
/// see that its notifications work before it needs them to.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ContactReadiness {
    pub kind: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What keeps this contact from being delivered to.
    pub issues: Vec<String>,
    /// When anything was last delivered to it.
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_delivery_at: Option<u64>,
    /// When a `933` test call's notice last reached it.
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_test_at: Option<u64>,
}

/// Whether anyone would be told about an emergency call today.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NotificationReadiness {
    pub contacts: Vec<ContactReadiness>,
    /// How many of them can be delivered to.
    pub deliverable: usize,
    /// Empty when the tenant would notify somebody.
    pub issues: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ReadinessResponse {
    pub summary: ReadinessSummary,
    /// On-site notification: Kari's Law asks for it, so the report says
    /// whether it would happen.
    pub notifications: NotificationReadiness,
    pub users: Vec<UserReadiness>,
}

#[utoipa::path(
    get,
    path = "/api/v1/emergency/readiness",
    tag = "emergency",
    operation_id = "emergency_readiness",
    responses(
        (status = 200, description = "OK", body = ReadinessResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
        notifications: notification_readiness(&state, &tenant.0, &config).await,
        users: rows,
    }))
}

/// The contacts an emergency call would reach, with when each last heard
/// anything and when a test last reached it (§8.4).
async fn notification_readiness(
    state: &ApiState,
    tenant_id: &fcp_core::TenantId,
    config: &EmergencyConfig,
) -> NotificationReadiness {
    let store = state.storage.as_ref().and_then(|s| s.emergency_store());
    let delivered = match &store {
        Some(store) => store
            .last_emergency_notifications(tenant_id, false)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let tests = match &store {
        Some(store) => store
            .last_emergency_notifications(tenant_id, true)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let last = |rows: &[EmergencyNotification], contact: &NotifyContact| {
        rows.iter()
            .find(|n| n.contact.kind == contact.kind && n.contact.target == contact.target)
            .and_then(|n| n.delivered_at)
            .map(unix_secs)
    };
    let contacts: Vec<ContactReadiness> = config
        .contacts
        .iter()
        .map(|c| ContactReadiness {
            kind: c.kind.to_string(),
            target: c.target.clone(),
            name: c.name.clone(),
            issues: c.problems(),
            last_delivery_at: last(&delivered, c),
            last_test_at: last(&tests, c),
        })
        .collect();
    let deliverable = contacts.iter().filter(|c| c.issues.is_empty()).count();
    let mut issues = Vec::new();
    if deliverable == 0 {
        issues.push(
            "nobody would be told about an emergency call: add a contact to \
             /v1/emergency/config (Kari's Law requires on-site notification)"
                .to_string(),
        );
    } else if contacts.iter().all(|c| c.last_test_at.is_none()) {
        issues.push(
            "no test call's notice has reached a contact yet: dial 933 to prove the chain"
                .to_string(),
        );
    }
    NotificationReadiness {
        contacts,
        deliverable,
        issues,
    }
}

// ─── Audit trail ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, utoipa::ToSchema)]
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
    /// Which trunk carried it; a name is unique only within its tenant.
    pub trunk_id: Option<String>,
    pub outcome: String,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub placed_at: u64,
    pub notified: bool,
    /// The final SIP status, once the call has ended.
    pub sip_status: Option<u16>,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub answered_at: Option<u64>,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
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
            trunk_id: r.trunk_id.map(|t| t.to_string()),
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

#[utoipa::path(
    get,
    path = "/api/v1/emergency/calls",
    tag = "emergency",
    operation_id = "emergency_list_calls",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<EmergencyCallResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_calls(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    // Emergency calls are rare: the newest thousand are the whole log for
    // a long time.
    let calls = store
        .list_emergency_calls(&tenant.0, crate::page::MAX_LIMIT)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let out: Vec<EmergencyCallResponse> = calls.into_iter().map(Into::into).collect();
    Ok(Json(crate::page::paginate(
        out,
        &page,
        crate::page::Order::Descending,
        |r| crate::page::time_key(r.placed_at, &r.call_id),
    )?))
}

/// One attempt to tell one contact about one emergency call (§8.3).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NotificationResponse {
    pub id: i64,
    pub call_id: String,
    pub call_record_id: Option<i64>,
    pub kind: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub attempt: i32,
    /// `delivered`, `retrying` or `failed`.
    pub status: String,
    pub error: Option<String>,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub delivered_at: Option<u64>,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub next_retry_at: Option<u64>,
    /// Whether this told someone about a test call.
    pub test: bool,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: u64,
}

impl From<EmergencyNotification> for NotificationResponse {
    fn from(n: EmergencyNotification) -> Self {
        Self {
            id: n.id,
            call_id: n.call_id.clone(),
            call_record_id: n.call_record_id,
            kind: n.contact.kind.to_string(),
            target: n.contact.target.clone(),
            name: n.contact.name.clone(),
            attempt: n.attempt,
            status: n.status.to_string(),
            error: n.error.clone(),
            delivered_at: n.delivered_at.map(unix_secs),
            next_retry_at: n.next_retry_at.map(unix_secs),
            test: n.is_test(),
            created_at: unix_secs(n.created_at),
        }
    }
}

/// `GET /emergency/calls/:call_id/notifications` — every attempt made to
/// tell someone about one emergency call, oldest first.
#[utoipa::path(
    get,
    path = "/api/v1/emergency/calls/{call_id}/notifications",
    tag = "emergency",
    operation_id = "emergency_list_call_notifications",
    params(("call_id" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<NotificationResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_call_notifications(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(call_id): Path<String>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    // Records keep the bare uuid; a client may hold either form.
    let mut rows = store
        .list_emergency_notifications(&tenant.0, call_id.trim())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let bare = call_id.trim().trim_start_matches("call-");
    if rows.is_empty() && bare != call_id.trim() {
        rows = store
            .list_emergency_notifications(&tenant.0, bare)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    let out: Vec<NotificationResponse> = rows.into_iter().map(Into::into).collect();
    Ok(Json(crate::page::paginate(
        out,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{:020}", r.id),
    )?))
}

// ─── The call channel ────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct NotifyCallRequest {
    /// The extension or SIP URI to ring.
    pub to: String,
    /// What to say once it is answered.
    pub text: String,
    /// How long to ring.
    #[serde(default = "default_ring_timeout_secs")]
    pub ring_timeout_secs: u64,
    /// How often the notice is spoken.
    #[serde(default = "default_notice_repeat")]
    pub repeat: u32,
    /// The call this is about, for the log.
    #[serde(default)]
    pub call_id: Option<String>,
    /// Whether it is about a test call.
    #[serde(default)]
    pub test: bool,
}

fn default_ring_timeout_secs() -> u64 {
    30
}

fn default_notice_repeat() -> u32 {
    2
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NotifyCallResponse {
    /// `answered`, `no_answer`, `busy` or `failed_<status>`; only an
    /// answered call told anybody anything.
    pub outcome: String,
    pub to: String,
}

/// `POST /emergency/notify-call` — ring someone and speak the notice to
/// them: the call channel of on-site notification, which the events
/// server asks this node for (`docs/E911_BLOCK_2.md` §8.1).
#[utoipa::path(
    post,
    path = "/api/v1/emergency/notify-call",
    tag = "emergency",
    operation_id = "emergency_notify_call",
    request_body = NotifyCallRequest,
    responses(
        (status = 200, description = "OK", body = NotifyCallResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn notify_call(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<NotifyCallRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.to.trim().is_empty() {
        return Err(ApiError::BadRequest("to is required".to_string()));
    }
    if req.text.trim().is_empty() {
        return Err(ApiError::BadRequest("text is required".to_string()));
    }
    let outcome = state
        .handler
        .announce_call(
            req.to.trim(),
            req.text.trim(),
            std::time::Duration::from_secs(req.ring_timeout_secs.clamp(5, 120)),
            req.repeat.clamp(1, 5),
        )
        .await
        .map_err(|e| ApiError::Internal(format!("Notification call failed: {e}")))?;
    info!(
        tenant_id = %tenant.0,
        to = %req.to,
        call_id = %req.call_id.as_deref().unwrap_or("-"),
        test = req.test,
        outcome = %outcome,
        "Emergency notification call"
    );
    Ok(Json(NotifyCallResponse {
        outcome: outcome.to_string(),
        to: req.to,
    }))
}

// ─── Validation ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ValidateAddressRequest {
    pub address: E911AddressResponse,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ValidationResponse {
    pub valid: bool,
    pub normalized: E911AddressResponse,
    pub issues: Vec<String>,
    /// What the validator could not do on an otherwise good address: the
    /// parts of the street it could not tell apart, say.
    pub notes: Vec<String>,
    pub validator: String,
    /// The provider's reference for this answer, when it gave one.
    pub provider_reference: Option<String>,
}

impl From<AddressValidation> for ValidationResponse {
    fn from(v: AddressValidation) -> Self {
        Self {
            valid: v.valid,
            normalized: v.normalized.into(),
            issues: v.issues,
            notes: v.notes,
            validator: v.validator,
            provider_reference: v.provider_reference,
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/emergency/validate-address",
    tag = "emergency",
    operation_id = "emergency_validate_address",
    request_body = ValidateAddressRequest,
    responses(
        (status = 200, description = "OK", body = ValidationResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NumberValidationResponse {
    pub validation: ValidationResponse,
    pub phone_number_id: String,
    pub e164: String,
    pub validated: bool,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub validated_at: Option<u64>,
}

/// Validate a phone number's E911 address and mark it validated on success.
#[utoipa::path(
    post,
    path = "/api/v1/phone-numbers/{id}/e911/validate",
    tag = "emergency",
    operation_id = "emergency_validate_phone_number",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "Valid", body = NumberValidationResponse),
        (status = 422, description = "Not valid: what the validator found", body = NumberValidationResponse),
        (status = 422, description = "Not valid: what the validator found", body = NumberValidationResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
        number.e911_address = Some(result.validated_address(SystemTime::now()));
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LocationValidationResponse {
    pub validation: ValidationResponse,
    pub location_id: String,
    pub name: String,
    pub validated: bool,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub validated_at: Option<u64>,
}

/// Validate a site's address and mark the site validated on success. The
/// normalised street, city, state and civic elements are written back, so
/// the emergency calls placed from this site carry them
/// (`docs/E911_BLOCK_2.md` §7).
#[utoipa::path(
    post,
    path = "/api/v1/locations/{id}/e911/validate",
    tag = "emergency",
    operation_id = "emergency_validate_location",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "Valid", body = LocationValidationResponse),
        (status = 422, description = "Not valid: what the validator found", body = LocationValidationResponse),
        (status = 422, description = "Not valid: what the validator found", body = LocationValidationResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn validate_location(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    use fcp_core::storage::SiteLocationStore;

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let id = fcp_core::LocationId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {e}")))?;
    let mut site = storage
        .get_location(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|s| s.tenant_id == tenant.0)
        .ok_or_else(|| ApiError::NotFound(format!("Location {id} not found")))?;
    let address = fcp_core::e911::site_address(&site).ok_or_else(|| {
        ApiError::BadRequest(
            "Location has no dispatchable address: set address_line1, city, state and postal_code"
                .to_string(),
        )
    })?;

    let result = state
        .e911_validator
        .validate(&address)
        .await
        .map_err(|e| ApiError::Internal(format!("Address validation failed: {e}")))?;
    if result.valid {
        let now = SystemTime::now();
        let normalized = result.validated_address(now);
        site.address_line1 = Some(normalized.address1.clone());
        site.city = Some(normalized.city.clone());
        site.state = Some(normalized.state.clone());
        site.postal_code = Some(normalized.postal_code.clone());
        site.country = Some(normalized.country.clone());
        // The floor and building are the site's own fields; keeping a copy
        // in the elements would go stale the next time they change.
        let mut civic = normalized.civic.clone();
        civic.floor = None;
        civic.building = None;
        site.civic = civic;
        site.e911_validated_at = Some(now);
        site.e911_validated_by = Some(result.validator.clone());
        site.e911_validation_reference = result.provider_reference.clone();
        site.updated_at = now;
        storage
            .put_location(site.clone())
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let _ = state
            .event_bus
            .publish(fcp_events::PlatformEvent::System(
                fcp_events::SystemEvent::ConfigChanged {
                    entity_type: "location".to_string(),
                    entity_id: site.id.to_string(),
                    tenant_id: site.tenant_id,
                    timestamp: now,
                },
            ))
            .await;
        info!(location = %site.id, validator = %result.validator, "Site E911 address validated");
    }
    Ok((
        if result.valid {
            StatusCode::OK
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        },
        Json(LocationValidationResponse {
            validation: result.into(),
            location_id: site.id.to_string(),
            name: site.name.clone(),
            validated: site.e911_validated(),
            validated_at: site.e911_validated_at.map(unix_secs),
        }),
    ))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeviceValidationResponse {
    pub validation: ValidationResponse,
    pub device_id: String,
    pub name: String,
    pub validated: bool,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub validated_at: Option<u64>,
}

/// Validate a device's E911 override address and mark it validated on
/// success (`docs/E911_BLOCK_2.md` §7).
#[utoipa::path(
    post,
    path = "/api/v1/devices/{id}/e911/validate",
    tag = "emergency",
    operation_id = "emergency_validate_device",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "Valid", body = DeviceValidationResponse),
        (status = 422, description = "Not valid: what the validator found", body = DeviceValidationResponse),
        (status = 422, description = "Not valid: what the validator found", body = DeviceValidationResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn validate_device(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    use fcp_core::storage::DeviceStore;

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let id = fcp_core::DeviceId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {e}")))?;
    let mut device = storage
        .get_device(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|d| d.tenant_id == tenant.0)
        .ok_or_else(|| ApiError::NotFound(format!("Device {id} not found")))?;
    let address = device.e911_override.clone().ok_or_else(|| {
        ApiError::BadRequest(
            "Device has no E911 override address to validate; it is located by its site, the \
             network it registers from, or a location it sends"
                .to_string(),
        )
    })?;

    let result = state
        .e911_validator
        .validate(&address)
        .await
        .map_err(|e| ApiError::Internal(format!("Address validation failed: {e}")))?;
    if result.valid {
        let now = SystemTime::now();
        device.e911_override = Some(result.validated_address(now));
        device.updated_at = now;
        storage
            .put_device(device.clone())
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let _ = state
            .event_bus
            .publish(fcp_events::PlatformEvent::System(
                fcp_events::SystemEvent::ConfigChanged {
                    entity_type: "device".to_string(),
                    entity_id: device.id.to_string(),
                    tenant_id: device.tenant_id,
                    timestamp: now,
                },
            ))
            .await;
        info!(device = %device.id, validator = %result.validator, "Device E911 address validated");
    }
    let stored = device.e911_override.as_ref();
    Ok((
        if result.valid {
            StatusCode::OK
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        },
        Json(DeviceValidationResponse {
            validation: result.into(),
            device_id: device.id.to_string(),
            name: device.name.clone(),
            validated: stored.is_some_and(|a| a.validated),
            validated_at: stored.and_then(|a| a.validated_at).map(unix_secs),
        }),
    ))
}
