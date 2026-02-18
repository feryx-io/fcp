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

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_subscription_expiry_secs: default_subscription_expiry(),
            max_watchers_per_presentity: default_max_watchers(),
            max_presentities: default_max_presentities(),
            auto_phone_state: default_true(),
        }
    }
}
