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

    /// Flow execution event
    Flow(FlowEvent),

    /// System-level event
    System(SystemEvent),

    /// User lifecycle event
    User(UserEvent),

    /// Registration event
    Registration(RegistrationEvent),

    /// Voicemail event
    Voicemail(VoicemailEvent),
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_name: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        a_leg_sdp: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        b_leg_sdp: Option<String>,
    },

    /// Call ended (BYE/timeout/error)
    Ended {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        end_time: SystemTime,
        duration_ms: u64,
        hangup_cause: String,
        hangup_party: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_name: Option<String>,
    },

    /// Call failed (4xx/5xx/6xx)
    Failed {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        reason: String,
        sip_code: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_name: Option<String>,
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

/// Flow execution events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FlowEvent {
    /// Flow execution started
    Started {
        call_id: CallId,
        flow_id: String,
        flow_version: u32,
        initial_node: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Flow node executed
    NodeExecuted {
        call_id: CallId,
        flow_id: String,
        node_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Flow waiting for user input (DTMF)
    WaitingForInput {
        call_id: CallId,
        flow_id: String,
        variable: String,
        timeout_secs: u32,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Flow execution completed
    Completed {
        call_id: CallId,
        flow_id: String,
        outcome: String, // "hangup", "stopped", "error"
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Flow execution failed with error
    Failed {
        call_id: CallId,
        flow_id: String,
        node_id: String,
        error: String,
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

/// User lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserEvent {
    /// User created
    Created {
        user_id: String,
        username: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// User updated
    Updated {
        user_id: String,
        username: String,
        tenant_id: TenantId,
        changed_fields: Vec<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// User deleted
    Deleted {
        user_id: String,
        username: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// User enabled
    Enabled {
        user_id: String,
        username: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// User disabled
    Disabled {
        user_id: String,
        username: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// Registration events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RegistrationEvent {
    /// User registered (SIP REGISTER success)
    Registered {
        aor: String,
        contact: String,
        tenant_id: TenantId,
        expires_secs: u32,
        user_agent: Option<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// User unregistered (explicit or expired)
    Unregistered {
        aor: String,
        contact: String,
        tenant_id: TenantId,
        reason: String, // "explicit", "expired", "replaced"
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Registration failed (auth failure, etc.)
    Failed {
        aor: String,
        tenant_id: TenantId,
        reason: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// Voicemail-related events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VoicemailEvent {
    /// Voicemail message recorded
    MessageRecorded {
        call_id: CallId,
        mailbox_id: String,
        message_id: String,
        caller: String,
        duration_secs: u32,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Voicemail message deleted
    MessageDeleted {
        mailbox_id: String,
        message_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Voicemail message played back
    MessagePlayed {
        call_id: CallId,
        mailbox_id: String,
        message_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// MWI (Message Waiting Indicator) updated
    MwiUpdated {
        mailbox_id: String,
        new_messages: u32,
        old_messages: u32,
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
            PlatformEvent::Flow(event) => event.call_id(),
            PlatformEvent::System(_) => None,
            PlatformEvent::User(_) => None,
            PlatformEvent::Registration(_) => None,
            PlatformEvent::Voicemail(event) => event.call_id(),
        }
    }

    /// Get the event timestamp.
    pub fn timestamp(&self) -> SystemTime {
        match self {
            PlatformEvent::Call(event) => event.timestamp(),
            PlatformEvent::Media(event) => event.timestamp(),
            PlatformEvent::Ivr(event) => event.timestamp(),
            PlatformEvent::Recording(event) => event.timestamp(),
            PlatformEvent::Flow(event) => event.timestamp(),
            PlatformEvent::System(event) => event.timestamp(),
            PlatformEvent::User(event) => event.timestamp(),
            PlatformEvent::Registration(event) => event.timestamp(),
            PlatformEvent::Voicemail(event) => event.timestamp(),
        }
    }
}

impl UserEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            UserEvent::Created { timestamp, .. }
            | UserEvent::Updated { timestamp, .. }
            | UserEvent::Deleted { timestamp, .. }
            | UserEvent::Enabled { timestamp, .. }
            | UserEvent::Disabled { timestamp, .. } => *timestamp,
        }
    }
}

impl RegistrationEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            RegistrationEvent::Registered { timestamp, .. }
            | RegistrationEvent::Unregistered { timestamp, .. }
            | RegistrationEvent::Failed { timestamp, .. } => *timestamp,
        }
    }
}

impl FlowEvent {
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            FlowEvent::Started { call_id, .. }
            | FlowEvent::NodeExecuted { call_id, .. }
            | FlowEvent::WaitingForInput { call_id, .. }
            | FlowEvent::Completed { call_id, .. }
            | FlowEvent::Failed { call_id, .. } => Some(call_id),
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            FlowEvent::Started { timestamp, .. }
            | FlowEvent::NodeExecuted { timestamp, .. }
            | FlowEvent::WaitingForInput { timestamp, .. }
            | FlowEvent::Completed { timestamp, .. }
            | FlowEvent::Failed { timestamp, .. } => *timestamp,
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

impl VoicemailEvent {
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            VoicemailEvent::MessageRecorded { call_id, .. }
            | VoicemailEvent::MessagePlayed { call_id, .. } => Some(call_id),
            VoicemailEvent::MessageDeleted { .. } | VoicemailEvent::MwiUpdated { .. } => None,
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            VoicemailEvent::MessageRecorded { timestamp, .. }
            | VoicemailEvent::MessageDeleted { timestamp, .. }
            | VoicemailEvent::MessagePlayed { timestamp, .. }
            | VoicemailEvent::MwiUpdated { timestamp, .. } => *timestamp,
        }
    }
}
