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
