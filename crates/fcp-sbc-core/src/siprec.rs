//! SIPREC recording configuration for the SBC.
//!
//! Provides a stub configuration for SIP-based media recording (RFC 7866).
//! The v1 implementation supports policy selection and server definitions
//! but does not contain recording runtime logic.

use serde::{Deserialize, Serialize};

/// Recording policy: when to record calls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecordingPolicy {
    /// No recording.
    #[default]
    None,

    /// Record all calls.
    All,

    /// Record calls matching selective recording rules.
    Selective,
}

/// A selective recording rule that matches calls by caller/callee pattern,
/// trunk name, and/or call direction.
///
/// When policy is `Selective`, a call is recorded if it matches at least
/// one enabled rule. All non-`None` fields must match (AND logic within
/// a rule); any rule matching is sufficient (OR logic across rules).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectiveRecordingRule {
    /// Rule name (for display/logging).
    pub name: String,

    /// Whether this rule is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Regex pattern matched against the caller URI (From).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_pattern: Option<String>,

    /// Regex pattern matched against the callee URI (To).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callee_pattern: Option<String>,

    /// Trunk names that this rule applies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk_names: Option<Vec<String>>,

    /// Call direction filter: "inbound" or "outbound".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
}

fn default_true() -> bool {
    true
}

/// A SIPREC recording server definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingServer {
    /// Server name (for display/logging).
    pub name: String,

    /// SIP URI of the recording server (e.g. "sip:recorder@example.com").
    pub uri: String,

    /// Whether this server is enabled.
    pub enabled: bool,

    /// Selection priority (lower = higher priority).
    pub priority: u32,
}

/// SIPREC recording configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiprecConfig {
    /// Recording policy.
    pub policy: RecordingPolicy,

    /// List of recording servers.
    pub servers: Vec<RecordingServer>,

    /// Include call metadata in SIPREC sessions.
    pub include_metadata: bool,

    /// Selective recording rules (used when policy is `Selective`).
    #[serde(default)]
    pub rules: Vec<SelectiveRecordingRule>,
}

impl Default for SiprecConfig {
    fn default() -> Self {
        Self {
            policy: RecordingPolicy::None,
            servers: Vec::new(),
            include_metadata: true,
            rules: Vec::new(),
        }
    }
}

impl SiprecConfig {
    /// Validate the configuration.
    ///
    /// Returns `Err` if:
    /// - Policy is not `None` but no enabled servers are configured
    /// - Any server URI does not start with "sip:" or "sips:"
    /// - Any server name is empty or contains CR/LF
    /// - Policy is `Selective` but no enabled rules are configured
    /// - Any rule has an invalid `direction` value
    pub fn validate(&self) -> Result<(), String> {
        if self.policy != RecordingPolicy::None {
            let has_enabled = self.servers.iter().any(|s| s.enabled);
            if !has_enabled {
                return Err(
                    "at least one enabled recording server is required when policy is not None"
                        .into(),
                );
            }
        }
        for (i, server) in self.servers.iter().enumerate() {
            if server.name.is_empty() {
                return Err(format!("servers[{i}]: name must not be empty"));
            }
            if server.name.contains('\r') || server.name.contains('\n') {
                return Err(format!(
                    "servers[{i}]: name must not contain CR/LF characters"
                ));
            }
            if !server.uri.starts_with("sip:") && !server.uri.starts_with("sips:") {
                return Err(format!(
                    "servers[{i}]: uri must start with \"sip:\" or \"sips:\""
                ));
            }
        }
        // Selective policy requires at least one enabled rule
        if self.policy == RecordingPolicy::Selective {
            let has_enabled_rule = self.rules.iter().any(|r| r.enabled);
            if !has_enabled_rule {
                return Err(
                    "at least one enabled rule is required when policy is Selective".into(),
                );
            }
        }
        // Validate rule direction values
        for (i, rule) in self.rules.iter().enumerate() {
            if let Some(ref dir) = rule.direction {
                if dir != "inbound" && dir != "outbound" {
                    return Err(format!(
                        "rules[{i}]: direction must be \"inbound\" or \"outbound\", got \"{dir}\""
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_server() -> Vec<RecordingServer> {
        vec![RecordingServer {
            name: "recorder-1".into(),
            uri: "sip:recorder@example.com".into(),
            enabled: true,
            priority: 1,
        }]
    }

    fn one_rule() -> SelectiveRecordingRule {
        SelectiveRecordingRule {
            name: "record-all-inbound".into(),
            enabled: true,
            caller_pattern: None,
            callee_pattern: None,
            trunk_names: None,
            direction: Some("inbound".into()),
        }
    }

    #[test]
    fn test_default() {
        let cfg = SiprecConfig::default();
        assert_eq!(cfg.policy, RecordingPolicy::None);
        assert!(cfg.servers.is_empty());
        assert!(cfg.include_metadata);
        assert!(cfg.rules.is_empty());
    }

    #[test]
    fn test_validate_valid_none() {
        assert!(SiprecConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_valid_all() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::All,
            servers: one_server(),
            include_metadata: true,
            rules: vec![],
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_valid_sips_uri() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::All,
            servers: vec![RecordingServer {
                name: "recorder-1".into(),
                uri: "sips:recorder@example.com".into(),
                enabled: true,
                priority: 1,
            }],
            include_metadata: true,
            rules: vec![],
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_no_enabled_server() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::All,
            servers: vec![RecordingServer {
                name: "recorder-1".into(),
                uri: "sip:recorder@example.com".into(),
                enabled: false,
                priority: 1,
            }],
            include_metadata: true,
            rules: vec![],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("enabled recording server"));
    }

    #[test]
    fn test_validate_no_servers() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::Selective,
            servers: Vec::new(),
            include_metadata: true,
            rules: vec![one_rule()],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("enabled recording server"));
    }

    #[test]
    fn test_validate_invalid_uri() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::None,
            servers: vec![RecordingServer {
                name: "recorder-1".into(),
                uri: "http://example.com".into(),
                enabled: true,
                priority: 1,
            }],
            include_metadata: true,
            rules: vec![],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("sip:"));
    }

    #[test]
    fn test_validate_empty_server_name() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::None,
            servers: vec![RecordingServer {
                name: String::new(),
                uri: "sip:recorder@example.com".into(),
                enabled: true,
                priority: 1,
            }],
            include_metadata: true,
            rules: vec![],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("name"));
    }

    #[test]
    fn test_validate_crlf_server_name() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::None,
            servers: vec![RecordingServer {
                name: "bad\r\nname".into(),
                uri: "sip:recorder@example.com".into(),
                enabled: true,
                priority: 1,
            }],
            include_metadata: true,
            rules: vec![],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_none_policy_skips_server_count() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::None,
            servers: Vec::new(),
            include_metadata: true,
            rules: vec![],
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::All,
            servers: one_server(),
            include_metadata: false,
            rules: vec![],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SiprecConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    // --- Selective recording rule validation ---

    #[test]
    fn test_validate_selective_requires_rules() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::Selective,
            servers: one_server(),
            include_metadata: true,
            rules: vec![], // no rules
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("enabled rule"));
    }

    #[test]
    fn test_validate_selective_with_rule() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::Selective,
            servers: one_server(),
            include_metadata: true,
            rules: vec![one_rule()],
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_selective_disabled_rules_not_counted() {
        let mut rule = one_rule();
        rule.enabled = false;
        let cfg = SiprecConfig {
            policy: RecordingPolicy::Selective,
            servers: one_server(),
            include_metadata: true,
            rules: vec![rule],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("enabled rule"));
    }

    #[test]
    fn test_validate_invalid_rule_direction() {
        let mut rule = one_rule();
        rule.direction = Some("bidirectional".into()); // invalid
        let cfg = SiprecConfig {
            policy: RecordingPolicy::All,
            servers: one_server(),
            include_metadata: true,
            rules: vec![rule],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("direction"));
        assert!(err.contains("bidirectional"));
    }

    #[test]
    fn test_serde_roundtrip_with_rules() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::Selective,
            servers: one_server(),
            include_metadata: true,
            rules: vec![SelectiveRecordingRule {
                name: "record-sales".into(),
                enabled: true,
                caller_pattern: Some("^\\+1".into()),
                callee_pattern: None,
                trunk_names: Some(vec!["carrier-a".into()]),
                direction: Some("inbound".into()),
            }],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SiprecConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
