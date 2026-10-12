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

pub mod notify;

use crate::{Device, E911Address, LocationId, PhoneNumber, SiteLocation, TenantId, TrunkId};
use async_trait::async_trait;
use notify::{EmergencyNotification, NotifyContact};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

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
    /// Who is told when an emergency call is placed (Kari's Law on-site
    /// notification, `docs/E911_BLOCK_2.md` §8): typed contacts, each an
    /// email address, an SMS number, an extension to ring or a webhook
    /// endpoint. Contacts saved as free text before block 2 are read by
    /// their shape (`NotifyContact::parse`).
    #[serde(default)]
    pub contacts: Vec<NotifyContact>,
    /// When true, a caller without a dispatchable location is still
    /// connected, but the record and event flag the gap prominently.
    /// (Blocking emergency calls is never an option.)
    pub require_dispatchable_location: bool,
    /// Device types whose own location (a PIDF-LO in the INVITE) this
    /// tenant trusts (`docs/E911_BLOCK_2.md` §6.3, §12.3). Empty trusts
    /// none; a device may still be trusted one by one.
    #[serde(default)]
    pub trust_device_locations: Vec<crate::DeviceType>,
    /// How long a user's declared location stands (§6.4): a day by default.
    #[serde(default = "default_declared_ttl")]
    pub declared_location_ttl: Duration,
    pub updated_at: SystemTime,
}

/// A day: how long a declared location stands unless the tenant says
/// otherwise.
pub const DEFAULT_DECLARED_LOCATION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

fn default_declared_ttl() -> Duration {
    DEFAULT_DECLARED_LOCATION_TTL
}

impl EmergencyConfig {
    /// The contacts as the plain strings `call.emergency` has always
    /// carried, for consumers written before block 2.
    pub fn contact_targets(&self) -> Vec<String> {
        self.contacts.iter().map(|c| c.display_target()).collect()
    }

    /// The contacts a notifier can actually deliver to; a tenant with none
    /// is a readiness issue (§8.3).
    pub fn deliverable_contacts(&self) -> Vec<&NotifyContact> {
        self.contacts
            .iter()
            .filter(|c| c.problems().is_empty())
            .collect()
    }

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
            contacts: Vec::new(),
            require_dispatchable_location: false,
            trust_device_locations: Vec::new(),
            declared_location_ttl: DEFAULT_DECLARED_LOCATION_TTL,
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
    /// A location the device sent with the call, from a device the tenant
    /// trusts to send one (`docs/E911_BLOCK_2.md` §6.3).
    DeviceSupplied,
    /// The site the client address falls in, by the tenant's networks.
    Network,
    /// The site location the device is assigned to.
    DeviceLocation,
    /// The site location the user is assigned to.
    UserLocation,
    /// Where the user last said they were, while that has not expired.
    Declared,
    /// The E911 address registered on the caller's phone number (DID).
    PhoneNumber,
    /// The location a site resolved for its own caller and sent up its
    /// link with the call (`docs/HYBRID_CONTROL_PLANE.md` §3.6): the cloud
    /// has no user or device of the site's to place the call by.
    SiteSupplied,
}

impl std::fmt::Display for LocationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LocationSource::DeviceOverride => "device_override",
            LocationSource::DeviceSupplied => "device_supplied",
            LocationSource::Network => "network",
            LocationSource::DeviceLocation => "device_location",
            LocationSource::UserLocation => "user_location",
            LocationSource::Declared => "declared",
            LocationSource::PhoneNumber => "phone_number",
            LocationSource::SiteSupplied => "site_supplied",
        };
        write!(f, "{s}")
    }
}

impl LocationSource {
    /// The PIDF-LO `method` (RFC 4119, IANA method tokens) for a location
    /// from this source.
    pub fn pidf_method(&self) -> &'static str {
        match self {
            // The network the device is on placed it.
            LocationSource::Network => "Wiremap",
            // The user said where they were.
            LocationSource::Declared => "Manual-Entry",
            // A device's own claim travels with its own method; this is the
            // fallback when it named none.
            LocationSource::DeviceSupplied => "Unknown",
            _ => "Manual",
        }
    }

    /// Whether a location from this source is only as good as its age: a
    /// user's declaration, which the record dates.
    pub fn is_declared(&self) -> bool {
        matches!(self, LocationSource::Declared)
    }
}

/// Which devices a network mapping places (`docs/E911_BLOCK_2.md` §6.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkApplies {
    /// Devices that move: softphones, mobile and browser clients. A desk
    /// phone keeps the site it was provisioned at.
    #[default]
    Nomadic,
    /// Every device on the network, desk phones included.
    All,
}

impl std::fmt::Display for NetworkApplies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NetworkApplies::Nomadic => "nomadic",
            NetworkApplies::All => "all",
        })
    }
}

impl std::str::FromStr for NetworkApplies {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "nomadic" => Ok(NetworkApplies::Nomadic),
            "all" => Ok(NetworkApplies::All),
            other => Err(format!("unknown applies_to {other:?}")),
        }
    }
}

/// Whether a device of this type moves, and so is placed by the network it
/// is on rather than by where it was provisioned.
pub fn is_nomadic(device_type: crate::DeviceType) -> bool {
    use crate::DeviceType::*;
    matches!(device_type, Softphone | WebRtc | Mobile)
}

/// A tenant's network: the site an address on it is at
/// (`docs/E911_BLOCK_2.md` §6.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmergencyNetwork {
    pub id: crate::EmergencyNetworkId,
    pub tenant_id: TenantId,
    /// The range, v4 or v6.
    pub cidr: ipnetwork::IpNetwork,
    /// The site an address on this network is at.
    pub location_id: LocationId,
    /// What to call it in the admin UI (`Albany floor 3 wired`).
    pub name: Option<String>,
    /// Breaks ties between networks of the same prefix length; lower first.
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub applies_to: NetworkApplies,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl EmergencyNetwork {
    pub fn new(tenant_id: TenantId, cidr: ipnetwork::IpNetwork, location_id: LocationId) -> Self {
        let now = SystemTime::now();
        Self {
            id: crate::EmergencyNetworkId::new(),
            tenant_id,
            cidr,
            location_id,
            name: None,
            priority: 0,
            applies_to: NetworkApplies::Nomadic,
            created_at: now,
            updated_at: now,
        }
    }

    /// Whether this network places a device of `device_type` (unknown when
    /// `None`, which counts as nomadic: a device the platform cannot name
    /// is not a desk phone it can vouch for).
    pub fn places(&self, device_type: Option<crate::DeviceType>) -> bool {
        match self.applies_to {
            NetworkApplies::All => true,
            NetworkApplies::Nomadic => device_type.map(is_nomadic).unwrap_or(true),
        }
    }
}

/// The network an address is on: the most specific match, then the lowest
/// priority, among those that place this kind of device.
pub fn match_network(
    networks: &[EmergencyNetwork],
    address: std::net::IpAddr,
    device_type: Option<crate::DeviceType>,
) -> Option<&EmergencyNetwork> {
    // An IPv4 address seen through a dual-stack socket arrives mapped.
    let address = match address {
        std::net::IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(std::net::IpAddr::V4)
            .unwrap_or(address),
        v4 => v4,
    };
    networks
        .iter()
        .filter(|n| n.cidr.contains(address) && n.places(device_type))
        .max_by_key(|n| (n.cidr.prefix(), std::cmp::Reverse(n.priority)))
}

/// Where a user last said they were (`docs/E911_BLOCK_2.md` §6.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeclaredLocation {
    pub user_id: crate::UserId,
    pub tenant_id: TenantId,
    pub address: E911Address,
    pub declared_at: SystemTime,
    /// When it stops being used; a day after it was declared by default.
    pub expires_at: SystemTime,
}

impl DeclaredLocation {
    pub fn is_current(&self, now: SystemTime) -> bool {
        self.expires_at > now
    }

    /// How long ago it was declared.
    pub fn age(&self, now: SystemTime) -> std::time::Duration {
        now.duration_since(self.declared_at).unwrap_or_default()
    }
}

/// How an emergency call's location travels to a trunk
/// (`[[trunks]] emergency_location`, `docs/E911_BLOCK_2.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationDelivery {
    /// The callback number as caller ID; the carrier maps it to an address.
    Elin,
    /// The location by value: a PIDF-LO body referenced by `Geolocation`.
    PidfLo,
}

impl std::fmt::Display for LocationDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LocationDelivery::Elin => "elin",
            LocationDelivery::PidfLo => "pidf_lo",
        })
    }
}

/// A WGS 84 point (EPSG:4326), in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
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
    /// Where the site is on the map, when its coordinates are set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<GeoPoint>,
    /// The address the caller was seen at, when one placed them
    /// (`docs/E911_BLOCK_2.md` §6.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_address: Option<String>,
    /// For a location the user declared, how long ago they declared it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_age: Option<Duration>,
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
    /// A location the device sent with the call, already checked against
    /// what this tenant trusts the device to send (§6.3).
    pub supplied: Option<&'a E911Address>,
    /// The network the caller's address is on, with its site (§6.1).
    pub network: Option<(&'a EmergencyNetwork, &'a SiteLocation)>,
    /// Where the caller was seen, for the record (§6.2).
    pub client_address: Option<std::net::IpAddr>,
    /// Where the user last said they were, if that has not expired (§6.4).
    pub declared: Option<&'a DeclaredLocation>,
}

/// Apply the precedence rules: device override → device site → user site →
/// phone-number address. Sites without a complete civic address are skipped.
///
/// The callback number is chosen independently of the address source, in
/// the order site ELIN (device site, then user site) → phone-number E911
/// callback → phone number itself, so a device override still goes out with
/// the site's ELIN.
pub fn resolve_dispatchable_location(inputs: &LocationInputs<'_>) -> Option<DispatchableLocation> {
    let client_address = inputs.client_address.map(|a| a.to_string());
    // The site the address came from, when a site gave it: its ELIN is the
    // callback number the carrier maps to that address.
    let (address, source, site, declared_age) =
        if let Some(address) = inputs.device.and_then(|d| d.e911_override.clone()) {
            (
                address,
                LocationSource::DeviceOverride,
                inputs.device_site,
                None,
            )
        } else if let Some(supplied) = inputs.supplied {
            (supplied.clone(), LocationSource::DeviceSupplied, None, None)
        } else if let Some((_, site)) = inputs
            .network
            .filter(|(_, site)| site_address(site).is_some())
        {
            (
                site_address(site).expect("checked"),
                LocationSource::Network,
                Some(site),
                None,
            )
        } else if let Some((site, address)) = inputs
            .device_site
            .and_then(|s| site_address(s).map(|a| (s, a)))
        {
            (address, LocationSource::DeviceLocation, Some(site), None)
        } else if let Some((site, address)) = inputs
            .user_site
            .and_then(|s| site_address(s).map(|a| (s, a)))
        {
            (address, LocationSource::UserLocation, Some(site), None)
        } else if let Some(declared) = inputs.declared.filter(|d| d.is_current(SystemTime::now())) {
            (
                declared.address.clone(),
                LocationSource::Declared,
                None,
                Some(declared.age(SystemTime::now())),
            )
        } else {
            // The address registered on the caller's number, or nothing
            // this platform can dispatch to.
            let address = inputs.phone_number.and_then(|n| n.e911_address.clone())?;
            (address, LocationSource::PhoneNumber, None, None)
        };

    // The callback number: the site the address came from first, then the
    // sites the caller is provisioned at, then the number's own.
    let callback_number = site
        .and_then(|s| non_empty(s.e911_callback_number.as_deref()))
        .or_else(|| {
            inputs
                .device_site
                .and_then(|s| non_empty(s.e911_callback_number.as_deref()))
        })
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

    Some(DispatchableLocation {
        address,
        callback_number,
        source,
        location_id: site.map(|s| s.id).or_else(|| {
            (source == LocationSource::DeviceOverride)
                .then(|| inputs.device.and_then(|d| d.location_id))
                .flatten()
        }),
        location_name: site.map(|s| s.name.clone()).or_else(|| {
            (source == LocationSource::DeviceOverride)
                .then(|| inputs.device_site.map(|s| s.name.clone()))
                .flatten()
        }),
        point: site.and_then(site_point),
        client_address,
        declared_age,
    })
}

/// What is known about the caller when their location is resolved.
#[derive(Debug, Default, Clone, Copy)]
pub struct CallerContext<'a> {
    /// The `user@host` of the device placing the call, which picks the
    /// caller's device among their devices.
    pub prefer_device_aor: Option<&'a str>,
    /// Where the caller was seen: a trusted edge's client address, the
    /// INVITE's own source, or the device's last registration (§6.2).
    pub client_address: Option<std::net::IpAddr>,
    /// A location the device sent with the call, not yet trusted (§6.3).
    pub supplied: Option<&'a E911Address>,
    /// The tenant's configuration, for which devices may supply a location.
    pub config: Option<&'a EmergencyConfig>,
}

/// Resolve a user's dispatchable location from storage: their devices
/// (preferring the one registered as `prefer_device_aor`, then any with an
/// E911 override or a site), the sites those point at, the tenant's
/// networks for the client address, the user's declared location and the
/// DID bound to them. Every lookup is best-effort; a storage error just
/// removes that input.
pub async fn resolve_for_user(
    storage: &crate::DynStorage,
    user: &crate::storage::User,
    caller: CallerContext<'_>,
) -> Option<DispatchableLocation> {
    use crate::storage::{DeviceStore, LineBindingStore, PhoneNumberStore, SiteLocationStore};

    let devices = storage
        .list_devices_by_user(&user.id)
        .await
        .unwrap_or_default();
    let wanted = caller
        .prefer_device_aor
        .map(|a| a.trim_start_matches("sip:").to_ascii_lowercase());
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

    // A location the device sent is used only from a device this tenant
    // trusts to send one (§6.3, §12.3).
    let supplied = caller.supplied.filter(|_| {
        device.is_some_and(|d| {
            d.e911_trust_location.unwrap_or_else(|| {
                caller
                    .config
                    .is_some_and(|c| c.trust_device_locations.contains(&d.device_type))
            })
        })
    });

    // Where the caller was seen: what the call itself said, else where
    // their device last registered from (§6.2).
    let client_address = match caller.client_address {
        Some(address) => Some(address),
        None => last_registration_address(storage, &user.tenant_id, device).await,
    };

    // The network the caller's address is on, and the site it names.
    let matched = match (client_address, storage.emergency_store()) {
        (Some(address), Some(store)) => {
            let networks = store
                .list_emergency_networks(&user.tenant_id)
                .await
                .unwrap_or_default();
            match match_network(&networks, address, device.map(|d| d.device_type)) {
                Some(network) => storage
                    .get_location(&network.location_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|site| (network.clone(), site)),
                None => None,
            }
        }
        _ => None,
    };

    // Where the user last said they were, while that stands.
    let declared = match storage.emergency_store() {
        Some(store) => store
            .get_declared_location(&user.tenant_id, &user.id)
            .await
            .ok()
            .flatten()
            .filter(|d| d.is_current(SystemTime::now())),
        None => None,
    };

    resolve_dispatchable_location(&LocationInputs {
        device,
        device_site: device_site.as_ref(),
        user_site: user_site.as_ref(),
        phone_number: phone_number.as_ref(),
        supplied,
        network: matched.as_ref().map(|(n, s)| (n, s)),
        client_address,
        declared: declared.as_ref(),
    })
}

/// The address a device last registered from, when it has registered.
async fn last_registration_address(
    storage: &crate::DynStorage,
    tenant_id: &TenantId,
    device: Option<&Device>,
) -> Option<std::net::IpAddr> {
    use crate::storage::RegistrationStore;

    let aor = device?.sip_uri.as_deref()?;
    let aor = format!("sip:{}", aor.trim_start_matches("sip:"));
    let registrations = storage
        .get_registrations_in_tenant(tenant_id, &aor)
        .await
        .ok()?;
    registrations
        .iter()
        .filter_map(|r| r.source_address.as_deref())
        .filter_map(parse_address)
        .next()
}

/// An address, an `address:port`, or a bracketed IPv6 form.
pub fn parse_address(value: &str) -> Option<std::net::IpAddr> {
    let value = value.trim();
    if let Ok(address) = value.parse::<std::net::IpAddr>() {
        return Some(address);
    }
    if let Ok(socket) = value.parse::<std::net::SocketAddr>() {
        return Some(socket.ip());
    }
    let bracketed = value.strip_prefix('[')?;
    let (inside, _) = bracketed.split_once(']')?;
    inside.parse().ok()
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
    let mut civic = site.civic.clone();
    if civic.floor.is_none() {
        civic.floor = non_empty(site.floor.as_deref());
    }
    if civic.building.is_none() {
        civic.building = non_empty(site.building.as_deref());
    }
    Some(E911Address {
        address1,
        address2,
        city,
        state,
        postal_code,
        country: non_empty(site.country.as_deref()).unwrap_or_else(|| "US".to_string()),
        validated: site.e911_validated(),
        validated_at: site.e911_validated_at,
        validated_by: site.e911_validated_by.clone(),
        provider_reference: site.e911_validation_reference.clone(),
        civic,
    })
}

/// A site's coordinates, when both are set and on the globe.
pub fn site_point(site: &SiteLocation) -> Option<GeoPoint> {
    let (latitude, longitude) = (site.latitude?, site.longitude?);
    ((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude)).then_some(
        GeoPoint {
            latitude,
            longitude,
        },
    )
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
    /// What the validator could not do, on an address that is otherwise
    /// fine: the parts of the street it could not tell apart, say. Notes
    /// do not make an address invalid (`docs/E911_BLOCK_2.md` §7).
    #[serde(default)]
    pub notes: Vec<String>,
    /// Name of the validator that produced this result.
    pub validator: String,
    /// The provider's own reference for this answer, when it gave one.
    #[serde(default)]
    pub provider_reference: Option<String>,
}

impl AddressValidation {
    /// The address as it should be stored once a validator accepted it:
    /// the normalised form, marked with who validated it and when.
    pub fn validated_address(&self, now: SystemTime) -> E911Address {
        let mut address = self.normalized.clone();
        address.validated = self.valid;
        address.validated_at = self.valid.then_some(now);
        address.validated_by = self.valid.then(|| self.validator.clone());
        address.provider_reference = self
            .valid
            .then(|| self.provider_reference.clone())
            .flatten();
        address
    }
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

        // Split the street into its elements where the shape is clear, so
        // the call's PIDF-LO carries HNO/RD/STS rather than a street line
        // (§7). What could not be told apart is said, not held against the
        // address.
        let notes = structure_address(&mut normalized);

        let valid = issues.is_empty();
        AddressValidation {
            valid,
            normalized,
            issues,
            notes,
            validator: "basic".to_string(),
            provider_reference: None,
        }
    }
}

/// Fill an address's civic elements (RFC 5139) from its street lines,
/// leaving every element the caller already set alone. Returns what could
/// not be told apart, as notes for whoever typed the address.
pub fn structure_address(address: &mut E911Address) -> Vec<String> {
    let mut notes = Vec::new();
    let street = collapse(&address.address1);
    let parsed = parse_street(&street);
    let c = &mut address.civic;
    fill(&mut c.house_number, parsed.house_number);
    fill(&mut c.house_number_suffix, parsed.house_number_suffix);
    fill(&mut c.street_predirection, parsed.predirection);
    fill(&mut c.street_name, parsed.street_name);
    fill(&mut c.street_suffix, parsed.suffix);
    fill(&mut c.street_postdirection, parsed.postdirection);

    if let Some(line2) = address.address2.as_deref().map(collapse) {
        if !line2.is_empty() {
            let (kind, value) = parse_second_line(&line2);
            let slot = match kind {
                SecondLine::Unit => &mut c.unit,
                SecondLine::Floor => &mut c.floor,
                SecondLine::Room => &mut c.room,
                SecondLine::Building => &mut c.building,
                SecondLine::Other => &mut c.additional,
            };
            fill(slot, Some(value));
        }
    }

    if !street.is_empty() {
        if c.house_number.as_deref().is_none_or(str::is_empty) {
            notes.push(format!(
                "no house number could be read from \"{street}\": set house_number to say which \
                 building this is"
            ));
        } else if c.street_suffix.as_deref().is_none_or(str::is_empty) {
            notes.push(format!(
                "no street suffix (St, Ave, Rd, …) could be read from \"{street}\": the street \
                 name is \"{}\"",
                c.street_name.as_deref().unwrap_or_default()
            ));
        }
    }
    notes
}

fn fill(slot: &mut Option<String>, value: Option<String>) {
    if slot.as_deref().is_none_or(|s| s.trim().is_empty()) {
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            *slot = Some(value);
        }
    }
}

#[derive(Debug, Default, PartialEq)]
struct StreetParts {
    house_number: Option<String>,
    house_number_suffix: Option<String>,
    predirection: Option<String>,
    street_name: Option<String>,
    suffix: Option<String>,
    postdirection: Option<String>,
}

/// USPS suffixes seen in dispatchable addresses, spelled out or
/// abbreviated, mapped to the abbreviation an MSAG expects.
const STREET_SUFFIXES: &[(&str, &str)] = &[
    ("alley", "Aly"),
    ("aly", "Aly"),
    ("avenue", "Ave"),
    ("ave", "Ave"),
    ("av", "Ave"),
    ("boulevard", "Blvd"),
    ("blvd", "Blvd"),
    ("circle", "Cir"),
    ("cir", "Cir"),
    ("court", "Ct"),
    ("ct", "Ct"),
    ("cove", "Cv"),
    ("cv", "Cv"),
    ("crossing", "Xing"),
    ("xing", "Xing"),
    ("drive", "Dr"),
    ("dr", "Dr"),
    ("expressway", "Expy"),
    ("expy", "Expy"),
    ("extension", "Ext"),
    ("ext", "Ext"),
    ("highway", "Hwy"),
    ("hwy", "Hwy"),
    ("lane", "Ln"),
    ("ln", "Ln"),
    ("loop", "Loop"),
    ("parkway", "Pkwy"),
    ("pkwy", "Pkwy"),
    ("path", "Path"),
    ("pike", "Pike"),
    ("place", "Pl"),
    ("pl", "Pl"),
    ("plaza", "Plz"),
    ("plz", "Plz"),
    ("road", "Rd"),
    ("rd", "Rd"),
    ("route", "Rte"),
    ("rte", "Rte"),
    ("row", "Row"),
    ("run", "Run"),
    ("square", "Sq"),
    ("sq", "Sq"),
    ("street", "St"),
    ("st", "St"),
    ("terrace", "Ter"),
    ("ter", "Ter"),
    ("trail", "Trl"),
    ("trl", "Trl"),
    ("turnpike", "Tpke"),
    ("tpke", "Tpke"),
    ("walk", "Walk"),
    ("way", "Way"),
    ("wy", "Way"),
];

const DIRECTIONS: &[(&str, &str)] = &[
    ("north", "N"),
    ("n", "N"),
    ("south", "S"),
    ("s", "S"),
    ("east", "E"),
    ("e", "E"),
    ("west", "W"),
    ("w", "W"),
    ("northeast", "NE"),
    ("ne", "NE"),
    ("northwest", "NW"),
    ("nw", "NW"),
    ("southeast", "SE"),
    ("se", "SE"),
    ("southwest", "SW"),
    ("sw", "SW"),
];

fn lookup(table: &[(&str, &str)], token: &str) -> Option<String> {
    let key = token
        .trim_matches(|c: char| c == '.' || c == ',')
        .to_ascii_lowercase();
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, abbrev)| abbrev.to_string())
}

/// `1455A N Market St SW` → house number 1455, suffix A, predirection N,
/// street Market, suffix St, postdirection SW. Anything that does not fit
/// stays in the street name.
fn parse_street(street: &str) -> StreetParts {
    let mut parts = StreetParts::default();
    let mut tokens: Vec<&str> = street.split_whitespace().collect();
    if tokens.is_empty() {
        return parts;
    }

    // The house number: leading digits, with a letter or fraction after
    // them either attached (`1455A`) or on its own (`1455 1/2`).
    let head = tokens[0];
    let digits: String = head.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() && tokens.len() > 1 {
        parts.house_number = Some(digits.clone());
        let rest: String = head[digits.len()..].trim_matches('-').to_string();
        if !rest.is_empty() {
            parts.house_number_suffix = Some(rest);
        }
        tokens.remove(0);
        let attached = parts.house_number_suffix.is_some();
        if !attached && tokens.len() > 1 {
            let next = tokens[0];
            let fraction =
                next.contains('/') && next.chars().all(|c| c.is_ascii_digit() || c == '/');
            let letter = next.len() == 1 && next.chars().all(|c| c.is_ascii_alphabetic());
            // A single letter is a house-number suffix only when it is not
            // a direction (`100 N Main St`).
            if fraction || (letter && lookup(DIRECTIONS, next).is_none()) {
                parts.house_number_suffix = Some(next.to_ascii_uppercase());
                tokens.remove(0);
            }
        }
    }

    // A leading direction, when a street name follows it.
    if tokens.len() > 1 {
        if let Some(direction) = lookup(DIRECTIONS, tokens[0]) {
            parts.predirection = Some(direction);
            tokens.remove(0);
        }
    }

    // A trailing direction, then the suffix before it; both only while a
    // street name is left.
    if tokens.len() > 2 {
        if let Some(direction) = lookup(DIRECTIONS, tokens[tokens.len() - 1]) {
            parts.postdirection = Some(direction);
            tokens.pop();
        }
    }
    if tokens.len() > 1 {
        if let Some(suffix) = lookup(STREET_SUFFIXES, tokens[tokens.len() - 1]) {
            parts.suffix = Some(suffix);
            tokens.pop();
        }
    }

    if !tokens.is_empty() {
        parts.street_name = Some(tokens.join(" "));
    }
    parts
}

#[derive(Clone, Copy)]
enum SecondLine {
    Unit,
    Floor,
    Room,
    Building,
    Other,
}

/// `Suite 300` → a unit, `4th Floor` → a floor, anything unrecognised →
/// additional location information (`LOC`).
fn parse_second_line(line: &str) -> (SecondLine, String) {
    const KINDS: &[(&str, SecondLine)] = &[
        ("suite", SecondLine::Unit),
        ("ste", SecondLine::Unit),
        ("unit", SecondLine::Unit),
        ("apartment", SecondLine::Unit),
        ("apt", SecondLine::Unit),
        ("floor", SecondLine::Floor),
        ("fl", SecondLine::Floor),
        ("flr", SecondLine::Floor),
        ("room", SecondLine::Room),
        ("rm", SecondLine::Room),
        ("building", SecondLine::Building),
        ("bldg", SecondLine::Building),
    ];
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let key = |t: &str| {
        t.trim_matches(|c: char| c == '.' || c == ',')
            .to_ascii_lowercase()
    };
    if tokens.len() == 2 {
        for (name, kind) in KINDS {
            if key(tokens[0]) == *name {
                return (*kind, tokens[1].to_string());
            }
            if key(tokens[1]) == *name {
                // `4th Floor`: the ordinal keeps its digits only.
                let value = tokens[0].trim_end_matches(|c: char| c.is_ascii_alphabetic());
                let value = if value.is_empty() { tokens[0] } else { value };
                return (*kind, value.to_string());
            }
        }
    }
    (SecondLine::Other, line.to_string())
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
    /// Which trunk that is: names are only unique within a tenant, and a
    /// tenant's own trunk carries its emergency calls
    /// (`docs/TENANT_TRUNKS.md` §5).
    #[serde(default)]
    pub trunk_id: Option<TrunkId>,
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
    /// number the carrier maps to an address), `pidf_lo` (by value), both
    /// as `elin+pidf_lo`, or none.
    #[serde(default)]
    pub delivery: Option<String>,
    /// Whether the location's street was in civic elements; `false` means it
    /// went out with its street line as the street name.
    #[serde(default)]
    pub location_structured: Option<bool>,
    /// The `Geolocation-Error` (or 424) a trunk answered the location with.
    #[serde(default)]
    pub geolocation_error: Option<String>,
}

/// What became of an emergency call, written onto its record when the call
/// ends (`docs/E911_BLOCK_2.md` §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmergencyCompletion {
    pub trunk_name: Option<String>,
    #[serde(default)]
    pub trunk_id: Option<TrunkId>,
    pub outcome: String,
    pub sip_status: Option<u16>,
    pub answered_at: Option<SystemTime>,
    pub ended_at: SystemTime,
    pub delivery: Option<String>,
    pub geolocation_error: Option<String>,
}

/// What this node's emergency calls did, for `/metrics`
/// (`docs/E911_BLOCK_2.md` §9).
#[derive(Debug, Default)]
pub struct EmergencyCounters {
    /// Calls by kind, location source and delivery method.
    calls: std::sync::Mutex<std::collections::BTreeMap<(String, String, String), u64>>,
    /// Calls placed with no location at all, and with an address that was
    /// never structured or never validated.
    gaps: std::sync::Mutex<std::collections::BTreeMap<&'static str, u64>>,
    /// `Geolocation-Error` answers by code.
    location_errors: std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
}

impl EmergencyCounters {
    /// The process's counters.
    pub fn global() -> &'static EmergencyCounters {
        static COUNTERS: std::sync::OnceLock<EmergencyCounters> = std::sync::OnceLock::new();
        COUNTERS.get_or_init(EmergencyCounters::default)
    }

    /// An emergency call was placed: its kind, where its location came from
    /// and how it travelled.
    pub fn placed(
        &self,
        kind: EmergencyKind,
        location: Option<&DispatchableLocation>,
        delivery: Option<&str>,
    ) {
        if let Ok(mut map) = self.calls.lock() {
            *map.entry((
                kind.to_string(),
                location
                    .map(|l| l.source.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                delivery.unwrap_or("none").to_string(),
            ))
            .or_insert(0) += 1;
        }
        let Ok(mut gaps) = self.gaps.lock() else {
            return;
        };
        match location {
            None => *gaps.entry("no_location").or_insert(0) += 1,
            Some(location) => {
                if !location.address.civic.is_structured() {
                    *gaps.entry("unstructured_address").or_insert(0) += 1;
                }
                if !location.address.validated {
                    *gaps.entry("unvalidated_address").or_insert(0) += 1;
                }
            }
        }
    }

    /// A trunk answered a location with an error (RFC 6442 §4.4).
    pub fn location_refused(&self, error: &str) {
        let code = error
            .split(';')
            .next()
            .unwrap_or("unknown")
            .trim()
            .to_string();
        if let Ok(mut map) = self.location_errors.lock() {
            *map.entry(code).or_insert(0) += 1;
        }
    }

    /// Prometheus text; `labels` is prepended to each series' labels
    /// (`node="…"`), or empty.
    pub fn prometheus(&self, labels: &str) -> String {
        let sep = if labels.is_empty() { "" } else { "," };
        let mut out = String::new();
        if let Ok(map) = self.calls.lock() {
            out.push_str(
                "# HELP fcp_emergency_calls_total Emergency calls placed, by kind, location source and delivery.\n\
                 # TYPE fcp_emergency_calls_total counter\n",
            );
            for ((kind, source, delivery), n) in map.iter() {
                out.push_str(&format!(
                    "fcp_emergency_calls_total{{{labels}{sep}kind=\"{kind}\",source=\"{source}\",delivery=\"{delivery}\"}} {n}\n"
                ));
            }
        }
        if let Ok(map) = self.gaps.lock() {
            out.push_str(
                "# HELP fcp_emergency_location_gaps_total Emergency calls whose location was missing, unstructured or unvalidated.\n\
                 # TYPE fcp_emergency_location_gaps_total counter\n",
            );
            for (gap, n) in map.iter() {
                out.push_str(&format!(
                    "fcp_emergency_location_gaps_total{{{labels}{sep}gap=\"{gap}\"}} {n}\n"
                ));
            }
        }
        if let Ok(map) = self.location_errors.lock() {
            out.push_str(
                "# HELP fcp_emergency_location_errors_total Geolocation-Error answers from emergency trunks, by code.\n\
                 # TYPE fcp_emergency_location_errors_total counter\n",
            );
            for (code, n) in map.iter() {
                out.push_str(&format!(
                    "fcp_emergency_location_errors_total{{{labels}{sep}code=\"{code}\"}} {n}\n"
                ));
            }
        }
        out
    }
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

    /// The tenant's networks, for placing a caller by address (§6.1).
    /// Defaulted so a store without them places nobody.
    async fn list_emergency_networks(
        &self,
        _tenant_id: &TenantId,
    ) -> anyhow::Result<Vec<EmergencyNetwork>> {
        Ok(Vec::new())
    }

    async fn get_emergency_network(
        &self,
        _id: &crate::EmergencyNetworkId,
    ) -> anyhow::Result<Option<EmergencyNetwork>> {
        Ok(None)
    }

    async fn put_emergency_network(&self, _network: &EmergencyNetwork) -> anyhow::Result<()> {
        anyhow::bail!("this store does not keep emergency networks")
    }

    async fn delete_emergency_network(
        &self,
        _id: &crate::EmergencyNetworkId,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Where a user last said they were (§6.4), whether or not it has
    /// expired; the caller decides.
    async fn get_declared_location(
        &self,
        _tenant_id: &TenantId,
        _user_id: &crate::UserId,
    ) -> anyhow::Result<Option<DeclaredLocation>> {
        Ok(None)
    }

    async fn put_declared_location(&self, _location: &DeclaredLocation) -> anyhow::Result<()> {
        anyhow::bail!("this store does not keep declared locations")
    }

    async fn delete_declared_location(
        &self,
        _tenant_id: &TenantId,
        _user_id: &crate::UserId,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    /// Complete the record `id` with what became of the call.
    async fn complete_emergency_call(
        &self,
        id: i64,
        completion: &EmergencyCompletion,
    ) -> anyhow::Result<()>;

    // ── On-site notification (§8) ─────────────────────────────────────
    //
    // Defaulted, so a store that keeps no notifications simply has none:
    // the notifier then delivers without a record of having done so, and
    // says as much in its logs.

    /// The record of an emergency call, found by the call's id — how a
    /// notifier, which sees only the event, reaches the record.
    async fn get_emergency_call_by_call_id(
        &self,
        _tenant_id: &TenantId,
        _call_id: &str,
    ) -> anyhow::Result<Option<EmergencyCallRecord>> {
        Ok(None)
    }

    /// Write one attempt at one contact and return its id.
    async fn insert_emergency_notification(
        &self,
        _notification: &EmergencyNotification,
    ) -> anyhow::Result<i64> {
        anyhow::bail!("this store does not keep emergency notifications")
    }

    /// Every attempt made for one call, oldest first.
    async fn list_emergency_notifications(
        &self,
        _tenant_id: &TenantId,
        _call_id: &str,
    ) -> anyhow::Result<Vec<EmergencyNotification>> {
        Ok(Vec::new())
    }

    /// Attempts whose retry is due, oldest first; the notifier takes them
    /// and clears the schedule as it does.
    async fn due_emergency_notifications(
        &self,
        _limit: usize,
    ) -> anyhow::Result<Vec<EmergencyNotification>> {
        Ok(Vec::new())
    }

    /// Stop this attempt being picked up again (it has been, or it is
    /// being retried by a row of its own).
    async fn clear_notification_retry(&self, _id: i64) -> anyhow::Result<()> {
        Ok(())
    }

    /// The last delivery to each of a tenant's contacts, for the readiness
    /// report's "when did this last work" (§8.4). `test_only` limits it to
    /// notices of test calls.
    async fn last_emergency_notifications(
        &self,
        _tenant_id: &TenantId,
        _test_only: bool,
    ) -> anyhow::Result<Vec<EmergencyNotification>> {
        Ok(Vec::new())
    }
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
            validated_by: None,
            provider_reference: None,
            civic: Default::default(),
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
            ..Default::default()
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
            ..Default::default()
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
            validated_by: None,
            provider_reference: None,
            civic: Default::default(),
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
            validated_by: None,
            provider_reference: None,
            civic: Default::default(),
        });
        assert!(!bad.valid);
        assert_eq!(bad.issues.len(), 4, "{:?}", bad.issues);
    }

    /// The street a person types comes back in RFC 5139 elements, so the
    /// call's PIDF-LO carries HNO/RD/STS (§5.1, §7).
    #[test]
    fn basic_validator_structures_the_street() {
        let mut address = addr("1455A N Market Street SW");
        address.address2 = Some("Suite 300".to_string());
        let checked = BasicAddressValidator::check(&address);
        assert!(checked.valid, "{:?}", checked.issues);
        let c = &checked.normalized.civic;
        assert_eq!(c.house_number.as_deref(), Some("1455"));
        assert_eq!(c.house_number_suffix.as_deref(), Some("A"));
        assert_eq!(c.street_predirection.as_deref(), Some("N"));
        assert_eq!(c.street_name.as_deref(), Some("Market"));
        assert_eq!(c.street_suffix.as_deref(), Some("St"), "spelled out or not");
        assert_eq!(c.street_postdirection.as_deref(), Some("SW"));
        assert_eq!(c.unit.as_deref(), Some("300"));
        assert!(c.is_structured());
        assert!(checked.notes.is_empty(), "{:?}", checked.notes);

        // A house number on its own line, and a floor written the other
        // way round.
        let mut address = addr("22 1/2 E Main Ave");
        address.address2 = Some("4th Floor".to_string());
        let checked = BasicAddressValidator::check(&address);
        let c = &checked.normalized.civic;
        assert_eq!(c.house_number.as_deref(), Some("22"));
        assert_eq!(c.house_number_suffix.as_deref(), Some("1/2"));
        assert_eq!(c.street_predirection.as_deref(), Some("E"));
        assert_eq!(c.street_name.as_deref(), Some("Main"));
        assert_eq!(c.street_suffix.as_deref(), Some("Ave"));
        assert_eq!(c.floor.as_deref(), Some("4"));
    }

    /// What the basic validator cannot tell apart it says, and a street it
    /// cannot split does not make the address invalid.
    #[test]
    fn basic_validator_says_what_it_could_not_split() {
        let checked = BasicAddressValidator::check(&addr("100 Broadway"));
        assert!(checked.valid, "{:?}", checked.issues);
        assert_eq!(
            checked.normalized.civic.house_number.as_deref(),
            Some("100")
        );
        assert_eq!(
            checked.normalized.civic.street_name.as_deref(),
            Some("Broadway")
        );
        assert_eq!(checked.notes.len(), 1, "{:?}", checked.notes);
        assert!(
            checked.notes[0].contains("street suffix"),
            "{:?}",
            checked.notes
        );

        let checked = BasicAddressValidator::check(&addr("Elm Street"));
        assert!(!checked.valid, "no house number is an issue of its own");
        assert!(
            checked.notes.iter().any(|n| n.contains("house number")),
            "{:?}",
            checked.notes
        );

        // An address already in elements keeps the elements it was given.
        let mut address = addr("1 State St");
        address.civic.street_name = Some("State Street".to_string());
        address.civic.house_number = Some("1".to_string());
        let checked = BasicAddressValidator::check(&address);
        assert_eq!(
            checked.normalized.civic.street_name.as_deref(),
            Some("State Street")
        );
        assert_eq!(
            checked.normalized.civic.street_suffix.as_deref(),
            Some("St")
        );
    }

    /// Only a validator marks an address validated, and it says which one.
    #[test]
    fn a_validated_address_carries_who_validated_it() {
        let checked = BasicAddressValidator::check(&addr("1 State St"));
        let now = SystemTime::now();
        let stored = checked.validated_address(now);
        assert!(stored.validated);
        assert_eq!(stored.validated_at, Some(now));
        assert_eq!(stored.validated_by.as_deref(), Some("basic"));

        // Editing any part of it forgets that.
        let mut edited = stored.clone();
        edited.address1 = "2 State St".to_string();
        edited.carry_validation(Some(&stored));
        assert!(!edited.validated);
        assert_eq!(edited.validated_by, None);

        // Re-sending the same address keeps it.
        let mut same = stored.clone();
        same.clear_validation();
        same.carry_validation(Some(&stored));
        assert!(same.validated);
        assert_eq!(same.validated_at, Some(now));

        let refused = BasicAddressValidator::check(&addr("Elm Street"));
        assert!(!refused.validated_address(now).validated);
    }

    #[test]
    fn summary_is_one_line() {
        let loc = DispatchableLocation {
            address: addr("1 State St"),
            callback_number: None,
            source: LocationSource::UserLocation,
            location_id: None,
            location_name: Some("HQ".to_string()),
            point: None,
            client_address: None,
            declared_age: None,
        };
        assert_eq!(loc.summary(), "HQ: 1 State St, Albany, NY 12207");
    }

    fn network(cidr: &str, site: &SiteLocation, applies_to: NetworkApplies) -> EmergencyNetwork {
        let mut n = EmergencyNetwork::new(TenantId::nil(), cidr.parse().unwrap(), site.id);
        n.applies_to = applies_to;
        n
    }

    #[test]
    fn the_network_a_caller_is_on_places_them_over_their_site() {
        let office = site("HQ", Some("1 State St"), Some("+15185550001"));
        let branch = {
            let mut s = site("Branch", Some("9 Elm St"), Some("+15185550009"));
            s.name = "Branch".to_string();
            s
        };
        let mut device = Device::new("phone".into(), DeviceType::Softphone, TenantId::nil());
        device.location_id = Some(office.id);
        let networks = [
            network("10.0.0.0/8", &branch, NetworkApplies::Nomadic),
            network("10.1.2.0/24", &branch, NetworkApplies::Nomadic),
        ];

        // The most specific network wins, and it places a softphone.
        let matched = match_network(
            &networks,
            "10.1.2.7".parse().unwrap(),
            Some(DeviceType::Softphone),
        )
        .expect("a network matches");
        assert_eq!(matched.cidr.prefix(), 24);
        let loc = resolve_dispatchable_location(&LocationInputs {
            device: Some(&device),
            device_site: Some(&office),
            network: Some((matched, &branch)),
            client_address: Some("10.1.2.7".parse().unwrap()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(loc.source, LocationSource::Network);
        assert_eq!(loc.location_name.as_deref(), Some("Branch"));
        assert_eq!(
            loc.callback_number.as_deref(),
            Some("+15185550009"),
            "the callback number is the site the address came from"
        );
        assert_eq!(loc.client_address.as_deref(), Some("10.1.2.7"));
        assert_eq!(loc.source.pidf_method(), "Wiremap");

        // A desk phone keeps the site it was provisioned at, unless the
        // network says it places everything.
        assert!(match_network(
            &networks,
            "10.1.2.7".parse().unwrap(),
            Some(DeviceType::DeskPhone)
        )
        .is_none());
        let all = [network("10.1.2.0/24", &branch, NetworkApplies::All)];
        assert!(match_network(
            &all,
            "10.1.2.7".parse().unwrap(),
            Some(DeviceType::DeskPhone)
        )
        .is_some());
        // An address on no network places nobody.
        assert!(match_network(&networks, "192.0.2.5".parse().unwrap(), None).is_none());
    }

    #[test]
    fn a_declared_location_stands_until_it_expires() {
        let now = SystemTime::now();
        let user_site = site("HQ", None, Some("+15185550001"));
        let declared = DeclaredLocation {
            user_id: UserId::new(),
            tenant_id: TenantId::nil(),
            address: addr("42 Hotel Rd"),
            declared_at: now - Duration::from_secs(3600),
            expires_at: now + Duration::from_secs(3600),
        };
        // The user's site has no street, so the declaration is the location.
        let loc = resolve_dispatchable_location(&LocationInputs {
            user_site: Some(&user_site),
            declared: Some(&declared),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(loc.source, LocationSource::Declared);
        assert!(loc.address.address1.contains("Hotel"));
        assert!(loc.declared_age.unwrap() >= Duration::from_secs(3600));
        assert_eq!(
            loc.callback_number.as_deref(),
            Some("+15185550001"),
            "the user's site still supplies the callback number"
        );

        let expired = DeclaredLocation {
            expires_at: now - Duration::from_secs(1),
            ..declared.clone()
        };
        assert!(!expired.is_current(now));
        assert!(resolve_dispatchable_location(&LocationInputs {
            user_site: Some(&user_site),
            declared: Some(&expired),
            ..Default::default()
        })
        .is_none());
    }

    #[test]
    fn a_trusted_devices_own_location_comes_first_after_its_override() {
        let supplied = addr("77 Device Ln");
        let user_site = site("HQ", Some("1 State St"), Some("+15185550001"));
        let loc = resolve_dispatchable_location(&LocationInputs {
            user_site: Some(&user_site),
            supplied: Some(&supplied),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(loc.source, LocationSource::DeviceSupplied);
        assert!(loc.address.address1.contains("Device"));
        assert_eq!(loc.source.pidf_method(), "Unknown");
    }
}
