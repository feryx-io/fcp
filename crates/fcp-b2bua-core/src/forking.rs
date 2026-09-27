//! Call forking policy configuration for B2BUA.
//!
//! Controls how the B2BUA forks outbound call attempts when multiple
//! destinations are available. This is *policy-level* configuration that
//! feeds into the runtime forking logic in `fcp_sip`'s handler.
//!
//! - **PBX**: Parallel or sequential forking for ring groups
//! - **SBC**: Single target (no forking, upstream routing decides)
//! - **Session Manager**: Sequential failover between nodes

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How the B2BUA distributes outbound INVITE attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ForkingMode {
    /// Send INVITE to a single destination only.
    #[default]
    Single,

    /// Send INVITEs to all destinations simultaneously.
    Parallel,

    /// Try destinations one at a time, moving to the next on failure/timeout.
    Sequential,
}

/// How early media (183 Session Progress) from forked legs is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EarlyMediaPolicy {
    /// Only relay early media from the first responding leg.
    #[default]
    FirstOnly,

    /// Do not relay any early media to the caller.
    None,
}

/// Policy-level configuration for call forking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ForkingConfig {
    /// Forking mode.
    pub mode: ForkingMode,

    /// Maximum time to wait for any leg to answer before giving up.
    #[serde(with = "duration_secs")]
    pub timeout: Duration,

    /// Maximum number of parallel outbound legs (only relevant for `Parallel` mode).
    pub max_parallel_legs: usize,

    /// How early media from forked legs is relayed to the caller.
    pub early_media_policy: EarlyMediaPolicy,

    /// Suppress duplicate provisional responses (e.g., multiple 180 Ringing).
    pub deduplicate_provisionals: bool,

    /// When one leg answers and others are cancelled, propagate CANCEL to all
    /// remaining legs.
    pub propagate_cancel: bool,
}

impl Default for ForkingConfig {
    fn default() -> Self {
        Self {
            mode: ForkingMode::Single,
            timeout: Duration::from_secs(30),
            max_parallel_legs: 10,
            early_media_policy: EarlyMediaPolicy::FirstOnly,
            deduplicate_provisionals: true,
            propagate_cancel: true,
        }
    }
}

impl ForkingConfig {
    /// PBX defaults for parallel forking (ring groups).
    pub fn pbx_parallel() -> Self {
        Self {
            mode: ForkingMode::Parallel,
            timeout: Duration::from_secs(30),
            max_parallel_legs: 10,
            early_media_policy: EarlyMediaPolicy::FirstOnly,
            deduplicate_provisionals: true,
            propagate_cancel: true,
        }
    }

    /// PBX defaults for sequential forking (hunt groups).
    pub fn pbx_sequential() -> Self {
        Self {
            mode: ForkingMode::Sequential,
            timeout: Duration::from_secs(20),
            max_parallel_legs: 1,
            early_media_policy: EarlyMediaPolicy::FirstOnly,
            deduplicate_provisionals: true,
            propagate_cancel: true,
        }
    }

    /// SBC defaults: single target, no forking.
    pub fn sbc_defaults() -> Self {
        Self::default()
    }

    /// Session Manager defaults: sequential failover between nodes.
    pub fn session_manager_defaults() -> Self {
        Self {
            mode: ForkingMode::Sequential,
            timeout: Duration::from_secs(5),
            max_parallel_legs: 1,
            early_media_policy: EarlyMediaPolicy::None,
            deduplicate_provisionals: true,
            propagate_cancel: true,
        }
    }

    /// Validate the configuration.
    ///
    /// Returns `Err` if:
    /// - `timeout` is zero or exceeds 300 seconds
    /// - `max_parallel_legs` is zero or exceeds 50
    pub fn validate(&self) -> Result<(), String> {
        if self.timeout.is_zero() {
            return Err("timeout must be greater than zero".into());
        }
        if self.timeout > Duration::from_secs(300) {
            return Err("timeout must not exceed 300 seconds".into());
        }
        if self.max_parallel_legs == 0 {
            return Err("max_parallel_legs must be greater than zero".into());
        }
        if self.max_parallel_legs > 50 {
            return Err("max_parallel_legs must not exceed 50".into());
        }
        Ok(())
    }
}

/// Custom serde module for `Duration` as u64 seconds.
///
/// Serializes `Duration` as a u64 representing whole seconds (subsecond
/// precision is truncated). No additional dependencies required.
mod duration_secs {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let cfg = ForkingConfig::default();
        assert_eq!(cfg.mode, ForkingMode::Single);
        assert_eq!(cfg.timeout, Duration::from_secs(30));
        assert_eq!(cfg.max_parallel_legs, 10);
        assert_eq!(cfg.early_media_policy, EarlyMediaPolicy::FirstOnly);
        assert!(cfg.deduplicate_provisionals);
        assert!(cfg.propagate_cancel);
    }

    #[test]
    fn test_pbx_parallel() {
        let cfg = ForkingConfig::pbx_parallel();
        assert_eq!(cfg.mode, ForkingMode::Parallel);
        assert_eq!(cfg.timeout, Duration::from_secs(30));
        assert_eq!(cfg.max_parallel_legs, 10);
    }

    #[test]
    fn test_pbx_sequential() {
        let cfg = ForkingConfig::pbx_sequential();
        assert_eq!(cfg.mode, ForkingMode::Sequential);
        assert_eq!(cfg.timeout, Duration::from_secs(20));
        assert_eq!(cfg.max_parallel_legs, 1);
    }

    #[test]
    fn test_sbc_defaults() {
        let cfg = ForkingConfig::sbc_defaults();
        assert_eq!(cfg.mode, ForkingMode::Single);
    }

    #[test]
    fn test_session_manager_defaults() {
        let cfg = ForkingConfig::session_manager_defaults();
        assert_eq!(cfg.mode, ForkingMode::Sequential);
        assert_eq!(cfg.timeout, Duration::from_secs(5));
        assert_eq!(cfg.early_media_policy, EarlyMediaPolicy::None);
    }

    #[test]
    fn test_validate_valid() {
        assert!(ForkingConfig::default().validate().is_ok());
        assert!(ForkingConfig::pbx_parallel().validate().is_ok());
        assert!(ForkingConfig::pbx_sequential().validate().is_ok());
        assert!(ForkingConfig::session_manager_defaults().validate().is_ok());
    }

    #[test]
    fn test_validate_zero_timeout() {
        let cfg = ForkingConfig {
            timeout: Duration::ZERO,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("greater than zero"));
    }

    #[test]
    fn test_validate_excessive_timeout() {
        let cfg = ForkingConfig {
            timeout: Duration::from_secs(301),
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("300 seconds"));
    }

    #[test]
    fn test_validate_zero_max_legs() {
        let cfg = ForkingConfig {
            max_parallel_legs: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("greater than zero"));
    }

    #[test]
    fn test_validate_excessive_max_legs() {
        let cfg = ForkingConfig {
            max_parallel_legs: 51,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("50"));
    }

    #[test]
    fn test_validate_boundary_values() {
        // Exactly 300s is valid
        let mut cfg = ForkingConfig {
            timeout: Duration::from_secs(300),
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());

        // Exactly 50 legs is valid
        cfg.max_parallel_legs = 50;
        assert!(cfg.validate().is_ok());

        // 1 second is valid
        cfg.timeout = Duration::from_secs(1);
        assert!(cfg.validate().is_ok());

        // 1 leg is valid
        cfg.max_parallel_legs = 1;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = ForkingConfig::pbx_parallel();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: ForkingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_duration_serde_as_seconds() {
        let cfg = ForkingConfig {
            timeout: Duration::from_secs(42),
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("42"));

        let deserialized: ForkingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.timeout, Duration::from_secs(42));
    }

    #[test]
    fn test_duration_serde_from_integer() {
        // Ensure we can deserialize from a bare integer
        let json = r#"{"mode":"Single","timeout":15,"max_parallel_legs":5,"early_media_policy":"FirstOnly","deduplicate_provisionals":false,"propagate_cancel":true}"#;
        let cfg: ForkingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.timeout, Duration::from_secs(15));
        assert_eq!(cfg.max_parallel_legs, 5);
        assert!(!cfg.deduplicate_provisionals);
    }
}
