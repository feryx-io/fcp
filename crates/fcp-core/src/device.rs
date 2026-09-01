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

    /// Firmware version
    pub firmware_version: Option<String>,

    /// Tenant this device belongs to
    pub tenant_id: TenantId,

    /// Owner of this device (optional)
    pub owner_user_id: Option<UserId>,

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

    // ===== Capabilities =====
    /// How many line appearances this device supports
    pub max_lines: u8,

    /// Video calling support
    pub supports_video: bool,

    /// Busy Lamp Field support (shared line appearance)
    pub supports_blf: bool,

    /// Supported audio codecs
    pub codec_support: Vec<AudioCodec>,

    // ===== Location & E911 =====
    /// Physical location description
    pub location: Option<String>,

    /// Reference to a SiteLocation entity
    pub location_id: Option<LocationId>,

    /// Per-device E911 override (for mobile users)
    pub e911_override: Option<E911Address>,

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
            firmware_version: None,
            tenant_id,
            owner_user_id: None,
            sip_uri: None,
            registration_state: RegistrationState::Unregistered,
            last_registration: None,
            last_ip_address: None,
            user_agent: None,
            max_lines: 1,
            supports_video: false,
            supports_blf: false,
            codec_support: vec![AudioCodec::Pcmu, AudioCodec::Pcma],
            location: None,
            location_id: None,
            e911_override: None,
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

    /// Check if device is currently registered.
    pub fn is_registered(&self) -> bool {
        matches!(self.registration_state, RegistrationState::Registered)
    }

    /// Set device location.
    pub fn set_location(&mut self, location: String) {
        self.location = Some(location);
        self.updated_at = SystemTime::now();
    }

    /// Set E911 override address.
    pub fn set_e911_override(&mut self, address: E911Address) {
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
