//! Core types for the presence service.

use fcp_core::TenantId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::time::Instant;

/// Basic presence status per RFC 3863 / RFC 4480.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    /// Available / online
    Open,
    /// Offline / unavailable
    Closed,
    /// Away from desk
    Away,
    /// Do not disturb
    Busy,
    /// Currently on a call
    OnThePhone,
    /// Incoming call ringing
    Ringing,
}

impl fmt::Display for PresenceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PresenceState::Open => write!(f, "open"),
            PresenceState::Closed => write!(f, "closed"),
            PresenceState::Away => write!(f, "away"),
            PresenceState::Busy => write!(f, "busy"),
            PresenceState::OnThePhone => write!(f, "on_the_phone"),
            PresenceState::Ringing => write!(f, "ringing"),
        }
    }
}

impl std::str::FromStr for PresenceState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "open" => Ok(PresenceState::Open),
            "closed" => Ok(PresenceState::Closed),
            "away" => Ok(PresenceState::Away),
            "busy" => Ok(PresenceState::Busy),
            "on_the_phone" => Ok(PresenceState::OnThePhone),
            "ringing" => Ok(PresenceState::Ringing),
            other => anyhow::bail!("Invalid presence state: {other}"),
        }
    }
}

/// Presence information for a single presentity (AOR).
#[derive(Debug, Clone)]
pub struct PresenceInfo {
    /// Address of Record (user@domain)
    pub aor: String,
    /// Current presence state
    pub state: PresenceState,
    /// Free-text status message
    pub note: Option<String>,
    /// Tenant this presentity belongs to
    pub tenant_id: TenantId,
    /// Last update time (runtime only, not serialized)
    pub updated_at: Instant,
}

/// A watcher subscription for a presentity.
#[derive(Debug, Clone)]
pub struct Watcher {
    /// Who is watching (AOR)
    pub watcher_aor: String,
    /// Who they are watching (AOR)
    pub presentity_aor: String,
    /// Unique subscription ID (from SIP dialog)
    pub subscription_id: String,
    /// When this subscription expires
    pub expires_at: Instant,
    /// Tenant context
    pub tenant_id: TenantId,
    /// SIP Contact URI where NOTIFY should be sent
    pub contact_uri: String,
    /// Event package: "presence" (RFC 3856) or "dialog" (RFC 4235 BLF)
    pub event_package: String,
    /// Call-ID from SUBSCRIBE (for in-dialog NOTIFY)
    pub sip_call_id: Option<String>,
    /// From-tag from SUBSCRIBE (becomes To-tag in NOTIFY)
    pub sip_from_tag: Option<String>,
    /// To-tag from our 200 OK (becomes From-tag in NOTIFY)
    pub sip_to_tag: Option<String>,
}

/// Event State Composition (ESC) for RFC 3903 PUBLISH tracking.
///
/// Each PUBLISH creates an ESC entry keyed by SIP-ETag. The ESC holds
/// the published presence state and expiry so that refresh/modify/remove
/// operations can be validated against the ETag.
#[derive(Debug, Clone)]
pub struct EventStateComposition {
    /// The SIP-ETag assigned to this publication.
    pub etag: String,
    /// Presentity AOR this publication belongs to.
    pub aor: String,
    /// The published presence state.
    pub state: PresenceState,
    /// Optional status note.
    pub note: Option<String>,
    /// Tenant context.
    pub tenant_id: TenantId,
    /// When this publication expires.
    pub expires_at: Instant,
}

/// Update request for presence state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceUpdate {
    /// Address of Record (user@domain)
    pub aor: String,
    /// New presence state
    pub state: PresenceState,
    /// Optional status note
    pub note: Option<String>,
}

/// Authorization policy for presence subscriptions.
///
/// Determines which watchers are allowed to subscribe to a presentity's state.
/// If no policy is set for a presentity, all watchers are allowed (default open).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresenceAuthPolicy {
    /// If `true`, watchers not in either list are allowed. If `false`, only
    /// watchers on the allow_list are permitted.
    pub default_allow: bool,
    /// Watcher AORs explicitly allowed to subscribe.
    pub allow_list: HashSet<String>,
    /// Watcher AORs explicitly blocked from subscribing.
    pub block_list: HashSet<String>,
}

impl PresenceAuthPolicy {
    /// Check if a watcher is authorized by this policy.
    ///
    /// Block list takes precedence over allow list.
    pub fn is_allowed(&self, watcher_aor: &str) -> bool {
        if self.block_list.contains(watcher_aor) {
            return false;
        }
        if self.allow_list.contains(watcher_aor) {
            return true;
        }
        self.default_allow
    }
}

impl PresenceState {
    /// Priority for multi-device presence composition.
    /// Higher value = more "active" state wins.
    pub fn composition_priority(&self) -> u8 {
        match self {
            PresenceState::OnThePhone => 5,
            PresenceState::Ringing => 4,
            PresenceState::Busy => 3,
            PresenceState::Open => 2,
            PresenceState::Away => 1,
            PresenceState::Closed => 0,
        }
    }

    /// Map to RPID activity element name (RFC 4480).
    pub fn to_rpid_activity(&self) -> Option<&'static str> {
        match self {
            PresenceState::OnThePhone => Some("on-the-phone"),
            PresenceState::Busy => Some("busy"),
            PresenceState::Away => Some("away"),
            _ => None,
        }
    }
}

/// Information about an active call for dialog-info generation.
#[derive(Debug, Clone)]
pub struct ActiveCallInfo {
    pub call_id: String,
    pub remote_uri: String,
    pub remote_display_name: Option<String>,
    pub direction: CallDirection,
    pub dialog_state: DialogState,
}

/// Call direction for dialog-info.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    Initiator,
    Recipient,
}

impl fmt::Display for CallDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallDirection::Initiator => write!(f, "initiator"),
            CallDirection::Recipient => write!(f, "recipient"),
        }
    }
}

/// Dialog state for dialog-info (RFC 4235).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogState {
    Trying,
    Proceeding,
    Early,
    Confirmed,
    Terminated,
}

impl fmt::Display for DialogState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialogState::Trying => write!(f, "trying"),
            DialogState::Proceeding => write!(f, "proceeding"),
            DialogState::Early => write!(f, "early"),
            DialogState::Confirmed => write!(f, "confirmed"),
            DialogState::Terminated => write!(f, "terminated"),
        }
    }
}
