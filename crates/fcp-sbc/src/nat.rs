//! NAT configuration helpers for the SBC.
//!
//! Bridges `fcp_sbc_core::NatConfig` into the media-layer config types
//! (`fcp_media::config::MediaConfig` and `fcp_core::config::MediaConfig`) so
//! the B2BUA handler can rewrite SDP with the correct external IP and RTP
//! port range.

use fcp_sbc_core::NatConfig;

/// Returns the IP address that should appear in outgoing SDP `c=` lines.
///
/// When NAT is enabled and `external_ip` is configured, that public address is
/// used.  Otherwise we fall back to the local network IP detected at startup.
pub fn effective_media_ip(nat: &NatConfig, local_ip: &str) -> String {
    if nat.enabled {
        if let Some(ref ext) = nat.external_ip {
            if !ext.is_empty() {
                return ext.clone();
            }
        }
    }
    local_ip.to_string()
}

/// Build the `fcp_media::config::MediaConfig` consumed by
/// `MediaManager::new()`.
///
/// Maps the SBC's NAT port range and public IP into forge-media's embedded
/// backend config.  The first STUN/TURN server (if any) is forwarded.
pub fn build_fcp_media_config(
    nat: &NatConfig,
    effective_ip: &str,
) -> fcp_media::config::MediaConfig {
    let stun_server = nat.stun_servers.first().map(|s| format!("{}:{}", s.host, s.port));
    let turn_server = nat.turn_servers.first().map(|t| format!("{}:{}", t.host, t.port));

    fcp_media::config::MediaConfig {
        backend: fcp_media::config::BackendType::Embedded,
        rtp_port_start: nat.rtp_port_start,
        rtp_port_end: nat.rtp_port_end,
        public_ip: Some(effective_ip.to_string()),
        stun_server,
        turn_server,
        ..Default::default()
    }
}

/// Build the `fcp_core::config::MediaConfig` passed to the B2BUA handler.
///
/// The handler uses `enabled` to decide whether media anchoring (and therefore
/// SDP rewriting) should be attempted.  We enable it only when NAT traversal
/// *and* SDP rewriting are both turned on.
pub fn build_fcp_core_media_config(
    nat: &NatConfig,
    effective_ip: &str,
) -> fcp_core::config::MediaConfig {
    let stun_server = nat.stun_servers.first().map(|s| format!("{}:{}", s.host, s.port));
    let turn_server = nat.turn_servers.first().map(|t| format!("{}:{}", t.host, t.port));

    fcp_core::config::MediaConfig {
        enabled: nat.enabled && nat.rewrite_sdp,
        rtp_port_start: nat.rtp_port_start,
        rtp_port_end: nat.rtp_port_end,
        enable_srtp: false,
        enable_transcoding: false,
        enable_recording: false,
        enable_dtmf: false,
        public_ip: Some(effective_ip.to_string()),
        stun_server,
        turn_server,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_sbc_core::nat::{StunServerConfig, TurnServerConfig};

    #[test]
    fn effective_ip_prefers_external_when_nat_enabled() {
        let nat = NatConfig {
            enabled: true,
            external_ip: Some("203.0.113.1".into()),
            ..Default::default()
        };
        assert_eq!(effective_media_ip(&nat, "10.0.0.5"), "203.0.113.1");
    }

    #[test]
    fn effective_ip_falls_back_when_no_external() {
        let nat = NatConfig {
            enabled: true,
            external_ip: None,
            ..Default::default()
        };
        assert_eq!(effective_media_ip(&nat, "10.0.0.5"), "10.0.0.5");
    }

    #[test]
    fn effective_ip_falls_back_when_nat_disabled() {
        let nat = NatConfig {
            enabled: false,
            external_ip: Some("203.0.113.1".into()),
            ..Default::default()
        };
        assert_eq!(effective_media_ip(&nat, "10.0.0.5"), "10.0.0.5");
    }

    #[test]
    fn effective_ip_falls_back_when_external_empty() {
        let nat = NatConfig {
            enabled: true,
            external_ip: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(effective_media_ip(&nat, "10.0.0.5"), "10.0.0.5");
    }

    #[test]
    fn build_fcp_media_config_maps_fields() {
        let nat = NatConfig {
            enabled: true,
            external_ip: Some("203.0.113.1".into()),
            rtp_port_start: 16000,
            rtp_port_end: 26000,
            rewrite_sdp: true,
            stun_servers: vec![StunServerConfig {
                host: "stun.example.com".into(),
                port: 3478,
            }],
            turn_servers: vec![TurnServerConfig {
                host: "turn.example.com".into(),
                port: 3479,
                username: "u".into(),
                password: "p".into(),
                use_tls: false,
            }],
            force_rport: true,
        };
        let cfg = build_fcp_media_config(&nat, "203.0.113.1");

        assert_eq!(cfg.rtp_port_start, 16000);
        assert_eq!(cfg.rtp_port_end, 26000);
        assert_eq!(cfg.public_ip.as_deref(), Some("203.0.113.1"));
        assert_eq!(cfg.stun_server.as_deref(), Some("stun.example.com:3478"));
        assert_eq!(cfg.turn_server.as_deref(), Some("turn.example.com:3479"));
    }

    #[test]
    fn build_fcp_core_media_config_enabled_flag() {
        let mut nat = NatConfig::default();

        // Both enabled + rewrite_sdp → enabled
        nat.enabled = true;
        nat.rewrite_sdp = true;
        let cfg = build_fcp_core_media_config(&nat, "10.0.0.1");
        assert!(cfg.enabled);

        // NAT enabled but rewrite_sdp off → disabled
        nat.rewrite_sdp = false;
        let cfg = build_fcp_core_media_config(&nat, "10.0.0.1");
        assert!(!cfg.enabled);

        // NAT disabled → disabled regardless
        nat.enabled = false;
        nat.rewrite_sdp = true;
        let cfg = build_fcp_core_media_config(&nat, "10.0.0.1");
        assert!(!cfg.enabled);
    }

    #[test]
    fn build_fcp_core_media_config_maps_ports() {
        let nat = NatConfig {
            rtp_port_start: 20000,
            rtp_port_end: 30000,
            ..Default::default()
        };
        let cfg = build_fcp_core_media_config(&nat, "10.0.0.1");
        assert_eq!(cfg.rtp_port_start, 20000);
        assert_eq!(cfg.rtp_port_end, 30000);
    }

    #[test]
    fn build_configs_no_stun_turn() {
        let nat = NatConfig::default();
        let media_cfg = build_fcp_media_config(&nat, "10.0.0.1");
        let core_cfg = build_fcp_core_media_config(&nat, "10.0.0.1");

        assert!(media_cfg.stun_server.is_none());
        assert!(media_cfg.turn_server.is_none());
        assert!(core_cfg.stun_server.is_none());
        assert!(core_cfg.turn_server.is_none());
    }
}
