//! Trunk/carrier configuration for the SBC.
//!
//! Defines trunk direction, transport settings, credentials, failover
//! policy, and the main `TrunkConfig` struct.

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

/// A trunk (carrier) configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkConfig {
    /// Trunk name (unique identifier).
    pub name: String,

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

    /// SRTP configuration for this trunk
    #[serde(default)]
    pub srtp: TrunkSrtpConfig,

    /// DTMF configuration for this trunk
    #[serde(default)]
    pub dtmf: TrunkDtmfConfig,
}

impl Default for TrunkConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
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
            srtp: TrunkSrtpConfig::default(),
            dtmf: TrunkDtmfConfig::default(),
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
        if let Some(ref creds) = self.credentials {
            if creds.username.is_empty() {
                return Err(format!(
                    "trunk {:?}: credentials username must not be empty",
                    self.name
                ));
            }
        }
        self.failover
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
}
