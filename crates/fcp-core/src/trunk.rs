//! SIP Trunk types for FCP.
//!
//! Trunks represent SIP connections to upstream carriers or downstream PBXes
//! for originating and terminating calls.

use crate::identifiers::{SiteId, TenantId};
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

/// Which end of a site's link trunk this row is
/// (`docs/HYBRID_CONTROL_PLANE.md` §3.6). The cloud makes one trunk per
/// site at enrollment, as its own end; the link, applying it at the site,
/// turns it into the site's end. An ordinary trunk has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkEnd {
    /// The cloud's end: the site registers here with the trunk's
    /// credentials, and calls to the site's numbers go down that
    /// registration. Never dialled, probed or registered from here.
    Cloud,
    /// The site's end: this deployment registers to `hosts` with the
    /// credentials, takes the cloud's calls on it, and sends the calls it
    /// cannot carry on a local trunk over it.
    Site,
}

/// Direction of a SIP trunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TrunkDirection {
    /// Outbound-only (send calls to carrier)
    #[default]
    #[serde(alias = "Upstream")]
    Upstream,
    /// Inbound-only (receive calls from carrier)
    #[serde(alias = "Downstream")]
    Downstream,
    /// Both inbound and outbound
    #[serde(alias = "Bidirectional")]
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

// ── What a trunk does with the calls it carries ──────────────────────────
//
// These were `fcp-sbc-core`'s: the shape a node's `[[trunks]]` TOML has
// always had. They live here so one type — [`Trunk`] — is both what a
// tenant manages through the API and what the call path holds, and a field
// the API accepts cannot quietly do nothing (`docs/TENANT_TRUNKS.md` §4).
// `fcp-sbc-core` re-exports them from their old paths.

/// Transport settings for a trunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkTransport {
    /// SIP transport protocol.
    pub protocol: crate::transport::TransportType,

    /// The port every host is dialled on unless the host names its own
    /// (`host:port`); unset means the protocol's default — 5060, or 5061
    /// for TLS (`docs/DEPLOYMENT_GAPS.md` §1.8).
    pub port: Option<u16>,

    /// Verify TLS certificates (only relevant for TLS/WSS).
    pub verify_tls: bool,
}

impl TrunkTransport {
    /// The port a host without one of its own is dialled on.
    pub fn effective_port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.protocol.default_port())
    }
}

/// One entry of a trunk's `hosts`, parsed: `10.0.0.1`, `10.0.0.1:5080`,
/// `sip.carrier.example`, `[2001:db8::1]:5061` and `2001:db8::1` are all
/// hosts. The one place `host:port` is read (`docs/DEPLOYMENT_GAPS.md`
/// §1.8): the router, both probes and inbound matching agree because they
/// all ask here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrunkHost {
    /// The name or address, without brackets.
    pub host: String,
    /// The host's own port, when it names one.
    pub port: Option<u16>,
}

impl TrunkHost {
    /// Parse one `hosts` entry. Refuses an empty host, a host with
    /// whitespace or CR/LF, a port that is not a number in 1–65535, and an
    /// IPv6 literal in brackets with something other than `:port` after them.
    pub fn parse(entry: &str) -> Result<Self, String> {
        let entry = entry.trim();
        if entry.is_empty() {
            return Err("host must not be empty".to_string());
        }
        if entry
            .chars()
            .any(|c| c.is_whitespace() || c == '\r' || c == '\n')
        {
            return Err(format!(
                "host {entry:?} must not contain whitespace or CR/LF characters"
            ));
        }
        let parse_port = |p: &str| -> Result<u16, String> {
            p.parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or_else(|| format!("host {entry:?}: port {p:?} must be 1–65535"))
        };
        if let Some(rest) = entry.strip_prefix('[') {
            let (host, after) = rest
                .split_once(']')
                .ok_or_else(|| format!("host {entry:?}: unclosed '['"))?;
            if host.is_empty() {
                return Err(format!("host {entry:?}: empty address in brackets"));
            }
            let port = match after {
                "" => None,
                p => Some(parse_port(p.strip_prefix(':').ok_or_else(|| {
                    format!("host {entry:?}: expected ':port' after ']'")
                })?)?),
            };
            return Ok(Self {
                host: host.to_string(),
                port,
            });
        }
        match entry.rsplit_once(':') {
            // One colon is a port; more is an IPv6 address without brackets,
            // which names no port.
            Some((host, port)) if !host.contains(':') => {
                if host.is_empty() {
                    return Err(format!("host {entry:?}: empty host before ':'"));
                }
                Ok(Self {
                    host: host.to_string(),
                    port: Some(parse_port(port)?),
                })
            }
            _ => Ok(Self {
                host: entry.to_string(),
                port: None,
            }),
        }
    }

    /// The host as a SIP URI writes it: an IPv6 literal in brackets.
    pub fn uri_host(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }

    /// The port this host is dialled on: its own, else `transport`'s.
    pub fn port_or(&self, transport: &TrunkTransport) -> u16 {
        self.port.unwrap_or_else(|| transport.effective_port())
    }
}

impl Default for TrunkTransport {
    fn default() -> Self {
        Self {
            protocol: crate::transport::TransportType::Udp,
            port: None,
            verify_tls: true,
        }
    }
}

/// Authentication credentials for a trunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkCredentials {
    /// Authentication username.
    pub username: String,

    /// Authentication password.
    pub password: String,

    /// Optional digest realm override.
    pub realm: Option<String>,
}

/// Failover policy for a trunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkFailover {
    /// Consecutive failures before the trunk is marked down.
    pub max_failures: u32,

    /// Seconds a down trunk waits before it is probed again.
    pub recovery_interval_secs: u64,

    /// Whether OPTIONS health checks are sent.
    pub health_check_enabled: bool,

    /// Seconds between OPTIONS health checks.
    pub health_check_interval_secs: u64,
}

impl Default for TrunkFailover {
    fn default() -> Self {
        Self {
            max_failures: 5,
            recovery_interval_secs: 30,
            health_check_enabled: true,
            health_check_interval_secs: 30,
        }
    }
}

impl TrunkFailover {
    /// Validate the failover policy.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_failures == 0 {
            return Err("failover.max_failures must be at least 1".into());
        }
        if self.recovery_interval_secs == 0 {
            return Err("failover.recovery_interval_secs must be at least 1".into());
        }
        if self.health_check_enabled && self.health_check_interval_secs == 0 {
            return Err("failover.health_check_interval_secs must be at least 1".into());
        }
        Ok(())
    }
}

/// SRTP mode for a trunk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SrtpMode {
    /// SRTP disabled — RTP/AVP only.
    #[default]
    #[serde(alias = "disabled")]
    Disabled,
    /// Prefer SRTP if the remote side offers it, but fall back to RTP.
    #[serde(alias = "preferred", alias = "prefer", alias = "optional")]
    Preferred,
    /// Require SRTP — reject calls that don't offer crypto.
    #[serde(alias = "required", alias = "require", alias = "mandatory")]
    Required,
}

impl SrtpMode {
    /// How the mode is stored and shown by the API.
    pub fn as_str(&self) -> &'static str {
        match self {
            SrtpMode::Disabled => "disabled",
            SrtpMode::Preferred => "preferred",
            SrtpMode::Required => "required",
        }
    }

    /// Read what the database and the API hold, in any case.
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "required" | "require" | "mandatory" => SrtpMode::Required,
            "preferred" | "prefer" | "optional" => SrtpMode::Preferred,
            _ => SrtpMode::Disabled,
        }
    }
}

/// SRTP configuration for a trunk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkSrtpConfig {
    /// SRTP mode.
    #[serde(default)]
    pub mode: SrtpMode,
    /// Allowed crypto suites (empty = all supported).
    #[serde(default)]
    pub allowed_suites: Vec<String>,
}

/// Codec configuration for a trunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkCodecConfig {
    /// Allowed codecs in preference order (e.g. `["PCMU", "PCMA"]`).
    /// Empty means no restriction.
    #[serde(default)]
    pub allowed_codecs: Vec<String>,

    /// Whether the media pipeline may transcode when the legs' codec sets
    /// do not overlap.
    #[serde(default = "default_allow_transcoding")]
    pub allow_transcoding: bool,
}

fn default_allow_transcoding() -> bool {
    true
}

impl Default for TrunkCodecConfig {
    fn default() -> Self {
        Self {
            allowed_codecs: Vec::new(),
            allow_transcoding: true,
        }
    }
}

/// DTMF handling mode for a trunk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrunkDtmfMode {
    /// DTMF detection and relay disabled.
    #[default]
    #[serde(alias = "disabled", alias = "inband")]
    Disabled,
    /// Detect DTMF events but do not relay them.
    #[serde(alias = "detect", alias = "sip_info", alias = "info")]
    Detect,
    /// Relay RFC 2833 telephone-event packets between legs.
    #[serde(alias = "relay", alias = "rfc2833")]
    Relay,
}

impl TrunkDtmfMode {
    /// How the mode is stored and shown by the API.
    pub fn as_str(&self) -> &'static str {
        match self {
            TrunkDtmfMode::Disabled => "inband",
            TrunkDtmfMode::Detect => "sip_info",
            TrunkDtmfMode::Relay => "rfc2833",
        }
    }

    /// Read what the database and the API hold, in any case.
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "rfc2833" | "relay" => TrunkDtmfMode::Relay,
            "sip_info" | "info" | "detect" => TrunkDtmfMode::Detect,
            _ => TrunkDtmfMode::Disabled,
        }
    }
}

/// DTMF configuration for a trunk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkDtmfConfig {
    /// DTMF handling mode.
    #[serde(default)]
    pub mode: TrunkDtmfMode,
}

/// Verification mode for inbound calls (STIR/SHAKEN, RFC 8224).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StirVerifyMode {
    #[default]
    #[serde(alias = "disabled")]
    Disabled,
    /// Verify and log the result, allow the call regardless.
    #[serde(alias = "log_only", alias = "logonly")]
    LogOnly,
    /// Verify, reject with 438 on failure.
    #[serde(alias = "enforce")]
    Enforce,
}

/// Signing mode for outbound calls (STIR/SHAKEN).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StirSignMode {
    #[default]
    #[serde(alias = "disabled")]
    Disabled,
    #[serde(alias = "enabled")]
    Enabled,
}

/// Attestation level (RFC 8588 §5.2.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AttestationLevel {
    /// Full — originator known and authorized for the calling number.
    #[default]
    A,
    /// Partial — originator known, number authorization unverified.
    B,
    /// Gateway — no relationship with the caller.
    C,
}

impl AttestationLevel {
    /// The single-character form used in a PASSporT.
    pub fn as_str(&self) -> &'static str {
        match self {
            AttestationLevel::A => "A",
            AttestationLevel::B => "B",
            AttestationLevel::C => "C",
        }
    }

    /// Parse the single-character form from a PASSporT claim.
    pub fn from_passport_str(s: &str) -> Option<Self> {
        match s {
            "A" => Some(AttestationLevel::A),
            "B" => Some(AttestationLevel::B),
            "C" => Some(AttestationLevel::C),
            _ => None,
        }
    }
}

fn default_max_iat_age() -> u64 {
    60
}

/// Per-trunk STIR/SHAKEN configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkStirShakenConfig {
    /// Verification mode for inbound calls on this trunk.
    #[serde(default)]
    pub verify_mode: StirVerifyMode,

    /// Signing mode for outbound calls on this trunk.
    #[serde(default)]
    pub sign_mode: StirSignMode,

    /// Attestation level used when signing outbound calls.
    #[serde(default)]
    pub attestation: AttestationLevel,

    /// Maximum age of the `iat` claim before a token is treated as expired.
    #[serde(default = "default_max_iat_age")]
    pub max_iat_age_secs: u64,
}

impl Default for TrunkStirShakenConfig {
    fn default() -> Self {
        Self {
            verify_mode: StirVerifyMode::default(),
            sign_mode: StirSignMode::default(),
            attestation: AttestationLevel::default(),
            max_iat_age_secs: default_max_iat_age(),
        }
    }
}

impl TrunkStirShakenConfig {
    /// Validate the per-trunk STIR/SHAKEN configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_iat_age_secs == 0 || self.max_iat_age_secs > 600 {
            return Err("stir_shaken.max_iat_age_secs must be between 1 and 600".into());
        }
        Ok(())
    }
}

/// A dialed-number transform attached to a trunk (egress direction).
///
/// Applied to the user part of the Request-URI when building the outbound
/// route: first the longest matching entry of `strip_prefixes` is removed,
/// then `prepend` is added.
///
/// Examples:
/// - Send E.164 without `+` to a carrier: `strip_prefixes = ["+"]`
/// - US carrier expecting 11 digits: `strip_prefixes = ["+1", "+"]`, `prepend = "1"`
///   (`+14155550100` → `14155550100`; `+44330030000` → `1443300030000` is
///   avoided because `+1` is checked first — order entries longest-first)
/// - Convert `00`-dialing to `+`: `strip_prefixes = ["00"]`, `prepend = "+"`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NumberTransform {
    /// Prefixes to strip; the longest matching entry wins. Optional.
    #[serde(default)]
    pub strip_prefixes: Vec<String>,

    /// Leading characters to strip when no prefix matched — the stored
    /// trunk's `strip_digits` (`docs/TENANT_TRUNKS.md` §4), which says how
    /// many to take off rather than which. Optional.
    #[serde(default)]
    pub strip_leading: usize,

    /// String prepended after stripping. Optional.
    #[serde(default)]
    pub prepend: String,
}

impl NumberTransform {
    /// Apply the transform to a dialed number (the URI user part).
    ///
    /// Non-numeric user parts (named users like `alice`) are returned
    /// unchanged: number transforms only apply to strings that consist of
    /// digits after an optional leading `+` and separators.
    pub fn apply(&self, number: &str) -> String {
        if !Self::looks_like_number(number) {
            return number.to_string();
        }

        let matched = self
            .strip_prefixes
            .iter()
            .filter(|p| !p.is_empty() && number.starts_with(p.as_str()))
            .max_by_key(|p| p.len())
            .map(|p| &number[p.len()..]);
        // A named prefix wins; `strip_leading` is what a trunk that only
        // says "take off two digits" gets.
        let stripped = match matched {
            Some(rest) => rest,
            None if self.strip_leading > 0 => {
                let take = number
                    .char_indices()
                    .nth(self.strip_leading)
                    .map(|(i, _)| i)
                    .unwrap_or(number.len());
                &number[take..]
            }
            None => number,
        };

        format!("{}{}", self.prepend, stripped)
    }

    /// Whether the transform changes anything.
    pub fn is_noop(&self) -> bool {
        self.strip_prefixes.is_empty() && self.strip_leading == 0 && self.prepend.is_empty()
    }

    /// Validate configuration values.
    pub fn validate(&self) -> Result<(), String> {
        for prefix in &self.strip_prefixes {
            if prefix.is_empty() {
                return Err("number_transform.strip_prefixes entries must not be empty".into());
            }
            if !prefix.chars().all(|c| c.is_ascii_digit() || c == '+') {
                return Err(format!(
                    "number_transform.strip_prefixes entry '{prefix}' may only contain digits and '+'"
                ));
            }
        }
        if !self
            .prepend
            .chars()
            .all(|c| c.is_ascii_digit() || c == '+' || c == '*' || c == '#')
        {
            return Err(format!(
                "number_transform.prepend '{}' may only contain digits, '+', '*', '#'",
                self.prepend
            ));
        }
        Ok(())
    }

    fn looks_like_number(user: &str) -> bool {
        let significant: Vec<char> = user
            .chars()
            .filter(|c| !matches!(c, '-' | '.' | '(' | ')' | ' '))
            .collect();
        match significant.split_first() {
            Some(('+', rest)) => !rest.is_empty() && rest.iter().all(|c| c.is_ascii_digit()),
            Some(_) => significant.iter().all(|c| c.is_ascii_digit()),
            None => false,
        }
    }
}

/// A SIP trunk: what a tenant manages through `/v1/trunks`, what a node
/// reads from its own `[[trunks]]`, and what the call path holds — one
/// model, so a field the API accepts cannot quietly do nothing
/// (`docs/TENANT_TRUNKS.md` §4).
///
/// The serde shape is the `[[trunks]]` TOML a node has always parsed:
/// every field a configuration file may leave out has a default. The
/// database stores the same trunk in its own columns; that mapping lives
/// in the storage backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trunk {
    /// Unique identifier. A trunk parsed from a node's own `[[trunks]]`
    /// has none of its own and is [`TrunkId::platform`] of its name; see
    /// [`Trunk::trunk_id`].
    #[serde(default = "unset_trunk_id")]
    pub id: TrunkId,

    /// The tenant whose calls this trunk carries. `None` is a platform
    /// trunk — one a node reads from its own `[[trunks]]`, shared by every
    /// tenant that has none of its own.
    #[serde(default)]
    pub tenant_id: Option<TenantId>,

    /// Human-readable name. A node's own trunks are unique by it.
    pub name: String,

    /// Whether the trunk is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Traffic direction.
    #[serde(default = "default_trunk_direction")]
    pub direction: TrunkDirection,

    /// Selection priority (lower = higher priority).
    #[serde(default = "default_priority")]
    pub priority: u32,

    /// Selection weight, for load balancing among equal priorities.
    #[serde(default = "default_weight")]
    pub weight: u32,

    /// Remote hosts (IP or FQDN).
    #[serde(default)]
    pub hosts: Vec<String>,

    /// Where the carrier's calls come from: IP addresses and CIDR ranges
    /// (`docs/TENANT_TRUNKS.md` §7). A carrier often sends from addresses
    /// it does not receive on. Empty means the trunk's `hosts`.
    #[serde(default)]
    pub inbound_sources: Vec<String>,
    /// The site this trunk runs at, in a hybrid deployment
    /// (`docs/HYBRID_CONTROL_PLANE.md` §2): a carrier of the site's own,
    /// or its link trunk. The cloud's nodes route by no trunk that has
    /// one; the link applies only its own site's to the site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_id: Option<SiteId>,
    /// Which end of a site's link this trunk is, when it is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkEnd>,

    /// Transport settings.
    #[serde(default)]
    pub transport: TrunkTransport,

    /// Digest credentials, when the carrier authenticates.
    #[serde(default)]
    pub credentials: Option<TrunkCredentials>,

    /// Whether to REGISTER with the remote peer.
    #[serde(default)]
    pub register: bool,

    /// Registration expiry in seconds (when `register` is true).
    #[serde(default)]
    pub register_expiry: Option<u32>,

    /// Failover and health-check policy.
    #[serde(default)]
    pub failover: TrunkFailover,

    /// Maximum concurrent calls (0 = unlimited).
    #[serde(default)]
    pub max_concurrent_calls: u32,

    /// Maximum calls per second (0 = unlimited).
    #[serde(default)]
    pub cps_limit: u32,

    /// Names of the HMR rules applied on this trunk.
    #[serde(default)]
    pub header_rules: Vec<String>,

    /// Emergency calls are routed to trunks with this flag first, in
    /// priority order, before any LCR choice.
    #[serde(default)]
    pub emergency_capable: bool,

    /// How an emergency call's location travels on this trunk
    /// (`docs/E911.md`): `elin`, `pidf_lo`, or both.
    #[serde(default = "default_emergency_location")]
    pub emergency_location: Vec<crate::e911::LocationDelivery>,

    /// SRTP settings.
    #[serde(default)]
    pub srtp: TrunkSrtpConfig,

    /// DTMF settings.
    #[serde(default)]
    pub dtmf: TrunkDtmfConfig,

    /// Allowed codecs and whether this trunk may be transcoded for.
    #[serde(default)]
    pub codec: TrunkCodecConfig,

    /// STIR/SHAKEN verification and signing.
    #[serde(default)]
    pub stir_shaken: TrunkStirShakenConfig,

    /// The digit plan applied to the dialed number as it leaves on this
    /// trunk.
    #[serde(default)]
    pub egress_number_transform: Option<NumberTransform>,

    /// Outbound caller ID override (replaces the From user/display).
    #[serde(default)]
    pub caller_id_override: Option<String>,

    /// Domain used in outbound From/Contact headers.
    #[serde(default)]
    pub from_domain: Option<String>,

    /// RFC 4028 session timer interval in seconds (0 = disabled).
    #[serde(default)]
    pub session_timer_secs: u32,

    /// T.38 fax relay.
    #[serde(default)]
    pub t38_enabled: bool,

    /// Outbound proxy the call is routed through before the trunk's hosts.
    #[serde(default)]
    pub outbound_proxy: Option<String>,

    /// Logical grouping, which a routing rule can name.
    #[serde(default)]
    pub trunk_group: Option<String>,

    /// Free-text description.
    #[serde(default)]
    pub description: Option<String>,

    /// Created timestamp.
    #[serde(default = "SystemTime::now")]
    pub created_at: SystemTime,

    /// Last modified timestamp.
    #[serde(default = "SystemTime::now")]
    pub updated_at: SystemTime,
}

/// What a trunk read from a configuration file has instead of a stored id.
fn unset_trunk_id() -> TrunkId {
    TrunkId::from_string(String::new())
}

fn default_true() -> bool {
    true
}

/// What a `[[trunks]]` entry that does not say has always meant.
fn default_trunk_direction() -> TrunkDirection {
    TrunkDirection::Bidirectional
}

fn default_priority() -> u32 {
    100
}

fn default_weight() -> u32 {
    100
}

fn default_emergency_location() -> Vec<crate::e911::LocationDelivery> {
    vec![crate::e911::LocationDelivery::Elin]
}

impl Default for Trunk {
    fn default() -> Self {
        let now = SystemTime::now();
        Self {
            id: TrunkId::new(),
            tenant_id: None,
            name: String::new(),
            enabled: true,
            direction: default_trunk_direction(),
            priority: default_priority(),
            weight: default_weight(),
            hosts: Vec::new(),
            inbound_sources: Vec::new(),
            site_id: None,
            link: None,
            transport: TrunkTransport::default(),
            credentials: None,
            register: false,
            register_expiry: None,
            failover: TrunkFailover::default(),
            max_concurrent_calls: 0,
            cps_limit: 0,
            header_rules: Vec::new(),
            emergency_capable: false,
            emergency_location: default_emergency_location(),
            srtp: TrunkSrtpConfig::default(),
            dtmf: TrunkDtmfConfig::default(),
            codec: TrunkCodecConfig::default(),
            stir_shaken: TrunkStirShakenConfig::default(),
            egress_number_transform: None,
            caller_id_override: None,
            from_domain: None,
            session_timer_secs: 0,
            t38_enabled: false,
            outbound_proxy: None,
            trunk_group: None,
            description: None,
            created_at: now,
            updated_at: now,
        }
    }
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

/// One of a trunk's `inbound_sources`: an address, or a CIDR range.
pub fn parse_inbound_source(source: &str) -> Result<ipnetwork::IpNetwork, String> {
    let source = source.trim();
    source
        .parse::<ipnetwork::IpNetwork>()
        .map_err(|_| format!("'{source}' is not an IP address or CIDR range"))
}

impl Trunk {
    /// Whether this trunk's carrier may send it calls.
    pub fn takes_inbound(&self) -> bool {
        self.enabled && self.direction != TrunkDirection::Upstream
    }

    /// The ranges `inbound_sources` names; entries that do not parse are
    /// skipped (the API refuses them, so only a hand-edited row has any).
    pub fn inbound_networks(&self) -> Vec<ipnetwork::IpNetwork> {
        self.inbound_sources
            .iter()
            .filter_map(|s| parse_inbound_source(s).ok())
            .collect()
    }

    /// A new trunk belonging to `tenant_id`, with the defaults the API
    /// applies when a field is not given.
    pub fn new(name: String, tenant_id: TenantId) -> Self {
        Self {
            name,
            tenant_id: Some(tenant_id),
            // A carrier carries calls both ways unless a tenant says
            // otherwise — as a `[[trunks]]` entry that says nothing does.
            // Inbound admission honours direction (§7), so an outbound-only
            // default would refuse the carrier's calls.
            direction: TrunkDirection::Bidirectional,
            priority: 10,
            dtmf: TrunkDtmfConfig {
                mode: TrunkDtmfMode::Relay,
            },
            failover: TrunkFailover {
                max_failures: 5,
                recovery_interval_secs: 60,
                health_check_enabled: true,
                health_check_interval_secs: 30,
            },
            register_expiry: Some(3600),
            transport: TrunkTransport {
                protocol: crate::transport::TransportType::Udp,
                // The protocol's default, whichever protocol it ends up with.
                port: None,
                verify_tls: true,
            },
            ..Default::default()
        }
    }

    /// What this trunk is identified by everywhere its state is kept —
    /// health, capacity, CPS, registration, and the call's record. Two
    /// tenants may both call a trunk `carrier-a`; they never share a key.
    pub fn trunk_id(&self) -> TrunkId {
        // A trunk read from a node's own `[[trunks]]` has no stored id:
        // it is identified by its name, the same way on every node.
        if self.id.as_str().is_empty() {
            return TrunkId::platform(&self.name);
        }
        self.id.clone()
    }

    /// The cloud's end of a site's link (`docs/HYBRID_CONTROL_PLANE.md`
    /// §3.6): a peer that registers here, reached down its registration
    /// and never dialled, probed or registered.
    pub fn is_link_peer(&self) -> bool {
        self.link == Some(LinkEnd::Cloud)
    }

    /// The site's end of its link: the trunk this deployment registers to
    /// the cloud with, and sends the cloud's calls over.
    pub fn is_link_to_cloud(&self) -> bool {
        self.link == Some(LinkEnd::Site)
    }

    /// Whether this deployment, which is `site` (or the cloud, `None`),
    /// routes calls out on this trunk: a trunk runs where its `site_id`
    /// says, and a link peer is reached by its registration, not routed
    /// to.
    pub fn routes_from(&self, site: Option<SiteId>) -> bool {
        !self.is_link_peer() && self.site_id == site
    }

    /// [`Trunk::trunk_id`] as the string the runtime maps key on.
    pub fn key(&self) -> String {
        self.trunk_id().to_string()
    }

    /// Whether this trunk carries `tenant`'s calls: its own trunk, or a
    /// platform trunk, which carries anyone's.
    pub fn serves(&self, tenant: Option<TenantId>) -> bool {
        match (self.tenant_id, tenant) {
            (None, _) => true,
            (Some(owner), Some(caller)) => owner == caller,
            (Some(_), None) => false,
        }
    }

    /// Validate a trunk as read from a configuration file or the API.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("trunk name must not be empty".into());
        }
        if self.name.contains('\r') || self.name.contains('\n') {
            return Err("trunk name must not contain CR/LF characters".into());
        }
        if self.hosts.is_empty() {
            return Err(format!("trunk {:?}: hosts must not be empty", self.name));
        }
        for (i, host) in self.hosts.iter().enumerate() {
            TrunkHost::parse(host).map_err(|e| format!("trunk {:?}: host {i}: {e}", self.name))?;
        }
        if self.weight > 100 {
            return Err(format!("trunk {:?}: weight must not exceed 100", self.name));
        }
        if let Some(expiry) = self.register_expiry {
            if !(60..=86400).contains(&expiry) {
                return Err(format!(
                    "trunk {:?}: register_expiry must be between 60 and 86400",
                    self.name
                ));
            }
        }
        if let Some(transform) = &self.egress_number_transform {
            transform
                .validate()
                .map_err(|e| format!("trunk {:?}: {e}", self.name))?;
        }
        if let Some(creds) = &self.credentials {
            if creds.username.is_empty() {
                return Err(format!(
                    "trunk {:?}: credentials username must not be empty",
                    self.name
                ));
            }
        }
        for (i, codec_name) in self.codec.allowed_codecs.iter().enumerate() {
            if codec_name.is_empty() {
                return Err(format!(
                    "trunk {:?}: codec.allowed_codecs[{i}] must not be empty",
                    self.name
                ));
            }
            if codec_name.contains('\r') || codec_name.contains('\n') {
                return Err(format!(
                    "trunk {:?}: codec.allowed_codecs[{i}] must not contain CR/LF characters",
                    self.name
                ));
            }
        }
        self.failover
            .validate()
            .map_err(|e| format!("trunk {:?}: {e}", self.name))?;
        self.stir_shaken
            .validate()
            .map_err(|e| format!("trunk {:?}: {e}", self.name))
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;

    #[test]
    fn a_trunk_host_is_read_one_way() {
        let h = TrunkHost::parse("10.0.0.1").unwrap();
        assert_eq!((h.host.as_str(), h.port), ("10.0.0.1", None));
        let h = TrunkHost::parse(" sip.carrier.example:5080 ").unwrap();
        assert_eq!(
            (h.host.as_str(), h.port),
            ("sip.carrier.example", Some(5080))
        );
        let h = TrunkHost::parse("[2001:db8::1]:5061").unwrap();
        assert_eq!((h.host.as_str(), h.port), ("2001:db8::1", Some(5061)));
        assert_eq!(h.uri_host(), "[2001:db8::1]");
        let h = TrunkHost::parse("2001:db8::1").unwrap();
        assert_eq!((h.host.as_str(), h.port), ("2001:db8::1", None));
        for bad in [
            "",
            "sip.example.com:",
            ":5060",
            "a b",
            "host:0",
            "host:99999",
            "host:abc",
            "[::1",
            "[::1]5060",
        ] {
            assert!(TrunkHost::parse(bad).is_err(), "{bad:?} parsed");
        }
        let tls = TrunkTransport {
            protocol: crate::transport::TransportType::Tls,
            port: None,
            verify_tls: true,
        };
        assert_eq!(tls.effective_port(), 5061);
        assert_eq!(TrunkHost::parse("a").unwrap().port_or(&tls), 5061);
        assert_eq!(TrunkHost::parse("a:5070").unwrap().port_or(&tls), 5070);
    }

    #[test]
    fn a_trunk_with_a_bad_host_does_not_validate() {
        let mut trunk = Trunk::new("carrier".to_string(), crate::TenantId::nil());
        trunk.hosts = vec!["good.example".into(), "bad host".into()];
        let err = trunk.validate().unwrap_err();
        assert!(err.contains("host 1"), "{err}");
        trunk.hosts = vec!["good.example:5080".into()];
        assert!(trunk.validate().is_ok());
        assert_eq!(trunk.transport.port, None, "the protocol's default");
    }
}
