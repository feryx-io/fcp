//! NAT traversal configuration for the SBC.
//!
//! Controls RTP port ranges, SDP rewriting, STUN/TURN server settings,
//! and rport enforcement.

use serde::{Deserialize, Serialize};

/// STUN server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StunServerConfig {
    /// STUN server hostname or IP.
    pub host: String,

    /// STUN server port.
    pub port: u16,
}

impl Default for StunServerConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 3478,
        }
    }
}

/// TURN server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnServerConfig {
    /// TURN server hostname or IP.
    pub host: String,

    /// TURN server port.
    pub port: u16,

    /// TURN authentication username.
    pub username: String,

    /// TURN authentication password.
    pub password: String,

    /// Whether to use TLS for the TURN connection.
    pub use_tls: bool,
}

impl Default for TurnServerConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 3478,
            username: String::new(),
            password: String::new(),
            use_tls: false,
        }
    }
}

/// NAT traversal configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct NatConfig {
    /// Whether NAT traversal is enabled.
    pub enabled: bool,

    /// External (public) IP address for SDP rewriting.
    pub external_ip: Option<String>,

    /// Start of the RTP port range.
    pub rtp_port_start: u16,

    /// End of the RTP port range (inclusive).
    pub rtp_port_end: u16,

    /// Rewrite SDP c= and m= lines with the external IP.
    pub rewrite_sdp: bool,

    /// STUN servers for reflexive address discovery.
    pub stun_servers: Vec<StunServerConfig>,

    /// TURN servers for relayed media.
    pub turn_servers: Vec<TurnServerConfig>,

    /// Force rport parameter in Via headers (RFC 3581).
    pub force_rport: bool,

    /// Enable SRTP media encryption. Not yet implemented — enabling this will
    /// log a warning at startup.
    #[serde(default)]
    pub enable_srtp: bool,

    /// Enable codec transcoding (requires media anchoring).
    /// When enabled, the forge-media pipeline handles codec conversion between
    /// call legs with incompatible codec sets.
    #[serde(default)]
    pub enable_transcoding: bool,

    /// Enable DTMF detection/relay (RFC 2833 + inband).
    /// The SBC media stack does not currently process DTMF — enabling this
    /// will log a warning at startup.
    #[serde(default)]
    pub enable_dtmf: bool,

    /// Enable XDP kernel-level RTP forwarding (Linux only, requires CAP_NET_ADMIN).
    #[serde(default)]
    pub xdp_enabled: bool,

    /// Network interface for XDP attachment (e.g., "eth0").
    #[serde(default = "default_xdp_interface")]
    pub xdp_interface: String,

    /// XDP mode: "native" (driver-level, fastest) or "generic" (software fallback).
    #[serde(default = "default_xdp_mode")]
    pub xdp_mode: String,
}

fn default_xdp_interface() -> String {
    "eth0".to_string()
}

fn default_xdp_mode() -> String {
    "generic".to_string()
}

impl Default for NatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            external_ip: None,
            rtp_port_start: 10000,
            rtp_port_end: 20000,
            rewrite_sdp: true,
            stun_servers: Vec::new(),
            turn_servers: Vec::new(),
            force_rport: true,
            enable_srtp: false,
            enable_transcoding: false,
            enable_dtmf: false,
            xdp_enabled: false,
            xdp_interface: default_xdp_interface(),
            xdp_mode: default_xdp_mode(),
        }
    }
}

impl NatConfig {
    /// Validate the configuration.
    ///
    /// Returns `Err` if:
    /// - `rtp_port_end` <= `rtp_port_start`
    /// - Port range start is below 1024
    /// - Port range contains fewer than 100 ports
    /// - `external_ip` is set but empty or contains CR/LF
    /// - Any STUN/TURN server host is empty or contains CR/LF
    pub fn validate(&self) -> Result<(), String> {
        if self.rtp_port_end <= self.rtp_port_start {
            return Err("rtp_port_end must be greater than rtp_port_start".into());
        }
        if self.rtp_port_start < 1024 {
            return Err("rtp_port_start must be at least 1024".into());
        }
        let range = (self.rtp_port_end as u32) - (self.rtp_port_start as u32);
        if range < 100 {
            return Err("RTP port range must contain at least 100 ports".into());
        }
        if let Some(ref ip) = self.external_ip {
            if ip.is_empty() {
                return Err("external_ip must not be empty when set".into());
            }
            if ip.contains('\r') || ip.contains('\n') {
                return Err("external_ip must not contain CR/LF characters".into());
            }
        }
        for (i, stun) in self.stun_servers.iter().enumerate() {
            if stun.host.is_empty() {
                return Err(format!("stun_servers[{i}]: host must not be empty"));
            }
            if stun.host.contains('\r') || stun.host.contains('\n') {
                return Err(format!(
                    "stun_servers[{i}]: host must not contain CR/LF characters"
                ));
            }
        }
        for (i, turn) in self.turn_servers.iter().enumerate() {
            if turn.host.is_empty() {
                return Err(format!("turn_servers[{i}]: host must not be empty"));
            }
            if turn.host.contains('\r') || turn.host.contains('\n') {
                return Err(format!(
                    "turn_servers[{i}]: host must not contain CR/LF characters"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let cfg = NatConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.external_ip.is_none());
        assert_eq!(cfg.rtp_port_start, 10000);
        assert_eq!(cfg.rtp_port_end, 20000);
        assert!(cfg.rewrite_sdp);
        assert!(cfg.stun_servers.is_empty());
        assert!(cfg.turn_servers.is_empty());
        assert!(cfg.force_rport);
        assert!(!cfg.enable_srtp);
        assert!(!cfg.enable_transcoding);
        assert!(!cfg.enable_dtmf);
    }

    #[test]
    fn test_validate_valid() {
        assert!(NatConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_with_servers() {
        let cfg = NatConfig {
            stun_servers: vec![StunServerConfig {
                host: "stun.example.com".into(),
                port: 3478,
            }],
            turn_servers: vec![TurnServerConfig {
                host: "turn.example.com".into(),
                port: 3478,
                username: "user".into(),
                password: "pass".into(),
                use_tls: true,
            }],
            external_ip: Some("203.0.113.1".into()),
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_port_range_inverted() {
        let cfg = NatConfig {
            rtp_port_start: 20000,
            rtp_port_end: 10000,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("rtp_port_end"));
    }

    #[test]
    fn test_validate_port_range_equal() {
        let cfg = NatConfig {
            rtp_port_start: 10000,
            rtp_port_end: 10000,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("rtp_port_end"));
    }

    #[test]
    fn test_validate_port_below_1024() {
        let cfg = NatConfig {
            rtp_port_start: 1023,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("1024"));
    }

    #[test]
    fn test_validate_port_range_too_small() {
        let cfg = NatConfig {
            rtp_port_start: 10000,
            rtp_port_end: 10099, // only 99 ports
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("100 ports"));
    }

    #[test]
    fn test_validate_empty_external_ip() {
        let cfg = NatConfig {
            external_ip: Some(String::new()),
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("external_ip"));
    }

    #[test]
    fn test_validate_crlf_external_ip() {
        let cfg = NatConfig {
            external_ip: Some("bad\r\nip".into()),
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_empty_stun_host() {
        let cfg = NatConfig {
            stun_servers: vec![StunServerConfig {
                host: String::new(),
                port: 3478,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("stun_servers"));
    }

    #[test]
    fn test_validate_crlf_stun_host() {
        let cfg = NatConfig {
            stun_servers: vec![StunServerConfig {
                host: "bad\nhost".into(),
                port: 3478,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_empty_turn_host() {
        let cfg = NatConfig {
            turn_servers: vec![TurnServerConfig {
                host: String::new(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("turn_servers"));
    }

    #[test]
    fn test_validate_boundary_values() {
        let mut cfg = NatConfig {
            rtp_port_start: 10000,
            rtp_port_end: 10100,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());

        // Port start at 1024 is valid
        cfg.rtp_port_start = 1024;
        cfg.rtp_port_end = 1200;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = NatConfig {
            external_ip: Some("203.0.113.1".into()),
            stun_servers: vec![StunServerConfig {
                host: "stun.example.com".into(),
                port: 3478,
            }],
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: NatConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_default_media_flags_false() {
        let cfg = NatConfig::default();
        assert!(!cfg.enable_srtp, "enable_srtp must default to false");
        assert!(
            !cfg.enable_transcoding,
            "enable_transcoding must default to false"
        );
        assert!(!cfg.enable_dtmf, "enable_dtmf must default to false");
    }

    #[test]
    fn test_serde_roundtrip_media_flags() {
        let cfg = NatConfig {
            enable_srtp: true,
            enable_transcoding: true,
            enable_dtmf: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: NatConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
        assert!(deserialized.enable_srtp);
        assert!(deserialized.enable_transcoding);
        assert!(deserialized.enable_dtmf);
    }
}
