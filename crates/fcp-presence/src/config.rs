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
        }
    }
}
