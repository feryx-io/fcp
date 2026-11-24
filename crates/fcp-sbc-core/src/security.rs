//! Security configuration for the SBC.
//!
//! Provides IP ACL (access control list) evaluation, malformed SIP message
//! policy, and size limits for incoming messages.

use serde::{Deserialize, Serialize};

// The address list is the edge's, kept in the shared handler so there is
// one implementation of it rather than two (`docs/EDGE_SECURITY.md` §2.4).
// Re-exported here because this is where the SBC's configuration has
// always named it.
pub use fcp_sip::edge::{AclAction, IpAcl, IpAclEntry};

/// Policy for handling malformed SIP messages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MalformedSipPolicy {
    /// Silently drop malformed messages.
    Drop,

    /// Respond with 400 Bad Request.
    Reject,

    /// Log the event and drop the message.
    #[default]
    LogAndDrop,
}

/// Security configuration for the SBC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct SecurityConfig {
    /// ACL for signaling traffic.
    pub signaling_acl: IpAcl,

    /// Optional separate ACL for media traffic.
    pub media_acl: Option<IpAcl>,

    /// How to handle malformed SIP messages.
    pub malformed_sip_policy: MalformedSipPolicy,

    /// Maximum SIP message size in bytes.
    pub max_message_size: usize,

    /// Maximum number of headers in a single SIP message.
    pub max_headers: usize,

    /// Maximum URI length in bytes.
    pub max_uri_length: usize,

    /// Block requests with suspicious URI patterns (e.g. path traversal).
    pub block_suspicious_uris: bool,

    /// Validate that the source IP matches Via/Contact headers.
    pub validate_source_ip: bool,

    /// Maximum number of tracked source IPs for rate limiting/ACLs.
    pub max_tracked_sources: usize,

    /// Messages a second one source address may send the SBC's SIP ports
    /// (UDP packets, TCP/TLS/WebSocket frames), dropped unread over it;
    /// `0` is no limit. Exempt: `trusted_sources`, `[core.e911]
    /// trusted_edges`, every trunk's inbound sources and live cluster
    /// members — a carrier or the core behind the SBC passes this rate with
    /// ordinary load. Under it, `[core.rate_limit]` counts requests.
    pub max_messages_per_source: u32,

    /// Addresses and CIDR ranges exempt from `max_messages_per_source`.
    pub trusted_sources: Vec<String>,

    /// Failed digest authentications from one source address within a
    /// minute before it is blocked for `auth_block_secs`: nothing from it
    /// is read until the block ends. Only wrong credentials answering a
    /// challenge this node issued count, and never from a source exempt
    /// from `max_messages_per_source`. `0` blocks nobody
    /// (`docs/EDGE_SECURITY.md` §2.8).
    pub max_auth_failures_per_source: u32,

    /// How long a source that failed authentication too often is blocked,
    /// in seconds (1 to 86,400).
    pub auth_block_secs: u64,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            signaling_acl: IpAcl::default(),
            media_acl: None,
            malformed_sip_policy: MalformedSipPolicy::LogAndDrop,
            max_message_size: 65536,
            max_headers: 100,
            max_uri_length: 2048,
            block_suspicious_uris: true,
            validate_source_ip: true,
            max_tracked_sources: 100_000,
            max_messages_per_source: fcp_sip::ingress::DEFAULT_MESSAGES_PER_SOURCE,
            trusted_sources: Vec::new(),
            max_auth_failures_per_source: fcp_sip::auth_block::DEFAULT_MAX_AUTH_FAILURES,
            auth_block_secs: fcp_sip::auth_block::DEFAULT_AUTH_BLOCK_SECS,
        }
    }
}

impl SecurityConfig {
    /// Validate the configuration.
    ///
    /// Returns `Err` if:
    /// - Any ACL rule has an invalid CIDR
    /// - Any ACL description contains CR/LF
    /// - `max_message_size` is zero or exceeds 1MB
    /// - `max_headers` is zero or exceeds 1000
    /// - `max_uri_length` is zero or exceeds 65536
    /// - `max_tracked_sources` exceeds 10,000,000
    pub fn validate(&self) -> Result<(), String> {
        self.signaling_acl.validate("signaling_acl")?;
        fcp_sip::auth_block::validate(self.max_auth_failures_per_source, self.auth_block_secs)?;
        if let Some(ref media) = self.media_acl {
            media.validate("media_acl")?;
        }
        if self.max_message_size == 0 || self.max_message_size > 1_048_576 {
            return Err("max_message_size must be between 1 and 1,048,576".into());
        }
        if self.max_headers == 0 || self.max_headers > 1000 {
            return Err("max_headers must be between 1 and 1000".into());
        }
        if self.max_uri_length == 0 || self.max_uri_length > 65536 {
            return Err("max_uri_length must be between 1 and 65536".into());
        }
        if self.max_tracked_sources > 10_000_000 {
            return Err("max_tracked_sources must not exceed 10,000,000".into());
        }
        fcp_sip::emergency::TrustedEdges::parse(&self.trusted_sources)
            .map_err(|e| format!("trusted_sources: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn test_default() {
        let cfg = SecurityConfig::default();
        assert!(cfg.signaling_acl.rules.is_empty());
        assert_eq!(cfg.signaling_acl.default_action, AclAction::Allow);
        assert!(cfg.media_acl.is_none());
        assert_eq!(cfg.malformed_sip_policy, MalformedSipPolicy::LogAndDrop);
        assert_eq!(cfg.max_message_size, 65536);
        assert_eq!(cfg.max_headers, 100);
        assert_eq!(cfg.max_uri_length, 2048);
        assert!(cfg.block_suspicious_uris);
        assert!(cfg.validate_source_ip);
        assert_eq!(cfg.max_tracked_sources, 100_000);
    }

    #[test]
    fn test_validate_valid() {
        assert!(SecurityConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_with_acl_rules() {
        let cfg = SecurityConfig {
            signaling_acl: IpAcl {
                rules: vec![
                    IpAclEntry {
                        cidr: "10.0.0.0/8".into(),
                        action: AclAction::Allow,
                        description: Some("Internal network".into()),
                    },
                    IpAclEntry {
                        cidr: "0.0.0.0/0".into(),
                        action: AclAction::Deny,
                        description: None,
                    },
                ],
                default_action: AclAction::Deny,
            },
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_cidr() {
        let cfg = SecurityConfig {
            signaling_acl: IpAcl {
                rules: vec![IpAclEntry {
                    cidr: "not-a-cidr".into(),
                    action: AclAction::Allow,
                    description: None,
                }],
                default_action: AclAction::Deny,
            },
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("invalid CIDR"));
    }

    #[test]
    fn test_validate_crlf_description() {
        let cfg = SecurityConfig {
            signaling_acl: IpAcl {
                rules: vec![IpAclEntry {
                    cidr: "10.0.0.0/8".into(),
                    action: AclAction::Allow,
                    description: Some("bad\ndescription".into()),
                }],
                default_action: AclAction::Allow,
            },
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_zero_message_size() {
        let cfg = SecurityConfig {
            max_message_size: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_message_size"));
    }

    #[test]
    fn test_validate_excessive_message_size() {
        let cfg = SecurityConfig {
            max_message_size: 1_048_577,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_message_size"));
    }

    #[test]
    fn test_validate_zero_max_headers() {
        let cfg = SecurityConfig {
            max_headers: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_headers"));
    }

    #[test]
    fn test_validate_excessive_max_headers() {
        let cfg = SecurityConfig {
            max_headers: 1001,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_headers"));
    }

    #[test]
    fn test_validate_zero_uri_length() {
        let cfg = SecurityConfig {
            max_uri_length: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_uri_length"));
    }

    #[test]
    fn test_validate_excessive_uri_length() {
        let cfg = SecurityConfig {
            max_uri_length: 65537,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_uri_length"));
    }

    #[test]
    fn test_validate_excessive_tracked_sources() {
        let cfg = SecurityConfig {
            max_tracked_sources: 10_000_001,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_tracked_sources"));
    }

    #[test]
    fn test_validate_boundary_values() {
        let mut cfg = SecurityConfig {
            max_message_size: 1,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
        cfg.max_message_size = 1_048_576;
        assert!(cfg.validate().is_ok());

        cfg.max_headers = 1;
        assert!(cfg.validate().is_ok());
        cfg.max_headers = 1000;
        assert!(cfg.validate().is_ok());

        cfg.max_uri_length = 1;
        assert!(cfg.validate().is_ok());
        cfg.max_uri_length = 65536;
        assert!(cfg.validate().is_ok());

        cfg.max_tracked_sources = 10_000_000;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_evaluate_allow() {
        let acl = IpAcl {
            rules: vec![IpAclEntry {
                cidr: "10.0.0.0/8".into(),
                action: AclAction::Allow,
                description: None,
            }],
            default_action: AclAction::Deny,
        };
        let ip: IpAddr = "10.1.2.3".parse().unwrap();
        assert_eq!(acl.evaluate(ip), AclAction::Allow);
    }

    #[test]
    fn test_evaluate_deny() {
        let acl = IpAcl {
            rules: vec![IpAclEntry {
                cidr: "10.0.0.0/8".into(),
                action: AclAction::Allow,
                description: None,
            }],
            default_action: AclAction::Deny,
        };
        let ip: IpAddr = "192.168.1.1".parse().unwrap();
        assert_eq!(acl.evaluate(ip), AclAction::Deny);
    }

    #[test]
    fn test_evaluate_first_match_wins() {
        let acl = IpAcl {
            rules: vec![
                IpAclEntry {
                    cidr: "10.0.0.0/24".into(),
                    action: AclAction::Deny,
                    description: None,
                },
                IpAclEntry {
                    cidr: "10.0.0.0/8".into(),
                    action: AclAction::Allow,
                    description: None,
                },
            ],
            default_action: AclAction::Deny,
        };
        // 10.0.0.5 matches /24 (Deny) first
        let ip: IpAddr = "10.0.0.5".parse().unwrap();
        assert_eq!(acl.evaluate(ip), AclAction::Deny);

        // 10.1.0.5 only matches /8 (Allow)
        let ip2: IpAddr = "10.1.0.5".parse().unwrap();
        assert_eq!(acl.evaluate(ip2), AclAction::Allow);
    }

    #[test]
    fn test_evaluate_default_action() {
        let acl = IpAcl {
            rules: Vec::new(),
            default_action: AclAction::Allow,
        };
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        assert_eq!(acl.evaluate(ip), AclAction::Allow);
    }

    #[test]
    fn test_evaluate_ipv6() {
        let acl = IpAcl {
            rules: vec![IpAclEntry {
                cidr: "::1/128".into(),
                action: AclAction::Allow,
                description: None,
            }],
            default_action: AclAction::Deny,
        };
        let ip: IpAddr = "::1".parse().unwrap();
        assert_eq!(acl.evaluate(ip), AclAction::Allow);

        let ip2: IpAddr = "::2".parse().unwrap();
        assert_eq!(acl.evaluate(ip2), AclAction::Deny);
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = SecurityConfig {
            signaling_acl: IpAcl {
                rules: vec![IpAclEntry {
                    cidr: "10.0.0.0/8".into(),
                    action: AclAction::Allow,
                    description: Some("Internal".into()),
                }],
                default_action: AclAction::Deny,
            },
            malformed_sip_policy: MalformedSipPolicy::Reject,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SecurityConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
