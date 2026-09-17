//! The trunks the call path routes by.
//!
//! One model — `fcp_core::Trunk` — is what a tenant manages through
//! `/v1/trunks`, what a node reads from its own `[[trunks]]`, and what the
//! router, health monitor and registrar work on
//! (`docs/TENANT_TRUNKS.md` §4). This module keeps the names this crate has
//! always used for it.

pub use fcp_core::trunk::{
    AttestationLevel, NumberTransform, SrtpMode, StirSignMode, StirVerifyMode, TrunkCodecConfig,
    TrunkCredentials, TrunkDirection, TrunkDtmfConfig, TrunkDtmfMode, TrunkFailover,
    TrunkSrtpConfig, TrunkStirShakenConfig, TrunkTransport,
};

/// A trunk as the call path holds it.
///
/// One model, `fcp_core::Trunk`: what a tenant manages through `/v1/trunks`,
/// what a node reads from its own `[[trunks]]`, and what the router, the
/// health monitor and the registrar work on — so a field the API accepts
/// cannot quietly do nothing (`docs/TENANT_TRUNKS.md` §4). The alias is
/// kept because `TrunkConfig` is what a configuration file's trunks are
/// called throughout this crate.
pub type TrunkConfig = fcp_core::Trunk;

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::transport::TransportType;

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

#[cfg(test)]
mod tenant_trunk_tests {
    use super::*;
    use fcp_core::{TenantId, TrunkId};

    fn stored(name: &str, tenant: TenantId) -> TrunkConfig {
        let mut trunk = TrunkConfig::new(name.to_string(), tenant);
        trunk.hosts = vec!["carrier.example.net".to_string()];
        trunk
    }

    /// A trunk a node read from its own TOML is identified by its name; a
    /// stored trunk by its id, so two tenants' `carrier-a` never meet
    /// (`docs/TENANT_TRUNKS.md` §4).
    #[test]
    fn a_trunk_is_identified_by_its_id() {
        let platform = TrunkConfig {
            name: "carrier-a".to_string(),
            id: TrunkId::from_string(String::new()),
            ..Default::default()
        };
        assert_eq!(platform.trunk_id(), TrunkId::platform("carrier-a"));
        assert!(platform.trunk_id().is_platform());
        assert_eq!(platform.key(), "platform:carrier-a");

        let (a, b) = (TenantId::new(), TenantId::new());
        let one = stored("carrier-a", a);
        let two = stored("carrier-a", b);
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

        let tenant_trunk = stored("carrier-a", a);
        assert!(tenant_trunk.serves(Some(a)));
        assert!(!tenant_trunk.serves(Some(b)));
        assert!(
            !tenant_trunk.serves(None),
            "a call with no tenant uses the platform's trunks"
        );
    }

    /// The `[[trunks]]` a node has always read parses into the one model,
    /// unchanged: this is the shape in every deployment's configuration
    /// and in the test floor.
    #[test]
    fn the_trunks_toml_a_node_reads_is_unchanged() {
        #[derive(serde::Deserialize)]
        struct Config {
            trunks: Vec<TrunkConfig>,
        }

        let config: Config = toml::from_str(
            r#"
[[trunks]]
name = "carrier-a"
enabled = true
direction = "Upstream"
hosts = ["10.0.0.9"]
weight = 100
priority = 10
register = false
max_concurrent_calls = 0
emergency_capable = true
emergency_location = ["elin", "pidf_lo"]
header_rules = ["strip-pai"]

[trunks.transport]
protocol = "Tls"
port = 5061
verify_tls = false

[trunks.failover]
max_failures = 3
recovery_interval_secs = 30
health_check_enabled = false
health_check_interval_secs = 60

[trunks.credentials]
username = "fcp"
password = "s3cret"

[trunks.srtp]
mode = "Required"

[trunks.egress_number_transform]
strip_prefixes = ["+1"]
prepend = "1"

[[trunks]]
name = "carrier-b"
hosts = ["10.0.0.10"]
"#,
        )
        .expect("the trunks a node reads");

        let a = &config.trunks[0];
        assert_eq!(a.name, "carrier-a");
        assert_eq!(a.direction, TrunkDirection::Upstream);
        assert_eq!(a.priority, 10);
        assert_eq!(
            a.transport.protocol,
            fcp_core::transport::TransportType::Tls
        );
        assert_eq!(a.transport.port, Some(5061));
        assert!(!a.transport.verify_tls);
        assert_eq!(a.failover.max_failures, 3);
        assert!(!a.failover.health_check_enabled);
        assert_eq!(
            a.credentials.as_ref().map(|c| c.username.as_str()),
            Some("fcp")
        );
        assert_eq!(a.srtp.mode, SrtpMode::Required);
        assert_eq!(a.header_rules, vec!["strip-pai".to_string()]);
        assert_eq!(
            a.emergency_location,
            vec![
                fcp_core::e911::LocationDelivery::Elin,
                fcp_core::e911::LocationDelivery::PidfLo
            ]
        );
        let transform = a.egress_number_transform.as_ref().expect("a transform");
        assert_eq!(transform.strip_prefixes, vec!["+1".to_string()]);
        assert_eq!(transform.apply("+15185550100"), "15185550100");
        // No tenant, no stored id: identified by its name, the same way on
        // every node.
        assert!(a.tenant_id.is_none());
        assert_eq!(a.key(), "platform:carrier-a");

        // A trunk that says almost nothing still has the defaults a
        // `[[trunks]]` entry has always had.
        let b = &config.trunks[1];
        assert!(b.enabled);
        assert_eq!(b.direction, TrunkDirection::Bidirectional);
        assert_eq!(b.priority, 100);
        assert_eq!(b.srtp.mode, SrtpMode::Disabled);
        assert_eq!(
            b.emergency_location,
            vec![fcp_core::e911::LocationDelivery::Elin]
        );
        assert!(b.validate().is_ok());
    }
}
