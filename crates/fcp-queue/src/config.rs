//! Queue system configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level queue system configuration (in call-manager's TOML).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QueueSystemConfig {
    /// Master enable/disable for the queue system.
    #[serde(default)]
    pub enabled: bool,

    /// Queue definitions keyed by queue name.
    #[serde(default)]
    pub queues: HashMap<String, QueueConfig>,

    /// Default wrap-up time after a call ends before agent becomes available.
    #[serde(default = "default_wrap_up_time")]
    pub wrap_up_time_secs: u32,

    /// Service level threshold: calls answered within this many seconds
    /// are considered "within SL" for the service_level_pct metric.
    #[serde(default = "default_service_level_threshold")]
    pub service_level_threshold_secs: u32,

    /// Default ring time per agent attempt before trying the next agent.
    #[serde(default = "default_max_ring_time")]
    pub max_ring_time_secs: u32,

    /// Directory containing queue audio prompt files (WAV, 8kHz mono 16-bit).
    /// If None, position announcements are disabled even if announce_position is true.
    #[serde(default)]
    pub queue_audio_dir: Option<String>,

    /// Interval for publishing stats snapshots to the event bus (seconds).
    /// Set to 0 to disable. Default: 5.
    #[serde(default = "default_stats_publish_interval")]
    pub stats_publish_interval_secs: u64,

    /// Default pause duration (seconds). 0 = no auto-resume (manual unpause only).
    #[serde(default)]
    pub auto_pause_resume_secs: u32,

    /// Max consecutive no-answers before auto-away. 0 = disabled.
    #[serde(default = "default_max_no_answer")]
    pub max_no_answer: u32,
}

impl Default for QueueSystemConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            queues: HashMap::new(),
            wrap_up_time_secs: default_wrap_up_time(),
            service_level_threshold_secs: default_service_level_threshold(),
            max_ring_time_secs: default_max_ring_time(),
            queue_audio_dir: None,
            stats_publish_interval_secs: default_stats_publish_interval(),
            auto_pause_resume_secs: 0,
            max_no_answer: default_max_no_answer(),
        }
    }
}

/// Configuration for a single queue.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QueueConfig {
    /// Display name (e.g., "Sales", "Support").
    pub name: String,

    /// Queue extension number (e.g., "6000"). INVITEs to this extension
    /// are routed to this queue.
    pub extension: String,

    /// Distribution strategy for selecting agents.
    #[serde(default)]
    pub strategy: DistributionStrategy,

    /// Maximum time a caller can wait in the queue (seconds).
    #[serde(default = "default_max_wait_time")]
    pub max_wait_time_secs: u32,

    /// Maximum number of callers that can wait in the queue simultaneously.
    #[serde(default = "default_max_queue_depth")]
    pub max_queue_depth: usize,

    /// Where to send calls when the queue overflows or max wait time is reached.
    /// e.g., "voicemail:6000" or another extension.
    #[serde(default)]
    pub overflow_destination: Option<String>,

    /// Music-on-hold class name. Uses the default MOH if None.
    #[serde(default)]
    pub moh_class: Option<String>,

    /// Whether to periodically announce the caller's position in the queue.
    #[serde(default = "default_true")]
    pub announce_position: bool,

    /// Interval between position announcements (seconds).
    #[serde(default = "default_announce_interval")]
    pub announce_interval_secs: u32,

    /// Whether to return the caller to the queue when an agent doesn't answer
    /// (instead of abandoning the attempt).
    #[serde(default = "default_true")]
    pub retry_on_no_answer: bool,

    /// Static agent extensions that belong to this queue.
    #[serde(default)]
    pub agents: Vec<String>,

    /// Queue priority when an agent is a member of multiple queues.
    /// Lower values = higher priority.
    #[serde(default)]
    pub priority: u32,

    /// Skills required for this queue. Only agents with all listed skills are eligible.
    /// Used with `SkillsBased` strategy (and optionally as a filter for other strategies).
    #[serde(default)]
    pub required_skills: Vec<String>,

    /// Skills assigned to static agents (maps agent extension → skills list).
    #[serde(default)]
    pub agent_skills: HashMap<String, Vec<String>>,

    /// DTMF exit options: digit → destination.
    /// e.g., {"1": "voicemail:6000", "0": "extension:100"}
    #[serde(default)]
    pub dtmf_exit_options: HashMap<String, String>,

    /// Enable callback / virtual queueing.
    #[serde(default)]
    pub callback_enabled: bool,

    /// DTMF digit to request callback (e.g., "2").
    #[serde(default)]
    pub callback_digit: Option<String>,

    /// Per-agent priority overrides within this queue.
    /// Maps agent_id → priority. Agents not listed use the global agent priority.
    #[serde(default)]
    pub agent_priorities: HashMap<String, u32>,
}

/// Distribution strategy for selecting which agent receives the next call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionStrategy {
    /// Rotate through agents in order, wrapping around.
    #[default]
    RoundRobin,

    /// Select the agent who has been idle (available) the longest.
    LongestIdle,

    /// Select the agent who has handled the fewest calls.
    FewestCalls,

    /// Select a random available agent.
    Random,

    /// Always start from the first agent in the list, picking the first available.
    Linear,

    /// Select an agent who has all required skills, using longest-idle as tiebreaker.
    SkillsBased,
}

fn default_wrap_up_time() -> u32 {
    10
}

fn default_service_level_threshold() -> u32 {
    20
}

fn default_max_ring_time() -> u32 {
    20
}

fn default_max_wait_time() -> u32 {
    300
}

fn default_max_queue_depth() -> usize {
    50
}

fn default_true() -> bool {
    true
}

fn default_announce_interval() -> u32 {
    30
}

fn default_stats_publish_interval() -> u64 {
    5
}

fn default_max_no_answer() -> u32 {
    3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = QueueSystemConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.wrap_up_time_secs, 10);
        assert_eq!(config.service_level_threshold_secs, 20);
        assert_eq!(config.max_ring_time_secs, 20);
        assert!(config.queues.is_empty());
    }

    #[test]
    fn test_distribution_strategy_default() {
        let strategy = DistributionStrategy::default();
        assert_eq!(strategy, DistributionStrategy::RoundRobin);
    }

    #[test]
    fn test_config_deserialize() {
        let toml_str = r#"
enabled = true
wrap_up_time_secs = 15
service_level_threshold_secs = 30
max_ring_time_secs = 25

[queues.sales]
name = "Sales"
extension = "6000"
strategy = "round_robin"
max_wait_time_secs = 120
max_queue_depth = 20
overflow_destination = "voicemail:6000"
announce_position = true
announce_interval_secs = 45
retry_on_no_answer = true
agents = ["100", "101", "102"]
priority = 1

[queues.support]
name = "Support"
extension = "6001"
strategy = "longest_idle"
agents = ["200", "201"]
"#;
        let config: QueueSystemConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.wrap_up_time_secs, 15);
        assert_eq!(config.queues.len(), 2);

        let sales = &config.queues["sales"];
        assert_eq!(sales.name, "Sales");
        assert_eq!(sales.extension, "6000");
        assert_eq!(sales.strategy, DistributionStrategy::RoundRobin);
        assert_eq!(sales.max_wait_time_secs, 120);
        assert_eq!(sales.agents.len(), 3);

        let support = &config.queues["support"];
        assert_eq!(support.strategy, DistributionStrategy::LongestIdle);
        // Defaults should apply
        assert_eq!(support.max_wait_time_secs, 300);
        assert_eq!(support.max_queue_depth, 50);
    }

    #[test]
    fn test_strategy_serde_roundtrip() {
        let strategies = vec![
            DistributionStrategy::RoundRobin,
            DistributionStrategy::LongestIdle,
            DistributionStrategy::FewestCalls,
            DistributionStrategy::Random,
            DistributionStrategy::Linear,
            DistributionStrategy::SkillsBased,
        ];
        for strategy in strategies {
            let json = serde_json::to_string(&strategy).unwrap();
            let deserialized: DistributionStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(strategy, deserialized);
        }
    }
}
