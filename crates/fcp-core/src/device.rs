//! Device management types for FCP.
//!
//! Devices represent physical or soft endpoints (desk phones, softphones, WebRTC clients)
//! that can register with the system and make/receive calls.

use crate::codec::AudioCodec;
use crate::identifiers::{DeviceId, LocationId, TenantId, UserId};
use crate::phone_number::E911Address;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::SystemTime;

/// A device endpoint (physical phone, softphone, WebRTC client, etc.).
///
/// `Debug` is implemented manually so `provisioning_token_hash` is redacted —
/// the hash is low-value on its own, but there is no reason to emit it into
/// trace logs or error reports.
#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    /// Unique identifier for this device
    pub id: DeviceId,

    /// Human-readable device name
    pub name: String,

    /// Type of device
    pub device_type: DeviceType,

    /// MAC address (for physical phones)
    pub mac_address: Option<String>,

    /// Device model (e.g., "Cisco 8861", "Zoiper 5", "WebRTC")
    pub model: Option<String>,

    /// The phone's vendor, which with its model picks its template when
    /// it names none (`docs/PROVISIONING.md` §5).
    #[serde(default)]
    pub vendor: Option<crate::config_template::PhoneVendor>,

    /// Firmware version
    pub firmware_version: Option<String>,

    /// Tenant this device belongs to
    pub tenant_id: TenantId,

    /// Owner of this device (optional)
    pub owner_user_id: Option<UserId>,

    /// The digest username of the device's own credentials
    /// (`docs/ENTERPRISE_CALLING.md` §3.8). Unset, the device registers
    /// with its owner's.
    #[serde(default)]
    pub sip_username: Option<String>,

    /// `MD5(sip_username:realm:password)` for the device's own credentials.
    #[serde(default)]
    pub sip_ha1_hash: Option<String>,

    /// The plaintext password of the device's own credentials, kept only
    /// for provisioning templates that need it, as a user's is.
    #[serde(default)]
    pub sip_provisioning_password: Option<String>,

    /// A shared phone a user may log in on (hoteling, §3.8).
    #[serde(default)]
    pub hoteling_host: bool,

    // ===== SIP Registration =====
    /// SIP URI for this device (e.g., "sip:alice-desk@192.168.1.80")
    pub sip_uri: Option<String>,

    /// Current registration state
    pub registration_state: RegistrationState,

    /// Last successful registration time
    pub last_registration: Option<SystemTime>,

    /// Last IP address the device registered from
    pub last_ip_address: Option<String>,

    /// User-Agent header from REGISTER
    pub user_agent: Option<String>,

    /// The provider that wakes it when it sleeps (`apns`, `fcm`): set by a
    /// REGISTER with a push address, gone when it unregisters
    /// (`docs/PUSH.md` §2.1).
    #[serde(default)]
    pub push_provider: Option<String>,

    // ===== Capabilities =====
    /// How many line appearances this device supports
    pub max_lines: u8,

    /// Video calling support
    pub supports_video: bool,

    /// Busy Lamp Field support (shared line appearance)
    pub supports_blf: bool,

    /// Supported audio codecs
    pub codec_support: Vec<AudioCodec>,

    /// The keys programmed on the phone, in order (`line_keys` in its
    /// template).
    #[serde(default)]
    pub line_keys: Vec<LineKey>,

    // ===== Location & E911 =====
    /// Physical location description
    pub location: Option<String>,

    /// Reference to a SiteLocation entity
    pub location_id: Option<LocationId>,

    /// Per-device E911 override (for mobile users)
    pub e911_override: Option<E911Address>,

    /// Whether this device's own location (a PIDF-LO it sends with a call)
    /// is trusted, over what the tenant trusts its device type to send
    /// (`docs/E911_BLOCK_2.md` §6.3). `None` follows the type.
    #[serde(default)]
    pub e911_trust_location: Option<bool>,

    // ===== Provisioning =====
    /// Provisioning URL (for auto-provisioning)
    pub provisioning_url: Option<String>,

    /// Configuration template ID
    pub config_template_id: Option<String>,

    /// SHA-256 hash of the per-device provisioning token.
    ///
    /// The plaintext token is only shown at device creation or supplied during
    /// rotation; provisioning requests must present it alongside the deployment
    /// provisioning token.
    #[serde(default)]
    pub provisioning_token_hash: Option<String>,

    // ===== Status =====
    /// Whether this device is enabled
    pub enabled: bool,

    /// Created timestamp
    pub created_at: SystemTime,

    /// Last modified timestamp
    pub updated_at: SystemTime,
}

impl Device {
    /// Create a new device.
    pub fn new(name: String, device_type: DeviceType, tenant_id: TenantId) -> Self {
        let now = SystemTime::now();

        Self {
            id: DeviceId::new(),
            name,
            device_type,
            mac_address: None,
            model: None,
            vendor: None,
            firmware_version: None,
            tenant_id,
            owner_user_id: None,
            sip_username: None,
            sip_ha1_hash: None,
            sip_provisioning_password: None,
            hoteling_host: false,
            sip_uri: None,
            registration_state: RegistrationState::Unregistered,
            last_registration: None,
            last_ip_address: None,
            user_agent: None,
            push_provider: None,
            max_lines: 1,
            supports_video: false,
            supports_blf: false,
            codec_support: vec![AudioCodec::Pcmu, AudioCodec::Pcma],
            line_keys: Vec::new(),
            location: None,
            location_id: None,
            e911_override: None,
            e911_trust_location: None,
            provisioning_url: None,
            config_template_id: None,
            provisioning_token_hash: None,
            enabled: true,
            created_at: now,
            updated_at: now,
        }
    }

    /// Update registration state.
    pub fn update_registration_state(
        &mut self,
        state: RegistrationState,
        ip_address: Option<String>,
        user_agent: Option<String>,
    ) {
        self.registration_state = state;

        if matches!(state, RegistrationState::Registered) {
            self.last_registration = Some(SystemTime::now());
        }

        if let Some(ip) = ip_address {
            self.last_ip_address = Some(ip);
        }

        if let Some(ua) = user_agent {
            self.user_agent = Some(ua);
        }

        self.updated_at = SystemTime::now();
    }

    /// The user part of the device's SIP URI: the name it registers and
    /// calls as.
    pub fn aor_user(&self) -> Option<&str> {
        let uri = self.sip_uri.as_deref()?;
        let rest = uri
            .strip_prefix("sips:")
            .or_else(|| uri.strip_prefix("sip:"))
            .unwrap_or(uri);
        let user = rest.split_once('@').map(|(u, _)| u)?;
        (!user.is_empty()).then_some(user)
    }

    /// Whether the device has credentials of its own.
    pub fn has_own_credentials(&self) -> bool {
        self.sip_username.is_some() && self.sip_ha1_hash.is_some()
    }

    /// Check if device is currently registered.
    pub fn is_registered(&self) -> bool {
        matches!(self.registration_state, RegistrationState::Registered)
    }

    /// Set device location.
    pub fn set_location(&mut self, location: String) {
        self.location = Some(location);
        self.updated_at = SystemTime::now();
    }

    /// Set E911 override address. An address that differs from the one
    /// stored arrives unvalidated, whatever it claims
    /// (`docs/E911_BLOCK_2.md` §7).
    pub fn set_e911_override(&mut self, address: E911Address) {
        let mut address = address;
        address.carry_validation(self.e911_override.as_ref());
        self.e911_override = Some(address);
        self.updated_at = SystemTime::now();
    }

    /// Enable or disable the device.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.updated_at = SystemTime::now();
    }
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("device_type", &self.device_type)
            .field("mac_address", &self.mac_address)
            .field("model", &self.model)
            .field("vendor", &self.vendor)
            .field("firmware_version", &self.firmware_version)
            .field("tenant_id", &self.tenant_id)
            .field("owner_user_id", &self.owner_user_id)
            .field("sip_uri", &self.sip_uri)
            .field("registration_state", &self.registration_state)
            .field("last_registration", &self.last_registration)
            .field("last_ip_address", &self.last_ip_address)
            .field("user_agent", &self.user_agent)
            .field("max_lines", &self.max_lines)
            .field("supports_video", &self.supports_video)
            .field("supports_blf", &self.supports_blf)
            .field("codec_support", &self.codec_support)
            .field("line_keys", &self.line_keys)
            .field("location", &self.location)
            .field("location_id", &self.location_id)
            .field("e911_override", &self.e911_override)
            .field("provisioning_url", &self.provisioning_url)
            .field("config_template_id", &self.config_template_id)
            .field(
                "provisioning_token_hash",
                &self.provisioning_token_hash.as_ref().map(|_| "<redacted>"),
            )
            .field("enabled", &self.enabled)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// What a REGISTER said about a device (`docs/PROVISIONING.md` §8),
/// written on its own so an administrator's edit is not lost to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRegistrationUpdate {
    pub state: RegistrationState,
    pub at: SystemTime,
    /// The contact's host and port.
    pub address: Option<String>,
    pub user_agent: Option<String>,
    /// Kept only where the device has none.
    pub vendor: Option<crate::config_template::PhoneVendor>,
    pub model: Option<String>,
    /// What the phone runs: replaces what the device had.
    pub firmware: Option<String>,
    /// The provider of the binding's push address: replaces what the device
    /// had when registered, and is cleared when not.
    pub push_provider: Option<String>,
}

impl DeviceRegistrationUpdate {
    /// Apply to a device as the store does.
    pub fn apply(&self, device: &mut Device) {
        device.registration_state = self.state;
        if self.state == RegistrationState::Registered {
            device.last_registration = Some(self.at);
            device.push_provider = self.push_provider.clone();
        } else {
            device.push_provider = None;
        }
        if self.address.is_some() {
            device.last_ip_address = self.address.clone();
        }
        if self.user_agent.is_some() {
            device.user_agent = self.user_agent.clone();
        }
        if device.vendor.is_none() {
            device.vendor = self.vendor;
        }
        if device.model.is_none() {
            device.model = self.model.clone();
        }
        if self.firmware.is_some() {
            device.firmware_version = self.firmware.clone();
        }
    }
}

/// A key programmed on a phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LineKey {
    pub kind: LineKeyKind,
    /// What it watches or dials: an extension, a number, a park slot.
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// What a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LineKeyKind {
    /// Busy lamp field: shows whether the extension is on a call, and
    /// calls or picks it up.
    Blf,
    SpeedDial,
    /// A park slot: shows whether a call is parked there.
    Park,
}

impl LineKeyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blf => "blf",
            Self::SpeedDial => "speed_dial",
            Self::Park => "park",
        }
    }
}

/// The most keys a device may have.
pub const MAX_LINE_KEYS: usize = 100;

/// Refuse keys a phone's configuration could not carry.
pub fn validate_line_keys(keys: &[LineKey]) -> Result<(), String> {
    if keys.len() > MAX_LINE_KEYS {
        return Err(format!("a device has at most {MAX_LINE_KEYS} line keys"));
    }
    for (i, key) in keys.iter().enumerate() {
        let n = i + 1;
        let value = key.value.trim();
        if value.is_empty() || value.len() > 128 {
            return Err(format!("line key {n}: value must be 1 to 128 characters"));
        }
        let label = key.label.as_deref().unwrap_or("");
        if label.len() > 64 {
            return Err(format!("line key {n}: label must be at most 64 characters"));
        }
        if [value, label]
            .iter()
            .any(|s| s.chars().any(char::is_control))
        {
            return Err(format!("line key {n}: no control characters"));
        }
    }
    Ok(())
}

/// Type of device endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum DeviceType {
    /// Physical desk phone
    DeskPhone,

    /// Software phone (PC/mobile app)
    #[default]
    Softphone,

    /// WebRTC browser client
    WebRtc,

    /// Conference room system
    ConferenceRoom,

    /// Analog telephone adapter
    Ata,

    /// SIP gateway
    Gateway,

    /// Mobile device
    Mobile,

    /// Other/unknown type
    Other,
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeskPhone => write!(f, "desk_phone"),
            Self::Softphone => write!(f, "softphone"),
            Self::WebRtc => write!(f, "webrtc"),
            Self::ConferenceRoom => write!(f, "conference_room"),
            Self::Ata => write!(f, "ata"),
            Self::Gateway => write!(f, "gateway"),
            Self::Mobile => write!(f, "mobile"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// SIP registration state for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RegistrationState {
    /// Device has never registered
    #[default]
    Unregistered,

    /// Device is currently registered
    Registered,

    /// Registration failed (authentication, network, etc.)
    Failed,

    /// Registration expired
    Expired,
}

impl fmt::Display for RegistrationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unregistered => write!(f, "unregistered"),
            Self::Registered => write!(f, "registered"),
            Self::Failed => write!(f, "failed"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_creation() {
        let tenant_id = TenantId::new();
        let device = Device::new(
            "Alice's Desk Phone".to_string(),
            DeviceType::DeskPhone,
            tenant_id,
        );

        assert_eq!(device.name, "Alice's Desk Phone");
        assert_eq!(device.device_type, DeviceType::DeskPhone);
        assert_eq!(device.tenant_id, tenant_id);
        assert_eq!(device.registration_state, RegistrationState::Unregistered);
        assert!(device.enabled);
        assert!(!device.is_registered());
    }

    #[test]
    fn test_registration_state_update() {
        let tenant_id = TenantId::new();
        let mut device = Device::new("Test Device".to_string(), DeviceType::Softphone, tenant_id);

        assert!(!device.is_registered());
        assert!(device.last_registration.is_none());

        device.update_registration_state(
            RegistrationState::Registered,
            Some("192.168.1.100".to_string()),
            Some("Zoiper 5.0".to_string()),
        );

        assert!(device.is_registered());
        assert!(device.last_registration.is_some());
        assert_eq!(device.last_ip_address, Some("192.168.1.100".to_string()));
        assert_eq!(device.user_agent, Some("Zoiper 5.0".to_string()));
    }

    #[test]
    fn test_device_location() {
        let tenant_id = TenantId::new();
        let mut device = Device::new("Test Device".to_string(), DeviceType::DeskPhone, tenant_id);

        assert!(device.location.is_none());

        device.set_location("Building A, Floor 2, Room 205".to_string());

        assert_eq!(
            device.location,
            Some("Building A, Floor 2, Room 205".to_string())
        );
    }

    #[test]
    fn test_device_enable_disable() {
        let tenant_id = TenantId::new();
        let mut device = Device::new("Test Device".to_string(), DeviceType::Softphone, tenant_id);

        assert!(device.enabled);

        device.set_enabled(false);
        assert!(!device.enabled);

        device.set_enabled(true);
        assert!(device.enabled);
    }

    #[test]
    fn test_device_type_display() {
        assert_eq!(DeviceType::DeskPhone.to_string(), "desk_phone");
        assert_eq!(DeviceType::Softphone.to_string(), "softphone");
        assert_eq!(DeviceType::WebRtc.to_string(), "webrtc");
        assert_eq!(DeviceType::ConferenceRoom.to_string(), "conference_room");
    }

    #[test]
    fn test_registration_state_display() {
        assert_eq!(RegistrationState::Unregistered.to_string(), "unregistered");
        assert_eq!(RegistrationState::Registered.to_string(), "registered");
        assert_eq!(RegistrationState::Failed.to_string(), "failed");
        assert_eq!(RegistrationState::Expired.to_string(), "expired");
    }

    #[test]
    fn test_debug_redacts_provisioning_token_hash() {
        let tenant_id = TenantId::new();
        let mut device = Device::new("Test Device".to_string(), DeviceType::DeskPhone, tenant_id);
        let secret_hash =
            "cafebabe1111222233334444555566667777888899990000aaaabbbbccccdddd".to_string();
        device.provisioning_token_hash = Some(secret_hash.clone());

        let rendered = format!("{:?}", device);
        assert!(rendered.contains("<redacted>"));
        assert!(
            !rendered.contains(&secret_hash),
            "Debug output leaked the hash: {}",
            rendered
        );

        device.provisioning_token_hash = None;
        let rendered_none = format!("{:?}", device);
        assert!(rendered_none.contains("provisioning_token_hash: None"));
        assert!(!rendered_none.contains("<redacted>"));
    }

    #[test]
    fn test_e911_override() {
        let tenant_id = TenantId::new();
        let mut device = Device::new("Test Device".to_string(), DeviceType::Mobile, tenant_id);

        assert!(device.e911_override.is_none());

        let address = E911Address::new(
            "123 Main St".to_string(),
            "Albany".to_string(),
            "NY".to_string(),
            "12201".to_string(),
            "US".to_string(),
        );

        device.set_e911_override(address.clone());

        assert_eq!(device.e911_override, Some(address));
    }
}
