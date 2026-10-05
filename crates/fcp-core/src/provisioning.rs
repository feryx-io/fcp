//! Provisioning that phones can use (`docs/PROVISIONING.md`,
//! `docs/IDENTITY_AND_DEVICES.md` phase 7).
//!
//! A tenant has a provisioning key, a secret in the path of its base URL
//! (`/provision/<key>/`), because a phone can be told a URL and little
//! else. Under it, each vendor's file names are served: the MAC is read
//! from the name, the device found in the tenant, and its configuration
//! rendered. Every fetch is recorded.

use std::time::SystemTime;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config_template::PhoneVendor;
use crate::{DeviceId, TenantId};

/// A tenant's provisioning key, as shown to its administrator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisioningKey {
    pub tenant_id: TenantId,
    pub key: String,
    pub created_at: SystemTime,
}

impl ProvisioningKey {
    /// A new key: 128 random bits as hex, safe in a URL path.
    pub fn generate(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            key: uuid::Uuid::new_v4().simple().to_string(),
            created_at: SystemTime::now(),
        }
    }
}

/// How a key is found: by its hash, so the table can be indexed on it.
pub fn hash_key(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}

/// One fetch of a provisioning file, kept on the device it named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFetch {
    pub tenant_id: TenantId,
    /// The device, when the MAC named one of the tenant's.
    pub device_id: Option<DeviceId>,
    /// The MAC the file named, when it named one.
    pub mac_address: Option<String>,
    pub file: String,
    pub at: SystemTime,
    pub address: Option<String>,
    pub user_agent: Option<String>,
    /// `served`, `pending` (a MAC nobody has a device for, noted to be
    /// claimed), `unknown_device`, `not_served`, `unauthorized`,
    /// `not_a_device_file`, `network_refused`.
    pub outcome: String,
}

/// A phone that fetched under a tenant's key with a MAC no device has,
/// kept so an administrator can claim it (`docs/PROVISIONING.md` §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDevice {
    pub tenant_id: TenantId,
    pub mac_address: String,
    pub first_seen: SystemTime,
    pub last_seen: SystemTime,
    pub address: Option<String>,
    pub user_agent: Option<String>,
    pub vendor: Option<PhoneVendor>,
    pub model: Option<String>,
    pub firmware: Option<String>,
    /// How many times it has asked.
    pub fetches: u32,
}

/// The pending phones kept per tenant: the oldest go first.
pub const PENDING_KEPT: usize = 1000;

/// Where keys and fetches are kept.
#[async_trait]
pub trait ProvisioningStore: Send + Sync {
    async fn get_provisioning_key(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Option<ProvisioningKey>>;
    /// Make or replace the tenant's key: the old one stops working.
    async fn put_provisioning_key(&self, key: &ProvisioningKey) -> anyhow::Result<()>;
    /// The tenant a key belongs to.
    async fn tenant_for_provisioning_key(&self, key: &str) -> anyhow::Result<Option<TenantId>>;
    /// Record a fetch, keeping the newest few hundred per tenant.
    async fn record_fetch(&self, fetch: &DeviceFetch) -> anyhow::Result<()>;
    /// A device's fetches, newest first.
    async fn device_fetches(
        &self,
        tenant_id: &TenantId,
        device_id: &DeviceId,
        limit: usize,
    ) -> anyhow::Result<Vec<DeviceFetch>>;
    /// Note a phone nobody has claimed: made on its first fetch, counted
    /// and refreshed on each after; the newest [`PENDING_KEPT`] are kept.
    async fn note_pending_device(&self, pending: &PendingDevice) -> anyhow::Result<()>;
    /// A tenant's pending phones, the most recently seen first.
    async fn list_pending_devices(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Vec<PendingDevice>>;
    async fn get_pending_device(
        &self,
        tenant_id: &TenantId,
        mac_address: &str,
    ) -> anyhow::Result<Option<PendingDevice>>;
    /// Forget a pending phone (claimed, imported or dismissed); whether
    /// there was one.
    async fn remove_pending_device(
        &self,
        tenant_id: &TenantId,
        mac_address: &str,
    ) -> anyhow::Result<bool>;

    /// A tenant's account with a vendor's redirection service
    /// (`docs/PROVISIONING.md` §10), its secret opened.
    async fn get_redirection_account(
        &self,
        tenant_id: &TenantId,
        service: RedirectionService,
    ) -> anyhow::Result<Option<RedirectionAccount>>;
    async fn list_redirection_accounts(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Vec<RedirectionAccount>>;
    /// Make or replace an account; its secret is sealed at rest.
    async fn put_redirection_account(&self, account: &RedirectionAccount) -> anyhow::Result<()>;
    async fn delete_redirection_account(
        &self,
        tenant_id: &TenantId,
        service: RedirectionService,
    ) -> anyhow::Result<bool>;
    /// Where a device stands with its vendor's service.
    async fn get_device_redirection(
        &self,
        device_id: &DeviceId,
    ) -> anyhow::Result<Option<DeviceRedirection>>;
    async fn put_device_redirection(&self, record: &DeviceRedirection) -> anyhow::Result<()>;
    async fn delete_device_redirection(&self, device_id: &DeviceId) -> anyhow::Result<()>;

    /// Firmware images (`docs/PROVISIONING.md` §11): the platform's, every
    /// tenant's to use.
    async fn put_firmware_image(
        &self,
        image: &crate::firmware::FirmwareImage,
    ) -> anyhow::Result<()>;
    async fn get_firmware_image(
        &self,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<crate::firmware::FirmwareImage>>;
    async fn list_firmware_images(&self) -> anyhow::Result<Vec<crate::firmware::FirmwareImage>>;
    /// Remove an image no tenant targets; whether there was one.
    async fn delete_firmware_image(&self, id: &uuid::Uuid) -> anyhow::Result<bool>;
    /// How many targets, across tenants, name an image.
    async fn firmware_image_targets(&self, id: &uuid::Uuid) -> anyhow::Result<u64>;
    /// A tenant's targets.
    async fn list_firmware_targets(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Vec<crate::firmware::FirmwareTarget>>;
    async fn put_firmware_target(
        &self,
        target: &crate::firmware::FirmwareTarget,
    ) -> anyhow::Result<()>;
    async fn delete_firmware_target(
        &self,
        tenant_id: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<bool>;
}

// ---- Zero-touch: vendors' redirection services (§10) ----

/// A vendor's redirection service: a phone out of its box asks it where
/// its provisioning server is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RedirectionService {
    /// Yealink's Redirection and Provisioning Service.
    YealinkRps,
    /// Poly's (Polycom's) Zero Touch Provisioning.
    PolyZtp,
}

impl RedirectionService {
    pub const ALL: [RedirectionService; 2] = [Self::YealinkRps, Self::PolyZtp];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::YealinkRps => "yealink_rps",
            Self::PolyZtp => "poly_ztp",
        }
    }

    /// The vendor whose phones ask it.
    pub fn vendor(self) -> PhoneVendor {
        match self {
            Self::YealinkRps => PhoneVendor::Yealink,
            Self::PolyZtp => PhoneVendor::Polycom,
        }
    }

    /// The service a vendor's phones ask, where FCP has a connector.
    pub fn for_vendor(vendor: PhoneVendor) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.vendor() == vendor)
    }
}

impl std::str::FromStr for RedirectionService {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| format!("unknown redirection service `{s}` (yealink_rps, poly_ztp)"))
    }
}

/// A tenant's account with a redirection service.
#[derive(Clone, PartialEq, Eq)]
pub struct RedirectionAccount {
    pub tenant_id: TenantId,
    pub service: RedirectionService,
    /// Off, devices are neither registered nor removed.
    pub enabled: bool,
    /// Yealink: the AccessKey ID; Poly: the API user id.
    pub key_id: String,
    /// Yealink: the AccessKey Secret; Poly: the API password. Sealed.
    pub secret: String,
    /// Poly: the Reference ID (`account-id`) devices are associated under.
    pub account_id: Option<String>,
    /// Poly: the profile, made in the ZTP console, naming the tenant's
    /// provisioning URL (`templateCriteria`).
    pub profile: Option<String>,
    pub updated_at: SystemTime,
}

impl std::fmt::Debug for RedirectionAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedirectionAccount")
            .field("tenant_id", &self.tenant_id)
            .field("service", &self.service)
            .field("enabled", &self.enabled)
            .field("key_id", &self.key_id)
            .field("secret", &"<redacted>")
            .field("account_id", &self.account_id)
            .field("profile", &self.profile)
            .finish()
    }
}

/// How a device's registration with its vendor went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RedirectionState {
    /// The service sends the phone to the tenant's URL.
    Registered,
    /// The service refused or could not be reached: `detail` says why.
    Failed,
}

/// Where a device stands with its vendor's service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRedirection {
    pub tenant_id: TenantId,
    pub device_id: DeviceId,
    pub service: RedirectionService,
    /// The MAC registered (12 hex digits): removed from the service when
    /// the device's changes or the device goes.
    pub mac_address: String,
    /// The service's own id for the device, where it has one (Yealink's).
    pub vendor_ref: Option<String>,
    pub state: RedirectionState,
    pub detail: Option<String>,
    /// Whether the phone was given credentials through the service.
    pub with_credentials: bool,
    pub at: SystemTime,
}

/// A tenant's provisioning settings (`docs/PROVISIONING.md` §3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TenantProvisioning {
    /// A phone must also give its own HTTP credentials: Basic, with the
    /// device's provisioning token as the password.
    #[serde(default)]
    pub require_device_credentials: bool,
    /// Where phones may fetch from (CIDRs); empty is anywhere.
    #[serde(default)]
    pub networks: Vec<String>,
}

impl TenantProvisioning {
    /// The settings as stored: networks checked and normalized.
    pub fn normalized(mut self) -> Result<Self, String> {
        let mut networks = Vec::new();
        for n in &self.networks {
            let n = n.trim();
            if n.is_empty() {
                continue;
            }
            let net: ipnetwork::IpNetwork =
                n.parse().map_err(|_| format!("'{n}' is not a network"))?;
            networks.push(net.to_string());
        }
        self.networks = networks;
        Ok(self)
    }

    /// Whether `address` may fetch.
    pub fn allows(&self, address: Option<std::net::IpAddr>) -> bool {
        if self.networks.is_empty() {
            return true;
        }
        let Some(address) = address else {
            return false;
        };
        self.networks
            .iter()
            .filter_map(|n| n.parse::<ipnetwork::IpNetwork>().ok())
            .any(|n| n.contains(address))
    }
}

/// What a file name asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestedFile {
    /// A device's own configuration.
    Device { mac: String, kind: FileKind },
    /// Polycom's master file for a MAC: it names the device's file.
    PolycomMaster { mac: String },
    /// A file every phone of a vendor asks for and FCP has nothing in
    /// (Yealink's `y0000000000xx.cfg`, Polycom's `000000000000.cfg`).
    Common,
}

/// How a device's file is served: its content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// `.cfg`: Yealink, Fanvil, Grandstream's text form.
    Text,
    /// `.xml`: Grandstream, Cisco, Snom, Polycom's device file.
    Xml,
    /// `.htm`: Snom's settings page.
    Html,
}

impl FileKind {
    pub fn content_type(self) -> &'static str {
        match self {
            FileKind::Text => "text/plain; charset=utf-8",
            FileKind::Xml => "application/xml; charset=utf-8",
            FileKind::Html => "text/html; charset=utf-8",
        }
    }
}

/// The 12 hex digits of a MAC, lower case, if `s` is one.
pub fn mac_of(s: &str) -> Option<String> {
    let s = s.replace([':', '-', '.'], "");
    (s.len() == 12 && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

/// The suffix FCP serves a Polycom device's own file under, named by its
/// master file.
pub const POLYCOM_DEVICE_SUFFIX: &str = "-fcp.cfg";

/// Read a provisioning file name the way each vendor's phones ask:
///
/// - Yealink, Fanvil: `<mac>.cfg`; Yealink's common `y0000000000xx.cfg`.
/// - Polycom: `<mac>.cfg` (a master naming `<mac>-fcp.cfg`),
///   `000000000000.cfg`.
/// - Grandstream: `cfg<mac>.xml`, `cfg<mac>`.
/// - Cisco: `SEP<MAC>.cnf.xml`, `spa<mac>.xml`.
/// - Snom: `snom<model>-<MAC>.htm`, `<mac>.xml`.
/// - Any: `<mac>`.
///
/// `polycom` says the requesting phone is a Polycom (from its User-Agent),
/// whose `<mac>.cfg` is a master file rather than its configuration.
pub fn parse_file(name: &str, polycom: bool) -> Option<RequestedFile> {
    let lower = name.to_ascii_lowercase();
    if lower == "000000000000.cfg" || (lower.starts_with("y0000000000") && lower.ends_with(".cfg"))
    {
        return Some(RequestedFile::Common);
    }
    if let Some(mac) = lower.strip_suffix(POLYCOM_DEVICE_SUFFIX).and_then(mac_of) {
        return Some(RequestedFile::Device {
            mac,
            kind: FileKind::Xml,
        });
    }
    if let Some(mac) = lower.strip_suffix(".cfg").and_then(mac_of) {
        return Some(if polycom {
            RequestedFile::PolycomMaster { mac }
        } else {
            RequestedFile::Device {
                mac,
                kind: FileKind::Text,
            }
        });
    }
    if let Some(rest) = lower.strip_prefix("cfg") {
        let (stem, kind) = match rest.strip_suffix(".xml") {
            Some(stem) => (stem, FileKind::Xml),
            None => (rest, FileKind::Text),
        };
        if let Some(mac) = mac_of(stem) {
            return Some(RequestedFile::Device { mac, kind });
        }
    }
    if let Some(mac) = lower
        .strip_prefix("sep")
        .and_then(|r| r.strip_suffix(".cnf.xml"))
        .and_then(mac_of)
    {
        return Some(RequestedFile::Device {
            mac,
            kind: FileKind::Xml,
        });
    }
    if let Some(mac) = lower
        .strip_prefix("spa")
        .and_then(|r| r.strip_suffix(".xml"))
        .and_then(mac_of)
    {
        return Some(RequestedFile::Device {
            mac,
            kind: FileKind::Xml,
        });
    }
    if lower.starts_with("snom") {
        if let Some(mac) = lower
            .strip_suffix(".htm")
            .and_then(|r| r.rsplit_once('-'))
            .and_then(|(_, mac)| mac_of(mac))
        {
            return Some(RequestedFile::Device {
                mac,
                kind: FileKind::Html,
            });
        }
    }
    if let Some(mac) = lower.strip_suffix(".xml").and_then(mac_of) {
        return Some(RequestedFile::Device {
            mac,
            kind: FileKind::Xml,
        });
    }
    mac_of(&lower).map(|mac| RequestedFile::Device {
        mac,
        kind: FileKind::Text,
    })
}

/// Polycom's master file for a MAC: the device's own file, the firmware
/// (`APP_FILE_PATH`) it is pointed at if any, and the directories it may
/// write to left empty.
pub fn polycom_master(mac: &str, firmware: Option<&str>) -> String {
    let app = firmware
        .unwrap_or("")
        .replace('&', "&amp;")
        .replace('"', "&quot;");
    format!(
        "<?xml version=\"1.0\" standalone=\"yes\"?>\n\
         <APPLICATION APP_FILE_PATH=\"{app}\" CONFIG_FILES=\"{mac}{POLYCOM_DEVICE_SUFFIX}\" \
         MISC_FILES=\"\" LOG_FILE_DIRECTORY=\"\" OVERRIDES_DIRECTORY=\"\" \
         CONTACTS_DIRECTORY=\"\" LICENSE_DIRECTORY=\"\" USER_PROFILES_DIRECTORY=\"\" \
         CALL_LISTS_DIRECTORY=\"\" COREFILE_DIRECTORY=\"\"/>\n"
    )
}

/// Whether a User-Agent is a Polycom's (Poly's) provisioning client.
pub fn is_polycom(user_agent: Option<&str>) -> bool {
    user_agent.is_some_and(|ua| {
        let ua = ua.to_ascii_lowercase();
        ua.contains("polycom") || ua.contains("poly/") || ua.contains("polyedge")
    })
}

// ---- Who a phone is (`docs/PROVISIONING.md` §5) ----

/// What a phone says it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhoneIdentity {
    pub vendor: Option<PhoneVendor>,
    pub model: Option<String>,
    pub firmware: Option<String>,
}

impl PhoneIdentity {
    /// What is known, from what the request says: the file name's vendor
    /// over the User-Agent's over the MAC's, and the User-Agent's model
    /// over the file name's.
    pub fn of_request(file: &str, user_agent: Option<&str>, mac: Option<&str>) -> Self {
        let from_ua = user_agent.and_then(identify).unwrap_or_default();
        let (file_vendor, file_model) = vendor_of_file(file);
        Self {
            vendor: file_vendor
                .or(from_ua.vendor)
                .or_else(|| mac.and_then(vendor_of_mac)),
            model: from_ua.model.or(file_model),
            firmware: from_ua.firmware,
        }
    }
}

fn is_version(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_digit()) && s.contains('.')
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Read a phone's User-Agent — from a provisioning fetch or a REGISTER —
/// for its vendor, model and firmware:
///
/// - `Yealink SIP-T54W 96.86.0.70 80:5e:c0:…`
/// - `FileTransport PolycomVVX-VVX_450-UA/6.4.0.4006`, `PolyEdge-Edge_E450-UA/8.1.0`
/// - `Grandstream Model HW GXP2170 SW 1.0.11.6 DevId …`, `Grandstream GXP2170 1.0.11.6`
/// - `Cisco/SPA504G-7.6.2b (…)`, `Cisco-CP8861-3PCC/11.3.1`
/// - `Fanvil X4U 2.12.1 0c383e…`
/// - `snomD785-SIP 10.1.33.33`, `snomD785/10.1.33.33`
pub fn identify(user_agent: &str) -> Option<PhoneIdentity> {
    let ua = user_agent.trim();
    let lower = ua.to_ascii_lowercase();
    let words: Vec<&str> = ua.split_whitespace().collect();
    let after = |i: usize| words.get(i).copied();

    if let Some(i) = words.iter().position(|w| w.eq_ignore_ascii_case("yealink")) {
        let model = after(i + 1).map(|m| {
            m.strip_prefix("SIP-")
                .or_else(|| m.strip_prefix("sip-"))
                .unwrap_or(m)
        });
        return Some(PhoneIdentity {
            vendor: Some(PhoneVendor::Yealink),
            model: model.and_then(non_empty),
            firmware: after(i + 2).filter(|v| is_version(v)).and_then(non_empty),
        });
    }
    if let Some(word) = words.iter().find(|w| {
        let w = w.to_ascii_lowercase();
        w.starts_with("polycom") || w.starts_with("polyedge") || w.starts_with("poly-")
    }) {
        let (product, firmware) = match word.split_once("-UA/") {
            Some((p, f)) => (p, Some(f.split(['_', ' ']).next().unwrap_or(f))),
            None => (*word, None),
        };
        let model = product.split_once('-').map(|(_, m)| m.replace('_', " "));
        return Some(PhoneIdentity {
            vendor: Some(PhoneVendor::Polycom),
            model: model.as_deref().and_then(non_empty),
            firmware: firmware.and_then(non_empty),
        });
    }
    if let Some(i) = words
        .iter()
        .position(|w| w.eq_ignore_ascii_case("grandstream"))
    {
        let labelled = |label: &str| {
            words
                .iter()
                .position(|w| w.eq_ignore_ascii_case(label))
                .and_then(|j| after(j + 1))
        };
        let (model, firmware) = match labelled("HW") {
            Some(model) => (Some(model), labelled("SW")),
            None => (after(i + 1), after(i + 2).filter(|v| is_version(v))),
        };
        return Some(PhoneIdentity {
            vendor: Some(PhoneVendor::Grandstream),
            model: model.and_then(non_empty),
            firmware: firmware.and_then(non_empty),
        });
    }
    if let Some(rest) = lower.find("cisco/").map(|i| &ua[i + 6..]) {
        let product = rest.split_whitespace().next().unwrap_or("");
        let (model, firmware) = match product.split_once('-') {
            Some((m, f)) => (m, Some(f)),
            None => (product, None),
        };
        return Some(PhoneIdentity {
            vendor: Some(PhoneVendor::Cisco),
            model: non_empty(model),
            firmware: firmware.and_then(non_empty),
        });
    }
    if let Some(rest) = lower.find("cisco-").map(|i| &ua[i + 6..]) {
        let product = rest.split_whitespace().next().unwrap_or("");
        let (model, firmware) = match product.split_once('/') {
            Some((m, f)) => (m, Some(f)),
            None => (product, None),
        };
        return Some(PhoneIdentity {
            vendor: Some(PhoneVendor::Cisco),
            model: non_empty(model),
            firmware: firmware.and_then(non_empty),
        });
    }
    if let Some(i) = words.iter().position(|w| w.eq_ignore_ascii_case("fanvil")) {
        return Some(PhoneIdentity {
            vendor: Some(PhoneVendor::Fanvil),
            model: after(i + 1).and_then(non_empty),
            firmware: after(i + 2).filter(|v| is_version(v)).and_then(non_empty),
        });
    }
    if let Some(at) = lower.find("snom") {
        let rest = &ua[at + 4..];
        let model: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let tail = &rest[model.len()..];
        let firmware = tail
            .split(|c: char| c.is_whitespace() || c == '/' || c == ';' || c == ')')
            .find(|t| is_version(t));
        return Some(PhoneIdentity {
            vendor: Some(PhoneVendor::Snom),
            model: non_empty(&model),
            firmware: firmware.and_then(non_empty),
        });
    }
    None
}

/// A phone's vendor from its MAC's first three bytes, for the vendors'
/// best-known prefixes.
pub fn vendor_of_mac(mac: &str) -> Option<PhoneVendor> {
    let mac = mac_of(mac)?;
    Some(match &mac[..6] {
        "001565" | "805ec0" | "249ad8" => PhoneVendor::Yealink,
        "0004f2" | "64167f" => PhoneVendor::Polycom,
        "000b82" | "c074ad" => PhoneVendor::Grandstream,
        "000413" => PhoneVendor::Snom,
        "0c383e" => PhoneVendor::Fanvil,
        _ => return None,
    })
}

/// The vendor a file name belongs to, where only one vendor's phones ask
/// for it, and the model when the name carries it (Snom's).
pub fn vendor_of_file(name: &str) -> (Option<PhoneVendor>, Option<String>) {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("sep") && lower.ends_with(".cnf.xml")
        || lower.starts_with("spa") && lower.ends_with(".xml")
    {
        return (Some(PhoneVendor::Cisco), None);
    }
    if lower.starts_with("cfg") {
        return (Some(PhoneVendor::Grandstream), None);
    }
    if lower.ends_with(POLYCOM_DEVICE_SUFFIX) {
        return (Some(PhoneVendor::Polycom), None);
    }
    if let Some(stem) = lower
        .strip_prefix("snom")
        .and_then(|r| r.strip_suffix(".htm"))
    {
        let model = stem
            .rsplit_once('-')
            .map(|(m, _)| m)
            .and_then(non_empty)
            .map(|m| m.to_ascii_uppercase());
        return (Some(PhoneVendor::Snom), model);
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(mac: &str, kind: FileKind) -> Option<RequestedFile> {
        Some(RequestedFile::Device {
            mac: mac.into(),
            kind,
        })
    }

    #[test]
    fn each_vendors_file_names() {
        let mac = "805ec0a1b2c3";
        assert_eq!(
            parse_file("805ec0a1b2c3.cfg", false),
            device(mac, FileKind::Text)
        );
        assert_eq!(
            parse_file("805EC0A1B2C3.cfg", false),
            device(mac, FileKind::Text)
        );
        assert_eq!(
            parse_file("y000000000096.cfg", false),
            Some(RequestedFile::Common)
        );
        assert_eq!(
            parse_file("805ec0a1b2c3.cfg", true),
            Some(RequestedFile::PolycomMaster { mac: mac.into() })
        );
        assert_eq!(
            parse_file("805ec0a1b2c3-fcp.cfg", true),
            device(mac, FileKind::Xml)
        );
        assert_eq!(
            parse_file("000000000000.cfg", true),
            Some(RequestedFile::Common)
        );
        assert_eq!(
            parse_file("cfg805ec0a1b2c3.xml", false),
            device(mac, FileKind::Xml)
        );
        assert_eq!(
            parse_file("cfg805ec0a1b2c3", false),
            device(mac, FileKind::Text)
        );
        assert_eq!(
            parse_file("SEP805EC0A1B2C3.cnf.xml", false),
            device(mac, FileKind::Xml)
        );
        assert_eq!(
            parse_file("spa805ec0a1b2c3.xml", false),
            device(mac, FileKind::Xml)
        );
        assert_eq!(
            parse_file("snomD785-805EC0A1B2C3.htm", false),
            device(mac, FileKind::Html)
        );
        assert_eq!(
            parse_file("805ec0a1b2c3.xml", false),
            device(mac, FileKind::Xml)
        );
        assert_eq!(
            parse_file("805ec0a1b2c3", false),
            device(mac, FileKind::Text)
        );
        assert_eq!(parse_file("firmware.rom", false), None);
        assert_eq!(parse_file("../etc/passwd", false), None);
    }

    #[test]
    fn networks_are_checked_and_matched() {
        let p = TenantProvisioning {
            networks: vec!["10.1.0.0/16".into(), " 192.0.2.7 ".into(), "".into()],
            ..Default::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(p.networks, vec!["10.1.0.0/16", "192.0.2.7/32"]);
        assert!(p.allows(Some("10.1.200.3".parse().unwrap())));
        assert!(p.allows(Some("192.0.2.7".parse().unwrap())));
        assert!(!p.allows(Some("192.0.2.8".parse().unwrap())));
        assert!(!p.allows(None));
        assert!(TenantProvisioning::default().allows(None));
        assert!(TenantProvisioning {
            networks: vec!["nowhere".into()],
            ..Default::default()
        }
        .normalized()
        .is_err());
    }

    #[test]
    fn a_key_is_long_and_url_safe() {
        let k = ProvisioningKey::generate(TenantId::new());
        assert_eq!(k.key.len(), 32);
        assert!(k.key.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash_key(&k.key).len(), 64);
    }

    fn who(ua: &str) -> (Option<PhoneVendor>, Option<String>, Option<String>) {
        let i = identify(ua).unwrap_or_default();
        (i.vendor, i.model, i.firmware)
    }

    fn some(
        v: PhoneVendor,
        m: &str,
        f: &str,
    ) -> (Option<PhoneVendor>, Option<String>, Option<String>) {
        (Some(v), Some(m.into()), (!f.is_empty()).then(|| f.into()))
    }

    #[test]
    fn each_vendors_user_agent_names_its_model_and_firmware() {
        use PhoneVendor::*;
        assert_eq!(
            who("Yealink SIP-T54W 96.86.0.70 80:5e:c0:a1:b2:c3"),
            some(Yealink, "T54W", "96.86.0.70")
        );
        assert_eq!(
            who("Yealink W60B 77.85.0.20"),
            some(Yealink, "W60B", "77.85.0.20")
        );
        assert_eq!(
            who("FileTransport PolycomVVX-VVX_450-UA/6.4.0.4006"),
            some(Polycom, "VVX 450", "6.4.0.4006")
        );
        assert_eq!(
            who("PolycomVVX-VVX_250-UA/6.3.1.8427_0004f2a1b2c3"),
            some(Polycom, "VVX 250", "6.3.1.8427")
        );
        assert_eq!(
            who("PolyEdge-Edge_E450-UA/8.1.0.1234"),
            some(Polycom, "Edge E450", "8.1.0.1234")
        );
        assert_eq!(
            who("Grandstream Model HW GXP2170 SW 1.0.11.6 DevId 000b82a1b2c3"),
            some(Grandstream, "GXP2170", "1.0.11.6")
        );
        assert_eq!(
            who("Grandstream GRP2614 1.0.5.15"),
            some(Grandstream, "GRP2614", "1.0.5.15")
        );
        assert_eq!(
            who("Cisco/SPA504G-7.6.2b (88755B123456)(CCQ1234)"),
            some(Cisco, "SPA504G", "7.6.2b")
        );
        assert_eq!(
            who("Cisco-CP8861-3PCC/11.3.1"),
            some(Cisco, "CP8861-3PCC", "11.3.1")
        );
        assert_eq!(
            who("Fanvil X4U 2.12.1 0c383e112233"),
            some(Fanvil, "X4U", "2.12.1")
        );
        assert_eq!(
            who("Mozilla/4.0 (compatible; snomD785-SIP 10.1.33.33 OpenSSL)"),
            some(Snom, "D785", "10.1.33.33")
        );
        assert_eq!(who("snomD785/10.1.33.33"), some(Snom, "D785", "10.1.33.33"));
        assert_eq!(identify("Zoiper rv2.10.8.3"), None);
        assert_eq!(identify(""), None);
    }

    #[test]
    fn a_requests_file_name_and_mac_name_its_vendor() {
        assert_eq!(
            vendor_of_mac("80:5E:C0:A1:B2:C3"),
            Some(PhoneVendor::Yealink)
        );
        assert_eq!(vendor_of_mac("0004f2000001"), Some(PhoneVendor::Polycom));
        assert_eq!(vendor_of_mac("aabbccddeeff"), None);
        assert_eq!(
            vendor_of_file("SEP805EC0A1B2C3.cnf.xml").0,
            Some(PhoneVendor::Cisco)
        );
        assert_eq!(
            vendor_of_file("cfg805ec0a1b2c3.xml").0,
            Some(PhoneVendor::Grandstream)
        );
        assert_eq!(
            vendor_of_file("snomD785-805EC0A1B2C3.htm"),
            (Some(PhoneVendor::Snom), Some("D785".into()))
        );
        assert_eq!(vendor_of_file("805ec0a1b2c3.cfg"), (None, None));
        // The file name's vendor wins; the MAC's is the last word.
        let id = PhoneIdentity::of_request("cfg001565a1b2c3.xml", None, Some("001565a1b2c3"));
        assert_eq!(id.vendor, Some(PhoneVendor::Grandstream));
        let id = PhoneIdentity::of_request("001565a1b2c3.cfg", None, Some("001565a1b2c3"));
        assert_eq!(id.vendor, Some(PhoneVendor::Yealink));
        let id = PhoneIdentity::of_request(
            "aabbccddeeff.cfg",
            Some("Fanvil X4U 2.12.1"),
            Some("aabbccddeeff"),
        );
        assert_eq!(
            (id.vendor, id.model.as_deref()),
            (Some(PhoneVendor::Fanvil), Some("X4U"))
        );
    }
}
