//! Queue configuration record types for persistent queue CRUD management.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::SystemTime;
use uuid::Uuid;

/// Unique identifier for a queue configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueueConfigId(String);

impl QueueConfigId {
    /// Create a new random queue config ID.
    pub fn new() -> Self {
        Self(format!("qcfg-{}", Uuid::new_v4()))
    }

    /// Create a queue config ID from a string.
    pub fn from_string(s: String) -> Self {
        Self(s)
    }
}

impl Default for QueueConfigId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for QueueConfigId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for QueueConfigId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

/// A persistent queue configuration record stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfigRecord {
    /// Unique identifier
    pub id: QueueConfigId,

    /// Tenant this queue belongs to
    pub tenant_id: crate::identifiers::TenantId,

    /// Human-readable queue name (e.g., "Sales", "Support")
    pub name: String,

    /// Queue extension number (e.g., "6000")
    pub extension: String,

    /// Distribution strategy (e.g., "round_robin", "longest_idle")
    pub strategy: String,

    /// Maximum time a caller can wait in the queue (seconds)
    pub max_wait_time_secs: i32,

    /// Maximum number of callers that can wait simultaneously
    pub max_queue_depth: i32,

    /// Where to send calls when the queue overflows
    pub overflow_destination: Option<String>,

    /// Music-on-hold class name
    pub moh_class: Option<String>,

    /// Whether to periodically announce the caller's position
    pub announce_position: bool,

    /// Interval between position announcements (seconds)
    pub announce_interval_secs: i32,

    /// Whether to return the caller to queue when agent doesn't answer
    pub retry_on_no_answer: bool,

    /// Static agent extensions that belong to this queue
    pub agents: Vec<String>,

    /// Queue priority (lower = higher priority)
    pub priority: i32,

    /// Skills required for this queue
    pub required_skills: Vec<String>,

    /// Enable automatic call recording
    pub record_calls: bool,

    /// Enable callback / virtual queueing
    pub callback_enabled: bool,

    /// IANA timezone name for schedule evaluation
    pub timezone: String,

    /// Whether the queue is enabled
    pub enabled: bool,

    /// The queue's hours (`docs/ENTERPRISE_CALLING.md` §3.5): outside them a
    /// caller goes to `closed_target`, or is told 480.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<crate::ScheduleId>,

    /// Where a caller goes when the queue's schedule is not open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_target: Option<crate::CallTarget>,

    /// Created timestamp
    pub created_at: SystemTime,

    /// Last modified timestamp
    pub updated_at: SystemTime,
}

impl QueueConfigRecord {
    /// Create a new queue config record with defaults.
    pub fn new(name: String, extension: String, tenant_id: crate::identifiers::TenantId) -> Self {
        let now = SystemTime::now();
        Self {
            id: QueueConfigId::new(),
            tenant_id,
            name,
            extension,
            strategy: "round_robin".to_string(),
            max_wait_time_secs: 300,
            max_queue_depth: 50,
            overflow_destination: None,
            moh_class: None,
            announce_position: true,
            announce_interval_secs: 30,
            retry_on_no_answer: true,
            agents: vec![],
            priority: 0,
            required_skills: vec![],
            record_calls: false,
            callback_enabled: false,
            timezone: "UTC".to_string(),
            enabled: true,
            schedule_id: None,
            closed_target: None,
            created_at: now,
            updated_at: now,
        }
    }
}
