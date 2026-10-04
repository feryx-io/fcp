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
    /// `served`, `unknown_device`, `disabled`, `no_template`,
    /// `unauthorized`, `not_a_device_file`, `network_refused`.
    pub outcome: String,
}

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
fn mac_of(s: &str) -> Option<String> {
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

/// Polycom's master file for a MAC: the device's own file, and the
/// directories it may write to left empty.
pub fn polycom_master(mac: &str) -> String {
    format!(
        "<?xml version=\"1.0\" standalone=\"yes\"?>\n\
         <APPLICATION APP_FILE_PATH=\"\" CONFIG_FILES=\"{mac}{POLYCOM_DEVICE_SUFFIX}\" \
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
}
