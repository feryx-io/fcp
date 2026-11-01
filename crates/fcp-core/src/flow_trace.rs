//! Flow execution trace types and storage trait.
//!
//! Provides persistent tracing of IVR flow execution — every node a call
//! traverses is recorded and queryable via the admin API.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// The type of flow execution event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowTraceEventType {
    /// Flow execution started
    Started,
    /// A flow node was executed
    NodeExecuted,
    /// Flow is waiting for DTMF input
    WaitingForInput,
    /// Flow execution completed normally
    Completed,
    /// Flow execution failed with an error
    Failed,
}

impl std::fmt::Display for FlowTraceEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Started => write!(f, "started"),
            Self::NodeExecuted => write!(f, "node_executed"),
            Self::WaitingForInput => write!(f, "waiting_for_input"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// A single flow execution trace entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowTraceEntry {
    /// Database auto-increment ID
    pub id: i64,
    /// FCP internal Call ID (e.g., "call-uuid")
    pub call_id: String,
    /// The flow's tenant, when the flow has one.
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Flow definition ID (e.g., "flow-uuid")
    pub flow_id: String,
    /// Flow name (denormalized for easy display)
    pub flow_name: Option<String>,
    /// Node ID within the flow (e.g., "greeting", "answer_6")
    pub node_id: Option<String>,
    /// Event type
    pub event_type: FlowTraceEventType,
    /// When this event occurred
    #[serde(with = "system_time_serde")]
    pub timestamp: SystemTime,
    /// Additional detail (e.g., outcome for Completed, error for Failed, variable for WaitingForInput)
    pub detail: Option<String>,
    /// Flow version
    pub flow_version: Option<u32>,
    /// Elapsed milliseconds since flow started (computed at insert time)
    pub elapsed_ms: Option<i64>,
}

/// Query parameters for listing flow traces.
#[derive(Debug, Clone, Default)]
pub struct FlowTraceQuery {
    /// Only this tenant's traces (as stored, `tenant-<uuid>`); every
    /// tenant's when `None`.
    pub tenant_id: Option<String>,
    /// Filter by call ID
    pub call_id: Option<String>,
    /// Filter by flow ID
    pub flow_id: Option<String>,
    /// Filter by event type
    pub event_type: Option<FlowTraceEventType>,
    /// Filter by time range start
    pub time_from: Option<SystemTime>,
    /// Filter by time range end
    pub time_to: Option<SystemTime>,
    /// Maximum results
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

/// Storage trait for flow execution traces.
#[async_trait]
pub trait FlowTraceStore: Send + Sync {
    /// Insert a single trace entry.
    async fn insert_flow_trace(&self, entry: &FlowTraceEntry) -> anyhow::Result<()>;

    /// Get all trace entries for a call, ordered by timestamp.
    async fn get_flow_traces_by_call(&self, call_id: &str) -> anyhow::Result<Vec<FlowTraceEntry>>;

    /// Query traces with filters and pagination. Returns (entries, total_count).
    async fn query_flow_traces(
        &self,
        query: &FlowTraceQuery,
    ) -> anyhow::Result<(Vec<FlowTraceEntry>, usize)>;

    /// Delete traces older than the specified number of days. Returns rows deleted.
    async fn delete_flow_traces_older_than(&self, days: u32) -> anyhow::Result<u64>;

    /// Delete a tenant's traces older than `before`.
    async fn delete_flow_traces_for_tenant_before(
        &self,
        _tenant_id: &str,
        _before: SystemTime,
    ) -> anyhow::Result<u64> {
        Ok(0)
    }

    /// Give traces written without a tenant the tenant of their call's
    /// record. Returns how many.
    async fn adopt_untenanted_traces(&self) -> anyhow::Result<u64> {
        Ok(0)
    }
}

/// Serde module for SystemTime serialization as epoch milliseconds.
mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time
            .duration_since(UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?;
        let millis = duration.as_millis() as u64;
        millis.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_millis(millis))
    }
}
