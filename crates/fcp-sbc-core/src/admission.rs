//! Call admission control configuration for the SBC.
//!
//! Controls the maximum calls-per-second (CPS), maximum concurrent calls
//! and registrations, per-trunk CPS limits, and overload behavior.

use serde::{Deserialize, Serialize};

/// Action to take when the SBC is overloaded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdmissionAction {
    /// Reject new calls with 503 Service Unavailable.
    #[default]
    Reject,

    /// Queue new calls for later processing (future use).
    Queue,
}

/// Per-trunk CPS limit entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkCpsLimit {
    /// Name of the trunk this limit applies to.
    pub trunk_name: String,

    /// Maximum calls per second for this trunk.
    pub max_cps: u32,
}

/// Call admission control configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionConfig {
    /// Maximum global calls per second.
    pub max_cps: u32,

    /// Maximum concurrent active calls.
    pub max_concurrent_calls: u32,

    /// Maximum concurrent registrations.
    pub max_concurrent_registrations: u32,

    /// Per-trunk CPS overrides.
    pub per_trunk_cps: Vec<TrunkCpsLimit>,

    /// Action to take when overloaded.
    pub overload_action: AdmissionAction,

    /// Include Retry-After header in 503 responses.
    pub include_retry_after: bool,

    /// Retry-After value in seconds.
    pub retry_after_secs: u64,

    /// Maximum number of calls to queue when `overload_action` is `Queue`.
    pub queue_max_size: u32,

    /// Timeout in seconds for queued calls before rejecting with 503.
    pub queue_timeout_secs: u64,
}

impl Default for AdmissionConfig {
    fn default() -> Self {
        Self {
            max_cps: 100,
            max_concurrent_calls: 1000,
            max_concurrent_registrations: 5000,
            per_trunk_cps: Vec::new(),
            overload_action: AdmissionAction::Reject,
            include_retry_after: true,
            retry_after_secs: 5,
            queue_max_size: 100,
            queue_timeout_secs: 10,
        }
    }
}

impl AdmissionConfig {
    /// Validate the configuration.
    ///
    /// Returns `Err` if:
    /// - `max_cps` exceeds 100,000
    /// - `max_concurrent_calls` exceeds 1,000,000
    /// - `retry_after_secs` is not in 1..=3600
    /// - Any trunk name is empty or contains CR/LF
    pub fn validate(&self) -> Result<(), String> {
        if self.max_cps > 100_000 {
            return Err("max_cps must not exceed 100,000".into());
        }
        if self.max_concurrent_calls > 1_000_000 {
            return Err("max_concurrent_calls must not exceed 1,000,000".into());
        }
        if self.retry_after_secs < 1 || self.retry_after_secs > 3600 {
            return Err("retry_after_secs must be between 1 and 3600".into());
        }
        if self.queue_max_size < 1 || self.queue_max_size > 10_000 {
            return Err("queue_max_size must be between 1 and 10,000".into());
        }
        if self.queue_timeout_secs < 1 || self.queue_timeout_secs > 60 {
            return Err("queue_timeout_secs must be between 1 and 60".into());
        }
        for entry in &self.per_trunk_cps {
            if entry.trunk_name.is_empty() {
                return Err("per_trunk_cps trunk_name must not be empty".into());
            }
            if entry.trunk_name.contains('\r') || entry.trunk_name.contains('\n') {
                return Err("per_trunk_cps trunk_name must not contain CR/LF characters".into());
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
        let cfg = AdmissionConfig::default();
        assert_eq!(cfg.max_cps, 100);
        assert_eq!(cfg.max_concurrent_calls, 1000);
        assert_eq!(cfg.max_concurrent_registrations, 5000);
        assert!(cfg.per_trunk_cps.is_empty());
        assert_eq!(cfg.overload_action, AdmissionAction::Reject);
        assert!(cfg.include_retry_after);
        assert_eq!(cfg.retry_after_secs, 5);
    }

    #[test]
    fn test_validate_valid() {
        assert!(AdmissionConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_with_trunk_limits() {
        let cfg = AdmissionConfig {
            per_trunk_cps: vec![
                TrunkCpsLimit {
                    trunk_name: "carrier-a".into(),
                    max_cps: 50,
                },
                TrunkCpsLimit {
                    trunk_name: "carrier-b".into(),
                    max_cps: 30,
                },
            ],
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_excessive_cps() {
        let cfg = AdmissionConfig {
            max_cps: 100_001,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_cps"));
    }

    #[test]
    fn test_validate_excessive_concurrent_calls() {
        let cfg = AdmissionConfig {
            max_concurrent_calls: 1_000_001,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_concurrent_calls"));
    }

    #[test]
    fn test_validate_retry_after_too_low() {
        let cfg = AdmissionConfig {
            retry_after_secs: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("retry_after_secs"));
    }

    #[test]
    fn test_validate_retry_after_too_high() {
        let cfg = AdmissionConfig {
            retry_after_secs: 3601,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("retry_after_secs"));
    }

    #[test]
    fn test_validate_empty_trunk_name() {
        let cfg = AdmissionConfig {
            per_trunk_cps: vec![TrunkCpsLimit {
                trunk_name: String::new(),
                max_cps: 10,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("trunk_name"));
    }

    #[test]
    fn test_validate_crlf_trunk_name() {
        let cfg = AdmissionConfig {
            per_trunk_cps: vec![TrunkCpsLimit {
                trunk_name: "bad\r\nname".into(),
                max_cps: 10,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_boundary_values() {
        let mut cfg = AdmissionConfig {
            max_cps: 100_000,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());

        // Exactly 1,000,000 concurrent calls is valid
        cfg.max_concurrent_calls = 1_000_000;
        assert!(cfg.validate().is_ok());

        // Retry-After boundaries
        cfg.retry_after_secs = 1;
        assert!(cfg.validate().is_ok());
        cfg.retry_after_secs = 3600;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = AdmissionConfig {
            per_trunk_cps: vec![TrunkCpsLimit {
                trunk_name: "carrier-a".into(),
                max_cps: 50,
            }],
            overload_action: AdmissionAction::Queue,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: AdmissionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
