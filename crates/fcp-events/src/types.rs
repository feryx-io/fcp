//! Event type definitions for the FCP platform.

use fcp_core::{CallId, SessionId, TenantId};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::SystemTime;

/// Top-level platform event type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum PlatformEvent {
    /// Call lifecycle event
    Call(CallEvent),

    /// Media-related event
    Media(MediaEvent),

    /// IVR-related event
    Ivr(IvrEvent),

    /// Recording-related event
    Recording(RecordingEvent),

    /// System-level event
    System(SystemEvent),
}

/// Call lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CallEvent {
    /// Call initiated (INVITE received/sent)
    Initiated {
        call_id: CallId,
        tenant_id: TenantId,
        from: String,
        to: String,
        direction: CallDirection,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Call is ringing (180/183 received)
    Ringing {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Call progress (183 Session Progress)
    Progress {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Call answered (200 OK)
    Answered {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        answer_time: SystemTime,
        codec: String,
    },

    /// Call ended (BYE/timeout/error)
    Ended {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        end_time: SystemTime,
        duration_ms: u64,
        hangup_cause: String,
        hangup_party: Option<String>,
    },

    /// Call failed (4xx/5xx/6xx)
    Failed {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        reason: String,
        sip_code: Option<u16>,
    },
}

/// Media-related events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MediaEvent {
    /// RTP session started
    SessionStarted {
        session_id: SessionId,
        call_id: CallId,
        codec: String,
        remote_addr: SocketAddr,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// RTP session ended
    SessionEnded {
        session_id: SessionId,
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        stats: MediaStats,
    },

    /// DTMF digit received
    DtmfReceived {
        call_id: CallId,
        digit: char,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Media quality degraded
    QualityWarning {
        call_id: CallId,
        packet_loss: f32,
        jitter_ms: f32,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// IVR-related events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IvrEvent {
    /// Prompt playback started
    PromptStarted {
        call_id: CallId,
        prompt_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Prompt playback completed
    PromptCompleted {
        call_id: CallId,
        prompt_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Input gathered from caller
    InputGathered {
        call_id: CallId,
        input: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Gather timed out (no input received)
    GatherTimeout {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// Recording-related events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecordingEvent {
    /// Recording started
    Started {
        recording_id: String,
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Recording completed
    Completed {
        recording_id: String,
        call_id: CallId,
        duration_ms: u64,
        size_bytes: u64,
        url: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Recording failed
    Failed {
        recording_id: String,
        call_id: CallId,
        reason: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// System-level events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SystemEvent {
    /// System started
    Started {
        node_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// System stopping
    Stopping {
        node_id: String,
        reason: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Health check passed
    HealthCheck {
        node_id: String,
        healthy: bool,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// Call direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallDirection {
    Inbound,
    Outbound,
}

/// Media statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaStats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packet_loss: f32,
    pub jitter_ms: f32,
}

/// Serde module for SystemTime serialization.
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
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }
}

impl PlatformEvent {
    /// Get the call ID associated with this event, if any.
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            PlatformEvent::Call(event) => event.call_id(),
            PlatformEvent::Media(event) => event.call_id(),
            PlatformEvent::Ivr(event) => event.call_id(),
            PlatformEvent::Recording(event) => event.call_id(),
            PlatformEvent::System(_) => None,
        }
    }

    /// Get the event timestamp.
    pub fn timestamp(&self) -> SystemTime {
        match self {
            PlatformEvent::Call(event) => event.timestamp(),
            PlatformEvent::Media(event) => event.timestamp(),
            PlatformEvent::Ivr(event) => event.timestamp(),
            PlatformEvent::Recording(event) => event.timestamp(),
            PlatformEvent::System(event) => event.timestamp(),
        }
    }
}

impl CallEvent {
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            CallEvent::Initiated { call_id, .. }
            | CallEvent::Ringing { call_id, .. }
            | CallEvent::Progress { call_id, .. }
            | CallEvent::Answered { call_id, .. }
            | CallEvent::Ended { call_id, .. }
            | CallEvent::Failed { call_id, .. } => Some(call_id),
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            CallEvent::Initiated { timestamp, .. }
            | CallEvent::Ringing { timestamp, .. }
            | CallEvent::Progress { timestamp, .. }
            | CallEvent::Failed { timestamp, .. } => *timestamp,
            CallEvent::Answered { answer_time, .. } => *answer_time,
            CallEvent::Ended { end_time, .. } => *end_time,
        }
    }
}

impl MediaEvent {
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            MediaEvent::SessionStarted { call_id, .. }
            | MediaEvent::SessionEnded { call_id, .. }
            | MediaEvent::DtmfReceived { call_id, .. }
            | MediaEvent::QualityWarning { call_id, .. } => Some(call_id),
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            MediaEvent::SessionStarted { timestamp, .. }
            | MediaEvent::SessionEnded { timestamp, .. }
            | MediaEvent::DtmfReceived { timestamp, .. }
            | MediaEvent::QualityWarning { timestamp, .. } => *timestamp,
        }
    }
}

impl IvrEvent {
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            IvrEvent::PromptStarted { call_id, .. }
            | IvrEvent::PromptCompleted { call_id, .. }
            | IvrEvent::InputGathered { call_id, .. }
            | IvrEvent::GatherTimeout { call_id, .. } => Some(call_id),
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            IvrEvent::PromptStarted { timestamp, .. }
            | IvrEvent::PromptCompleted { timestamp, .. }
            | IvrEvent::InputGathered { timestamp, .. }
            | IvrEvent::GatherTimeout { timestamp, .. } => *timestamp,
        }
    }
}

impl RecordingEvent {
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            RecordingEvent::Started { call_id, .. }
            | RecordingEvent::Completed { call_id, .. }
            | RecordingEvent::Failed { call_id, .. } => Some(call_id),
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            RecordingEvent::Started { timestamp, .. }
            | RecordingEvent::Completed { timestamp, .. }
            | RecordingEvent::Failed { timestamp, .. } => *timestamp,
        }
    }
}

impl SystemEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            SystemEvent::Started { timestamp, .. }
            | SystemEvent::Stopping { timestamp, .. }
            | SystemEvent::HealthCheck { timestamp, .. } => *timestamp,
        }
    }
}
