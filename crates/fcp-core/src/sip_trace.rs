//! SIP message tracing types and storage trait.
//!
//! This module provides the data model and storage abstraction for capturing
//! raw SIP messages exchanged during calls. Messages are stored in PostgreSQL
//! and can be displayed as interactive ladder diagrams in the admin UI.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// A captured SIP message with raw wire format and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipTraceMessage {
    /// Database auto-increment ID
    pub id: i64,
    /// FCP internal Call ID (e.g., "call-uuid")
    pub call_id: String,
    /// SIP Call-ID header value
    pub sip_call_id: String,
    /// Tenant context
    pub tenant_id: String,
    /// When the message was captured
    #[serde(with = "system_time_serde")]
    pub timestamp: SystemTime,
    /// "inbound" or "outbound"
    pub direction: String,
    /// SIP method (INVITE, BYE, ACK) or status code (100, 180, 200)
    pub method_or_status: String,
    /// CSeq header value (e.g., "1 INVITE")
    pub cseq: String,
    /// From header URI
    pub from_uri: String,
    /// To header URI
    pub to_uri: String,
    /// Source IP:port
    pub source_addr: String,
    /// Destination IP:port
    pub dest_addr: String,
    /// Transport protocol: "udp", "tcp", "tls", "ws"
    pub transport: String,
    /// Full SIP message text (start line + headers; body may be stripped)
    pub raw_message: String,
    /// Which cluster node captured this message
    pub node_id: Option<String>,
}

/// Storage trait for SIP trace messages.
#[async_trait]
pub trait SipTraceStore: Send + Sync {
    /// Insert a single SIP trace message.
    async fn insert_sip_trace(&self, msg: &SipTraceMessage) -> anyhow::Result<()>;

    /// Insert multiple SIP trace messages in a single batch.
    async fn insert_sip_traces_batch(&self, msgs: &[SipTraceMessage]) -> anyhow::Result<()>;

    /// Get all SIP trace messages for an FCP call ID, ordered by timestamp.
    async fn get_sip_traces(&self, call_id: &str) -> anyhow::Result<Vec<SipTraceMessage>>;

    /// Get all SIP trace messages by SIP Call-ID header, ordered by timestamp.
    async fn get_sip_traces_by_sip_call_id(
        &self,
        sip_call_id: &str,
    ) -> anyhow::Result<Vec<SipTraceMessage>>;

    /// Delete SIP trace messages older than the specified number of days.
    /// Returns the number of rows deleted.
    async fn delete_sip_traces_older_than(&self, days: u32) -> anyhow::Result<u64>;

    /// Delete one tenant's traces older than `before` (retention policy).
    async fn delete_sip_traces_for_tenant_before(
        &self,
        _tenant_id: &str,
        _before: SystemTime,
    ) -> anyhow::Result<u64> {
        Ok(0)
    }

    /// Give traces captured before their call's tenant was known
    /// (`_system`) the tenant of the call's record. Returns how many.
    async fn adopt_system_traces(&self) -> anyhow::Result<u64> {
        Ok(0)
    }

    /// Delete `_system` traces older than `before`: messages that belong to
    /// no tenant's call (registrations, options, calls never recorded).
    async fn delete_system_traces_before(&self, _before: SystemTime) -> anyhow::Result<u64> {
        Ok(0)
    }
}

/// Serde module for SystemTime serialization as epoch seconds.
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
        // Use milliseconds for sub-second precision in SIP traces
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
