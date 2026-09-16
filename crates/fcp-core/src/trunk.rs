//! SIP Trunk types for FCP.
//!
//! Trunks represent SIP connections to upstream carriers or downstream PBXes
//! for originating and terminating calls.

use crate::identifiers::TenantId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::SystemTime;
use uuid::Uuid;

/// Unique identifier for a SIP trunk.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrunkId(String);

impl TrunkId {
    /// Create a new random trunk ID.
    pub fn new() -> Self {
        Self(format!("trunk-{}", Uuid::new_v4()))
    }

    /// Create a trunk ID from a string.
    pub fn from_string(s: String) -> Self {
        Self(s)
    }

    /// The id of a platform trunk — one a node reads from its own
    /// `[[trunks]]` rather than from the database (`docs/TENANT_TRUNKS.md`
    /// §4). Derived from the name, so it is the same on every node and
    /// across restarts, and cannot collide with a stored trunk's id.
    pub fn platform(name: &str) -> Self {
        Self(format!("platform:{name}"))
    }

    /// Whether this is a platform trunk's id.
    pub fn is_platform(&self) -> bool {
        self.0.starts_with("platform:")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for TrunkId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TrunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for TrunkId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

/// Direction of a SIP trunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TrunkDirection {
    /// Outbound-only (send calls to carrier)
    #[default]
    Upstream,
    /// Inbound-only (receive calls from carrier)
    Downstream,
    /// Both inbound and outbound
    Bidirectional,
}

impl fmt::Display for TrunkDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Upstream => write!(f, "upstream"),
            Self::Downstream => write!(f, "downstream"),
            Self::Bidirectional => write!(f, "bidirectional"),
        }
    }
}

impl std::str::FromStr for TrunkDirection {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "upstream" => Ok(Self::Upstream),
            "downstream" => Ok(Self::Downstream),
            "bidirectional" => Ok(Self::Bidirectional),
            _ => Err(format!("Invalid trunk direction: {}", s)),
        }
    }
}

/// A SIP trunk configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trunk {
    /// Unique identifier
    pub id: TrunkId,

    /// The tenant whose calls this trunk carries. `None` is a platform
    /// trunk — one a node reads from its own `[[trunks]]`, shared by every
    /// tenant that has none of its own (`docs/TENANT_TRUNKS.md` §4).
    pub tenant_id: Option<TenantId>,

    /// Human-readable name
    pub name: String,

    /// Whether the trunk is enabled
    pub enabled: bool,

    /// Traffic direction
    pub direction: TrunkDirection,

    /// Selection priority (lower = higher priority)
    pub priority: i32,

    /// Selection weight (for load balancing among equal priority)
    pub weight: i32,

    /// Remote hosts (IP:port or hostname:port)
    pub hosts: Vec<String>,

    /// Transport protocol (udp, tcp, tls)
    pub transport_protocol: String,

    /// Transport port
    pub transport_port: i32,

    /// Whether to verify TLS certificates
    pub verify_tls: bool,

    /// Authentication username (optional)
    pub auth_username: Option<String>,

    /// Authentication password (optional)
    pub auth_password: Option<String>,

    /// Authentication realm (optional)
    pub auth_realm: Option<String>,

    /// Whether to send REGISTER to this trunk
    pub register: bool,

    /// Registration expiry in seconds
    pub register_expiry: i32,

    /// Maximum concurrent calls (0 = unlimited)
    pub max_concurrent_calls: i32,

    /// Maximum consecutive failures before marking down
    pub max_failures: i32,

    /// Recovery interval in seconds
    pub recovery_interval_secs: i32,

    /// Whether to send OPTIONS health checks
    pub health_check_enabled: bool,

    /// Health check interval in seconds
    pub health_check_interval_secs: i32,

    /// Preferred codecs in priority order (e.g. ["PCMU", "PCMA", "G722"])
    /// Empty = accept all codecs
    pub codec_prefs: Vec<String>,

    /// Outbound caller ID override (replaces From header user/display)
    /// e.g. "+15551234567" or "Company Name <+15551234567>"
    pub caller_id_override: Option<String>,

    /// Override domain in outbound From/Contact headers
    /// e.g. "sip.carrier.com" instead of local IP
    pub from_domain: Option<String>,

    /// Strip N leading digits from dialed number before sending
    pub strip_digits: i32,

    /// Prepend this string to dialed number after stripping
    /// e.g. "+1" for E.164 formatting
    pub prepend_digits: Option<String>,

    /// Maximum calls per second (0 = unlimited)
    pub cps_limit: i32,

    /// Free-text description/notes
    pub description: Option<String>,

    /// SRTP mode: "disabled", "optional", "mandatory"
    pub srtp_mode: String,

    /// DTMF mode: "rfc2833", "sip_info", "inband"
    pub dtmf_mode: String,

    /// RFC 4028 session timer interval in seconds (0 = disabled)
    pub session_timer_secs: i32,

    /// T.38 fax relay: enable/disable
    pub t38_enabled: bool,

    /// Outbound proxy (route through this proxy before reaching trunk)
    /// e.g. "sip:proxy.carrier.com:5060"
    pub outbound_proxy: Option<String>,

    /// Trunk group name for logical grouping (e.g. "US Carriers")
    pub trunk_group: Option<String>,

    /// Custom SIP header rules as JSON array
    /// e.g. [{"action":"add","name":"X-Custom","value":"foo"}]
    pub header_rules: Option<String>,
    /// Emergency calls (911) prefer trunks with this flag; a flagged trunk is
    /// tried before any LCR choice, in priority order.
    #[serde(default)]
    pub emergency_capable: bool,

    /// Created timestamp
    pub created_at: SystemTime,

    /// Last modified timestamp
    pub updated_at: SystemTime,
}

/// Aggregated statistics for a trunk computed from CDRs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrunkStats {
    /// Total calls through this trunk (last 24h)
    pub total_calls_24h: u64,
    /// Answered calls (last 24h)
    pub answered_calls_24h: u64,
    /// Answer-Seizure Ratio as percentage (0-100)
    pub asr: f64,
    /// Average Call Duration in seconds (answered calls only)
    pub acd_secs: f64,
    /// Average Post-Dial Delay in milliseconds
    pub pdd_ms: f64,
    /// Call volume per hour for last 24h (24 buckets, oldest first)
    pub hourly_volume: Vec<u64>,
}

impl Trunk {
    /// Create a new trunk with defaults, belonging to `tenant_id`.
    pub fn new(name: String, tenant_id: TenantId) -> Self {
        let now = SystemTime::now();
        Self {
            id: TrunkId::new(),
            tenant_id: Some(tenant_id),
            name,
            enabled: true,
            direction: TrunkDirection::Upstream,
            priority: 10,
            weight: 100,
            hosts: vec![],
            transport_protocol: "udp".to_string(),
            transport_port: 5060,
            verify_tls: true,
            auth_username: None,
            auth_password: None,
            auth_realm: None,
            register: false,
            register_expiry: 3600,
            max_concurrent_calls: 0,
            max_failures: 5,
            recovery_interval_secs: 60,
            health_check_enabled: true,
            health_check_interval_secs: 30,
            codec_prefs: vec![],
            caller_id_override: None,
            from_domain: None,
            strip_digits: 0,
            prepend_digits: None,
            cps_limit: 0,
            description: None,
            srtp_mode: "disabled".to_string(),
            dtmf_mode: "rfc2833".to_string(),
            session_timer_secs: 0,
            t38_enabled: false,
            outbound_proxy: None,
            trunk_group: None,
            header_rules: None,
            emergency_capable: false,
            created_at: now,
            updated_at: now,
        }
    }
}
