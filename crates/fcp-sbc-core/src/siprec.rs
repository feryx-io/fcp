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

    /// Record calls matching specific criteria (future: rules engine).
    Selective,
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
}

impl Default for SiprecConfig {
    fn default() -> Self {
        Self {
            policy: RecordingPolicy::None,
            servers: Vec::new(),
            include_metadata: true,
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let cfg = SiprecConfig::default();
        assert_eq!(cfg.policy, RecordingPolicy::None);
        assert!(cfg.servers.is_empty());
        assert!(cfg.include_metadata);
    }

    #[test]
    fn test_validate_valid_none() {
        assert!(SiprecConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_valid_all() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::All,
            servers: vec![RecordingServer {
                name: "recorder-1".into(),
                uri: "sip:recorder@example.com".into(),
                enabled: true,
                priority: 1,
            }],
            include_metadata: true,
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
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_none_policy_skips_server_count() {
        // None policy doesn't require servers
        let cfg = SiprecConfig {
            policy: RecordingPolicy::None,
            servers: Vec::new(),
            include_metadata: true,
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = SiprecConfig {
            policy: RecordingPolicy::All,
            servers: vec![RecordingServer {
                name: "recorder-1".into(),
                uri: "sip:recorder@example.com".into(),
                enabled: true,
                priority: 1,
            }],
            include_metadata: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SiprecConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
