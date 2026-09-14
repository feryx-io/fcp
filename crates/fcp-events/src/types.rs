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

    /// Presence status event
    Presence(PresenceEvent),

    /// SIP protocol trace event
    Sip(SipTraceEvent),

    /// Billing event (rating, balance changes, enforcement)
    Billing(BillingEvent),

    /// Call detail record event (`docs/CDR_PIPELINE.md` §10)
    Cdr(CdrEvent),
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

    /// An emergency call (911 or a test number) was placed. Carries what
    /// consumers need for on-site notification (Kari's Law) and the record.
    Emergency {
        call_id: CallId,
        tenant_id: TenantId,
        from: String,
        dialed: String,
        /// `live` or `test`.
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        callback_number: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location_source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location_summary: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_name: Option<String>,
        /// People/URIs the tenant asked to be told about emergency calls.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        notify_contacts: Vec<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
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

    /// Configuration entity changed (user, extension, device, etc.).
    /// Used for cache invalidation across cluster nodes.
    ConfigChanged {
        /// Type of entity changed (e.g. "user", "extension", "device")
        entity_type: String,
        /// Identifier of the changed entity
        entity_id: String,
        /// Tenant the entity belongs to
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A new node joined the cluster (discovered via heartbeat).
    NodeJoined {
        node_id: String,
        node_type: String,
        address: String,
        /// Deployment region the node advertises, when it has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A node left the cluster (heartbeat expired or graceful shutdown).
    NodeLeft {
        node_id: String,
        reason: String,
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

/// What a `video_overload` from a node that predates shedding did: the
/// frame rate was the only thing it could give up.
fn video_overload_fps() -> String {
    "fps".to_string()
}

/// The stream a `video_state_changed` from a node that predates screen
/// sharing was about: the camera, the only one there was.
fn video_stream_camera() -> String {
    "camera".to_string()
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
        /// Whether the participant joined with a negotiated video stream.
        #[serde(default)]
        has_video: bool,
        /// Set when the participant arrived by migration from a draining
        /// node (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5): that
        /// node's id. The same participant id left it with
        /// `migrated_to`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        migrated_from: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Participant left a conference room
    ParticipantLeft {
        room_number: String,
        participant_id: String,
        /// Set when the participant did not hang up but moved to another
        /// node as this one drained: that node's id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        migrated_to: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A participant's video changed state: `off` (no video negotiated),
    /// `on` (frames arriving), `lost` (negotiated but nothing decodable
    /// lately), `disabled` (a host turned it off) or `failed` (the decoder
    /// gave up on it). `stream` says which of the participant's streams:
    /// `camera`, or `content` for a shared screen (§15.6); an event from
    /// a node that predates screen sharing carries none and reads as
    /// `camera`.
    VideoStateChanged {
        room_number: String,
        participant_id: String,
        state: String,
        #[serde(default = "video_stream_camera")]
        stream: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A participant took the room's content floor: their shared screen
    /// is what the room shows (§15.6). One per room at a time; emitted by
    /// the node the presenter is on.
    ContentStarted {
        room_number: String,
        participant_id: String,
        node_id: String,
        /// The codec the screen arrives in (SDP name).
        codec: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A share ended. `reason` is `ended` (the presenter stopped), `idle`
    /// (no packets for the idle timeout), `host` (a host stopped it),
    /// `replaced` (another node's presenter took the room-wide floor),
    /// `left` (the presenter left) or `failed` (the decoder gave up).
    ContentStopped {
        room_number: String,
        participant_id: String,
        node_id: String,
        reason: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// The room's video overran its frame budget three ticks in a row and
    /// gave something up to keep up. `action` says what:
    ///
    /// - `shed`: one composite (`output`: `all`, `excluding:<id>` or
    ///   `local-only`) dropped a rung of the bitrate ladder, from
    ///   `from_resolution` to `to_resolution`, and everyone's frame rate
    ///   was left alone — `from_fps` and `to_fps` are both the room's rate;
    /// - `fps`: shedding had nothing left to give, every composite being at
    ///   the bottom rung, and the room's frame rate was halved.
    ///
    /// An event from a node that predates shedding carries no `action` and
    /// reads as `fps`, the only thing such a node could do.
    VideoOverload {
        room_number: String,
        #[serde(default = "video_overload_fps")]
        action: String,
        from_fps: u32,
        to_fps: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_resolution: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_resolution: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// The room's video layout changed: a host or the API set the layout
    /// (`grid`, `active_speaker`, `spotlight`, `pip`), pinned or
    /// spotlighted a participant, or a pinned or spotlighted participant
    /// left. In a room spread over several conference nodes each hosting
    /// node emits its own copy.
    VideoLayoutChanged {
        room_number: String,
        layout: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pinned: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spotlight: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// The room's active speaker changed. In a room spread over several
    /// nodes this is emitted once, by the node the speaker is on, so a
    /// listener sees one change rather than one per node.
    ActiveSpeakerChanged {
        room_number: String,
        /// The participant now holding the floor.
        participant_id: String,
        /// The node they are on.
        node_id: String,
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
        /// The registry's id for it, which the admin API lists and
        /// downloads by.
        recording_id: String,
        /// `wav` (mixed audio) or `webm` (composite video and audio).
        format: String,
        /// The node holding the file.
        node_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Conference recording stopped
    RecordingStopped {
        room_number: String,
        recording_id: String,
        format: String,
        node_id: String,
        /// How long it ran.
        duration_secs: u64,
        /// How big the file is.
        size_bytes: u64,
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

    /// Dial-out invitation initiated to an external participant
    DialOutInvited {
        room_number: String,
        participant_id: String,
        destination: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Dial-out invitation failed (callee rejected or timed out)
    DialOutFailed {
        room_number: String,
        destination: String,
        reason: String,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Agent forcefully kicked from the queue system (admin action)
    AgentKicked {
        agent_id: String,
        reason: String,
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

    /// Caller transferred from one queue to another (queue chaining)
    CallerTransferred {
        from_queue: String,
        to_queue: String,
        call_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Supervisor joined a call (monitor/whisper/barge)
    SupervisorJoined {
        call_id: String,
        supervisor_id: String,
        mode: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Supervisor left a call
    SupervisorLeft {
        call_id: String,
        supervisor_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A callback attempt was made (recipient did not answer)
    CallbackAttempted {
        queue_name: String,
        call_id: String,
        callback_uri: String,
        attempt: u32,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Callback completed successfully (recipient answered and bridged to agent)
    CallbackCompleted {
        queue_name: String,
        call_id: String,
        callback_uri: String,
        agent_id: String,
        attempts: u32,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Service level dropped below configured warning threshold
    ServiceLevelWarning {
        queue_name: String,
        current_pct: f64,
        threshold_pct: f64,
        calls_offered: u64,
        calls_within_sl: u64,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Callback failed permanently (max attempts exceeded or cancelled)
    CallbackFailed {
        queue_name: String,
        call_id: String,
        callback_uri: String,
        reason: String,
        attempts: u32,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Queue entered draining mode — no new calls accepted, existing calls continue
    QueueDraining {
        queue_name: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Queue fully drained — drain mode active, zero waiting calls, zero active sessions
    QueueDrained {
        queue_name: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Campaign outbound call placed to a destination
    CampaignCallPlaced {
        queue_name: String,
        destination: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Campaign call answered and connected to an agent
    CampaignCallConnected {
        queue_name: String,
        destination: String,
        agent_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Campaign call failed (no answer, busy, error, or abandoned)
    CampaignCallFailed {
        queue_name: String,
        destination: String,
        outcome: String,
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

/// Presence-related events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PresenceEvent {
    /// User presence status changed
    StatusChanged {
        aor: String,
        old_state: String,
        new_state: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A watcher subscribed to a presentity
    WatcherAdded {
        presentity_aor: String,
        watcher_aor: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A watcher unsubscribed from a presentity
    WatcherRemoved {
        presentity_aor: String,
        watcher_aor: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// PUBLISH received from a user agent
    PublishReceived {
        aor: String,
        state: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Dialog state changed (call initiated, answered, ended, etc.)
    DialogChanged {
        aor: String,
        call_id: String,
        dialog_state: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A PUBLISH is nearing expiry (configurable threshold, default 80%).
    PublishExpiryWarning {
        aor: String,
        etag: String,
        elapsed_pct: u8,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A PUBLISH has expired and been removed.
    PublishExpired {
        aor: String,
        etag: String,
        state: String,
        tenant_id: TenantId,
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

    /// Mailbox created (triggers voicemail server to register routes)
    MailboxCreated {
        mailbox_id: String,
        user_id: String,
        extension: String,
        tenant_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Mailbox updated (triggers voicemail server to refresh routes)
    MailboxUpdated {
        mailbox_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Mailbox deleted (triggers voicemail server to remove routes)
    MailboxDeleted {
        mailbox_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// SIP protocol trace events for ladder diagram visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SipTraceEvent {
    /// Raw SIP message captured from the wire
    MessageCaptured {
        call_id: String,
        sip_call_id: String,
        direction: String,
        method_or_status: String,
        from_uri: String,
        to_uri: String,
        source_addr: String,
        dest_addr: String,
        transport: String,
        cseq: String,
        raw_message: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        node_id: Option<String>,
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
    /// Get a dotted event type name suitable for webhook filter matching.
    ///
    /// Returns names like `"call.initiated"`, `"voicemail.message_recorded"`,
    /// `"presence.status_changed"`, etc.
    pub fn event_type_name(&self) -> &'static str {
        match self {
            PlatformEvent::Call(e) => match e {
                CallEvent::Initiated { .. } => "call.initiated",
                CallEvent::Ringing { .. } => "call.ringing",
                CallEvent::Progress { .. } => "call.progress",
                CallEvent::Answered { .. } => "call.answered",
                CallEvent::Ended { .. } => "call.ended",
                CallEvent::Failed { .. } => "call.failed",
                CallEvent::Emergency { .. } => "call.emergency",
                CallEvent::Parked { .. } => "call.parked",
                CallEvent::Unparked { .. } => "call.unparked",
            },
            PlatformEvent::Media(e) => match e {
                MediaEvent::SessionStarted { .. } => "media.session_started",
                MediaEvent::SessionEnded { .. } => "media.session_ended",
                MediaEvent::DtmfReceived { .. } => "media.dtmf_received",
                MediaEvent::QualityWarning { .. } => "media.quality_warning",
            },
            PlatformEvent::Ivr(e) => match e {
                IvrEvent::PromptStarted { .. } => "ivr.prompt_started",
                IvrEvent::PromptCompleted { .. } => "ivr.prompt_completed",
                IvrEvent::InputGathered { .. } => "ivr.input_gathered",
                IvrEvent::GatherTimeout { .. } => "ivr.gather_timeout",
            },
            PlatformEvent::Recording(e) => match e {
                RecordingEvent::Started { .. } => "recording.started",
                RecordingEvent::Completed { .. } => "recording.completed",
                RecordingEvent::Failed { .. } => "recording.failed",
            },
            PlatformEvent::Flow(e) => match e {
                FlowEvent::Started { .. } => "flow.started",
                FlowEvent::NodeExecuted { .. } => "flow.node_executed",
                FlowEvent::WaitingForInput { .. } => "flow.waiting_for_input",
                FlowEvent::Completed { .. } => "flow.completed",
                FlowEvent::Failed { .. } => "flow.failed",
            },
            PlatformEvent::System(e) => match e {
                SystemEvent::Started { .. } => "system.started",
                SystemEvent::Stopping { .. } => "system.stopping",
                SystemEvent::HealthCheck { .. } => "system.health_check",
                SystemEvent::ConfigChanged { .. } => "system.config_changed",
                SystemEvent::NodeJoined { .. } => "system.node_joined",
                SystemEvent::NodeLeft { .. } => "system.node_left",
            },
            PlatformEvent::User(e) => match e {
                UserEvent::Created { .. } => "user.created",
                UserEvent::Updated { .. } => "user.updated",
                UserEvent::Deleted { .. } => "user.deleted",
                UserEvent::Enabled { .. } => "user.enabled",
                UserEvent::Disabled { .. } => "user.disabled",
            },
            PlatformEvent::Registration(e) => match e {
                RegistrationEvent::Registered { .. } => "registration.registered",
                RegistrationEvent::Refreshed { .. } => "registration.refreshed",
                RegistrationEvent::Unregistered { .. } => "registration.unregistered",
                RegistrationEvent::Expired { .. } => "registration.expired",
                RegistrationEvent::Failed { .. } => "registration.failed",
            },
            PlatformEvent::Voicemail(e) => match e {
                VoicemailEvent::MessageRecorded { .. } => "voicemail.message_recorded",
                VoicemailEvent::MessageDeleted { .. } => "voicemail.message_deleted",
                VoicemailEvent::MessagePlayed { .. } => "voicemail.message_played",
                VoicemailEvent::MwiUpdated { .. } => "voicemail.mwi_updated",
                VoicemailEvent::MailboxCreated { .. } => "voicemail.mailbox_created",
                VoicemailEvent::MailboxUpdated { .. } => "voicemail.mailbox_updated",
                VoicemailEvent::MailboxDeleted { .. } => "voicemail.mailbox_deleted",
            },
            PlatformEvent::Conference(e) => match e {
                ConferenceEvent::ParticipantJoined { .. } => "conference.participant_joined",
                ConferenceEvent::ParticipantLeft { .. } => "conference.participant_left",
                ConferenceEvent::RoomLocked { .. } => "conference.room_locked",
                ConferenceEvent::RoomUnlocked { .. } => "conference.room_unlocked",
                ConferenceEvent::RecordingStarted { .. } => "conference.recording_started",
                ConferenceEvent::RecordingStopped { .. } => "conference.recording_stopped",
                ConferenceEvent::PinFailed { .. } => "conference.pin_failed",
                ConferenceEvent::RoomDestroyed { .. } => "conference.room_destroyed",
                ConferenceEvent::MeetingCreated { .. } => "conference.meeting_created",
                ConferenceEvent::MeetingUpdated { .. } => "conference.meeting_updated",
                ConferenceEvent::MeetingDeleted { .. } => "conference.meeting_deleted",
                ConferenceEvent::MeetingStarted { .. } => "conference.meeting_started",
                ConferenceEvent::MeetingEnded { .. } => "conference.meeting_ended",
                ConferenceEvent::DialOutInvited { .. } => "conference.dial_out_invited",
                ConferenceEvent::DialOutFailed { .. } => "conference.dial_out_failed",
                ConferenceEvent::VideoStateChanged { .. } => "conference.video_state_changed",
                ConferenceEvent::ContentStarted { .. } => "conference.content_started",
                ConferenceEvent::ContentStopped { .. } => "conference.content_stopped",
                ConferenceEvent::VideoOverload { .. } => "conference.video_overload",
                ConferenceEvent::VideoLayoutChanged { .. } => "conference.video_layout_changed",
                ConferenceEvent::ActiveSpeakerChanged { .. } => "conference.active_speaker_changed",
            },
            PlatformEvent::Queue(e) => match e {
                QueueEvent::CallerEnqueued { .. } => "queue.caller_enqueued",
                QueueEvent::CallerDequeued { .. } => "queue.caller_dequeued",
                QueueEvent::CallerAbandoned { .. } => "queue.caller_abandoned",
                QueueEvent::CallerOverflowed { .. } => "queue.caller_overflowed",
                QueueEvent::AgentLoggedIn { .. } => "queue.agent_logged_in",
                QueueEvent::AgentLoggedOut { .. } => "queue.agent_logged_out",
                QueueEvent::AgentStateChanged { .. } => "queue.agent_state_changed",
                QueueEvent::AgentKicked { .. } => "queue.agent_kicked",
                QueueEvent::CallbackInitiated { .. } => "queue.callback_initiated",
                QueueEvent::StatsSnapshot { .. } => "queue.stats_snapshot",
                QueueEvent::CallerTransferred { .. } => "queue.caller_transferred",
                QueueEvent::SupervisorJoined { .. } => "queue.supervisor_joined",
                QueueEvent::SupervisorLeft { .. } => "queue.supervisor_left",
                QueueEvent::CallbackAttempted { .. } => "queue.callback_attempted",
                QueueEvent::CallbackCompleted { .. } => "queue.callback_completed",
                QueueEvent::CallbackFailed { .. } => "queue.callback_failed",
                QueueEvent::ServiceLevelWarning { .. } => "queue.service_level_warning",
                QueueEvent::QueueDraining { .. } => "queue.queue_draining",
                QueueEvent::QueueDrained { .. } => "queue.queue_drained",
                QueueEvent::CampaignCallPlaced { .. } => "queue.campaign_call_placed",
                QueueEvent::CampaignCallConnected { .. } => "queue.campaign_call_connected",
                QueueEvent::CampaignCallFailed { .. } => "queue.campaign_call_failed",
            },
            PlatformEvent::Presence(e) => match e {
                PresenceEvent::StatusChanged { .. } => "presence.status_changed",
                PresenceEvent::WatcherAdded { .. } => "presence.watcher_added",
                PresenceEvent::WatcherRemoved { .. } => "presence.watcher_removed",
                PresenceEvent::PublishReceived { .. } => "presence.publish_received",
                PresenceEvent::DialogChanged { .. } => "presence.dialog_changed",
                PresenceEvent::PublishExpiryWarning { .. } => "presence.publish_expiry_warning",
                PresenceEvent::PublishExpired { .. } => "presence.publish_expired",
            },
            PlatformEvent::Sip(e) => match e {
                SipTraceEvent::MessageCaptured { .. } => "sip.message_captured",
            },
            PlatformEvent::Billing(e) => match e {
                BillingEvent::CallRated { .. } => "billing.call_rated",
                BillingEvent::BalanceChanged { .. } => "billing.balance_changed",
                BillingEvent::BalanceLow { .. } => "billing.balance_low",
                BillingEvent::BalanceDepleted { .. } => "billing.balance_depleted",
                BillingEvent::CallRejected { .. } => "billing.call_rejected",
                BillingEvent::CallCutOff { .. } => "billing.call_cut_off",
            },
            PlatformEvent::Cdr(e) => match e {
                CdrEvent::Written { .. } => "cdr.written",
            },
        }
    }

    /// Extract the tenant ID from this event, if the event variant carries one.
    pub fn tenant_id(&self) -> Option<&TenantId> {
        match self {
            PlatformEvent::Call(e) => match e {
                CallEvent::Initiated { tenant_id, .. } => Some(tenant_id),
                _ => None,
            },
            PlatformEvent::User(e) => match e {
                UserEvent::Created { tenant_id, .. }
                | UserEvent::Updated { tenant_id, .. }
                | UserEvent::Deleted { tenant_id, .. }
                | UserEvent::Enabled { tenant_id, .. }
                | UserEvent::Disabled { tenant_id, .. } => Some(tenant_id),
            },
            PlatformEvent::Registration(e) => match e {
                RegistrationEvent::Registered { tenant_id, .. }
                | RegistrationEvent::Refreshed { tenant_id, .. }
                | RegistrationEvent::Unregistered { tenant_id, .. }
                | RegistrationEvent::Expired { tenant_id, .. }
                | RegistrationEvent::Failed { tenant_id, .. } => Some(tenant_id),
            },
            PlatformEvent::Conference(e) => match e {
                ConferenceEvent::ParticipantJoined { tenant_id, .. }
                | ConferenceEvent::ParticipantLeft { tenant_id, .. }
                | ConferenceEvent::RoomLocked { tenant_id, .. }
                | ConferenceEvent::RoomUnlocked { tenant_id, .. }
                | ConferenceEvent::RecordingStarted { tenant_id, .. }
                | ConferenceEvent::RecordingStopped { tenant_id, .. }
                | ConferenceEvent::PinFailed { tenant_id, .. }
                | ConferenceEvent::RoomDestroyed { tenant_id, .. }
                | ConferenceEvent::MeetingCreated { tenant_id, .. }
                | ConferenceEvent::MeetingUpdated { tenant_id, .. }
                | ConferenceEvent::MeetingDeleted { tenant_id, .. }
                | ConferenceEvent::MeetingStarted { tenant_id, .. }
                | ConferenceEvent::MeetingEnded { tenant_id, .. }
                | ConferenceEvent::DialOutInvited { tenant_id, .. }
                | ConferenceEvent::DialOutFailed { tenant_id, .. }
                | ConferenceEvent::VideoStateChanged { tenant_id, .. }
                | ConferenceEvent::ContentStarted { tenant_id, .. }
                | ConferenceEvent::ContentStopped { tenant_id, .. }
                | ConferenceEvent::VideoOverload { tenant_id, .. }
                | ConferenceEvent::VideoLayoutChanged { tenant_id, .. }
                | ConferenceEvent::ActiveSpeakerChanged { tenant_id, .. } => Some(tenant_id),
            },
            PlatformEvent::Queue(e) => match e {
                QueueEvent::CallerEnqueued { tenant_id, .. }
                | QueueEvent::CallerDequeued { tenant_id, .. }
                | QueueEvent::CallerAbandoned { tenant_id, .. }
                | QueueEvent::CallerOverflowed { tenant_id, .. }
                | QueueEvent::AgentLoggedIn { tenant_id, .. }
                | QueueEvent::AgentLoggedOut { tenant_id, .. }
                | QueueEvent::AgentStateChanged { tenant_id, .. }
                | QueueEvent::AgentKicked { tenant_id, .. }
                | QueueEvent::CallbackInitiated { tenant_id, .. }
                | QueueEvent::StatsSnapshot { tenant_id, .. }
                | QueueEvent::CallerTransferred { tenant_id, .. }
                | QueueEvent::SupervisorJoined { tenant_id, .. }
                | QueueEvent::SupervisorLeft { tenant_id, .. }
                | QueueEvent::CallbackAttempted { tenant_id, .. }
                | QueueEvent::CallbackCompleted { tenant_id, .. }
                | QueueEvent::CallbackFailed { tenant_id, .. }
                | QueueEvent::ServiceLevelWarning { tenant_id, .. }
                | QueueEvent::QueueDraining { tenant_id, .. }
                | QueueEvent::QueueDrained { tenant_id, .. }
                | QueueEvent::CampaignCallPlaced { tenant_id, .. }
                | QueueEvent::CampaignCallConnected { tenant_id, .. }
                | QueueEvent::CampaignCallFailed { tenant_id, .. } => Some(tenant_id),
            },
            PlatformEvent::Presence(e) => match e {
                PresenceEvent::StatusChanged { tenant_id, .. }
                | PresenceEvent::WatcherAdded { tenant_id, .. }
                | PresenceEvent::WatcherRemoved { tenant_id, .. }
                | PresenceEvent::PublishReceived { tenant_id, .. }
                | PresenceEvent::DialogChanged { tenant_id, .. }
                | PresenceEvent::PublishExpiryWarning { tenant_id, .. }
                | PresenceEvent::PublishExpired { tenant_id, .. } => Some(tenant_id),
            },
            PlatformEvent::System(e) => match e {
                SystemEvent::ConfigChanged { tenant_id, .. } => Some(tenant_id),
                SystemEvent::Started { .. }
                | SystemEvent::Stopping { .. }
                | SystemEvent::HealthCheck { .. }
                | SystemEvent::NodeJoined { .. }
                | SystemEvent::NodeLeft { .. } => None,
            },
            PlatformEvent::Billing(e) => Some(e.tenant_id()),
            PlatformEvent::Cdr(CdrEvent::Written { tenant_id, .. }) => Some(tenant_id),
            PlatformEvent::Media(_)
            | PlatformEvent::Ivr(_)
            | PlatformEvent::Recording(_)
            | PlatformEvent::Flow(_)
            | PlatformEvent::Voicemail(_)
            | PlatformEvent::Sip(_) => None,
        }
    }

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
            PlatformEvent::Presence(_) => None,
            PlatformEvent::Sip(_) => None,
            PlatformEvent::Billing(event) => event.call_id(),
            PlatformEvent::Cdr(CdrEvent::Written { call_id, .. }) => Some(call_id),
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
            PlatformEvent::Presence(event) => event.timestamp(),
            PlatformEvent::Sip(event) => event.timestamp(),
            PlatformEvent::Billing(event) => event.timestamp(),
            PlatformEvent::Cdr(CdrEvent::Written { timestamp, .. }) => *timestamp,
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
            | CallEvent::Emergency { call_id, .. }
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
            | CallEvent::Emergency { timestamp, .. }
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
            | SystemEvent::HealthCheck { timestamp, .. }
            | SystemEvent::ConfigChanged { timestamp, .. }
            | SystemEvent::NodeJoined { timestamp, .. }
            | SystemEvent::NodeLeft { timestamp, .. } => *timestamp,
        }
    }
}

impl VoicemailEvent {
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            VoicemailEvent::MessageRecorded { call_id, .. }
            | VoicemailEvent::MessagePlayed { call_id, .. } => Some(call_id),
            VoicemailEvent::MessageDeleted { .. }
            | VoicemailEvent::MwiUpdated { .. }
            | VoicemailEvent::MailboxCreated { .. }
            | VoicemailEvent::MailboxUpdated { .. }
            | VoicemailEvent::MailboxDeleted { .. } => None,
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            VoicemailEvent::MessageRecorded { timestamp, .. }
            | VoicemailEvent::MessageDeleted { timestamp, .. }
            | VoicemailEvent::MessagePlayed { timestamp, .. }
            | VoicemailEvent::MwiUpdated { timestamp, .. }
            | VoicemailEvent::MailboxCreated { timestamp, .. }
            | VoicemailEvent::MailboxUpdated { timestamp, .. }
            | VoicemailEvent::MailboxDeleted { timestamp, .. } => *timestamp,
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
            | ConferenceEvent::MeetingEnded { timestamp, .. }
            | ConferenceEvent::DialOutInvited { timestamp, .. }
            | ConferenceEvent::DialOutFailed { timestamp, .. }
            | ConferenceEvent::VideoStateChanged { timestamp, .. }
            | ConferenceEvent::ContentStarted { timestamp, .. }
            | ConferenceEvent::ContentStopped { timestamp, .. }
            | ConferenceEvent::VideoOverload { timestamp, .. }
            | ConferenceEvent::VideoLayoutChanged { timestamp, .. }
            | ConferenceEvent::ActiveSpeakerChanged { timestamp, .. } => *timestamp,
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
            | QueueEvent::AgentKicked { timestamp, .. }
            | QueueEvent::StatsSnapshot { timestamp, .. }
            | QueueEvent::CallerTransferred { timestamp, .. }
            | QueueEvent::SupervisorJoined { timestamp, .. }
            | QueueEvent::SupervisorLeft { timestamp, .. }
            | QueueEvent::CallbackAttempted { timestamp, .. }
            | QueueEvent::CallbackCompleted { timestamp, .. }
            | QueueEvent::CallbackFailed { timestamp, .. }
            | QueueEvent::ServiceLevelWarning { timestamp, .. }
            | QueueEvent::QueueDraining { timestamp, .. }
            | QueueEvent::QueueDrained { timestamp, .. }
            | QueueEvent::CampaignCallPlaced { timestamp, .. }
            | QueueEvent::CampaignCallConnected { timestamp, .. }
            | QueueEvent::CampaignCallFailed { timestamp, .. } => *timestamp,
        }
    }
}

impl PresenceEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            PresenceEvent::StatusChanged { timestamp, .. }
            | PresenceEvent::WatcherAdded { timestamp, .. }
            | PresenceEvent::WatcherRemoved { timestamp, .. }
            | PresenceEvent::PublishReceived { timestamp, .. }
            | PresenceEvent::DialogChanged { timestamp, .. }
            | PresenceEvent::PublishExpiryWarning { timestamp, .. }
            | PresenceEvent::PublishExpired { timestamp, .. } => *timestamp,
        }
    }
}

impl SipTraceEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            SipTraceEvent::MessageCaptured { timestamp, .. } => *timestamp,
        }
    }
}

/// Billing events: call rating, balance movements and enforcement actions.
///
/// These are the external billing-system hooks: subscribe a webhook to
/// `billing.*` to mirror the ledger into an invoicing system, alert on low
/// balances, or audit rejected/cut-off calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BillingEvent {
    /// An answered call was rated against the tenant's rate plan.
    CallRated {
        tenant_id: TenantId,
        call_id: CallId,
        /// Total cost in `currency` (4 decimal places).
        cost: fcp_core::billing::Decimal,
        currency: String,
        /// Rate prefix that matched, `None` when the plan default applied.
        matched_prefix: Option<String>,
        /// Seconds billed after applying the billing increment.
        billable_secs: u64,
        /// Destination the call was rated for (dialed digits).
        destination: String,
        /// Balance after the charge was applied, when the tenant tracks one.
        balance_after: Option<fcp_core::billing::Decimal>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A ledger entry was applied (top-up, call charge, adjustment).
    BalanceChanged {
        tenant_id: TenantId,
        transaction_id: String,
        /// `top_up`, `call_charge` or `adjustment`.
        kind: String,
        /// Signed amount: positive credits, negative debits.
        amount: fcp_core::billing::Decimal,
        balance_after: fcp_core::billing::Decimal,
        currency: String,
        call_id: Option<CallId>,
        description: Option<String>,
        external_ref: Option<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// The balance crossed below the tenant's low-balance threshold.
    BalanceLow {
        tenant_id: TenantId,
        balance: fcp_core::billing::Decimal,
        threshold: fcp_core::billing::Decimal,
        currency: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// The balance (plus credit limit) reached zero on an enforcing tenant;
    /// further billable calls will be refused.
    BalanceDepleted {
        tenant_id: TenantId,
        balance: fcp_core::billing::Decimal,
        credit_limit: fcp_core::billing::Decimal,
        currency: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A call was refused at admission (402) for insufficient balance.
    CallRejected {
        tenant_id: TenantId,
        /// Dialed digits the admission check priced.
        destination: String,
        /// Funds available (balance + credit limit).
        available: fcp_core::billing::Decimal,
        /// Cost of the first billing increment that could not be covered.
        required: fcp_core::billing::Decimal,
        currency: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// An established call was hung up because its affordable duration
    /// ran out.
    CallCutOff {
        tenant_id: TenantId,
        call_id: CallId,
        /// Seconds the call was allowed to run.
        max_duration_secs: u64,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

/// Call detail record events. `Written` is announced once a record is in
/// the database, rated where it could be: what an external billing or
/// analytics system subscribes to (`cdr.written`). Delivery is at least
/// once — a replayed batch announces its records again — so consumers skip
/// a `cdr_id` they have seen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CdrEvent {
    Written {
        /// The record's own identity.
        cdr_id: uuid::Uuid,
        call_id: CallId,
        tenant_id: TenantId,
        /// `call`, `queue`, `conference` or `gateway`.
        kind: fcp_core::storage::CdrKind,
        disposition: fcp_core::storage::CallDisposition,
        #[serde(with = "system_time_serde")]
        start_time: SystemTime,
        #[serde(with = "system_time_serde")]
        end_time: SystemTime,
        /// Answered duration.
        duration_ms: u64,
        cost: Option<fcp_core::billing::Decimal>,
        currency: Option<String>,
        /// The node that owned the call.
        node_id: Option<String>,
        /// When the record was written.
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

impl BillingEvent {
    pub fn tenant_id(&self) -> &TenantId {
        match self {
            BillingEvent::CallRated { tenant_id, .. }
            | BillingEvent::BalanceChanged { tenant_id, .. }
            | BillingEvent::BalanceLow { tenant_id, .. }
            | BillingEvent::BalanceDepleted { tenant_id, .. }
            | BillingEvent::CallRejected { tenant_id, .. }
            | BillingEvent::CallCutOff { tenant_id, .. } => tenant_id,
        }
    }

    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            BillingEvent::CallRated { call_id, .. } | BillingEvent::CallCutOff { call_id, .. } => {
                Some(call_id)
            }
            BillingEvent::BalanceChanged { call_id, .. } => call_id.as_ref(),
            BillingEvent::BalanceLow { .. }
            | BillingEvent::BalanceDepleted { .. }
            | BillingEvent::CallRejected { .. } => None,
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            BillingEvent::CallRated { timestamp, .. }
            | BillingEvent::BalanceChanged { timestamp, .. }
            | BillingEvent::BalanceLow { timestamp, .. }
            | BillingEvent::BalanceDepleted { timestamp, .. }
            | BillingEvent::CallRejected { timestamp, .. }
            | BillingEvent::CallCutOff { timestamp, .. } => *timestamp,
        }
    }
}

#[cfg(test)]
mod content_event_tests {
    use super::*;

    #[test]
    fn a_video_state_from_an_older_node_reads_as_the_camera() {
        let tenant = TenantId::new();
        let json = serde_json::json!({
            "type": "video_state_changed",
            "room_number": "1000",
            "participant_id": "alice",
            "state": "on",
            "tenant_id": tenant,
            "timestamp": 1_700_000_000u64,
        });
        let ev: ConferenceEvent = serde_json::from_value(json).unwrap();
        match &ev {
            ConferenceEvent::VideoStateChanged { stream, .. } => assert_eq!(stream, "camera"),
            other => panic!("unexpected {other:?}"),
        }
        let started = PlatformEvent::Conference(ConferenceEvent::ContentStarted {
            room_number: "1000".into(),
            participant_id: "alice".into(),
            node_id: "conf-a".into(),
            codec: "VP8".into(),
            tenant_id: tenant,
            timestamp: SystemTime::now(),
        });
        assert_eq!(started.event_type_name(), "conference.content_started");
        assert_eq!(started.tenant_id(), Some(&tenant));
        let stopped = PlatformEvent::Conference(ConferenceEvent::ContentStopped {
            room_number: "1000".into(),
            participant_id: "alice".into(),
            node_id: "conf-a".into(),
            reason: "idle".into(),
            tenant_id: tenant,
            timestamp: SystemTime::now(),
        });
        assert_eq!(stopped.event_type_name(), "conference.content_stopped");
        let text = serde_json::to_string(&stopped).unwrap();
        assert!(text.contains("\"reason\":\"idle\""));
        let back: PlatformEvent = serde_json::from_str(&text).unwrap();
        assert_eq!(back.event_type_name(), "conference.content_stopped");
    }
}
