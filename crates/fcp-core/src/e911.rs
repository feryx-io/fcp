//! Emergency calling (E911) support.
//!
//! This module holds the pieces that are independent of SIP and storage:
//!
//! - [`EmergencyConfig`]: per-tenant emergency numbers, test numbers, and
//!   notification contacts, with US defaults (`911`, test `933`).
//! - [`resolve_dispatchable_location`]: the precedence rules that turn what
//!   we know about a caller (device override, device site, user site, DID
//!   address) into a single dispatchable location plus a callback number.
//!   The callback number is the ELIN the carrier has on file for that
//!   location; it becomes the outbound caller ID on emergency calls.
//! - [`E911AddressValidator`] and the built-in [`BasicAddressValidator`]:
//!   completeness and format checks; external providers implement the same
//!   trait.
//! - [`EmergencyCallRecord`] and [`EmergencyStore`]: the audit trail every
//!   emergency call leaves behind (RAY BAUM's Act record keeping, Kari's Law
//!   notification).
//!
//! Emergency calls must never be blocked for lack of configuration or
//! balance: callers decide policy from these types, but the defaults here
//! err on the side of placing the call.

use crate::{Device, E911Address, LocationId, PhoneNumber, SiteLocation, TenantId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Numbers treated as live emergency calls when a tenant has no config.
pub const DEFAULT_EMERGENCY_NUMBERS: &[&str] = &["911"];
/// Numbers treated as emergency *test* calls when a tenant has no config.
/// `933` is the North American E911 address-verification test number.
pub const DEFAULT_TEST_NUMBERS: &[&str] = &["933"];
/// Outside-line prefixes accepted in front of an emergency number
/// (`9911`). Kari's Law requires direct dialing to work as well.
pub const DEFAULT_DIAL_PREFIXES: &[&str] = &["9"];

/// Per-tenant emergency calling configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmergencyConfig {
    pub tenant_id: TenantId,
    /// Dialed strings that are live emergency calls (`911`, `112`, …).
    pub emergency_numbers: Vec<String>,
    /// Dialed strings that are emergency test calls (`933`).
    pub test_numbers: Vec<String>,
    /// Outside-line prefixes stripped before matching (`9` → `9911`).
    pub dial_prefixes: Vec<String>,
    /// Contacts to notify when an emergency call is placed (Kari's Law
    /// on-site notification). Free-form: email addresses, SIP URIs, or
    /// phone numbers; delivery is the webhook/event consumer's job.
    pub notify_contacts: Vec<String>,
    /// When true, a caller without a dispatchable location is still
    /// connected, but the record and event flag the gap prominently.
    /// (Blocking emergency calls is never an option.)
    pub require_dispatchable_location: bool,
    pub updated_at: SystemTime,
}

impl EmergencyConfig {
    /// US defaults for a tenant with no stored configuration.
    pub fn defaults(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            emergency_numbers: DEFAULT_EMERGENCY_NUMBERS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            test_numbers: DEFAULT_TEST_NUMBERS.iter().map(|s| s.to_string()).collect(),
            dial_prefixes: DEFAULT_DIAL_PREFIXES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            notify_contacts: Vec::new(),
            require_dispatchable_location: false,
            updated_at: SystemTime::now(),
        }
    }

    /// Classify a dialed destination. Accepts a bare number, a SIP URI, or
    /// anything with a `user@host` shape; only the user part is matched.
    pub fn classify(&self, dialed: &str) -> Option<EmergencyKind> {
        let user = dialed_user_part(dialed);
        if user.is_empty() {
            return None;
        }
        let candidates = std::iter::once(user.to_string()).chain(
            self.dial_prefixes
                .iter()
                .filter_map(|p| user.strip_prefix(p.as_str()).map(str::to_string)),
        );
        for candidate in candidates {
            if self.emergency_numbers.iter().any(|n| n == &candidate) {
                return Some(EmergencyKind::Live);
            }
            if self.test_numbers.iter().any(|n| n == &candidate) {
                return Some(EmergencyKind::Test);
            }
        }
        None
    }
}

/// Live emergency call or an address-verification test call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmergencyKind {
    Live,
    Test,
}

impl std::fmt::Display for EmergencyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmergencyKind::Live => write!(f, "live"),
            EmergencyKind::Test => write!(f, "test"),
        }
    }
}

/// Extract the user part of a dialed destination (`sip:911@host` → `911`).
pub fn dialed_user_part(dialed: &str) -> &str {
    let dialed = dialed.trim();
    let unbracketed = dialed.strip_prefix('<').unwrap_or(dialed);
    let without_scheme = unbracketed
        .strip_prefix("sips:")
        .or_else(|| unbracketed.strip_prefix("sip:"))
        .or_else(|| unbracketed.strip_prefix("tel:"))
        .unwrap_or(unbracketed);
    without_scheme
        .split('@')
        .next()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
}

/// Where a dispatchable location came from, in precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationSource {
    /// `Device.e911_override` set by an admin for one physical phone.
    DeviceOverride,
    /// The site location the device is assigned to.
    DeviceLocation,
    /// The site location the user is assigned to.
    UserLocation,
    /// The E911 address registered on the caller's phone number (DID).
    PhoneNumber,
}

impl std::fmt::Display for LocationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LocationSource::DeviceOverride => "device_override",
            LocationSource::DeviceLocation => "device_location",
            LocationSource::UserLocation => "user_location",
            LocationSource::PhoneNumber => "phone_number",
        };
        write!(f, "{s}")
    }
}

/// The civic address and callback number sent with an emergency call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchableLocation {
    pub address: E911Address,
    /// ELIN / callback DID the PSAP can call back and that maps to this
    /// address in the carrier's ALI database. Used as outbound caller ID.
    pub callback_number: Option<String>,
    pub source: LocationSource,
    pub location_id: Option<LocationId>,
    pub location_name: Option<String>,
}

impl DispatchableLocation {
    /// One-line summary for logs, events, and notifications.
    pub fn summary(&self) -> String {
        let a = &self.address;
        let mut parts = vec![a.address1.clone()];
        if let Some(a2) = a.address2.as_deref().filter(|s| !s.trim().is_empty()) {
            parts.push(a2.to_string());
        }
        parts.push(format!("{}, {} {}", a.city, a.state, a.postal_code));
        let mut s = parts.join(", ");
        if let Some(name) = &self.location_name {
            s = format!("{name}: {s}");
        }
        s
    }
}

/// Everything known about a caller that can yield a dispatchable location.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocationInputs<'a> {
    /// The registered device placing the call, if it could be identified.
    pub device: Option<&'a Device>,
    /// The site the device is assigned to (`Device.location_id`).
    pub device_site: Option<&'a SiteLocation>,
    /// The site the user is assigned to (`User.location_id`).
    pub user_site: Option<&'a SiteLocation>,
    /// The caller's DID, for its registered E911 address and callback.
    pub phone_number: Option<&'a PhoneNumber>,
}

/// Apply the precedence rules: device override → device site → user site →
/// phone-number address. Sites without a complete civic address are skipped.
///
/// The callback number is chosen independently of the address source, in
/// the order site ELIN (device site, then user site) → phone-number E911
/// callback → phone number itself, so a device override still goes out with
/// the site's ELIN.
pub fn resolve_dispatchable_location(inputs: &LocationInputs<'_>) -> Option<DispatchableLocation> {
    let callback_number = inputs
        .device_site
        .and_then(|s| non_empty(s.e911_callback_number.as_deref()))
        .or_else(|| {
            inputs
                .user_site
                .and_then(|s| non_empty(s.e911_callback_number.as_deref()))
        })
        .or_else(|| {
            inputs
                .phone_number
                .and_then(|n| non_empty(n.e911_callback.as_deref()))
        })
        .or_else(|| inputs.phone_number.map(|n| n.e164.clone()));

    if let Some(address) = inputs.device.and_then(|d| d.e911_override.clone()) {
        return Some(DispatchableLocation {
            address,
            callback_number,
            source: LocationSource::DeviceOverride,
            location_id: inputs.device.and_then(|d| d.location_id),
            location_name: inputs.device_site.map(|s| s.name.clone()),
        });
    }
    if let Some((site, address)) = inputs
        .device_site
        .and_then(|s| site_address(s).map(|a| (s, a)))
    {
        return Some(DispatchableLocation {
            address,
            callback_number,
            source: LocationSource::DeviceLocation,
            location_id: Some(site.id),
            location_name: Some(site.name.clone()),
        });
    }
    if let Some((site, address)) = inputs
        .user_site
        .and_then(|s| site_address(s).map(|a| (s, a)))
    {
        return Some(DispatchableLocation {
            address,
            callback_number,
            source: LocationSource::UserLocation,
            location_id: Some(site.id),
            location_name: Some(site.name.clone()),
        });
    }
    if let Some(address) = inputs.phone_number.and_then(|n| n.e911_address.clone()) {
        return Some(DispatchableLocation {
            address,
            callback_number,
            source: LocationSource::PhoneNumber,
            location_id: None,
            location_name: None,
        });
    }
    None
}

/// Resolve a user's dispatchable location from storage: their devices
/// (preferring the one registered as `prefer_device_aor`, then any with an
/// E911 override or a site), the sites those point at, and the DID bound to
/// them. Every lookup is best-effort; a storage error just removes that
/// input.
pub async fn resolve_for_user(
    storage: &crate::DynStorage,
    user: &crate::storage::User,
    prefer_device_aor: Option<&str>,
) -> Option<DispatchableLocation> {
    use crate::storage::{DeviceStore, LineBindingStore, PhoneNumberStore, SiteLocationStore};

    let devices = storage
        .list_devices_by_user(&user.id)
        .await
        .unwrap_or_default();
    let wanted = prefer_device_aor.map(|a| a.trim_start_matches("sip:").to_ascii_lowercase());
    let device = devices
        .iter()
        .find(|d| {
            d.sip_uri.as_deref().is_some_and(|s| {
                wanted.as_deref()
                    == Some(s.trim_start_matches("sip:").to_ascii_lowercase().as_str())
            })
        })
        .or_else(|| {
            devices
                .iter()
                .find(|d| d.e911_override.is_some() || d.location_id.is_some())
        })
        .or_else(|| devices.first());

    let device_site = match device.and_then(|d| d.location_id) {
        Some(id) => storage.get_location(&id).await.ok().flatten(),
        None => None,
    };
    let user_site = match user.location_id {
        Some(id) => storage.get_location(&id).await.ok().flatten(),
        None => None,
    };
    let phone_number = match storage
        .list_line_bindings_by_user(&user.id)
        .await
        .ok()
        .and_then(|b| b.into_iter().next())
    {
        Some(binding) => storage
            .get_phone_number(&binding.phone_number_id)
            .await
            .ok()
            .flatten(),
        None => None,
    };

    resolve_dispatchable_location(&LocationInputs {
        device,
        device_site: device_site.as_ref(),
        user_site: user_site.as_ref(),
        phone_number: phone_number.as_ref(),
    })
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A site's civic address, when it is complete enough to dispatch to.
/// Floor and building become the second address line when none is set.
pub fn site_address(site: &SiteLocation) -> Option<E911Address> {
    let address1 = non_empty(site.address_line1.as_deref())?;
    let city = non_empty(site.city.as_deref())?;
    let state = non_empty(site.state.as_deref())?;
    let postal_code = non_empty(site.postal_code.as_deref())?;
    let address2 = non_empty(site.address_line2.as_deref()).or_else(|| {
        let mut parts = Vec::new();
        if let Some(b) = non_empty(site.building.as_deref()) {
            parts.push(format!("Building {b}"));
        }
        if let Some(f) = non_empty(site.floor.as_deref()) {
            parts.push(format!("Floor {f}"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    });
    Some(E911Address {
        address1,
        address2,
        city,
        state,
        postal_code,
        country: non_empty(site.country.as_deref()).unwrap_or_else(|| "US".to_string()),
        validated: false,
        validated_at: None,
    })
}

// ─── Address validation ───────────────────────────────────────────────────

/// Outcome of validating a civic address.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddressValidation {
    /// True when no issues were found; the address may be marked validated.
    pub valid: bool,
    /// The address with whitespace trimmed and codes upper-cased.
    pub normalized: E911Address,
    /// Human-readable problems, empty when valid.
    pub issues: Vec<String>,
    /// Name of the validator that produced this result.
    pub validator: String,
}

/// Pluggable E911 address validation. The built-in
/// [`BasicAddressValidator`] checks completeness and format only; a
/// provider-backed implementation (Bandwidth, Intrado, SmartyStreets, …)
/// can verify against the MSAG/ALI database.
#[async_trait]
pub trait E911AddressValidator: Send + Sync {
    fn name(&self) -> &str;
    async fn validate(&self, address: &E911Address) -> anyhow::Result<AddressValidation>;
}

/// Completeness and format checks with no external calls.
#[derive(Debug, Default, Clone, Copy)]
pub struct BasicAddressValidator;

impl BasicAddressValidator {
    /// Synchronous form of [`E911AddressValidator::validate`].
    pub fn check(address: &E911Address) -> AddressValidation {
        let mut issues = Vec::new();
        let mut normalized = address.clone();
        normalized.address1 = collapse(&address.address1);
        normalized.address2 = address
            .address2
            .as_deref()
            .map(collapse)
            .filter(|s| !s.is_empty());
        normalized.city = collapse(&address.city);
        normalized.state = collapse(&address.state).to_ascii_uppercase();
        normalized.postal_code = collapse(&address.postal_code);
        normalized.country = {
            let c = collapse(&address.country).to_ascii_uppercase();
            if c.is_empty() {
                "US".to_string()
            } else {
                c
            }
        };

        if normalized.address1.is_empty() {
            issues.push("address1 (street address) is required".to_string());
        } else if !normalized.address1.chars().any(|c| c.is_ascii_digit()) {
            issues.push("address1 should include a house or building number".to_string());
        }
        if normalized.city.is_empty() {
            issues.push("city is required".to_string());
        }
        let us = matches!(normalized.country.as_str(), "US" | "USA" | "CA");
        if normalized.state.is_empty() {
            issues.push("state/province is required".to_string());
        } else if us
            && !(normalized.state.len() == 2
                && normalized.state.chars().all(|c| c.is_ascii_alphabetic()))
        {
            issues.push("state must be a two-letter code (e.g. NY)".to_string());
        }
        if normalized.postal_code.is_empty() {
            issues.push("postal_code is required".to_string());
        } else if normalized.country == "US" {
            let digits: Vec<char> = normalized.postal_code.chars().collect();
            let zip5 = digits.len() == 5 && digits.iter().all(|c| c.is_ascii_digit());
            let zip9 = digits.len() == 10
                && digits[5] == '-'
                && digits
                    .iter()
                    .enumerate()
                    .all(|(i, c)| i == 5 || c.is_ascii_digit());
            if !(zip5 || zip9) {
                issues.push("postal_code must be a 5-digit or ZIP+4 US code".to_string());
            }
        }

        let valid = issues.is_empty();
        AddressValidation {
            valid,
            normalized,
            issues,
            validator: "basic".to_string(),
        }
    }
}

#[async_trait]
impl E911AddressValidator for BasicAddressValidator {
    fn name(&self) -> &str {
        "basic"
    }

    async fn validate(&self, address: &E911Address) -> anyhow::Result<AddressValidation> {
        Ok(Self::check(address))
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ─── Audit trail ──────────────────────────────────────────────────────────

/// One emergency call, recorded when it is placed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmergencyCallRecord {
    /// Database id; 0 before insertion.
    pub id: i64,
    pub tenant_id: TenantId,
    /// The call's id, a bare UUID as on its CDR.
    pub call_id: String,
    pub sip_call_id: Option<String>,
    pub caller_uri: String,
    pub caller_username: Option<String>,
    pub dialed_number: String,
    pub kind: EmergencyKind,
    /// The caller ID the call went out with.
    pub callback_number: Option<String>,
    pub location: Option<DispatchableLocation>,
    /// The trunk the call went out on: the first chosen at placement, the
    /// one that answered once the call completes.
    pub trunk_name: Option<String>,
    /// `placed` until the call ends, then its disposition: `answered`,
    /// `busy`, `no_answer`, `failed`, `cancelled`.
    pub outcome: String,
    pub placed_at: SystemTime,
    /// Set once on-site notification has been delivered.
    pub notified: bool,
    /// The final SIP status of the emergency leg, once the call ends.
    #[serde(default)]
    pub sip_status: Option<u16>,
    #[serde(default)]
    pub answered_at: Option<SystemTime>,
    #[serde(default)]
    pub ended_at: Option<SystemTime>,
    /// How the location travelled with the call: `elin` (the callback
    /// number the carrier maps to an address), or none.
    #[serde(default)]
    pub delivery: Option<String>,
}

/// What became of an emergency call, written onto its record when the call
/// ends (`docs/E911_BLOCK_2.md` §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmergencyCompletion {
    pub trunk_name: Option<String>,
    pub outcome: String,
    pub sip_status: Option<u16>,
    pub answered_at: Option<SystemTime>,
    pub ended_at: SystemTime,
    pub delivery: Option<String>,
}

/// Persistence for emergency configuration and the call audit trail.
#[async_trait]
pub trait EmergencyStore: Send + Sync {
    async fn get_emergency_config(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Option<EmergencyConfig>>;
    async fn put_emergency_config(&self, config: &EmergencyConfig) -> anyhow::Result<()>;
    /// Insert a record and return its id.
    async fn insert_emergency_call(&self, record: &EmergencyCallRecord) -> anyhow::Result<i64>;
    async fn list_emergency_calls(
        &self,
        tenant_id: &TenantId,
        limit: usize,
    ) -> anyhow::Result<Vec<EmergencyCallRecord>>;
    async fn mark_emergency_call_notified(&self, id: i64) -> anyhow::Result<()>;
    /// Complete the record `id` with what became of the call.
    async fn complete_emergency_call(
        &self,
        id: i64,
        completion: &EmergencyCompletion,
    ) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceType, LocationId, PhoneNumberId, UserId};

    fn addr(line1: &str) -> E911Address {
        E911Address {
            address1: line1.to_string(),
            address2: None,
            city: "Albany".to_string(),
            state: "NY".to_string(),
            postal_code: "12207".to_string(),
            country: "US".to_string(),
            validated: false,
            validated_at: None,
        }
    }

    fn site(name: &str, line1: Option<&str>, elin: Option<&str>) -> SiteLocation {
        let mut s = SiteLocation::new(name.to_string(), TenantId::nil());
        s.address_line1 = line1.map(str::to_string);
        s.city = Some("Albany".to_string());
        s.state = Some("NY".to_string());
        s.postal_code = Some("12207".to_string());
        s.floor = Some("3".to_string());
        s.e911_callback_number = elin.map(str::to_string);
        s
    }

    fn number(callback: Option<&str>, e911: Option<E911Address>) -> PhoneNumber {
        let mut n = PhoneNumber::new("+15185550100".to_string(), TenantId::nil()).unwrap();
        n.e911_callback = callback.map(str::to_string);
        n.e911_address = e911;
        n
    }

    #[test]
    fn classify_matches_defaults_and_prefixes_and_uris() {
        let cfg = EmergencyConfig::defaults(TenantId::nil());
        assert_eq!(cfg.classify("911"), Some(EmergencyKind::Live));
        assert_eq!(
            cfg.classify("sip:911@pbx.example.com"),
            Some(EmergencyKind::Live)
        );
        assert_eq!(cfg.classify("9911"), Some(EmergencyKind::Live));
        assert_eq!(cfg.classify("933"), Some(EmergencyKind::Test));
        assert_eq!(
            cfg.classify("<sip:933@pbx.example.com;transport=udp>"),
            Some(EmergencyKind::Test)
        );
        assert_eq!(cfg.classify("9111"), None);
        assert_eq!(cfg.classify("+15185550911"), None);
        assert_eq!(cfg.classify("sip:alice@pbx.example.com"), None);
        assert_eq!(cfg.classify(""), None);
    }

    #[test]
    fn classify_honours_tenant_numbers() {
        let mut cfg = EmergencyConfig::defaults(TenantId::nil());
        cfg.emergency_numbers = vec!["112".to_string(), "999".to_string()];
        cfg.dial_prefixes = vec!["0".to_string()];
        assert_eq!(cfg.classify("112"), Some(EmergencyKind::Live));
        assert_eq!(cfg.classify("0999"), Some(EmergencyKind::Live));
        assert_eq!(cfg.classify("911"), None);
        assert_eq!(cfg.classify("9112"), None);
    }

    #[test]
    fn resolution_precedence_and_callback_selection() {
        let mut device = Device::new("desk".to_string(), DeviceType::DeskPhone, TenantId::nil());
        device.location_id = Some(LocationId::new());
        let device_site = site("HQ", Some("1 State St"), Some("+15185550001"));
        let user_site = site("Branch", Some("2 Main St"), Some("+15185550002"));
        let did = number(Some("+15185550003"), Some(addr("3 Elm St")));

        // Nothing at all.
        assert!(resolve_dispatchable_location(&LocationInputs::default()).is_none());

        // Phone number only.
        let loc = resolve_dispatchable_location(&LocationInputs {
            phone_number: Some(&did),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(loc.source, LocationSource::PhoneNumber);
        assert_eq!(loc.address.address1, "3 Elm St");
        assert_eq!(loc.callback_number.as_deref(), Some("+15185550003"));

        // User site beats the DID and lends its ELIN.
        let loc = resolve_dispatchable_location(&LocationInputs {
            user_site: Some(&user_site),
            phone_number: Some(&did),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(loc.source, LocationSource::UserLocation);
        assert_eq!(loc.address.address2.as_deref(), Some("Floor 3"));
        assert_eq!(loc.callback_number.as_deref(), Some("+15185550002"));

        // Device site beats the user site.
        let loc = resolve_dispatchable_location(&LocationInputs {
            device: Some(&device),
            device_site: Some(&device_site),
            user_site: Some(&user_site),
            phone_number: Some(&did),
        })
        .unwrap();
        assert_eq!(loc.source, LocationSource::DeviceLocation);
        assert_eq!(loc.location_name.as_deref(), Some("HQ"));
        assert_eq!(loc.callback_number.as_deref(), Some("+15185550001"));

        // A device override wins but still uses the site's ELIN.
        device.set_e911_override(addr("Suite 400, 9 Override Ave"));
        let loc = resolve_dispatchable_location(&LocationInputs {
            device: Some(&device),
            device_site: Some(&device_site),
            user_site: Some(&user_site),
            phone_number: Some(&did),
        })
        .unwrap();
        assert_eq!(loc.source, LocationSource::DeviceOverride);
        assert!(loc.address.address1.contains("Override"));
        assert_eq!(loc.callback_number.as_deref(), Some("+15185550001"));

        // An incomplete site is skipped, not dispatched.
        let incomplete = site("Kiosk", None, Some("+15185550009"));
        let loc = resolve_dispatchable_location(&LocationInputs {
            device_site: Some(&incomplete),
            phone_number: Some(&did),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(loc.source, LocationSource::PhoneNumber);
        assert_eq!(
            loc.callback_number.as_deref(),
            Some("+15185550009"),
            "ELIN still applies"
        );

        // Without any callback configured the DID itself is the callback.
        let bare = number(None, Some(addr("3 Elm St")));
        let loc = resolve_dispatchable_location(&LocationInputs {
            phone_number: Some(&bare),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(loc.callback_number.as_deref(), Some("+15185550100"));
        let _ = (UserId::new(), PhoneNumberId::new());
    }

    #[test]
    fn basic_validator_normalises_and_reports_issues() {
        let ok = BasicAddressValidator::check(&E911Address {
            address1: "  1  State   St ".to_string(),
            address2: Some("  ".to_string()),
            city: "Albany".to_string(),
            state: "ny".to_string(),
            postal_code: "12207-1234".to_string(),
            country: "".to_string(),
            validated: false,
            validated_at: None,
        });
        assert!(ok.valid, "{:?}", ok.issues);
        assert_eq!(ok.normalized.address1, "1 State St");
        assert_eq!(ok.normalized.address2, None);
        assert_eq!(ok.normalized.state, "NY");
        assert_eq!(ok.normalized.country, "US");

        let bad = BasicAddressValidator::check(&E911Address {
            address1: "State Street".to_string(),
            address2: None,
            city: "".to_string(),
            state: "New York".to_string(),
            postal_code: "1220".to_string(),
            country: "US".to_string(),
            validated: false,
            validated_at: None,
        });
        assert!(!bad.valid);
        assert_eq!(bad.issues.len(), 4, "{:?}", bad.issues);
    }

    #[test]
    fn summary_is_one_line() {
        let loc = DispatchableLocation {
            address: addr("1 State St"),
            callback_number: None,
            source: LocationSource::UserLocation,
            location_id: None,
            location_name: Some("HQ".to_string()),
        };
        assert_eq!(loc.summary(), "HQ: 1 State St, Albany, NY 12207");
    }
}
