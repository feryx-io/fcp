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

    /// Default max concurrent calls per agent. Default: 1.
    #[serde(default = "default_one")]
    pub default_max_concurrent_calls: u32,

    /// Directory for storing call recordings.
    #[serde(default)]
    pub recording_dir: Option<String>,

    /// Optional webhook URL for queue events. All QueueEvent types are POSTed
    /// as JSON to this URL. Uses the same delivery infrastructure as the
    /// general webhook dispatcher.
    #[serde(default)]
    pub webhook_url: Option<String>,
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
            default_max_concurrent_calls: default_one(),
            recording_dir: None,
            webhook_url: None,
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
    /// If `overflow_destinations` is also set, this is prepended to that list.
    #[serde(default)]
    pub overflow_destination: Option<String>,

    /// Ordered list of overflow destinations to try in sequence.
    /// Each entry is tried in order; if it fails, the next is attempted.
    /// e.g., ["queue:support", "voicemail:6000", "sip:operator@example.com"]
    #[serde(default)]
    pub overflow_destinations: Vec<String>,

    /// Music-on-hold class name. Uses the default MOH if None.
    #[serde(default)]
    pub moh_class: Option<String>,

    /// Whether to periodically announce the caller's position in the queue.
    #[serde(default = "default_true")]
    pub announce_position: bool,

    /// Interval between position announcements (seconds).
    #[serde(default = "default_announce_interval")]
    pub announce_interval_secs: u32,

    /// Whether to announce estimated wait time along with position.
    /// Requires `announce_position = true`. Uses average handle time
    /// and available agent count to estimate wait.
    #[serde(default)]
    pub announce_eta: bool,

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

    /// Per-agent weight for the `Weighted` distribution strategy.
    /// Maps agent_id → weight (e.g., senior=40, junior=20). Default weight is 1.
    /// Higher weight = proportionally more calls.
    #[serde(default)]
    pub agent_weights: HashMap<String, u32>,

    /// Seconds between automatic priority escalations. 0 = disabled.
    /// A call's priority decreases (higher urgency) by 1 per interval elapsed.
    #[serde(default)]
    pub priority_escalation_secs: u32,

    /// Business hours schedule. Empty = always open.
    #[serde(default)]
    pub schedule: Vec<ScheduleEntry>,

    /// Where to route calls when the queue is closed (outside schedule).
    /// If None, falls back to overflow_destination. If both None, reject with 480.
    #[serde(default)]
    pub closed_destination: Option<String>,

    /// IANA timezone name for schedule evaluation (e.g., "America/New_York").
    /// Defaults to "UTC".
    #[serde(default = "default_timezone")]
    pub timezone: String,

    /// Entry prompt filename (relative to queue_audio_dir). Played to caller
    /// when they first enter the queue, before MOH begins.
    #[serde(default)]
    pub entry_prompt: Option<String>,

    /// Connect prompt filename (relative to queue_audio_dir). Played to caller
    /// after an agent answers, before the two-way audio bridge starts.
    #[serde(default)]
    pub connect_prompt: Option<String>,

    /// Enable automatic call recording for this queue.
    #[serde(default)]
    pub record_calls: bool,

    /// Recording format: "wav" or "opus". Default: "wav".
    #[serde(default = "default_recording_format")]
    pub recording_format: String,

    /// Maximum callback attempts before abandoning. 0 = unlimited. Default: 3.
    #[serde(default = "default_callback_max_attempts")]
    pub callback_max_attempts: u32,

    /// Seconds to wait between callback attempts. Default: 60.
    #[serde(default = "default_callback_retry_delay")]
    pub callback_retry_delay_secs: u32,

    /// SLA warning threshold percentage. When service level drops below this
    /// value, a `ServiceLevelWarning` event is emitted. 0 = disabled. Default: 0.
    #[serde(default)]
    pub sla_warning_threshold_pct: f64,
}

impl QueueConfig {
    /// Returns the merged overflow destination list.
    /// If `overflow_destination` (legacy single field) is set, it's prepended
    /// to the `overflow_destinations` list. De-duplicated.
    pub fn effective_overflow_destinations(&self) -> Vec<String> {
        let mut dests = Vec::new();
        if let Some(ref d) = self.overflow_destination {
            if !d.is_empty() {
                dests.push(d.clone());
            }
        }
        for d in &self.overflow_destinations {
            if !dests.contains(d) {
                dests.push(d.clone());
            }
        }
        dests
    }
}

/// A time-of-day schedule entry for queue business hours.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScheduleEntry {
    /// Days of week: 0=Monday .. 6=Sunday (ISO 8601 weekday numbering).
    pub days: Vec<u8>,
    /// Start time in "HH:MM" format (24-hour).
    pub start_time: String,
    /// End time in "HH:MM" format (24-hour).
    pub end_time: String,
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

    /// Select agents proportional to configured weight. Higher weight = more calls.
    /// Uses a running counter approach for smooth distribution.
    Weighted,
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

fn default_one() -> u32 {
    1
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_recording_format() -> String {
    "wav".to_string()
}

fn default_callback_max_attempts() -> u32 {
    3
}

fn default_callback_retry_delay() -> u32 {
    60
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
    fn test_effective_overflow_destinations() {
        let toml_str = r#"
enabled = true

[queues.sales]
name = "Sales"
extension = "6000"
overflow_destination = "queue:support"
overflow_destinations = ["voicemail:6000", "sip:operator@example.com"]
agents = ["100"]

[queues.support]
name = "Support"
extension = "6001"
overflow_destination = "voicemail:6001"
agents = ["200"]

[queues.minimal]
name = "Minimal"
extension = "6002"
agents = ["300"]
"#;
        let config: QueueSystemConfig = toml::from_str(toml_str).unwrap();

        // Sales: legacy + list merged, legacy first
        let sales = &config.queues["sales"];
        let dests = sales.effective_overflow_destinations();
        assert_eq!(dests, vec!["queue:support", "voicemail:6000", "sip:operator@example.com"]);

        // Support: legacy only
        let support = &config.queues["support"];
        let dests = support.effective_overflow_destinations();
        assert_eq!(dests, vec!["voicemail:6001"]);

        // Minimal: no overflow
        let minimal = &config.queues["minimal"];
        let dests = minimal.effective_overflow_destinations();
        assert!(dests.is_empty());
    }

    #[test]
    fn test_effective_overflow_deduplication() {
        let toml_str = r#"
enabled = true

[queues.test]
name = "Test"
extension = "6000"
overflow_destination = "voicemail:6000"
overflow_destinations = ["voicemail:6000", "sip:op@example.com"]
agents = ["100"]
"#;
        let config: QueueSystemConfig = toml::from_str(toml_str).unwrap();
        let test_q = &config.queues["test"];
        let dests = test_q.effective_overflow_destinations();
        // "voicemail:6000" should appear only once
        assert_eq!(dests, vec!["voicemail:6000", "sip:op@example.com"]);
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
            DistributionStrategy::Weighted,
        ];
        for strategy in strategies {
            let json = serde_json::to_string(&strategy).unwrap();
            let deserialized: DistributionStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(strategy, deserialized);
        }
    }
}
