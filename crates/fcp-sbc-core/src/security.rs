//! Security configuration for the SBC.
//!
//! Provides IP ACL (access control list) evaluation, malformed SIP message
//! policy, and size limits for incoming messages.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// ACL rule action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AclAction {
    /// Allow the traffic.
    #[default]
    Allow,

    /// Deny the traffic.
    Deny,
}

/// A single IP ACL rule entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpAclEntry {
    /// CIDR notation network (e.g. "10.0.0.0/8", "192.168.1.1/32").
    pub cidr: String,

    /// Action to take when this rule matches.
    pub action: AclAction,

    /// Optional description for this rule.
    pub description: Option<String>,
}

/// IP access control list.
///
/// Rules are evaluated in order; the first matching rule wins.
/// If no rule matches, `default_action` is returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpAcl {
    /// Ordered list of ACL rules.
    pub rules: Vec<IpAclEntry>,

    /// Action when no rule matches.
    pub default_action: AclAction,
}

impl Default for IpAcl {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            default_action: AclAction::Allow,
        }
    }
}

impl IpAcl {
    /// Evaluate the ACL for a given IP address.
    ///
    /// Returns the action from the first matching rule, or `default_action`
    /// if no rule matches. Unparseable CIDR entries are skipped.
    pub fn evaluate(&self, ip: IpAddr) -> AclAction {
        for entry in &self.rules {
            if let Ok(network) = entry.cidr.parse::<ipnetwork::IpNetwork>() {
                if network.contains(ip) {
                    return entry.action;
                }
            }
        }
        self.default_action
    }

    /// Validate all ACL entries.
    ///
    /// Returns `Err` if:
    /// - Any `cidr` value cannot be parsed as an IP network
    /// - Any `description` contains CR/LF characters
    pub fn validate(&self, context: &str) -> Result<(), String> {
        for (i, entry) in self.rules.iter().enumerate() {
            if entry.cidr.parse::<ipnetwork::IpNetwork>().is_err() {
                return Err(format!(
                    "{context}: rule {i} has invalid CIDR: {:?}",
                    entry.cidr
                ));
            }
            if let Some(ref desc) = entry.description {
                if desc.contains('\r') || desc.contains('\n') {
                    return Err(format!(
                        "{context}: rule {i} description must not contain CR/LF characters"
                    ));
                }
            }
        }
        Ok(())
    }
}

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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
