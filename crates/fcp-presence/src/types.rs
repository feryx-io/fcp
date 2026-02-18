//! Core types for the presence service.

use fcp_core::TenantId;
use serde::{Deserialize, Serialize};
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
    pub allow_list: Vec<String>,
    /// Watcher AORs explicitly blocked from subscribing.
    pub block_list: Vec<String>,
}

impl PresenceAuthPolicy {
    /// Check if a watcher is authorized by this policy.
    ///
    /// Block list takes precedence over allow list.
    pub fn is_allowed(&self, watcher_aor: &str) -> bool {
        if self.block_list.iter().any(|b| b == watcher_aor) {
            return false;
        }
        if self.allow_list.iter().any(|a| a == watcher_aor) {
            return true;
        }
        self.default_allow
    }
}
