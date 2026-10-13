//! Line binding types for connecting phone numbers to users and devices.
//!
//! A LineBinding connects a PhoneNumber to an owner (User) and one or more Devices,
//! defining how calls should be routed and handled.

use crate::identifiers::{DeviceId, LineBindingId, PhoneNumberId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::SystemTime;

/// A binding that connects a phone number to users and devices.
///
/// LineBinding is the "glue" that connects PhoneNumbers to Users and Devices.
/// It defines:
/// - Who owns the number (for billing, voicemail)
/// - Which devices should ring
/// - How devices should ring (simultaneously, sequentially, etc.)
///
/// Forwarding and the ring time are the owner's calling settings
/// (`crate::calling`, since v0.25.276).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineBinding {
    /// Unique identifier for this binding
    pub id: LineBindingId,

    /// The phone number being bound
    pub phone_number_id: PhoneNumberId,

    /// Owner of this line (receives voicemail, bills, etc.)
    /// Can be None for service numbers (IVRs, queues)
    pub owner_user_id: Option<UserId>,

    /// Type of binding (user, IVR, queue, etc.)
    pub binding_type: BindingType,

    /// Devices where this line appears
    pub devices: Vec<DeviceBinding>,

    /// Ring strategy for distributing calls
    pub ring_strategy: RingStrategy,

    /// Tenant this binding belongs to
    pub tenant_id: TenantId,

    // ===== Caller ID =====
    /// Override outbound caller ID for this line
    pub outbound_caller_id: Option<String>,

    /// Override outbound CNAM for this line
    pub outbound_cnam: Option<String>,

    // ===== IVR / Flow =====
    /// Flow ID to execute for IVR bindings
    pub flow_id: Option<String>,

    // ===== Schedule (docs/ENTERPRISE_CALLING.md §3.5) =====
    /// The number's hours: outside them, calls go to `closed_target`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<crate::ScheduleId>,

    /// Where calls go when the schedule is closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_target: Option<crate::CallTarget>,

    /// Where calls go on a holiday; the closed target when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holiday_target: Option<crate::CallTarget>,

    // ===== Shared call appearance (docs/ENTERPRISE_CALLING.md §3.9) =====
    /// The line is shared: every device bound to it registers the number as
    /// an account of its own, and they share its appearances (BroadSoft
    /// `call-info` and `line-seize`).
    #[serde(default)]
    pub shared: bool,

    /// How many calls a shared line carries at once, each an appearance
    /// (1 to 24).
    #[serde(default = "default_max_appearances")]
    pub max_appearances: u8,

    /// A phone on the shared line may barge into an active appearance: its
    /// INVITE naming the appearance joins the call, heard by both parties
    /// (`docs/ENTERPRISE_CALLING.md` §3.11).
    #[serde(default)]
    pub barge_in: bool,

    // ===== Music on hold (docs/ENTERPRISE_CALLING.md §3.3) =====
    /// What a caller to this number hears on hold, unless the holder chose
    /// their own: a prompt of the tenant by id, or
    /// [`HOLD_MUSIC_NONE`](crate::calling::HOLD_MUSIC_NONE) for silence.
    /// `None`: the tenant's hold music, else the node's built-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_music: Option<String>,

    // ===== Audit =====
    /// When this binding was created
    pub created_at: SystemTime,

    /// Last modification timestamp
    pub updated_at: SystemTime,
}

impl LineBinding {
    /// Create a new line binding.
    pub fn new(
        phone_number_id: PhoneNumberId,
        owner_user_id: Option<UserId>,
        binding_type: BindingType,
        tenant_id: TenantId,
    ) -> Self {
        let now = SystemTime::now();

        Self {
            id: LineBindingId::new(),
            phone_number_id,
            owner_user_id,
            binding_type,
            devices: Vec::new(),
            ring_strategy: RingStrategy::Simultaneous,
            tenant_id,
            outbound_caller_id: None,
            outbound_cnam: None,
            flow_id: None,
            schedule_id: None,
            closed_target: None,
            holiday_target: None,
            shared: false,
            max_appearances: DEFAULT_MAX_APPEARANCES,
            barge_in: false,
            hold_music: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Add a device to this line binding.
    pub fn add_device(&mut self, device_binding: DeviceBinding) {
        // Remove existing binding for this device if present
        self.devices
            .retain(|d| d.device_id != device_binding.device_id);
        self.devices.push(device_binding);
        self.updated_at = SystemTime::now();
    }

    /// Remove a device from this line binding.
    pub fn remove_device(&mut self, device_id: DeviceId) {
        self.devices.retain(|d| d.device_id != device_id);
        self.updated_at = SystemTime::now();
    }

    /// Get enabled devices sorted by priority.
    pub fn enabled_devices(&self) -> Vec<&DeviceBinding> {
        let mut devices: Vec<&DeviceBinding> = self.devices.iter().filter(|d| d.enabled).collect();
        devices.sort_by_key(|d| d.priority);
        devices
    }
}

/// A shared line's appearances unless it says otherwise.
pub const DEFAULT_MAX_APPEARANCES: u8 = 4;

/// The most appearances a shared line may have: more line keys than any
/// desk phone carries.
pub const MAX_APPEARANCES_LIMIT: u8 = 24;

fn default_max_appearances() -> u8 {
    DEFAULT_MAX_APPEARANCES
}

/// Type of line binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingType {
    /// Normal user line
    User,

    /// Auto-attendant/IVR
    Ivr,

    /// Call center queue
    Queue,

    /// Hunt group (sequential/round-robin)
    HuntGroup,

    /// Conference bridge
    ConferenceBridge,

    /// Direct voicemail access
    Voicemail,

    /// Call parking slot
    Parking,

    /// Overhead paging
    Paging,

    /// Fax service
    Fax,

    /// Emergency callback line
    Emergency,
}

impl fmt::Display for BindingType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Ivr => write!(f, "ivr"),
            Self::Queue => write!(f, "queue"),
            Self::HuntGroup => write!(f, "hunt_group"),
            Self::ConferenceBridge => write!(f, "conference_bridge"),
            Self::Voicemail => write!(f, "voicemail"),
            Self::Parking => write!(f, "parking"),
            Self::Paging => write!(f, "paging"),
            Self::Fax => write!(f, "fax"),
            Self::Emergency => write!(f, "emergency"),
        }
    }
}

/// Device binding - links a device to a line.
///
/// A device can have multiple line appearances (like a multi-line phone),
/// and a line can appear on multiple devices (shared line).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceBinding {
    /// The device where this line appears
    pub device_id: DeviceId,

    /// Priority for sequential ringing (1 = highest)
    pub priority: u8,

    /// Which line button on the phone (1-6 typically)
    pub line_appearance: u8,

    /// Enable shared call appearance (BLF, pickup)
    pub shared_line: bool,

    /// Whether this device binding is enabled
    pub enabled: bool,
}

impl DeviceBinding {
    /// Create a new device binding.
    pub fn new(device_id: DeviceId) -> Self {
        Self {
            device_id,
            priority: 1,
            line_appearance: 1,
            shared_line: false,
            enabled: true,
        }
    }

    /// Create a new device binding with custom settings.
    pub fn with_settings(
        device_id: DeviceId,
        priority: u8,
        line_appearance: u8,
        shared_line: bool,
    ) -> Self {
        Self {
            device_id,
            priority,
            line_appearance,
            shared_line,
            enabled: true,
        }
    }
}

/// Ring strategy for distributing incoming calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RingStrategy {
    /// Ring all devices at once
    #[default]
    Simultaneous,

    /// Ring devices in priority order with timeout
    Sequential,

    /// Distribute calls evenly (hunt group)
    RoundRobin,

    /// Ring device that's been idle longest
    LongestIdle,

    /// Only ring primary device (priority 1)
    Primary,
}

impl fmt::Display for RingStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Simultaneous => write!(f, "simultaneous"),
            Self::Sequential => write!(f, "sequential"),
            Self::RoundRobin => write!(f, "round_robin"),
            Self::LongestIdle => write!(f, "longest_idle"),
            Self::Primary => write!(f, "primary"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_binding_creation() {
        let phone_number_id = PhoneNumberId::new();
        let user_id = UserId::new();
        let tenant_id = TenantId::new();

        let binding =
            LineBinding::new(phone_number_id, Some(user_id), BindingType::User, tenant_id);

        assert_eq!(binding.phone_number_id, phone_number_id);
        assert_eq!(binding.owner_user_id, Some(user_id));
        assert_eq!(binding.binding_type, BindingType::User);
        assert_eq!(binding.devices.len(), 0);
        assert_eq!(binding.ring_strategy, RingStrategy::Simultaneous);
    }

    #[test]
    fn test_add_remove_devices() {
        let phone_number_id = PhoneNumberId::new();
        let tenant_id = TenantId::new();
        let mut binding = LineBinding::new(phone_number_id, None, BindingType::User, tenant_id);

        // Add device
        let device_id = DeviceId::new();
        let device_binding = DeviceBinding::new(device_id);
        binding.add_device(device_binding);

        assert_eq!(binding.devices.len(), 1);
        assert_eq!(binding.devices[0].device_id, device_id);

        // Remove device
        binding.remove_device(device_id);
        assert_eq!(binding.devices.len(), 0);
    }

    #[test]
    fn test_enabled_devices_sorted() {
        let phone_number_id = PhoneNumberId::new();
        let tenant_id = TenantId::new();
        let mut binding = LineBinding::new(phone_number_id, None, BindingType::User, tenant_id);

        // Add devices with different priorities
        let device1 = DeviceId::new();
        let device2 = DeviceId::new();
        let device3 = DeviceId::new();

        let mut db1 = DeviceBinding::new(device1);
        db1.priority = 3;
        binding.add_device(db1);

        let mut db2 = DeviceBinding::new(device2);
        db2.priority = 1;
        binding.add_device(db2);

        let mut db3 = DeviceBinding::new(device3);
        db3.priority = 2;
        db3.enabled = false; // Disabled
        binding.add_device(db3);

        // Get enabled devices - should be sorted by priority
        let enabled = binding.enabled_devices();
        assert_eq!(enabled.len(), 2); // device3 is disabled
        assert_eq!(enabled[0].device_id, device2); // priority 1
        assert_eq!(enabled[1].device_id, device1); // priority 3
    }

    #[test]
    fn test_device_binding_with_settings() {
        let device_id = DeviceId::new();
        let binding = DeviceBinding::with_settings(device_id, 2, 3, true);

        assert_eq!(binding.device_id, device_id);
        assert_eq!(binding.priority, 2);
        assert_eq!(binding.line_appearance, 3);
        assert!(binding.shared_line);
        assert!(binding.enabled);
    }

    #[test]
    fn test_binding_type_display() {
        assert_eq!(BindingType::User.to_string(), "user");
        assert_eq!(BindingType::Ivr.to_string(), "ivr");
        assert_eq!(BindingType::Queue.to_string(), "queue");
        assert_eq!(BindingType::HuntGroup.to_string(), "hunt_group");
    }

    #[test]
    fn test_ring_strategy_display() {
        assert_eq!(RingStrategy::Simultaneous.to_string(), "simultaneous");
        assert_eq!(RingStrategy::Sequential.to_string(), "sequential");
        assert_eq!(RingStrategy::RoundRobin.to_string(), "round_robin");
    }
}
