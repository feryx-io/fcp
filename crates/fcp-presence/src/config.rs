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
