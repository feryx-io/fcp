//! Per-IP and per-method rate limiting policy configuration for the SBC.
//!
//! Defines rate limit entries, method-specific limits (INVITE, REGISTER,
//! OPTIONS, auth attempts), and an aggregate policy config.

use serde::{Deserialize, Serialize};

/// A single rate limit entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitEntry {
    /// Whether this rate limit is active.
    pub enabled: bool,

    /// Maximum requests allowed in the window.
    pub max_requests: u32,

    /// Window duration in seconds.
    pub window_secs: u64,

    /// Optional burst capacity (requests allowed above steady-state before throttling).
    pub burst_capacity: Option<u32>,
}

impl RateLimitEntry {
    /// Validate this entry.
    ///
    /// Returns `Err` if enabled and:
    /// - `max_requests` is zero or exceeds 1,000,000
    /// - `window_secs` is zero or exceeds 86,400
    /// - `burst_capacity` exceeds 10,000
    pub fn validate(&self, context: &str) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.max_requests == 0 {
            return Err(format!(
                "{context}: max_requests must be greater than zero when enabled"
            ));
        }
        if self.max_requests > 1_000_000 {
            return Err(format!("{context}: max_requests must not exceed 1,000,000"));
        }
        if self.window_secs == 0 {
            return Err(format!(
                "{context}: window_secs must be greater than zero when enabled"
            ));
        }
        if self.window_secs > 86_400 {
            return Err(format!("{context}: window_secs must not exceed 86,400"));
        }
        if let Some(burst) = self.burst_capacity {
            if burst > 10_000 {
                return Err(format!("{context}: burst_capacity must not exceed 10,000"));
            }
        }
        Ok(())
    }
}

/// Per-method rate limits applied per source IP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodRateLimits {
    /// INVITE rate limit.
    pub invite: RateLimitEntry,

    /// REGISTER rate limit.
    pub register: RateLimitEntry,

    /// OPTIONS rate limit.
    pub options: RateLimitEntry,

    /// Authentication attempt rate limit (failed 401/407 retries).
    pub auth_attempts: RateLimitEntry,
}

impl Default for MethodRateLimits {
    fn default() -> Self {
        Self {
            invite: RateLimitEntry {
                enabled: true,
                max_requests: 30,
                window_secs: 60,
                burst_capacity: Some(10),
            },
            register: RateLimitEntry {
                enabled: true,
                max_requests: 60,
                window_secs: 3600,
                burst_capacity: Some(10),
            },
            options: RateLimitEntry {
                enabled: true,
                max_requests: 60,
                window_secs: 60,
                burst_capacity: Some(20),
            },
            auth_attempts: RateLimitEntry {
                enabled: true,
                max_requests: 10,
                window_secs: 300,
                burst_capacity: Some(3),
            },
        }
    }
}

impl MethodRateLimits {
    /// Validate all method entries.
    pub fn validate(&self, prefix: &str) -> Result<(), String> {
        self.invite.validate(&format!("{prefix}.invite"))?;
        self.register.validate(&format!("{prefix}.register"))?;
        self.options.validate(&format!("{prefix}.options"))?;
        self.auth_attempts
            .validate(&format!("{prefix}.auth_attempts"))?;
        Ok(())
    }
}

/// Aggregate rate limiting policy configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RateLimitPolicyConfig {
    /// Whether rate limiting is enabled globally.
    pub enabled: bool,

    /// Per-source-IP method rate limits.
    pub per_ip: MethodRateLimits,

    /// Global (aggregate) rate limit across all sources.
    pub global: RateLimitEntry,

    /// Seconds before idle per-IP entries are cleaned up.
    pub idle_cleanup_secs: u64,
}

impl Default for RateLimitPolicyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            per_ip: MethodRateLimits::default(),
            global: RateLimitEntry {
                enabled: true,
                max_requests: 10_000,
                window_secs: 60,
                burst_capacity: None,
            },
            idle_cleanup_secs: 300,
        }
    }
}

impl RateLimitPolicyConfig {
    /// Validate the configuration.
    ///
    /// Returns `Err` if any enabled entry fails validation.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        self.per_ip.validate("per_ip")?;
        self.global.validate("global")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let cfg = RateLimitPolicyConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.per_ip.invite.max_requests, 30);
        assert_eq!(cfg.per_ip.invite.window_secs, 60);
        assert_eq!(cfg.per_ip.invite.burst_capacity, Some(10));
        assert_eq!(cfg.per_ip.register.max_requests, 60);
        assert_eq!(cfg.per_ip.register.window_secs, 3600);
        assert_eq!(cfg.per_ip.options.max_requests, 60);
        assert_eq!(cfg.per_ip.options.window_secs, 60);
        assert_eq!(cfg.per_ip.auth_attempts.max_requests, 10);
        assert_eq!(cfg.per_ip.auth_attempts.window_secs, 300);
        assert_eq!(cfg.per_ip.auth_attempts.burst_capacity, Some(3));
        assert_eq!(cfg.global.max_requests, 10_000);
        assert_eq!(cfg.global.window_secs, 60);
        assert_eq!(cfg.idle_cleanup_secs, 300);
    }

    #[test]
    fn test_validate_valid() {
        assert!(RateLimitPolicyConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_disabled_skips() {
        let cfg = RateLimitPolicyConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_zero_max_requests() {
        let mut cfg = RateLimitPolicyConfig::default();
        cfg.per_ip.invite.max_requests = 0;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_requests"));
        assert!(err.contains("invite"));
    }

    #[test]
    fn test_validate_excessive_max_requests() {
        let mut cfg = RateLimitPolicyConfig::default();
        cfg.global.max_requests = 1_000_001;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_requests"));
    }

    #[test]
    fn test_validate_zero_window() {
        let mut cfg = RateLimitPolicyConfig::default();
        cfg.per_ip.register.window_secs = 0;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("window_secs"));
    }

    #[test]
    fn test_validate_excessive_window() {
        let mut cfg = RateLimitPolicyConfig::default();
        cfg.per_ip.options.window_secs = 86_401;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("window_secs"));
    }

    #[test]
    fn test_validate_excessive_burst() {
        let mut cfg = RateLimitPolicyConfig::default();
        cfg.per_ip.auth_attempts.burst_capacity = Some(10_001);
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("burst_capacity"));
    }

    #[test]
    fn test_validate_disabled_entry_skips() {
        let mut cfg = RateLimitPolicyConfig::default();
        cfg.per_ip.invite.enabled = false;
        cfg.per_ip.invite.max_requests = 0; // would be invalid if enabled
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_boundary_values() {
        let mut cfg = RateLimitPolicyConfig::default();

        cfg.global.max_requests = 1_000_000;
        assert!(cfg.validate().is_ok());

        cfg.global.window_secs = 86_400;
        assert!(cfg.validate().is_ok());

        cfg.per_ip.invite.burst_capacity = Some(10_000);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = RateLimitPolicyConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: RateLimitPolicyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
