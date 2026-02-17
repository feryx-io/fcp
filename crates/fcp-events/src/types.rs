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

    /// Conference event
    Conference(ConferenceEvent),

    /// Queue/ACD event
    Queue(QueueEvent),
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// Call is ringing (180/183 received)
    Ringing {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// Call progress (183 Session Progress)
    Progress {
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// Call parked in a parking orbit
    Parked {
        call_id: CallId,
        orbit: u16,
        parker_extension: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Parked call retrieved (unparked)
    Unparked {
        call_id: CallId,
        orbit: u16,
        retriever_extension: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
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

    /// Registration refreshed (re-REGISTER with same contact)
    Refreshed {
        aor: String,
        contact: String,
        tenant_id: TenantId,
        expires_secs: u32,
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

    /// Registration expired (TTL expired without refresh)
    Expired {
        aor: String,
        contact: String,
        tenant_id: TenantId,
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

/// Conference-related events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConferenceEvent {
    /// Participant joined a conference room
    ParticipantJoined {
        room_number: String,
        participant_id: String,
        is_host: bool,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Participant left a conference room
    ParticipantLeft {
        room_number: String,
        participant_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Conference room was locked
    RoomLocked {
        room_number: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Conference room was unlocked
    RoomUnlocked {
        room_number: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Conference recording started
    RecordingStarted {
        room_number: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Conference recording stopped
    RecordingStopped {
        room_number: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Participant failed PIN authentication
    PinFailed {
        room_number: String,
        participant_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Empty conference room was destroyed
    RoomDestroyed {
        room_number: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Meeting configuration created via API
    MeetingCreated {
        meeting_id: String,
        room_number: String,
        name: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Meeting configuration updated via API
    MeetingUpdated {
        meeting_id: String,
        room_number: String,
        name: String,
        tenant_id: TenantId,
        changed_fields: Vec<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Meeting configuration deleted via API
    MeetingDeleted {
        meeting_id: String,
        room_number: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Conference session started (first participant joined a room)
    MeetingStarted {
        room_number: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Conference session ended (last participant left, room still exists)
    MeetingEnded {
        room_number: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// Queue/ACD-related events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueueEvent {
    /// Caller entered a queue
    CallerEnqueued {
        queue_name: String,
        call_id: String,
        caller: String,
        position: usize,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Caller was connected to an agent (dequeued)
    CallerDequeued {
        queue_name: String,
        call_id: String,
        agent_id: String,
        wait_time_secs: u64,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Caller hung up while waiting in the queue
    CallerAbandoned {
        queue_name: String,
        call_id: String,
        wait_time_secs: u64,
        position: usize,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Caller was sent to overflow destination (queue full or max wait exceeded)
    CallerOverflowed {
        queue_name: String,
        call_id: String,
        destination: String,
        reason: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Agent logged into a queue
    AgentLoggedIn {
        queue_name: String,
        agent_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Agent logged out of a queue
    AgentLoggedOut {
        queue_name: String,
        agent_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Agent state changed (e.g., available → on_call)
    AgentStateChanged {
        agent_id: String,
        old_state: String,
        new_state: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Caller requested a callback (virtual queueing)
    CallbackInitiated {
        queue_name: String,
        call_id: String,
        callback_uri: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Periodic stats snapshot for wallboard consumers
    StatsSnapshot {
        stats: Vec<QueueStatsSnapshot>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// Queue statistics snapshot for event publishing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStatsSnapshot {
    pub queue_name: String,
    pub calls_waiting: usize,
    pub agents_available: usize,
    pub agents_on_call: usize,
    pub agents_total: usize,
    pub avg_wait_time_secs: f64,
    pub longest_wait_secs: u64,
    pub calls_handled_today: u64,
    pub calls_abandoned_today: u64,
    pub service_level_pct: f64,
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
            PlatformEvent::Conference(_) => None,
            PlatformEvent::Queue(_) => None,
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
            PlatformEvent::Conference(event) => event.timestamp(),
            PlatformEvent::Queue(event) => event.timestamp(),
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
            | RegistrationEvent::Refreshed { timestamp, .. }
            | RegistrationEvent::Unregistered { timestamp, .. }
            | RegistrationEvent::Expired { timestamp, .. }
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
            | CallEvent::Failed { call_id, .. }
            | CallEvent::Parked { call_id, .. }
            | CallEvent::Unparked { call_id, .. } => Some(call_id),
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            CallEvent::Initiated { timestamp, .. }
            | CallEvent::Ringing { timestamp, .. }
            | CallEvent::Progress { timestamp, .. }
            | CallEvent::Failed { timestamp, .. }
            | CallEvent::Parked { timestamp, .. }
            | CallEvent::Unparked { timestamp, .. } => *timestamp,
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

impl ConferenceEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            ConferenceEvent::ParticipantJoined { timestamp, .. }
            | ConferenceEvent::ParticipantLeft { timestamp, .. }
            | ConferenceEvent::RoomLocked { timestamp, .. }
            | ConferenceEvent::RoomUnlocked { timestamp, .. }
            | ConferenceEvent::RecordingStarted { timestamp, .. }
            | ConferenceEvent::RecordingStopped { timestamp, .. }
            | ConferenceEvent::PinFailed { timestamp, .. }
            | ConferenceEvent::RoomDestroyed { timestamp, .. }
            | ConferenceEvent::MeetingCreated { timestamp, .. }
            | ConferenceEvent::MeetingUpdated { timestamp, .. }
            | ConferenceEvent::MeetingDeleted { timestamp, .. }
            | ConferenceEvent::MeetingStarted { timestamp, .. }
            | ConferenceEvent::MeetingEnded { timestamp, .. } => *timestamp,
        }
    }
}

impl QueueEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            QueueEvent::CallerEnqueued { timestamp, .. }
            | QueueEvent::CallerDequeued { timestamp, .. }
            | QueueEvent::CallerAbandoned { timestamp, .. }
            | QueueEvent::CallerOverflowed { timestamp, .. }
            | QueueEvent::CallbackInitiated { timestamp, .. }
            | QueueEvent::AgentLoggedIn { timestamp, .. }
            | QueueEvent::AgentLoggedOut { timestamp, .. }
            | QueueEvent::AgentStateChanged { timestamp, .. }
            | QueueEvent::StatsSnapshot { timestamp, .. } => *timestamp,
        }
    }
}
