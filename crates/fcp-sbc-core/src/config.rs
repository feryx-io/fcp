//! Aggregate SBC configuration.
//!
//! `SbcConfig` combines the B2BUA config with all SBC-specific sub-configs
//! (admission, rate limiting, security, trunks, NAT, SIPREC, metrics)
//! into a single top-level struct.

use std::collections::HashSet;
use std::net::SocketAddr;

use fcp_b2bua_core::forking::ForkingConfig;
use fcp_b2bua_core::media_anchoring::{MediaAnchoringConfig, MediaAnchoringMode};
use fcp_b2bua_core::session::{B2buaConfig, SessionTimerConfig};
use fcp_b2bua_core::topology::{TopologyHidingConfig, TopologyHidingMode};
use fcp_b2bua_core::transfer::TransferConfig;
use fcp_core::transport::TransportType;
use serde::{Deserialize, Serialize};

use crate::admission::AdmissionConfig;
use crate::hmr::HeaderManipulationConfig;
use crate::lcr::LcrConfig;
use crate::metrics::SbcMetricsConfig;
use crate::nat::NatConfig;
use crate::rate_limit::RateLimitPolicyConfig;
use crate::security::SecurityConfig;
use crate::siprec::SiprecConfig;
use crate::stir_shaken::StirShakenConfig;
use crate::trunk::TrunkConfig;

/// Optional storage configuration for CDR persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SbcStorageConfig {
    /// Enable storage (and CDR generation). Default: false.
    #[serde(default)]
    pub enabled: bool,

    /// PostgreSQL connection URL. Default: "postgresql://localhost/fcp"
    #[serde(default = "default_storage_database_url")]
    pub database_url: String,
}

fn default_storage_database_url() -> String {
    "postgresql://localhost/fcp".to_string()
}

impl Default for SbcStorageConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            database_url: default_storage_database_url(),
        }
    }
}

/// Top-level SBC configuration.
///
/// Aggregates the B2BUA config (from `fcp-b2bua-core`) with all SBC-specific
/// sub-configurations. The default uses SBC-tuned B2BUA settings (full
/// topology hiding, always anchor media, single forking, SBC transfer defaults).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SbcConfig {
    /// B2BUA behavior configuration (topology hiding, media anchoring, etc.).
    pub b2bua: B2buaConfig,

    /// Call admission control.
    pub admission: AdmissionConfig,

    /// Rate limiting policy.
    pub rate_limit: RateLimitPolicyConfig,

    /// Security policy (IP ACLs, malformed SIP handling, size limits).
    pub security: SecurityConfig,

    /// Trunk/carrier definitions.
    pub trunks: Vec<TrunkConfig>,

    /// NAT traversal configuration.
    pub nat: NatConfig,

    /// SIPREC recording configuration.
    pub siprec: SiprecConfig,

    /// Metrics collection configuration.
    pub metrics: SbcMetricsConfig,

    /// External domain for the SBC (used in Via/Contact headers).
    pub external_domain: String,

    /// Enabled transport protocols.
    pub transports: Vec<TransportType>,

    /// LCR (Least Cost Routing) configuration.
    #[serde(default)]
    pub lcr: LcrConfig,

    /// Header manipulation rules.
    #[serde(default)]
    pub header_manipulation: HeaderManipulationConfig,

    /// Optional storage for CDR persistence.
    #[serde(default)]
    pub storage: SbcStorageConfig,

    /// STIR/SHAKEN caller ID verification and signing (RFC 8224/8225/8588).
    #[serde(default)]
    pub stir_shaken: StirShakenConfig,

    /// TLS bind address (e.g. "0.0.0.0:5061"). None = TLS disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_bind: Option<String>,

    /// Path to PEM certificate file for TLS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_cert: Option<String>,

    /// Path to PEM private key file for TLS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_key: Option<String>,
}

impl Default for SbcConfig {
    fn default() -> Self {
        Self {
            b2bua: B2buaConfig {
                topology_hiding: TopologyHidingConfig {
                    mode: TopologyHidingMode::Full,
                    hide_via: true,
                    hide_record_route: true,
                    hide_contact: true,
                    local_domain: None,
                    hide_user_agent: true,
                    custom_user_agent: None,
                },
                media_anchoring: MediaAnchoringConfig {
                    mode: MediaAnchoringMode::Always,
                    anchor_for_nat: true,
                    anchor_for_recording: true,
                    anchor_for_dtmf: true,
                    anchor_for_transcoding: true,
                    rewrite_sdp_connection: true,
                },
                forking: ForkingConfig::sbc_defaults(),
                transfer: TransferConfig::sbc_defaults(),
                max_forwards: 70,
                relay_provisional_responses: true,
                insert_record_route: true,
                session_timer: SessionTimerConfig {
                    enabled: true,
                    session_expires_secs: 1800,
                    min_se_secs: 90,
                },
                max_extension_digits: 6,
                voicemail_server_uri: None,
            },
            admission: AdmissionConfig::default(),
            rate_limit: RateLimitPolicyConfig::default(),
            security: SecurityConfig::default(),
            trunks: Vec::new(),
            nat: NatConfig::default(),
            siprec: SiprecConfig::default(),
            metrics: SbcMetricsConfig::default(),
            lcr: LcrConfig::default(),
            external_domain: String::new(),
            transports: vec![TransportType::Udp, TransportType::Tcp],
            header_manipulation: HeaderManipulationConfig::default(),
            stir_shaken: StirShakenConfig::default(),
            storage: SbcStorageConfig::default(),
            tls_bind: None,
            tls_cert: None,
            tls_key: None,
        }
    }
}

impl SbcConfig {
    /// Validate the full SBC configuration.
    ///
    /// Delegates to all sub-config `validate()` methods and additionally
    /// checks:
    /// - `external_domain` is non-empty and contains no CR/LF
    /// - `transports` is non-empty
    /// - Trunk names are unique
    pub fn validate(&self) -> Result<(), String> {
        // B2BUA sub-configs
        self.b2bua.topology_hiding.validate()?;
        self.b2bua.forking.validate()?;
        self.b2bua.transfer.validate()?;
        self.b2bua.session_timer.validate()?;

        // SBC sub-configs
        self.admission.validate()?;
        self.rate_limit.validate()?;
        self.security.validate()?;
        self.nat.validate()?;
        self.siprec.validate()?;
        self.metrics.validate()?;

        self.header_manipulation.validate()?;
        self.stir_shaken.validate()?;

        self.lcr.validate()?;
        // Validate LCR route trunk_name references
        let trunk_names: Vec<String> = self.trunks.iter().map(|t| t.name.clone()).collect();
        self.lcr.validate_trunk_refs(&trunk_names)?;

        for (i, trunk) in self.trunks.iter().enumerate() {
            trunk.validate().map_err(|e| format!("trunks[{i}]: {e}"))?;
        }

        // Check trunk name uniqueness
        let mut seen_names = HashSet::new();
        for trunk in &self.trunks {
            if !seen_names.insert(&trunk.name) {
                return Err(format!("duplicate trunk name: {:?}", trunk.name));
            }
        }

        if self.external_domain.is_empty() {
            return Err("external_domain must not be empty".into());
        }
        if self.external_domain.contains('\r') || self.external_domain.contains('\n') {
            return Err("external_domain must not contain CR/LF characters".into());
        }

        if self.transports.is_empty() {
            return Err("transports must not be empty".into());
        }

        // TLS: if any TLS field is set, all three must be provided
        let tls_fields = [
            self.tls_bind.is_some(),
            self.tls_cert.is_some(),
            self.tls_key.is_some(),
        ];
        let tls_count = tls_fields.iter().filter(|&&v| v).count();
        if tls_count > 0 && tls_count < 3 {
            return Err(
                "TLS requires all three fields: tls_bind, tls_cert, and tls_key".into(),
            );
        }
        if let Some(ref bind) = self.tls_bind {
            if bind.parse::<SocketAddr>().is_err() {
                return Err(format!("tls_bind is not a valid socket address: {:?}", bind));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> SbcConfig {
        SbcConfig {
            external_domain: "sbc.example.com".into(),
            ..Default::default()
        }
    }

    #[test]
    fn test_default() {
        let cfg = SbcConfig::default();
        // B2BUA is SBC-tuned
        assert_eq!(cfg.b2bua.topology_hiding.mode, TopologyHidingMode::Full);
        assert!(cfg.b2bua.topology_hiding.hide_via);
        assert_eq!(cfg.b2bua.media_anchoring.mode, MediaAnchoringMode::Always);
        assert_eq!(cfg.b2bua.max_forwards, 70);
        assert!(cfg.b2bua.relay_provisional_responses);
        assert!(cfg.b2bua.insert_record_route);

        // Session timer: SBC default has it enabled
        assert!(cfg.b2bua.session_timer.enabled);
        assert_eq!(cfg.b2bua.session_timer.session_expires_secs, 1800);
        assert_eq!(cfg.b2bua.session_timer.min_se_secs, 90);

        // SBC defaults
        assert!(cfg.trunks.is_empty());
        assert!(cfg.external_domain.is_empty());
        assert_eq!(cfg.transports, vec![TransportType::Udp, TransportType::Tcp]);
    }

    #[test]
    fn test_validate_valid() {
        assert!(valid_config().validate().is_ok());
    }

    #[test]
    fn test_validate_with_trunks() {
        let mut cfg = valid_config();
        cfg.trunks = vec![
            TrunkConfig {
                name: "carrier-a".into(),
                hosts: vec!["sip.carrier-a.com".into()],
                ..Default::default()
            },
            TrunkConfig {
                name: "carrier-b".into(),
                hosts: vec!["sip.carrier-b.com".into()],
                ..Default::default()
            },
        ];
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_duplicate_trunk_names() {
        let mut cfg = valid_config();
        cfg.trunks = vec![
            TrunkConfig {
                name: "carrier-a".into(),
                hosts: vec!["sip.carrier-a.com".into()],
                ..Default::default()
            },
            TrunkConfig {
                name: "carrier-a".into(),
                hosts: vec!["sip2.carrier-a.com".into()],
                ..Default::default()
            },
        ];
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("duplicate trunk name"));
    }

    #[test]
    fn test_validate_empty_external_domain() {
        let cfg = SbcConfig::default();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("external_domain"));
    }

    #[test]
    fn test_validate_crlf_external_domain() {
        let mut cfg = valid_config();
        cfg.external_domain = "bad\r\ndomain".into();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_empty_transports() {
        let mut cfg = valid_config();
        cfg.transports.clear();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("transports"));
    }

    #[test]
    fn test_validate_propagates_admission_error() {
        let mut cfg = valid_config();
        cfg.admission.max_cps = 100_001;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_cps"));
    }

    #[test]
    fn test_validate_propagates_trunk_error() {
        let mut cfg = valid_config();
        cfg.trunks = vec![TrunkConfig {
            name: String::new(),
            hosts: vec!["sip.example.com".into()],
            ..Default::default()
        }];
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("name"));
    }

    #[test]
    fn test_validate_propagates_security_error() {
        let mut cfg = valid_config();
        cfg.security.max_message_size = 0;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_message_size"));
    }

    #[test]
    fn test_validate_propagates_nat_error() {
        let mut cfg = valid_config();
        cfg.nat.rtp_port_start = 20000;
        cfg.nat.rtp_port_end = 10000;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("rtp_port"));
    }

    #[test]
    fn test_validate_propagates_metrics_error() {
        let mut cfg = valid_config();
        cfg.metrics.snapshot_interval_secs = 1;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("snapshot_interval_secs"));
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = valid_config();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SbcConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_serde_roundtrip_with_trunks() {
        let mut cfg = valid_config();
        cfg.trunks = vec![TrunkConfig {
            name: "carrier-a".into(),
            hosts: vec!["sip.carrier.com".into()],
            ..Default::default()
        }];
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SbcConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_sbc_storage_config_defaults() {
        let cfg = SbcStorageConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.database_url, "postgresql://localhost/fcp");
    }

    #[test]
    fn test_sbc_storage_config_serde_roundtrip() {
        let cfg = SbcStorageConfig {
            enabled: true,
            database_url: "postgresql://localhost/test_sbc".to_string(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SbcStorageConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_sbc_config_includes_storage_default() {
        let cfg = SbcConfig::default();
        assert!(!cfg.storage.enabled);
        assert_eq!(cfg.storage.database_url, "postgresql://localhost/fcp");
    }

    #[test]
    fn test_sbc_storage_config_deserialize_missing_fields() {
        // When storage section is empty JSON object, defaults should apply
        let json = "{}";
        let cfg: SbcStorageConfig = serde_json::from_str(json).unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.database_url, "postgresql://localhost/fcp");
    }

    #[test]
    fn test_validate_tls_partial_config() {
        // Only tls_bind set — should fail because cert/key are missing
        let mut cfg = valid_config();
        cfg.tls_bind = Some("0.0.0.0:5061".into());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("tls_bind, tls_cert, and tls_key"));

        // Only tls_cert and tls_key — missing tls_bind
        let mut cfg = valid_config();
        cfg.tls_cert = Some("/path/to/cert.pem".into());
        cfg.tls_key = Some("/path/to/key.pem".into());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("tls_bind, tls_cert, and tls_key"));
    }

    #[test]
    fn test_validate_tls_complete_config() {
        let mut cfg = valid_config();
        cfg.tls_bind = Some("0.0.0.0:5061".into());
        cfg.tls_cert = Some("/path/to/cert.pem".into());
        cfg.tls_key = Some("/path/to/key.pem".into());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_tls_invalid_bind() {
        let mut cfg = valid_config();
        cfg.tls_bind = Some("not-an-address".into());
        cfg.tls_cert = Some("/path/to/cert.pem".into());
        cfg.tls_key = Some("/path/to/key.pem".into());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("tls_bind"));
        assert!(err.contains("valid socket address"));
    }

    #[test]
    fn test_serde_roundtrip_tls() {
        let mut cfg = valid_config();
        cfg.tls_bind = Some("0.0.0.0:5061".into());
        cfg.tls_cert = Some("/etc/ssl/cert.pem".into());
        cfg.tls_key = Some("/etc/ssl/key.pem".into());
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SbcConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
