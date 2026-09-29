//! Core types for the presence service.

use fcp_core::TenantId;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    /// When this publication was created or last modified.
    pub created_at: Instant,
}

/// A pending NOTIFY delivery that needs to be retried.
#[derive(Debug, Clone)]
pub struct NotifyRetryEntry {
    /// The watcher to send the NOTIFY to.
    pub watcher: Watcher,
    /// The Subscription-State header value to use.
    pub subscription_state: String,
    /// Number of delivery attempts so far.
    pub attempt: u32,
    /// When the next retry should be attempted.
    pub next_retry_at: Instant,
    /// When the entry was originally created.
    pub created_at: Instant,
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
///
/// Evaluation order (first match wins):
/// 1. Block list (AOR exact match) → deny
/// 2. Per-event block list → deny
/// 3. Block patterns (domain wildcard) → deny
/// 4. Allow list (AOR exact match) → allow
/// 5. Per-event allow list → allow (if non-empty, restricts to listed AORs)
/// 6. Allow patterns (domain wildcard) → allow
/// 7. Time window check → deny if outside windows
/// 8. Default
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PresenceAuthPolicy {
    /// If `true`, watchers not in either list are allowed. If `false`, only
    /// watchers on the allow_list are permitted.
    pub default_allow: bool,
    /// Watcher AORs explicitly allowed to subscribe.
    #[serde(default)]
    pub allow_list: HashSet<String>,
    /// Watcher AORs explicitly blocked from subscribing.
    #[serde(default)]
    pub block_list: HashSet<String>,

    /// Event-package-specific allow lists. Key: event package name.
    /// If a watcher's event package has an entry here, only listed AORs
    /// may subscribe for that specific package.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub event_allow: HashMap<String, HashSet<String>>,

    /// Event-package-specific block lists.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub event_block: HashMap<String, HashSet<String>>,

    /// Optional time windows during which subscriptions are allowed.
    /// Empty = always allowed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub time_windows: Vec<TimeWindow>,

    /// Group-based allow patterns. Supports domain wildcards like `*@domain`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_patterns: Vec<String>,

    /// Group-based block patterns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub block_patterns: Vec<String>,
}

/// A time window during which subscriptions are allowed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeWindow {
    /// Start hour (0-23, UTC).
    pub start_hour: u8,
    /// End hour (0-23, UTC). If end < start, wraps past midnight.
    pub end_hour: u8,
    /// Days of week (0=Sun, 1=Mon, ..., 6=Sat). Empty = all days.
    #[serde(default)]
    pub days: Vec<u8>,
}

impl PresenceAuthPolicy {
    /// Check if a watcher is authorized for a specific event package.
    pub fn is_allowed_for_event(&self, watcher_aor: &str, event_package: &str) -> bool {
        // 1. Block list (highest priority)
        if self.block_list.contains(watcher_aor) {
            return false;
        }

        // 2. Per-event block
        if let Some(blocked) = self.event_block.get(event_package) {
            if blocked.contains(watcher_aor) {
                return false;
            }
        }

        // 3. Block patterns (glob-style domain matching)
        if self.matches_any_pattern(watcher_aor, &self.block_patterns) {
            return false;
        }

        // 4. Allow list (explicit)
        if self.allow_list.contains(watcher_aor) {
            return true;
        }

        // 5. Per-event allow (if non-empty, restricts to listed AORs only)
        if let Some(allowed) = self.event_allow.get(event_package) {
            if !allowed.is_empty() {
                return allowed.contains(watcher_aor);
            }
        }

        // 6. Allow patterns
        if self.matches_any_pattern(watcher_aor, &self.allow_patterns) {
            return true;
        }

        // 7. Time window check (if any defined)
        if !self.time_windows.is_empty() && !self.is_within_time_window() {
            return false;
        }

        // 8. Default
        self.default_allow
    }

    /// Backward-compatible check (defaults to "presence" event package).
    pub fn is_allowed(&self, watcher_aor: &str) -> bool {
        self.is_allowed_for_event(watcher_aor, "presence")
    }

    /// Check if an AOR matches any pattern in the list.
    /// Supports `*@domain` for domain wildcards.
    fn matches_any_pattern(&self, aor: &str, patterns: &[String]) -> bool {
        for pattern in patterns {
            if let Some(domain_pattern) = pattern.strip_prefix("*@") {
                // Domain wildcard: *@domain matches anything@domain
                if let Some(domain) = aor.split('@').nth(1) {
                    if domain_pattern == domain {
                        return true;
                    }
                }
            } else if pattern == aor {
                return true;
            }
        }
        false
    }

    /// Check if the current time falls within any configured time window.
    fn is_within_time_window(&self) -> bool {
        use std::time::SystemTime;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let secs_in_day = now % 86400;
        let current_hour = (secs_in_day / 3600) as u8;
        // Unix epoch (Jan 1, 1970) was a Thursday = day 4
        let current_day = ((now / 86400 + 4) % 7) as u8;

        self.time_windows.iter().any(|w| {
            let day_ok = w.days.is_empty() || w.days.contains(&current_day);
            let hour_ok = if w.start_hour <= w.end_hour {
                current_hour >= w.start_hour && current_hour < w.end_hour
            } else {
                // Wraps past midnight
                current_hour >= w.start_hour || current_hour < w.end_hour
            };
            day_ok && hour_ok
        })
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

    /// Whether this state represents an explicit DND (Do Not Disturb) condition.
    ///
    /// Only `Busy` returns true. `Closed` (offline) is NOT DND — offline agents
    /// should fail the ring attempt naturally, not be silently skipped by queue
    /// distribution.
    pub fn is_dnd_state(&self) -> bool {
        matches!(self, PresenceState::Busy)
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
