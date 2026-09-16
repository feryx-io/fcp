//! Trunk/carrier configuration for the SBC.
//!
//! Defines trunk direction, transport settings, credentials, failover
//! policy, and the main `TrunkConfig` struct.

use crate::stir_shaken::TrunkStirShakenConfig;
use fcp_core::transport::TransportType;
use serde::{Deserialize, Serialize};

/// Direction of traffic for a trunk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrunkDirection {
    /// Upstream (outbound to carrier/PSTN).
    Upstream,

    /// Downstream (inbound from carrier/users).
    Downstream,

    /// Both directions.
    #[default]
    Bidirectional,
}

/// Transport settings for a trunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkTransport {
    /// SIP transport protocol.
    pub protocol: TransportType,

    /// Optional explicit port (otherwise uses protocol default).
    pub port: Option<u16>,

    /// Verify TLS certificates (only relevant for TLS/WSS).
    pub verify_tls: bool,
}

impl Default for TrunkTransport {
    fn default() -> Self {
        Self {
            protocol: TransportType::Udp,
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
    /// Number of consecutive failures before marking trunk as down.
    pub max_failures: u32,

    /// Seconds before retrying a failed trunk.
    pub recovery_interval_secs: u64,

    /// Whether to send periodic OPTIONS health checks.
    pub health_check_enabled: bool,

    /// Seconds between health check probes.
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
    /// Validate the failover configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_failures == 0 {
            return Err("failover max_failures must be greater than zero".into());
        }
        if self.max_failures > 1000 {
            return Err("failover max_failures must not exceed 1000".into());
        }
        if self.recovery_interval_secs == 0 || self.recovery_interval_secs > 86400 {
            return Err("failover recovery_interval_secs must be between 1 and 86400".into());
        }
        if self.health_check_enabled
            && (self.health_check_interval_secs == 0 || self.health_check_interval_secs > 3600)
        {
            return Err(
                "failover health_check_interval_secs must be between 1 and 3600 when enabled"
                    .into(),
            );
        }
        Ok(())
    }
}

/// SRTP policy for a trunk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SrtpMode {
    /// SRTP disabled — RTP/AVP only
    #[default]
    Disabled,
    /// Prefer SRTP if the remote side offers it, but fall back to RTP
    Preferred,
    /// Require SRTP — reject calls that don't offer crypto
    Required,
}

/// SRTP configuration for a trunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkSrtpConfig {
    /// SRTP mode
    pub mode: SrtpMode,
    /// Allowed crypto suites (empty = all supported)
    #[serde(default)]
    pub allowed_suites: Vec<String>,
}

impl Default for TrunkSrtpConfig {
    fn default() -> Self {
        Self {
            mode: SrtpMode::Disabled,
            allowed_suites: Vec::new(),
        }
    }
}

/// Codec configuration for a trunk.
///
/// Controls which audio codecs are allowed on this trunk and whether
/// transcoding is permitted when codec negotiation would otherwise fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkCodecConfig {
    /// Allowed codecs in preference order (e.g., `["PCMU", "PCMA"]`).
    /// Empty means no restriction — all codecs are allowed.
    #[serde(default)]
    pub allowed_codecs: Vec<String>,

    /// Whether the media pipeline may transcode when the A-leg and B-leg
    /// codec sets don't overlap. When `false`, calls that require transcoding
    /// will be rejected.
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
    /// DTMF detection and relay disabled
    #[default]
    Disabled,
    /// Detect DTMF events but don't relay (publish to EventBus only)
    Detect,
    /// Relay RFC 2833 telephone-event packets between legs
    Relay,
}

/// DTMF configuration for a trunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkDtmfConfig {
    /// DTMF handling mode
    pub mode: TrunkDtmfMode,
}

impl Default for TrunkDtmfConfig {
    fn default() -> Self {
        Self {
            mode: TrunkDtmfMode::Disabled,
        }
    }
}

/// A trunk (carrier) as the call path sees it: what a node reads from its
/// own `[[trunks]]`, and what a tenant's stored trunk becomes
/// (`From<&fcp_core::Trunk>`, `docs/TENANT_TRUNKS.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkConfig {
    /// Trunk name: what people read, and what a node's own trunks are
    /// unique by.
    pub name: String,

    /// The stored trunk this came from. A trunk a node read from its own
    /// TOML has none, and is identified by `TrunkId::platform(name)`; see
    /// [`TrunkConfig::key`].
    #[serde(skip)]
    pub id: Option<fcp_core::TrunkId>,

    /// The tenant whose calls this trunk carries. `None` is a platform
    /// trunk, shared by every tenant that has none of its own.
    #[serde(skip)]
    pub tenant_id: Option<fcp_core::TenantId>,

    /// Whether this trunk is enabled.
    pub enabled: bool,

    /// Traffic direction.
    pub direction: TrunkDirection,

    /// Priority for trunk selection (lower = higher priority).
    pub priority: u32,

    /// Weight for weighted round-robin selection among equal-priority trunks.
    pub weight: u32,

    /// Remote host addresses (IP or FQDN).
    pub hosts: Vec<String>,

    /// Transport settings.
    pub transport: TrunkTransport,

    /// Optional authentication credentials.
    pub credentials: Option<TrunkCredentials>,

    /// Whether to REGISTER with the remote peer.
    pub register: bool,

    /// Registration expiry in seconds (when `register` is true).
    pub register_expiry: Option<u32>,

    /// Failover policy.
    pub failover: TrunkFailover,

    /// Maximum concurrent calls on this trunk (0 = unlimited).
    pub max_concurrent_calls: u32,

    /// Names of HMR rules to apply for this trunk (empty = use all matching global rules).
    #[serde(default)]
    pub header_rules: Vec<String>,
    /// Emergency calls (911) are routed to trunks with this flag first, in
    /// priority order, before falling back to the normal LCR choice.
    #[serde(default)]
    pub emergency_capable: bool,

    /// How an emergency call's location travels on this trunk
    /// (`docs/E911_BLOCK_2.md` §5): `elin`, the callback number as caller
    /// ID, which the carrier maps to an address; `pidf_lo`, the location by
    /// value in the INVITE (RFC 6442); or both. The callback number goes out
    /// either way. Default `["elin"]`.
    #[serde(default = "default_emergency_location")]
    pub emergency_location: Vec<fcp_core::e911::LocationDelivery>,

    /// SRTP configuration for this trunk
    #[serde(default)]
    pub srtp: TrunkSrtpConfig,

    /// DTMF configuration for this trunk
    #[serde(default)]
    pub dtmf: TrunkDtmfConfig,

    /// Codec configuration for this trunk
    #[serde(default)]
    pub codec: TrunkCodecConfig,

    /// STIR/SHAKEN caller ID verification and signing configuration
    #[serde(default)]
    pub stir_shaken: TrunkStirShakenConfig,

    /// Egress dialed-number transform: applied to the Request-URI user part
    /// when this trunk is selected as the outbound route (transform stage).
    #[serde(default)]
    pub egress_number_transform: Option<crate::number_transform::NumberTransform>,
}

fn default_emergency_location() -> Vec<fcp_core::e911::LocationDelivery> {
    vec![fcp_core::e911::LocationDelivery::Elin]
}

impl TrunkConfig {
    /// What this trunk is identified by everywhere its state is kept —
    /// health, capacity, CPS, registration, and the call's record. Two
    /// tenants may both call a trunk `carrier-a`; they never share a key.
    pub fn trunk_id(&self) -> fcp_core::TrunkId {
        self.id
            .clone()
            .unwrap_or_else(|| fcp_core::TrunkId::platform(&self.name))
    }

    /// [`TrunkConfig::trunk_id`] as the string the runtime maps key on.
    pub fn key(&self) -> String {
        self.trunk_id().to_string()
    }

    /// Whether this trunk carries `tenant`'s calls: its own trunk, or a
    /// platform trunk, which carries anyone's.
    pub fn serves(&self, tenant: Option<fcp_core::TenantId>) -> bool {
        match (self.tenant_id, tenant) {
            (None, _) => true,
            (Some(owner), Some(caller)) => owner == caller,
            (Some(_), None) => false,
        }
    }
}

impl Default for TrunkConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: None,
            tenant_id: None,
            enabled: true,
            direction: TrunkDirection::default(),
            priority: 100,
            weight: 100,
            hosts: Vec::new(),
            transport: TrunkTransport::default(),
            credentials: None,
            register: false,
            register_expiry: None,
            failover: TrunkFailover::default(),
            max_concurrent_calls: 0,
            header_rules: Vec::new(),
            emergency_capable: false,
            emergency_location: default_emergency_location(),
            srtp: TrunkSrtpConfig::default(),
            dtmf: TrunkDtmfConfig::default(),
            codec: TrunkCodecConfig::default(),
            stir_shaken: TrunkStirShakenConfig::default(),
            egress_number_transform: None,
        }
    }
}

impl TrunkConfig {
    /// Validate the trunk configuration.
    ///
    /// Returns `Err` if:
    /// - `name` is empty or contains CR/LF
    /// - `hosts` is empty
    /// - `weight` exceeds 100
    /// - `register_expiry` is set but not in 60..=86400
    /// - `credentials` username is empty
    /// - Failover validation fails
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
            if host.is_empty() {
                return Err(format!("trunk {:?}: host {i} must not be empty", self.name));
            }
            if host.contains('\r') || host.contains('\n') {
                return Err(format!(
                    "trunk {:?}: host {i} must not contain CR/LF characters",
                    self.name
                ));
            }
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
        if let Some(ref transform) = self.egress_number_transform {
            transform
                .validate()
                .map_err(|e| format!("trunk {:?}: {e}", self.name))?;
        }
        if let Some(ref creds) = self.credentials {
            if creds.username.is_empty() {
                return Err(format!(
                    "trunk {:?}: credentials username must not be empty",
                    self.name
                ));
            }
        }
        // Validate codec names (no empty strings, no CR/LF)
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
            .map_err(|e| format!("trunk {:?}: {e}", self.name))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_trunk() -> TrunkConfig {
        TrunkConfig {
            name: "carrier-a".into(),
            hosts: vec!["sip.carrier.com".into()],
            ..Default::default()
        }
    }

    #[test]
    fn test_default() {
        let cfg = TrunkConfig::default();
        assert!(cfg.name.is_empty());
        assert!(cfg.enabled);
        assert_eq!(cfg.direction, TrunkDirection::Bidirectional);
        assert_eq!(cfg.priority, 100);
        assert_eq!(cfg.weight, 100);
        assert!(cfg.hosts.is_empty());
        assert_eq!(cfg.transport.protocol, TransportType::Udp);
        assert!(cfg.transport.verify_tls);
        assert!(cfg.credentials.is_none());
        assert!(!cfg.register);
        assert!(cfg.register_expiry.is_none());
        assert_eq!(cfg.failover.max_failures, 5);
        assert_eq!(cfg.max_concurrent_calls, 0);
    }

    #[test]
    fn test_validate_valid() {
        assert!(valid_trunk().validate().is_ok());
    }

    #[test]
    fn test_validate_empty_name() {
        let mut cfg = valid_trunk();
        cfg.name = String::new();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("name"));
    }

    #[test]
    fn test_validate_crlf_name() {
        let mut cfg = valid_trunk();
        cfg.name = "bad\r\nname".into();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_empty_hosts() {
        let mut cfg = valid_trunk();
        cfg.hosts.clear();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("hosts"));
    }

    #[test]
    fn test_validate_empty_host_entry() {
        let mut cfg = valid_trunk();
        cfg.hosts.push(String::new());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("host"));
    }

    #[test]
    fn test_validate_crlf_host() {
        let mut cfg = valid_trunk();
        cfg.hosts = vec!["bad\nhost".into()];
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_excessive_weight() {
        let mut cfg = valid_trunk();
        cfg.weight = 101;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("weight"));
    }

    #[test]
    fn test_validate_register_expiry_too_low() {
        let mut cfg = valid_trunk();
        cfg.register_expiry = Some(59);
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("register_expiry"));
    }

    #[test]
    fn test_validate_register_expiry_too_high() {
        let mut cfg = valid_trunk();
        cfg.register_expiry = Some(86401);
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("register_expiry"));
    }

    #[test]
    fn test_validate_empty_credentials_username() {
        let mut cfg = valid_trunk();
        cfg.credentials = Some(TrunkCredentials {
            username: String::new(),
            password: "secret".into(),
            realm: None,
        });
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("username"));
    }

    #[test]
    fn test_validate_failover_zero_max_failures() {
        let mut cfg = valid_trunk();
        cfg.failover.max_failures = 0;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_failures"));
    }

    #[test]
    fn test_validate_boundary_values() {
        let mut cfg = valid_trunk();

        cfg.weight = 0;
        assert!(cfg.validate().is_ok());
        cfg.weight = 100;
        assert!(cfg.validate().is_ok());

        cfg.register_expiry = Some(60);
        assert!(cfg.validate().is_ok());
        cfg.register_expiry = Some(86400);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_with_credentials() {
        let mut cfg = valid_trunk();
        cfg.credentials = Some(TrunkCredentials {
            username: "user".into(),
            password: "pass".into(),
            realm: Some("example.com".into()),
        });
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_srtp_mode_serde() {
        let modes = vec![SrtpMode::Disabled, SrtpMode::Preferred, SrtpMode::Required];
        for mode in modes {
            let json = serde_json::to_string(&mode).unwrap();
            let deserialized: SrtpMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, deserialized);
        }
    }

    #[test]
    fn test_trunk_config_with_srtp() {
        let cfg = TrunkConfig {
            name: "secure-trunk".into(),
            hosts: vec!["sip.secure.com".into()],
            srtp: TrunkSrtpConfig {
                mode: SrtpMode::Required,
                allowed_suites: vec!["AES_CM_128_HMAC_SHA1_80".into()],
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: TrunkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
        assert_eq!(deserialized.srtp.mode, SrtpMode::Required);
        assert_eq!(deserialized.srtp.allowed_suites.len(), 1);
    }

    #[test]
    fn test_trunk_config_srtp_default() {
        let cfg = TrunkConfig::default();
        assert_eq!(cfg.srtp.mode, SrtpMode::Disabled);
        assert!(cfg.srtp.allowed_suites.is_empty());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = TrunkConfig {
            name: "carrier-a".into(),
            hosts: vec!["sip.carrier.com".into(), "sip2.carrier.com".into()],
            transport: TrunkTransport {
                protocol: TransportType::Tls,
                port: Some(5061),
                verify_tls: true,
            },
            credentials: Some(TrunkCredentials {
                username: "user".into(),
                password: "pass".into(),
                realm: Some("carrier.com".into()),
            }),
            register: true,
            register_expiry: Some(3600),
            direction: TrunkDirection::Upstream,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: TrunkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_dtmf_mode_serde() {
        let modes = vec![
            TrunkDtmfMode::Disabled,
            TrunkDtmfMode::Detect,
            TrunkDtmfMode::Relay,
        ];
        for mode in modes {
            let json = serde_json::to_string(&mode).unwrap();
            let deserialized: TrunkDtmfMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, deserialized);
        }
    }

    #[test]
    fn test_trunk_config_with_dtmf() {
        let cfg = TrunkConfig {
            name: "dtmf-trunk".into(),
            hosts: vec!["sip.example.com".into()],
            dtmf: TrunkDtmfConfig {
                mode: TrunkDtmfMode::Relay,
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: TrunkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
        assert_eq!(deserialized.dtmf.mode, TrunkDtmfMode::Relay);
    }

    #[test]
    fn test_trunk_config_dtmf_default() {
        let cfg = TrunkConfig::default();
        assert_eq!(cfg.dtmf.mode, TrunkDtmfMode::Disabled);
    }

    // ── TrunkCodecConfig tests ──────────────────────────────────────────

    #[test]
    fn test_codec_config_default() {
        let cfg = TrunkCodecConfig::default();
        assert!(cfg.allowed_codecs.is_empty());
        assert!(cfg.allow_transcoding);
    }

    #[test]
    fn test_codec_config_serde_roundtrip() {
        let cfg = TrunkCodecConfig {
            allowed_codecs: vec!["PCMU".into(), "PCMA".into()],
            allow_transcoding: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: TrunkCodecConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
        assert_eq!(deserialized.allowed_codecs.len(), 2);
        assert!(!deserialized.allow_transcoding);
    }

    #[test]
    fn test_codec_config_serde_defaults() {
        // When codec section is missing from JSON, defaults should apply
        let json = r#"{"allowed_codecs":["G722"]}"#;
        let cfg: TrunkCodecConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.allowed_codecs, vec!["G722"]);
        assert!(cfg.allow_transcoding); // default = true
    }

    #[test]
    fn test_trunk_config_codec_default() {
        let cfg = TrunkConfig::default();
        assert!(cfg.codec.allowed_codecs.is_empty());
        assert!(cfg.codec.allow_transcoding);
    }

    #[test]
    fn test_trunk_config_with_codec() {
        let cfg = TrunkConfig {
            name: "codec-trunk".into(),
            hosts: vec!["sip.carrier.com".into()],
            codec: TrunkCodecConfig {
                allowed_codecs: vec!["PCMU".into(), "PCMA".into()],
                allow_transcoding: true,
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: TrunkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
        assert_eq!(deserialized.codec.allowed_codecs.len(), 2);
    }

    #[test]
    fn test_validate_codec_empty_name() {
        let mut cfg = valid_trunk();
        cfg.codec.allowed_codecs = vec!["PCMU".into(), "".into()];
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("allowed_codecs"));
    }

    #[test]
    fn test_validate_codec_crlf_name() {
        let mut cfg = valid_trunk();
        cfg.codec.allowed_codecs = vec!["bad\ncodec".into()];
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_codec_valid() {
        let mut cfg = valid_trunk();
        cfg.codec.allowed_codecs = vec!["PCMU".into(), "PCMA".into(), "G722".into()];
        assert!(cfg.validate().is_ok());
    }
}

// ── A stored trunk becomes a runtime one ─────────────────────────────────

/// The one place the model a tenant manages (`fcp_core::Trunk`, what
/// `/v1/trunks` reads and writes) becomes the model the call path uses
/// (`docs/TENANT_TRUNKS.md` §4).
///
/// Fields the stored trunk does not have yet — the structured SRTP suites,
/// STIR/SHAKEN modes, the emergency location — take the same defaults a
/// TOML trunk gets when it leaves them out, so a stored trunk behaves like
/// a plainly-written one until the columns arrive with the phases that
/// need them.
impl From<&fcp_core::Trunk> for TrunkConfig {
    fn from(trunk: &fcp_core::Trunk) -> Self {
        let credentials = trunk
            .auth_username
            .as_ref()
            .map(|username| TrunkCredentials {
                username: username.clone(),
                password: trunk.auth_password.clone().unwrap_or_default(),
                realm: trunk.auth_realm.clone(),
            });
        // The stored trunk says "strip these many digits, then prepend
        // this"; the transform stage speaks in prefixes and a prepend.
        let strip = trunk.strip_digits.max(0) as usize;
        let prepend = trunk.prepend_digits.clone().unwrap_or_default();
        let egress_number_transform =
            (strip > 0 || !prepend.is_empty()).then(|| crate::number_transform::NumberTransform {
                strip_prefixes: Vec::new(),
                strip_leading: strip,
                prepend,
            });
        Self {
            name: trunk.name.clone(),
            id: Some(trunk.id.clone()),
            tenant_id: trunk.tenant_id,
            enabled: trunk.enabled,
            direction: match trunk.direction {
                fcp_core::TrunkDirection::Upstream => TrunkDirection::Upstream,
                fcp_core::TrunkDirection::Downstream => TrunkDirection::Downstream,
                fcp_core::TrunkDirection::Bidirectional => TrunkDirection::Bidirectional,
            },
            priority: trunk.priority.max(0) as u32,
            weight: trunk.weight.max(0) as u32,
            hosts: trunk.hosts.clone(),
            transport: TrunkTransport {
                protocol: match trunk.transport_protocol.to_ascii_lowercase().as_str() {
                    "tcp" => fcp_core::transport::TransportType::Tcp,
                    "tls" => fcp_core::transport::TransportType::Tls,
                    "ws" => fcp_core::transport::TransportType::Ws,
                    "wss" => fcp_core::transport::TransportType::Wss,
                    _ => fcp_core::transport::TransportType::Udp,
                },
                port: u16::try_from(trunk.transport_port).ok().filter(|p| *p > 0),
                verify_tls: trunk.verify_tls,
            },
            credentials,
            register: trunk.register,
            register_expiry: u32::try_from(trunk.register_expiry).ok().filter(|e| *e > 0),
            failover: TrunkFailover {
                max_failures: trunk.max_failures.max(1) as u32,
                recovery_interval_secs: trunk.recovery_interval_secs.max(1) as u64,
                health_check_enabled: trunk.health_check_enabled,
                health_check_interval_secs: trunk.health_check_interval_secs.max(1) as u64,
            },
            max_concurrent_calls: trunk.max_concurrent_calls.max(0) as u32,
            // The stored form is one comma-separated list of rule names.
            header_rules: trunk
                .header_rules
                .as_deref()
                .map(|rules| {
                    rules
                        .split(',')
                        .map(str::trim)
                        .filter(|r| !r.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            emergency_capable: trunk.emergency_capable,
            emergency_location: default_emergency_location(),
            srtp: TrunkSrtpConfig {
                mode: match trunk.srtp_mode.to_ascii_lowercase().as_str() {
                    "required" | "require" => SrtpMode::Required,
                    "preferred" | "prefer" => SrtpMode::Preferred,
                    _ => SrtpMode::Disabled,
                },
                allowed_suites: Vec::new(),
            },
            dtmf: TrunkDtmfConfig {
                mode: match trunk.dtmf_mode.to_ascii_lowercase().as_str() {
                    "rfc2833" | "relay" => TrunkDtmfMode::Relay,
                    "detect" | "info" => TrunkDtmfMode::Detect,
                    _ => TrunkDtmfMode::Disabled,
                },
            },
            codec: TrunkCodecConfig {
                allowed_codecs: trunk.codec_prefs.clone(),
                allow_transcoding: true,
            },
            stir_shaken: TrunkStirShakenConfig::default(),
            egress_number_transform,
        }
    }
}

#[cfg(test)]
mod tenant_trunk_tests {
    use super::*;
    use fcp_core::{TenantId, Trunk, TrunkId};

    fn stored(name: &str, tenant: TenantId) -> Trunk {
        let mut trunk = Trunk::new(name.to_string(), tenant);
        trunk.hosts = vec!["carrier.example.net".to_string()];
        trunk.transport_protocol = "tls".to_string();
        trunk.transport_port = 5061;
        trunk.auth_username = Some("fcp".to_string());
        trunk.auth_password = Some("s3cret".to_string());
        trunk.strip_digits = 2;
        trunk.prepend_digits = Some("+1".to_string());
        trunk.header_rules = Some("strip-pai, add-diversion".to_string());
        trunk.srtp_mode = "required".to_string();
        trunk.dtmf_mode = "rfc2833".to_string();
        trunk.codec_prefs = vec!["PCMU".to_string(), "G722".to_string()];
        trunk.emergency_capable = true;
        trunk
    }

    /// A trunk a node read from its own TOML is identified by its name;
    /// a stored trunk by its id, so two tenants' `carrier-a` never meet
    /// (`docs/TENANT_TRUNKS.md` §4).
    #[test]
    fn a_trunk_is_identified_by_its_id() {
        let platform = TrunkConfig {
            name: "carrier-a".to_string(),
            ..Default::default()
        };
        assert_eq!(platform.trunk_id(), TrunkId::platform("carrier-a"));
        assert!(platform.trunk_id().is_platform());
        assert_eq!(platform.key(), "platform:carrier-a");

        let (a, b) = (TenantId::new(), TenantId::new());
        let one = TrunkConfig::from(&stored("carrier-a", a));
        let two = TrunkConfig::from(&stored("carrier-a", b));
        assert_eq!(one.name, two.name);
        assert_ne!(one.key(), two.key(), "same name, different trunks");
        assert!(!one.trunk_id().is_platform());
    }

    /// A platform trunk carries anyone's calls; a tenant's carries only
    /// its own.
    #[test]
    fn a_trunk_serves_its_own_tenant_or_everyone() {
        let (a, b) = (TenantId::new(), TenantId::new());
        let platform = TrunkConfig::default();
        assert!(platform.serves(Some(a)));
        assert!(platform.serves(None));

        let tenant_trunk = TrunkConfig::from(&stored("carrier-a", a));
        assert!(tenant_trunk.serves(Some(a)));
        assert!(!tenant_trunk.serves(Some(b)));
        assert!(
            !tenant_trunk.serves(None),
            "a call with no tenant uses the platform's trunks"
        );
    }

    /// The one conversion between the model a tenant manages and the model
    /// the call path uses.
    #[test]
    fn a_stored_trunk_becomes_a_runtime_one() {
        let tenant = TenantId::new();
        let trunk = stored("carrier-a", tenant);
        let config = TrunkConfig::from(&trunk);

        assert_eq!(config.name, "carrier-a");
        assert_eq!(config.tenant_id, Some(tenant));
        assert_eq!(config.id, Some(trunk.id.clone()));
        assert_eq!(config.hosts, vec!["carrier.example.net".to_string()]);
        assert_eq!(
            config.transport.protocol,
            fcp_core::transport::TransportType::Tls
        );
        assert_eq!(config.transport.port, Some(5061));
        let credentials = config.credentials.expect("credentials");
        assert_eq!(credentials.username, "fcp");
        assert_eq!(credentials.password, "s3cret");
        assert_eq!(config.srtp.mode, SrtpMode::Required);
        assert_eq!(config.dtmf.mode, TrunkDtmfMode::Relay);
        assert_eq!(
            config.codec.allowed_codecs,
            vec!["PCMU".to_string(), "G722".to_string()]
        );
        assert_eq!(
            config.header_rules,
            vec!["strip-pai".to_string(), "add-diversion".to_string()],
            "the stored comma-separated list becomes rule names"
        );
        assert!(config.emergency_capable);
        assert_eq!(
            config.emergency_location,
            vec![fcp_core::e911::LocationDelivery::Elin],
            "a stored trunk takes the default until it can say otherwise"
        );

        // strip_digits + prepend_digits is the simple form of the transform.
        let transform = config.egress_number_transform.expect("a transform");
        assert_eq!(transform.strip_leading, 2);
        assert_eq!(transform.prepend, "+1");
        assert_eq!(transform.apply("915185550100"), "+15185550100");

        // A trunk that says nothing about digits gets no transform.
        let plain = TrunkConfig::from(&Trunk::new("plain".to_string(), tenant));
        assert!(plain.egress_number_transform.is_none());
    }
}
