//! Configuration for the presence service.

use serde::{Deserialize, Serialize};

/// Presence service configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PresenceConfig {
    /// Whether the presence service is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Default subscription expiry in seconds. Default: 3600.
    #[serde(default = "default_subscription_expiry")]
    pub default_subscription_expiry_secs: u32,

    /// Max watchers per presentity. 0 = unlimited. Default: 100.
    #[serde(default = "default_max_watchers")]
    pub max_watchers_per_presentity: usize,

    /// Max total presentities tracked. Default: 10000.
    #[serde(default = "default_max_presentities")]
    pub max_presentities: usize,

    /// Auto-set OnThePhone when a call is active. Default: true.
    #[serde(default = "default_true")]
    pub auto_phone_state: bool,

    /// Max PUBLISH expiry in seconds. Devices requesting more get capped. Default: 3600.
    #[serde(default = "default_max_publish_expiry")]
    pub max_publish_expiry_secs: u32,

    /// Cleanup interval in seconds for expired watchers/PUBLISHes. Default: 60.
    #[serde(default = "default_cleanup_interval")]
    pub cleanup_interval_secs: u64,

    /// Delay in ms before sending NOTIFY after state change. Default: 50.
    #[serde(default = "default_notify_delay")]
    pub notify_delay_ms: u64,

    /// Minimum subscription expiry in seconds. Default: 60.
    #[serde(default = "default_min_subscription_expiry")]
    pub min_subscription_expiry_secs: u32,

    /// Maximum subscription expiry in seconds. Default: 86400.
    #[serde(default = "default_max_subscription_expiry")]
    pub max_subscription_expiry_secs: u32,

    /// Minimum PUBLISH expiry in seconds. Default: 60.
    #[serde(default = "default_min_publish_expiry")]
    pub min_publish_expiry_secs: u32,

    /// Whether watcher-info event package is enabled. Default: false.
    #[serde(default)]
    pub winfo_enabled: bool,

    /// Whether multi-device presence composition is enabled. Default: false.
    #[serde(default)]
    pub composition_enabled: bool,

    /// Maximum number of NOTIFY delivery retries. Default: 3.
    #[serde(default = "default_max_notify_retries")]
    pub max_notify_retries: u32,

    /// Base delay in milliseconds for NOTIFY retry backoff. Default: 1000 (1s).
    #[serde(default = "default_notify_retry_base_delay_ms")]
    pub notify_retry_base_delay_ms: u64,

    /// Maximum delay in milliseconds for NOTIFY retry backoff. Default: 30000 (30s).
    #[serde(default = "default_notify_retry_max_delay_ms")]
    pub notify_retry_max_delay_ms: u64,

    /// SUBSCRIBE rate limit per source IP (requests per minute). Default: 20.
    #[serde(default = "default_subscribe_rate_limit_per_ip")]
    pub subscribe_rate_limit_per_ip: u32,

    /// SUBSCRIBE rate limit per presentity AOR (requests per minute). Default: 50.
    #[serde(default = "default_subscribe_rate_limit_per_aor")]
    pub subscribe_rate_limit_per_aor: u32,

    /// PUBLISH rate limit per source IP (requests per minute). Default: 30.
    #[serde(default = "default_publish_rate_limit_per_ip")]
    pub publish_rate_limit_per_ip: u32,

    /// PUBLISH rate limit per presentity AOR (requests per minute). Default: 60.
    #[serde(default = "default_publish_rate_limit_per_aor")]
    pub publish_rate_limit_per_aor: u32,
}

fn default_subscription_expiry() -> u32 {
    3600
}

fn default_max_watchers() -> usize {
    100
}

fn default_max_presentities() -> usize {
    10000
}

fn default_true() -> bool {
    true
}

fn default_max_publish_expiry() -> u32 {
    3600
}

fn default_cleanup_interval() -> u64 {
    60
}

fn default_notify_delay() -> u64 {
    50
}

fn default_min_subscription_expiry() -> u32 {
    60
}

fn default_max_subscription_expiry() -> u32 {
    86400
}

fn default_min_publish_expiry() -> u32 {
    60
}

fn default_max_notify_retries() -> u32 {
    3
}

fn default_notify_retry_base_delay_ms() -> u64 {
    1000
}

fn default_notify_retry_max_delay_ms() -> u64 {
    30000
}

fn default_subscribe_rate_limit_per_ip() -> u32 {
    20
}

fn default_subscribe_rate_limit_per_aor() -> u32 {
    50
}

fn default_publish_rate_limit_per_ip() -> u32 {
    30
}

fn default_publish_rate_limit_per_aor() -> u32 {
    60
}

impl PresenceConfig {
    /// Validate configuration constraints and auto-correct invalid values.
    ///
    /// Returns a list of warning messages for any corrections made.
    /// This is fail-open: invalid values are corrected rather than rejected.
    pub fn validate(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();

        // Subscription expiry ordering
        if self.min_subscription_expiry_secs > self.max_subscription_expiry_secs {
            warnings.push(format!(
                "min_subscription_expiry ({}) > max_subscription_expiry ({}), swapping",
                self.min_subscription_expiry_secs, self.max_subscription_expiry_secs
            ));
            std::mem::swap(
                &mut self.min_subscription_expiry_secs,
                &mut self.max_subscription_expiry_secs,
            );
        }

        // Clamp default subscription expiry to [min, max]
        let clamped = self.default_subscription_expiry_secs.clamp(
            self.min_subscription_expiry_secs,
            self.max_subscription_expiry_secs,
        );
        if clamped != self.default_subscription_expiry_secs {
            warnings.push(format!(
                "default_subscription_expiry ({}) outside [{}, {}], clamped to {}",
                self.default_subscription_expiry_secs,
                self.min_subscription_expiry_secs,
                self.max_subscription_expiry_secs,
                clamped,
            ));
            self.default_subscription_expiry_secs = clamped;
        }

        // PUBLISH expiry ordering
        if self.min_publish_expiry_secs > self.max_publish_expiry_secs {
            warnings.push(format!(
                "min_publish_expiry ({}) > max_publish_expiry ({}), swapping",
                self.min_publish_expiry_secs, self.max_publish_expiry_secs
            ));
            std::mem::swap(
                &mut self.min_publish_expiry_secs,
                &mut self.max_publish_expiry_secs,
            );
        }

        // Rate limits must be > 0
        for (name, val) in [
            ("subscribe_rate_limit_per_ip", &mut self.subscribe_rate_limit_per_ip),
            ("subscribe_rate_limit_per_aor", &mut self.subscribe_rate_limit_per_aor),
            ("publish_rate_limit_per_ip", &mut self.publish_rate_limit_per_ip),
            ("publish_rate_limit_per_aor", &mut self.publish_rate_limit_per_aor),
        ] {
            if *val == 0 {
                warnings.push(format!("{name} is 0, setting to 1"));
                *val = 1;
            }
        }

        // Retry backoff ordering
        if self.notify_retry_base_delay_ms > self.notify_retry_max_delay_ms {
            warnings.push(format!(
                "notify_retry_base_delay_ms ({}) > notify_retry_max_delay_ms ({}), swapping",
                self.notify_retry_base_delay_ms, self.notify_retry_max_delay_ms
            ));
            std::mem::swap(
                &mut self.notify_retry_base_delay_ms,
                &mut self.notify_retry_max_delay_ms,
            );
        }

        // Max presentities must be > 0
        if self.max_presentities == 0 {
            warnings.push("max_presentities is 0, setting to 1".to_string());
            self.max_presentities = 1;
        }

        warnings
    }
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_subscription_expiry_secs: default_subscription_expiry(),
            max_watchers_per_presentity: default_max_watchers(),
            max_presentities: default_max_presentities(),
            auto_phone_state: default_true(),
            max_publish_expiry_secs: default_max_publish_expiry(),
            cleanup_interval_secs: default_cleanup_interval(),
            notify_delay_ms: default_notify_delay(),
            min_subscription_expiry_secs: default_min_subscription_expiry(),
            max_subscription_expiry_secs: default_max_subscription_expiry(),
            min_publish_expiry_secs: default_min_publish_expiry(),
            winfo_enabled: false,
            composition_enabled: false,
            max_notify_retries: default_max_notify_retries(),
            notify_retry_base_delay_ms: default_notify_retry_base_delay_ms(),
            notify_retry_max_delay_ms: default_notify_retry_max_delay_ms(),
            subscribe_rate_limit_per_ip: default_subscribe_rate_limit_per_ip(),
            subscribe_rate_limit_per_aor: default_subscribe_rate_limit_per_aor(),
            publish_rate_limit_per_ip: default_publish_rate_limit_per_ip(),
            publish_rate_limit_per_aor: default_publish_rate_limit_per_aor(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_swaps_inverted_subscription_expiry() {
        let mut config = PresenceConfig {
            min_subscription_expiry_secs: 7200,
            max_subscription_expiry_secs: 3600,
            default_subscription_expiry_secs: 5000,
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(!warnings.is_empty());
        assert_eq!(config.min_subscription_expiry_secs, 3600);
        assert_eq!(config.max_subscription_expiry_secs, 7200);
    }

    #[test]
    fn test_validate_swaps_inverted_publish_expiry() {
        let mut config = PresenceConfig {
            min_publish_expiry_secs: 7200,
            max_publish_expiry_secs: 3600,
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(warnings.iter().any(|w| w.contains("min_publish_expiry")));
        assert_eq!(config.min_publish_expiry_secs, 3600);
        assert_eq!(config.max_publish_expiry_secs, 7200);
    }

    #[test]
    fn test_validate_clamps_default_expiry_to_range() {
        let mut config = PresenceConfig {
            min_subscription_expiry_secs: 100,
            max_subscription_expiry_secs: 200,
            default_subscription_expiry_secs: 3600,
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(warnings.iter().any(|w| w.contains("default_subscription_expiry")));
        assert_eq!(config.default_subscription_expiry_secs, 200);

        // Below min
        let mut config2 = PresenceConfig {
            min_subscription_expiry_secs: 500,
            max_subscription_expiry_secs: 1000,
            default_subscription_expiry_secs: 100,
            ..Default::default()
        };
        config2.validate();
        assert_eq!(config2.default_subscription_expiry_secs, 500);
    }

    #[test]
    fn test_validate_fixes_zero_rate_limits() {
        let mut config = PresenceConfig {
            subscribe_rate_limit_per_ip: 0,
            subscribe_rate_limit_per_aor: 0,
            publish_rate_limit_per_ip: 0,
            publish_rate_limit_per_aor: 0,
            ..Default::default()
        };
        let warnings = config.validate();
        assert_eq!(warnings.len(), 4);
        assert_eq!(config.subscribe_rate_limit_per_ip, 1);
        assert_eq!(config.subscribe_rate_limit_per_aor, 1);
        assert_eq!(config.publish_rate_limit_per_ip, 1);
        assert_eq!(config.publish_rate_limit_per_aor, 1);
    }

    #[test]
    fn test_validate_swaps_inverted_retry_delays() {
        let mut config = PresenceConfig {
            notify_retry_base_delay_ms: 30000,
            notify_retry_max_delay_ms: 1000,
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(warnings.iter().any(|w| w.contains("notify_retry_base_delay_ms")));
        assert_eq!(config.notify_retry_base_delay_ms, 1000);
        assert_eq!(config.notify_retry_max_delay_ms, 30000);
    }

    #[test]
    fn test_validate_fixes_zero_max_presentities() {
        let mut config = PresenceConfig {
            max_presentities: 0,
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(warnings.iter().any(|w| w.contains("max_presentities")));
        assert_eq!(config.max_presentities, 1);
    }

    #[test]
    fn test_validate_valid_config_no_warnings() {
        let mut config = PresenceConfig::default();
        let warnings = config.validate();
        assert!(warnings.is_empty(), "Default config should have no warnings: {:?}", warnings);
    }
}
