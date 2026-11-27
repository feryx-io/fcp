//! Event type definitions for the FCP platform.

use fcp_core::{CallId, SessionId, TenantId, UserId};
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

    /// Compliance event: legal holds (`docs/COMPLIANCE_FOLLOWUPS.md` §5)
    Compliance(ComplianceEvent),

    /// Trunk event: what a carrier's trunk is doing on this node
    /// (`docs/TENANT_TRUNKS.md` §9)
    Trunk(TrunkEvent),

    /// Speech event: a tenant nearing or at a spending cap
    /// (`docs/SPEECH_CAPS.md` §2)
    Speech(SpeechEvent),
}

fn characters() -> String {
    "characters".to_string()
}

/// A tenant's speech against its caps, said once per cap and month by the
/// admin's sweep (`docs/SPEECH_CAPS.md`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpeechEvent {
    /// 80 % of the month's cap on a provider synthesized.
    CapNear {
        tenant_id: TenantId,
        /// A platform provider's name, or `account:<uuid>`.
        provider: String,
        /// `characters` or `minutes` (`docs/TENANT_RECOGNITION.md` R3);
        /// `cap` and `used` are in it.
        #[serde(default = "characters")]
        unit: String,
        /// Synthesized characters the month allows.
        cap: u64,
        /// Synthesized so far this month.
        used: u64,
        /// `YYYY-MM` (UTC).
        month: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
    /// The cap reached: the default provider speaks for the tenant on this
    /// one until the month turns or the cap is raised.
    CapReached {
        tenant_id: TenantId,
        provider: String,
        /// `characters` or `minutes` (`docs/TENANT_RECOGNITION.md` R3);
        /// `cap` and `used` are in it.
        #[serde(default = "characters")]
        unit: String,
        cap: u64,
        used: u64,
        month: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

impl SpeechEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            SpeechEvent::CapNear { timestamp, .. } | SpeechEvent::CapReached { timestamp, .. } => {
                *timestamp
            }
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        match self {
            SpeechEvent::CapNear { tenant_id, .. } | SpeechEvent::CapReached { tenant_id, .. } => {
                tenant_id
            }
        }
    }
}

/// What a trunk is doing, on the node it is doing it on.
///
/// A registering trunk is held by one node of a cluster
/// (`docs/TENANT_TRUNKS.md` §8), so these say which node as well as which
/// trunk: two nodes reporting the same trunk is how a split is seen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TrunkEvent {
    /// The carrier accepted this node's REGISTER for the trunk.
    Registered {
        trunk_id: String,
        trunk_name: String,
        /// The platform's own trunks have no tenant.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        /// The account bound at the carrier, when the trunk has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<String>,
        /// How long the binding holds, in seconds.
        expires_secs: u32,
        node_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// What this node's probes make of the trunk changed. Health is per
    /// node (`docs/TENANT_TRUNKS.md` §8): another node may still reach a
    /// carrier this one cannot, so the event names which node found what.
    HealthChanged {
        trunk_id: String,
        trunk_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        /// `healthy`, `degraded` or `down`.
        status: String,
        /// What it was before, so a consumer sees the direction.
        previous: String,
        consecutive_failures: u32,
        node_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// The REGISTER was rejected, timed out or could not be sent.
    RegistrationFailed {
        trunk_id: String,
        trunk_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<String>,
        /// What the carrier said, or what went wrong here.
        reason: String,
        node_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

impl TrunkEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            TrunkEvent::Registered { timestamp, .. }
            | TrunkEvent::RegistrationFailed { timestamp, .. }
            | TrunkEvent::HealthChanged { timestamp, .. } => *timestamp,
        }
    }

    pub fn tenant_id(&self) -> Option<&TenantId> {
        match self {
            TrunkEvent::Registered { tenant_id, .. }
            | TrunkEvent::RegistrationFailed { tenant_id, .. }
            | TrunkEvent::HealthChanged { tenant_id, .. } => tenant_id.as_ref(),
        }
    }
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
        /// Which trunk that is: two tenants may both have a "carrier-a"
        /// (`docs/TENANT_TRUNKS.md` §4).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_id: Option<String>,
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
        /// The address the caller was seen at, when one placed them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_address: Option<String>,
        /// The tenant's site the caller was placed at, whose own contacts
        /// are told first.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_name: Option<String>,
        /// People/URIs told about this call: the site's contacts, then
        /// the tenant's.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        notify_contacts: Vec<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// An emergency call ended: what became of it, for the record and for
    /// whoever followed `call.emergency` (`docs/E911_BLOCK_2.md` §4).
    EmergencyCompleted {
        call_id: CallId,
        tenant_id: TenantId,
        dialed: String,
        /// `live` or `test`.
        kind: String,
        /// `answered`, `busy`, `no_answer`, `failed`, `cancelled`, …
        outcome: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sip_status: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_name: Option<String>,
        /// Which trunk carried it (`docs/TENANT_TRUNKS.md` §5).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_id: Option<String>,
        /// How the location travelled: `elin`, or none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery: Option<String>,
        #[serde(default)]
        duration_ms: u64,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// A contact the tenant asked to be told about emergency calls could
    /// not be told, after every attempt (`docs/E911_BLOCK_2.md` §8.3).
    EmergencyNotificationFailed {
        call_id: CallId,
        tenant_id: TenantId,
        /// `email`, `sms`, `call` or `webhook`.
        contact_kind: String,
        /// The address, number, extension or endpoint id.
        target: String,
        /// How many attempts were made.
        attempts: u32,
        /// Why the last attempt failed.
        error: String,
        /// Whether this was a test call's notice.
        #[serde(default)]
        test: bool,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// Call is ringing (180/183 received)
    Ringing {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// Call progress (183 Session Progress)
    Progress {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// Call answered (200 OK)
    Answered {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
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
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        end_time: SystemTime,
        duration_ms: u64,
        hangup_cause: String,
        hangup_party: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_name: Option<String>,
        /// Which trunk carried it (`docs/TENANT_TRUNKS.md` §4).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// Call failed (4xx/5xx/6xx)
    Failed {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        reason: String,
        sip_code: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_name: Option<String>,
        /// Which trunk it was tried on (`docs/TENANT_TRUNKS.md` §4).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trunk_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// Call parked in a parking orbit
    Parked {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        /// The slot, as the tenant numbers it (`docs/PARKING.md`).
        #[serde(default)]
        slot: String,
        /// The slot as a number, for consumers of the old field: 0 when it
        /// is not one (a leading zero, more than 65535). Use `slot`.
        orbit: u16,
        parker_extension: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Parked call retrieved (unparked)
    Unparked {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        /// The slot it was retrieved from.
        #[serde(default)]
        slot: String,
        /// The slot as a number (see `Parked`). Use `slot`.
        orbit: u16,
        retriever_extension: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A parked call nobody took back in its lot's time
    /// (`docs/PARKING.md` §3): it rings its parker, and `fallback` is
    /// where it goes if the parker does not take it.
    ParkTimedOut {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        slot: String,
        lot_id: String,
        parker_extension: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A phone reported a call it was on as poor (`docs/CALL_DIAGNOSTICS.md`
    /// §2): its estimated score (E-model MOS, 1 to 4.5) under 3.1 — what
    /// the user heard (`mos`) or what the far end did (`far_mos`).
    QualityPoor {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        /// The user whose phone reported it, by id and name.
        user_id: String,
        username: String,
        /// The phone (its device), when the report came from an app.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_id: Option<String>,
        mos: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        far_mos: Option<f32>,
        /// The far end's packets lost over the call, in percent.
        loss_pct: f32,
        jitter_ms: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rtt_ms: Option<f32>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A leg put on hold by its far party (`docs/ENTERPRISE_CALLING.md`
    /// §3.2): `held_leg` is `a` or `b`.
    Held {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        held_leg: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// A piece of a call's live transcript, from a flow's `Transcribe`
    /// node (`docs/DEVELOPER_SURFACE.md` phase 7): `leg` is `caller` or
    /// `callee`; a partial transcript (`is_final` false) may be revised by
    /// the next one.
    Transcript {
        call_id: CallId,
        tenant_id: TenantId,
        leg: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
        is_final: bool,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// The held leg taken off hold.
    Resumed {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        held_leg: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// A supervisor joined the call (`docs/ENTERPRISE_CALLING.md` §3.11):
    /// `mode` is `monitor`, `whisper` or `barge`; `party` the supervisor's
    /// address, `user_id` theirs when a user.
    PartyJoined {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        mode: String,
        party: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_id: Option<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    /// The supervisor left (hung up, was removed, or the call ended).
    PartyLeft {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        party: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
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
    /// A call's recording started, after its consent step.
    Started {
        recording_id: String,
        call_id: CallId,
        tenant_id: TenantId,
        /// Why it is recorded: `api`, `flow`, `queue`, `user`, `on_demand`,
        /// `number` or `platform`.
        source: String,
        /// What the parties were told: `announced`, `failed` or
        /// `not_required`.
        consent: String,
        /// The party who asked, for an `on_demand` recording.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        started_by: Option<UserId>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Recording completed
    Completed {
        recording_id: String,
        call_id: CallId,
        tenant_id: TenantId,
        duration_ms: u64,
        size_bytes: u64,
        url: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A recording that was to start did not. `reason` is `consent_refused`
    /// when the announcement could not be played and the tenant's policy
    /// says not to record without one.
    Failed {
        recording_id: String,
        call_id: CallId,
        tenant_id: TenantId,
        source: String,
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
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        flow_id: String,
        flow_version: u32,
        initial_node: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Flow node executed
    NodeExecuted {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        flow_id: String,
        node_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Flow waiting for user input (DTMF)
    WaitingForInput {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        flow_id: String,
        variable: String,
        timeout_secs: u32,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Flow execution completed
    Completed {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        flow_id: String,
        outcome: String, // "hangup", "stopped", "error"
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Flow execution failed with error
    Failed {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
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

    /// A node began draining: it refuses new calls and is waiting for
    /// the ones it has (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    NodeDraining {
        node_id: String,
        /// Calls it was carrying when the drain began.
        calls: usize,
        /// How long it will wait for them before hanging up what is left.
        wait_secs: u64,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A draining node stopped waiting: every call had ended, or the
    /// deadline came and the rest were hung up.
    NodeDrained {
        node_id: String,
        /// Calls still up at the deadline, each sent a BYE. Zero when the
        /// node emptied on its own.
        calls_ended: usize,
        /// Seconds the node waited.
        waited_secs: u64,
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

    /// A node left the cluster: `reason` is `shutdown` when it said so
    /// and `heartbeat_expired` when it went quiet.
    NodeLeft {
        node_id: String,
        reason: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A site's link came up or went down (`docs/HYBRID_CONTROL_PLANE.md`
    /// §3.4): the first heartbeat after none, or none for too long.
    SiteLinkChanged {
        site_id: String,
        tenant_id: TenantId,
        name: String,
        up: bool,
        /// How long the previous state lasted, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lasted_secs: Option<u64>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A service came up: first seen, or back after it was down
    /// (`docs/SERVICES.md` §2). Said once by the admin that keeps the
    /// record of every service.
    ServiceUp {
        /// `[service] node_id` of the process.
        service_id: String,
        /// `call_manager`, `voicemail`, …
        kind: String,
        host: String,
        version: String,
        /// The site it runs at; none for the platform's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site_id: Option<String>,
        /// How long it was down, when it was down before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        down_secs: Option<u64>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A service stopped saying it is there (`docs/SERVICES.md` §2). Said
    /// once, by the admin, when its beacon expired; never while Redis
    /// itself cannot be read.
    ServiceDown {
        service_id: String,
        kind: String,
        host: String,
        version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site_id: Option<String>,
        /// It said it was stopping: shut down on purpose, not lost.
        stopped: bool,
        /// Its last beacon (Unix seconds).
        last_seen: u64,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A phone registered running another firmware than its tenant's
    /// target points it at (`docs/PROVISIONING.md` §11): said once each
    /// time what it runs changes, by the node that took the REGISTER.
    DeviceFirmwareBehind {
        tenant_id: TenantId,
        device_id: String,
        device_name: String,
        /// What it runs, from its User-Agent.
        running: String,
        /// What its target installs.
        target: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A push to wake a phone app was refused because the provider no
    /// longer knows its token (`docs/PUSH.md` §2.3): the app was removed
    /// or its token changed. Its binding loses the push address, so the
    /// app is not woken again until it registers a new one.
    DevicePushFailed {
        tenant_id: TenantId,
        /// The device, when the node knows it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_id: Option<String>,
        aor: String,
        contact: String,
        /// `apns` or `fcm`.
        provider: String,
        /// What the provider said.
        reason: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// The set of releases the cluster's nodes run changed while there
    /// is, or was, more than one (`docs/BACKUP_AND_RESTORE.md` §5): the
    /// first node of an upgrade started, or the last old one stopped.
    /// Said by every node that sees it, never acted on.
    VersionSkew {
        /// The node whose discovery saw the change.
        node_id: String,
        /// Release → the node ids on it.
        versions: std::collections::BTreeMap<String, Vec<String>>,
        /// More than one release is running now.
        skew: bool,
        /// One sentence for a person.
        summary: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A node's own view of the cluster stopped or started looking whole
    /// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3). Said, never acted on.
    ClusterViewChanged {
        node_id: String,
        /// The region of the node whose view it is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<String>,
        /// `whole`, `store_unreachable` or `remote_regions_lost`.
        state: String,
        /// The state before this one.
        previous: String,
        /// The regions whose every node went at once.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lost_regions: Vec<String>,
        /// Whether the node still reaches its database, when it can tell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database_reachable: Option<bool>,
        /// How long the previous state lasted, when it was not whole.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lasted_secs: Option<u64>,
        summary: String,
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

    /// A deleted user is deprovisioned: suspended now, purged at
    /// `purge_after` unless restored (`docs/USERS.md` §4).
    Deprovisioned {
        user_id: String,
        username: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        purge_after: SystemTime,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A user renamed (`docs/USERS.md` §6); their SIP name is unchanged.
    Renamed {
        user_id: String,
        old_username: String,
        username: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A deprovisioned user restored within the grace period.
    Restored {
        user_id: String,
        username: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// User deleted: purged, gone from every store
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

    /// The user's calling settings changed (`docs/ENTERPRISE_CALLING.md`
    /// §3.1): DND, call waiting, a forward, the ring time — by the API,
    /// the self-service page or a feature code.
    CallingChanged {
        user_id: String,
        username: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// One of the user's own contacts was made, changed or deleted
    /// (`docs/RECORD_AND_CONTACTS.md` §3), through `/me/contacts` or by a
    /// site, so the apps they are signed in to read their contacts again.
    /// It names the contact, never what it holds.
    ContactChanged {
        user_id: String,
        tenant_id: TenantId,
        contact_id: String,
        /// `created`, `updated` or `deleted`.
        change: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A user signed in to the admin API or the user portal
    /// (`docs/IDENTITY_AND_DEVICES.md` phase 1): `method` is `password`,
    /// later `oidc`, `saml`.
    SignedIn {
        user_id: String,
        username: String,
        tenant_id: TenantId,
        method: String,
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_ip: Option<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A sign-in was refused: `reason` is `bad_credentials`, `locked`,
    /// `disabled` or `suspended`. The tenant is the user's when the
    /// username was found.
    SignInFailed {
        username: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_ip: Option<String>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A user logged in on a host phone (`hoteling.login`,
    /// `docs/ENTERPRISE_CALLING.md` §3.8).
    HotelingLogin {
        session_id: String,
        user_id: String,
        username: String,
        device_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        expires_at: SystemTime,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A hoteling session ended (`hoteling.logout`): `reason` is
    /// `logout`, `admin`, `expired` or `replaced`.
    HotelingLogout {
        session_id: String,
        user_id: String,
        device_id: String,
        tenant_id: TenantId,
        reason: String,
        /// Whether whoever ended it has told the phone to re-read its
        /// configuration: a call manager has; the API cannot, so the call
        /// managers do it when they hear this.
        #[serde(default)]
        resynced: bool,
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
        /// The Contact's `+sip.instance` (RFC 5626): the same phone across
        /// addresses, which a call waiting for a woken app matches
        /// (`docs/PUSH.md` §2.2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<String>,
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

    /// A participant raised their hand (`docs/WEBINARS.md` §2.5): by `*4`,
    /// an `fcp-qa` INFO or their app.
    HandRaised {
        room_number: String,
        participant_id: String,
        /// The name the room shows them by.
        display_name: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A raised hand came down: its owner lowered it (`by_host` false), or
    /// a host lowered it or brought its owner to the stage.
    HandLowered {
        room_number: String,
        participant_id: String,
        by_host: bool,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A host brought a webinar's attendee to the stage: they are heard by
    /// the room until sent back.
    StageJoined {
        room_number: String,
        participant_id: String,
        display_name: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A host sent an attendee they had brought to the stage back to the
    /// audience.
    StageLeft {
        room_number: String,
        participant_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A question was asked in a room (`docs/WEBINARS.md` §2.5). The text
    /// is not carried: it is the room's, read through its questions.
    QuestionAsked {
        room_number: String,
        question_id: String,
        participant_id: String,
        anonymous: bool,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A host or panelist showed a question to the room.
    QuestionPublished {
        room_number: String,
        question_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A host or panelist answered a question, in text or aloud (`live`).
    QuestionAnswered {
        room_number: String,
        question_id: String,
        live: bool,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A host or panelist dismissed a question.
    QuestionDismissed {
        room_number: String,
        question_id: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A host or panelist deleted a chat message (`docs/WEBINARS.md`
    /// §2.5): `participant_id` sent it; `by` deleted it (a participant id,
    /// or `None` through the API).
    ChatDeleted {
        room_number: String,
        message_id: String,
        participant_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A host or panelist muted (`on`) or unmuted a participant's chat.
    ChatMuted {
        room_number: String,
        participant_id: String,
        on: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A host took a webinar live: its lobby is let in (`docs/WEBINARS.md`
    /// §2.6). `by` is the host's participant id, or `None` through the API.
    WebinarLive {
        room_number: String,
        sitting_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A webinar ended: a host ended it (`by` their participant id, or
    /// `api`), or its stage was empty for the grace period while live
    /// (`stage_left`). Everyone in it is released.
    WebinarEnded {
        room_number: String,
        sitting_id: String,
        by: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A webinar's RTMP push reached its live service and is publishing
    /// (`docs/WEBINARS.md` W7): the first time, or again after a loss.
    /// `host` is the ingest's; the stream key is never in an event.
    RtmpLive {
        room_number: String,
        sitting_id: String,
        host: String,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A webinar's RTMP push has failed for a minute and is still being
    /// retried: once for each outage. `error` is the last one.
    RtmpFailed {
        room_number: String,
        sitting_id: String,
        host: String,
        error: String,
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
        /// What the room was told: `announced`, `failed` or `not_required`.
        #[serde(default)]
        consent: Option<String>,
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

    /// An appearance of a shared line changed (`line.appearance_changed`,
    /// `docs/ENTERPRISE_CALLING.md` §3.9): every node sends the line's
    /// watchers its lamps. `state` is `idle` when the index was freed.
    AppearanceChanged {
        line_binding_id: String,
        number: String,
        index: u8,
        state: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        /// The node that changed it: it has told its own watchers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// What others see of a person changed (`presence.user_changed`,
    /// `docs/PRESENCE.md`): the status they chose with calls, do not
    /// disturb, idleness and being signed out laid over it. `state` is
    /// `available`, `away`, `busy`, `dnd`, `ringing`, `on_call` or
    /// `offline`; `label` is their own status's, `idle` says an away is
    /// the app's doing.
    UserChanged {
        user_id: String,
        username: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extension: Option<String>,
        state: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default)]
        idle: bool,
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A call began or stopped counting against one of its parties
    /// (`presence.user_call`): the call manager's word, from the parties
    /// it counts for call waiting, that a person is on a call. The admin
    /// works presence out from it; apps are not sent it.
    UserCall {
        user_id: String,
        call_id: String,
        on_call: bool,
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
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        mailbox_id: String,
        message_id: String,
        caller: String,
        duration_secs: u32,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// A message's speech recognised, when its mailbox asks for
    /// transcription (`docs/DEVELOPER_SURFACE.md` phase 6). The text is
    /// also stored on the message.
    MessageTranscribed {
        tenant_id: TenantId,
        mailbox_id: String,
        message_id: String,
        transcription: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Voicemail message deleted
    MessageDeleted {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        mailbox_id: String,
        message_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Voicemail message played back
    MessagePlayed {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        call_id: CallId,
        mailbox_id: String,
        message_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// MWI (Message Waiting Indicator) updated
    MwiUpdated {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
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
        tenant_id: TenantId,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Mailbox updated (triggers voicemail server to refresh routes)
    MailboxUpdated {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
        mailbox_id: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },

    /// Mailbox deleted (triggers voicemail server to remove routes)
    MailboxDeleted {
        /// The tenant, for the webhooks that need one to find their
        /// endpoints. Absent only from an older node's event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<TenantId>,
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
    /// Every name [`Self::event_type_name`] gives, in its order: what a
    /// webhook may subscribe to (docs/DEVELOPER_SURFACE.md §3, phase 5).
    /// `every_event_type_is_listed` keeps it in step.
    pub const EVENT_TYPES: &'static [&'static str] = &[
        "call.initiated",
        "call.ringing",
        "call.progress",
        "call.answered",
        "call.ended",
        "call.failed",
        "call.emergency",
        "call.emergency_completed",
        "emergency.notification_failed",
        "call.parked",
        "call.unparked",
        "call.park_timed_out",
        "call.quality_poor",
        "call.held",
        "call.resumed",
        "call.transcript",
        "call.party_joined",
        "call.party_left",
        "media.session_started",
        "media.session_ended",
        "media.dtmf_received",
        "media.quality_warning",
        "ivr.prompt_started",
        "ivr.prompt_completed",
        "ivr.input_gathered",
        "ivr.gather_timeout",
        "recording.started",
        "recording.completed",
        "recording.failed",
        "flow.started",
        "flow.node_executed",
        "flow.waiting_for_input",
        "flow.completed",
        "flow.failed",
        "system.started",
        "system.stopping",
        "system.health_check",
        "system.config_changed",
        "system.node_draining",
        "system.node_drained",
        "system.node_joined",
        "system.node_left",
        "system.cluster_view_changed",
        "system.version_skew",
        "system.site_link_changed",
        "system.service_up",
        "system.service_down",
        "device.firmware_behind",
        "device.push_failed",
        "user.created",
        "user.updated",
        "user.deleted",
        "user.deprovisioned",
        "user.restored",
        "user.renamed",
        "user.enabled",
        "user.disabled",
        "user.calling_changed",
        "user.contact_changed",
        "user.signed_in",
        "user.sign_in_failed",
        "hoteling.login",
        "hoteling.logout",
        "registration.registered",
        "registration.refreshed",
        "registration.unregistered",
        "registration.expired",
        "registration.failed",
        "voicemail.message_recorded",
        "voicemail.transcribed",
        "voicemail.message_deleted",
        "voicemail.message_played",
        "voicemail.mwi_updated",
        "voicemail.mailbox_created",
        "voicemail.mailbox_updated",
        "voicemail.mailbox_deleted",
        "conference.participant_joined",
        "conference.participant_left",
        "conference.room_locked",
        "conference.room_unlocked",
        "conference.recording_started",
        "conference.recording_stopped",
        "conference.pin_failed",
        "conference.room_destroyed",
        "conference.meeting_created",
        "conference.meeting_updated",
        "conference.meeting_deleted",
        "conference.meeting_started",
        "conference.meeting_ended",
        "conference.dial_out_invited",
        "conference.dial_out_failed",
        "conference.video_state_changed",
        "conference.content_started",
        "conference.content_stopped",
        "conference.video_overload",
        "conference.video_layout_changed",
        "conference.active_speaker_changed",
        "conference.hand_raised",
        "conference.hand_lowered",
        "conference.stage_joined",
        "conference.stage_left",
        "conference.question_asked",
        "conference.question_published",
        "conference.question_answered",
        "conference.question_dismissed",
        "conference.chat_deleted",
        "conference.chat_muted",
        "conference.webinar_live",
        "conference.webinar_ended",
        "conference.rtmp_live",
        "conference.rtmp_failed",
        "queue.caller_enqueued",
        "queue.caller_dequeued",
        "queue.caller_abandoned",
        "queue.caller_overflowed",
        "queue.agent_logged_in",
        "queue.agent_logged_out",
        "queue.agent_state_changed",
        "queue.agent_kicked",
        "queue.callback_initiated",
        "queue.stats_snapshot",
        "queue.caller_transferred",
        "queue.supervisor_joined",
        "queue.supervisor_left",
        "queue.callback_attempted",
        "queue.callback_completed",
        "queue.callback_failed",
        "queue.service_level_warning",
        "queue.queue_draining",
        "queue.queue_drained",
        "queue.campaign_call_placed",
        "queue.campaign_call_connected",
        "queue.campaign_call_failed",
        "presence.status_changed",
        "presence.watcher_added",
        "presence.watcher_removed",
        "presence.publish_received",
        "presence.dialog_changed",
        "presence.user_changed",
        "presence.user_call",
        "presence.publish_expiry_warning",
        "presence.publish_expired",
        "line.appearance_changed",
        "sip.message_captured",
        "billing.call_rated",
        "billing.balance_changed",
        "billing.balance_low",
        "billing.balance_depleted",
        "billing.call_rejected",
        "billing.call_cut_off",
        "cdr.written",
        "cdr.archived",
        "trunk.registered",
        "trunk.registration_failed",
        "trunk.health_changed",
        "compliance.hold_placed",
        "compliance.hold_released",
        "compliance.subject_request_completed",
        "speech.cap_near",
        "speech.cap_reached",
    ];

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
                CallEvent::EmergencyCompleted { .. } => "call.emergency_completed",
                CallEvent::EmergencyNotificationFailed { .. } => "emergency.notification_failed",
                CallEvent::Parked { .. } => "call.parked",
                CallEvent::Unparked { .. } => "call.unparked",
                CallEvent::ParkTimedOut { .. } => "call.park_timed_out",
                CallEvent::QualityPoor { .. } => "call.quality_poor",
                CallEvent::Held { .. } => "call.held",
                CallEvent::Resumed { .. } => "call.resumed",
                CallEvent::Transcript { .. } => "call.transcript",
                CallEvent::PartyJoined { .. } => "call.party_joined",
                CallEvent::PartyLeft { .. } => "call.party_left",
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
                SystemEvent::NodeDraining { .. } => "system.node_draining",
                SystemEvent::NodeDrained { .. } => "system.node_drained",
                SystemEvent::NodeJoined { .. } => "system.node_joined",
                SystemEvent::NodeLeft { .. } => "system.node_left",
                SystemEvent::ClusterViewChanged { .. } => "system.cluster_view_changed",
                SystemEvent::VersionSkew { .. } => "system.version_skew",
                SystemEvent::SiteLinkChanged { .. } => "system.site_link_changed",
                SystemEvent::ServiceUp { .. } => "system.service_up",
                SystemEvent::ServiceDown { .. } => "system.service_down",
                SystemEvent::DeviceFirmwareBehind { .. } => "device.firmware_behind",
                SystemEvent::DevicePushFailed { .. } => "device.push_failed",
            },
            PlatformEvent::User(e) => match e {
                UserEvent::Created { .. } => "user.created",
                UserEvent::Updated { .. } => "user.updated",
                UserEvent::Deleted { .. } => "user.deleted",
                UserEvent::Deprovisioned { .. } => "user.deprovisioned",
                UserEvent::Restored { .. } => "user.restored",
                UserEvent::Renamed { .. } => "user.renamed",
                UserEvent::Enabled { .. } => "user.enabled",
                UserEvent::Disabled { .. } => "user.disabled",
                UserEvent::CallingChanged { .. } => "user.calling_changed",
                UserEvent::ContactChanged { .. } => "user.contact_changed",
                UserEvent::HotelingLogin { .. } => "hoteling.login",
                UserEvent::HotelingLogout { .. } => "hoteling.logout",
                UserEvent::SignedIn { .. } => "user.signed_in",
                UserEvent::SignInFailed { .. } => "user.sign_in_failed",
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
                VoicemailEvent::MessageTranscribed { .. } => "voicemail.transcribed",
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
                ConferenceEvent::HandRaised { .. } => "conference.hand_raised",
                ConferenceEvent::HandLowered { .. } => "conference.hand_lowered",
                ConferenceEvent::StageJoined { .. } => "conference.stage_joined",
                ConferenceEvent::StageLeft { .. } => "conference.stage_left",
                ConferenceEvent::QuestionAsked { .. } => "conference.question_asked",
                ConferenceEvent::QuestionPublished { .. } => "conference.question_published",
                ConferenceEvent::QuestionAnswered { .. } => "conference.question_answered",
                ConferenceEvent::QuestionDismissed { .. } => "conference.question_dismissed",
                ConferenceEvent::ChatDeleted { .. } => "conference.chat_deleted",
                ConferenceEvent::ChatMuted { .. } => "conference.chat_muted",
                ConferenceEvent::WebinarLive { .. } => "conference.webinar_live",
                ConferenceEvent::WebinarEnded { .. } => "conference.webinar_ended",
                ConferenceEvent::RtmpLive { .. } => "conference.rtmp_live",
                ConferenceEvent::RtmpFailed { .. } => "conference.rtmp_failed",
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
                PresenceEvent::AppearanceChanged { .. } => "line.appearance_changed",
                PresenceEvent::UserChanged { .. } => "presence.user_changed",
                PresenceEvent::UserCall { .. } => "presence.user_call",
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
                CdrEvent::Archived { .. } => "cdr.archived",
            },
            PlatformEvent::Speech(e) => match e {
                SpeechEvent::CapNear { .. } => "speech.cap_near",
                SpeechEvent::CapReached { .. } => "speech.cap_reached",
            },
            PlatformEvent::Trunk(e) => match e {
                TrunkEvent::Registered { .. } => "trunk.registered",
                TrunkEvent::RegistrationFailed { .. } => "trunk.registration_failed",
                TrunkEvent::HealthChanged { .. } => "trunk.health_changed",
            },
            PlatformEvent::Compliance(e) => match e {
                ComplianceEvent::HoldPlaced { .. } => "compliance.hold_placed",
                ComplianceEvent::HoldReleased { .. } => "compliance.hold_released",
                ComplianceEvent::SubjectRequestCompleted { .. } => {
                    "compliance.subject_request_completed"
                }
            },
        }
    }

    /// Extract the tenant ID from this event, if the event variant carries one.
    pub fn tenant_id(&self) -> Option<&TenantId> {
        match self {
            PlatformEvent::Call(e) => match e {
                // The call events that carry a tenant. Emergency events do
                // and were nevertheless invisible to the webhook
                // dispatcher, which needs one to find the endpoints
                // (`docs/E911_BLOCK_2.md` §8).
                CallEvent::Initiated { tenant_id, .. }
                | CallEvent::Emergency { tenant_id, .. }
                | CallEvent::EmergencyCompleted { tenant_id, .. }
                | CallEvent::EmergencyNotificationFailed { tenant_id, .. }
                | CallEvent::Transcript { tenant_id, .. } => Some(tenant_id),
                CallEvent::Ringing { tenant_id, .. }
                | CallEvent::Progress { tenant_id, .. }
                | CallEvent::Answered { tenant_id, .. }
                | CallEvent::Ended { tenant_id, .. }
                | CallEvent::Failed { tenant_id, .. }
                | CallEvent::Parked { tenant_id, .. }
                | CallEvent::Unparked { tenant_id, .. }
                | CallEvent::ParkTimedOut { tenant_id, .. }
                | CallEvent::QualityPoor { tenant_id, .. }
                | CallEvent::Held { tenant_id, .. }
                | CallEvent::Resumed { tenant_id, .. }
                | CallEvent::PartyJoined { tenant_id, .. }
                | CallEvent::PartyLeft { tenant_id, .. } => tenant_id.as_ref(),
            },
            PlatformEvent::User(e) => match e {
                UserEvent::Created { tenant_id, .. }
                | UserEvent::Updated { tenant_id, .. }
                | UserEvent::Deleted { tenant_id, .. }
                | UserEvent::Deprovisioned { tenant_id, .. }
                | UserEvent::Restored { tenant_id, .. }
                | UserEvent::Renamed { tenant_id, .. }
                | UserEvent::Enabled { tenant_id, .. }
                | UserEvent::Disabled { tenant_id, .. }
                | UserEvent::CallingChanged { tenant_id, .. }
                | UserEvent::ContactChanged { tenant_id, .. }
                | UserEvent::HotelingLogin { tenant_id, .. }
                | UserEvent::HotelingLogout { tenant_id, .. }
                | UserEvent::SignedIn { tenant_id, .. } => Some(tenant_id),
                UserEvent::SignInFailed { tenant_id, .. } => tenant_id.as_ref(),
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
                | ConferenceEvent::ActiveSpeakerChanged { tenant_id, .. }
                | ConferenceEvent::HandRaised { tenant_id, .. }
                | ConferenceEvent::HandLowered { tenant_id, .. }
                | ConferenceEvent::StageJoined { tenant_id, .. }
                | ConferenceEvent::StageLeft { tenant_id, .. }
                | ConferenceEvent::QuestionAsked { tenant_id, .. }
                | ConferenceEvent::QuestionPublished { tenant_id, .. }
                | ConferenceEvent::QuestionAnswered { tenant_id, .. }
                | ConferenceEvent::QuestionDismissed { tenant_id, .. }
                | ConferenceEvent::ChatDeleted { tenant_id, .. }
                | ConferenceEvent::ChatMuted { tenant_id, .. }
                | ConferenceEvent::WebinarLive { tenant_id, .. }
                | ConferenceEvent::WebinarEnded { tenant_id, .. }
                | ConferenceEvent::RtmpLive { tenant_id, .. }
                | ConferenceEvent::RtmpFailed { tenant_id, .. } => Some(tenant_id),
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
                | PresenceEvent::PublishExpired { tenant_id, .. }
                | PresenceEvent::AppearanceChanged { tenant_id, .. }
                | PresenceEvent::UserChanged { tenant_id, .. }
                | PresenceEvent::UserCall { tenant_id, .. } => Some(tenant_id),
            },
            PlatformEvent::System(e) => match e {
                SystemEvent::ConfigChanged { tenant_id, .. }
                | SystemEvent::DeviceFirmwareBehind { tenant_id, .. }
                | SystemEvent::DevicePushFailed { tenant_id, .. } => Some(tenant_id),
                SystemEvent::Started { .. }
                | SystemEvent::Stopping { .. }
                | SystemEvent::HealthCheck { .. }
                | SystemEvent::NodeDraining { .. }
                | SystemEvent::NodeDrained { .. }
                | SystemEvent::NodeJoined { .. }
                | SystemEvent::NodeLeft { .. }
                | SystemEvent::ClusterViewChanged { .. }
                | SystemEvent::VersionSkew { .. }
                | SystemEvent::SiteLinkChanged { .. }
                | SystemEvent::ServiceUp { .. }
                | SystemEvent::ServiceDown { .. } => None,
            },
            PlatformEvent::Voicemail(e) => match e {
                VoicemailEvent::MessageTranscribed { tenant_id, .. }
                | VoicemailEvent::MailboxCreated { tenant_id, .. } => Some(tenant_id),
                VoicemailEvent::MessageRecorded { tenant_id, .. }
                | VoicemailEvent::MessageDeleted { tenant_id, .. }
                | VoicemailEvent::MessagePlayed { tenant_id, .. }
                | VoicemailEvent::MwiUpdated { tenant_id, .. }
                | VoicemailEvent::MailboxUpdated { tenant_id, .. }
                | VoicemailEvent::MailboxDeleted { tenant_id, .. } => tenant_id.as_ref(),
            },
            PlatformEvent::Flow(e) => match e {
                FlowEvent::Started { tenant_id, .. }
                | FlowEvent::NodeExecuted { tenant_id, .. }
                | FlowEvent::WaitingForInput { tenant_id, .. }
                | FlowEvent::Completed { tenant_id, .. }
                | FlowEvent::Failed { tenant_id, .. } => tenant_id.as_ref(),
            },
            PlatformEvent::Billing(e) => Some(e.tenant_id()),
            PlatformEvent::Cdr(
                CdrEvent::Written { tenant_id, .. } | CdrEvent::Archived { tenant_id, .. },
            ) => Some(tenant_id),
            PlatformEvent::Compliance(
                ComplianceEvent::HoldPlaced { tenant_id, .. }
                | ComplianceEvent::HoldReleased { tenant_id, .. }
                | ComplianceEvent::SubjectRequestCompleted { tenant_id, .. },
            ) => Some(tenant_id),
            PlatformEvent::Trunk(e) => e.tenant_id(),
            PlatformEvent::Speech(e) => Some(e.tenant_id()),
            PlatformEvent::Recording(
                RecordingEvent::Started { tenant_id, .. }
                | RecordingEvent::Completed { tenant_id, .. }
                | RecordingEvent::Failed { tenant_id, .. },
            ) => Some(tenant_id),
            PlatformEvent::Media(_) | PlatformEvent::Ivr(_) | PlatformEvent::Sip(_) => None,
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
            PlatformEvent::Cdr(CdrEvent::Archived { .. }) => None,
            PlatformEvent::Compliance(_) => None,
            PlatformEvent::Trunk(_) => None,
            PlatformEvent::Speech(_) => None,
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
            PlatformEvent::Cdr(
                CdrEvent::Written { timestamp, .. } | CdrEvent::Archived { timestamp, .. },
            ) => *timestamp,
            PlatformEvent::Compliance(
                ComplianceEvent::HoldPlaced { timestamp, .. }
                | ComplianceEvent::HoldReleased { timestamp, .. }
                | ComplianceEvent::SubjectRequestCompleted { timestamp, .. },
            ) => *timestamp,
            PlatformEvent::Trunk(event) => event.timestamp(),
            PlatformEvent::Speech(event) => event.timestamp(),
        }
    }
}

impl UserEvent {
    pub fn timestamp(&self) -> SystemTime {
        match self {
            UserEvent::Created { timestamp, .. }
            | UserEvent::Updated { timestamp, .. }
            | UserEvent::Deleted { timestamp, .. }
            | UserEvent::Deprovisioned { timestamp, .. }
            | UserEvent::Restored { timestamp, .. }
            | UserEvent::Renamed { timestamp, .. }
            | UserEvent::Enabled { timestamp, .. }
            | UserEvent::Disabled { timestamp, .. }
            | UserEvent::CallingChanged { timestamp, .. }
            | UserEvent::ContactChanged { timestamp, .. }
            | UserEvent::HotelingLogin { timestamp, .. }
            | UserEvent::HotelingLogout { timestamp, .. }
            | UserEvent::SignedIn { timestamp, .. }
            | UserEvent::SignInFailed { timestamp, .. } => *timestamp,
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
            | CallEvent::EmergencyCompleted { call_id, .. }
            | CallEvent::EmergencyNotificationFailed { call_id, .. }
            | CallEvent::Parked { call_id, .. }
            | CallEvent::Unparked { call_id, .. }
            | CallEvent::ParkTimedOut { call_id, .. }
            | CallEvent::QualityPoor { call_id, .. }
            | CallEvent::Held { call_id, .. }
            | CallEvent::Resumed { call_id, .. }
            | CallEvent::Transcript { call_id, .. }
            | CallEvent::PartyJoined { call_id, .. }
            | CallEvent::PartyLeft { call_id, .. } => Some(call_id),
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            CallEvent::Initiated { timestamp, .. }
            | CallEvent::Ringing { timestamp, .. }
            | CallEvent::Progress { timestamp, .. }
            | CallEvent::Failed { timestamp, .. }
            | CallEvent::Emergency { timestamp, .. }
            | CallEvent::EmergencyCompleted { timestamp, .. }
            | CallEvent::EmergencyNotificationFailed { timestamp, .. }
            | CallEvent::Parked { timestamp, .. }
            | CallEvent::Unparked { timestamp, .. }
            | CallEvent::ParkTimedOut { timestamp, .. }
            | CallEvent::QualityPoor { timestamp, .. }
            | CallEvent::Held { timestamp, .. }
            | CallEvent::Resumed { timestamp, .. }
            | CallEvent::Transcript { timestamp, .. }
            | CallEvent::PartyJoined { timestamp, .. }
            | CallEvent::PartyLeft { timestamp, .. } => *timestamp,
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
            | SystemEvent::NodeDraining { timestamp, .. }
            | SystemEvent::NodeDrained { timestamp, .. }
            | SystemEvent::NodeJoined { timestamp, .. }
            | SystemEvent::NodeLeft { timestamp, .. }
            | SystemEvent::ClusterViewChanged { timestamp, .. }
            | SystemEvent::VersionSkew { timestamp, .. }
            | SystemEvent::SiteLinkChanged { timestamp, .. }
            | SystemEvent::ServiceUp { timestamp, .. }
            | SystemEvent::ServiceDown { timestamp, .. }
            | SystemEvent::DeviceFirmwareBehind { timestamp, .. }
            | SystemEvent::DevicePushFailed { timestamp, .. } => *timestamp,
        }
    }
}

impl VoicemailEvent {
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            VoicemailEvent::MessageRecorded { call_id, .. }
            | VoicemailEvent::MessagePlayed { call_id, .. } => Some(call_id),
            VoicemailEvent::MessageTranscribed { .. }
            | VoicemailEvent::MessageDeleted { .. }
            | VoicemailEvent::MwiUpdated { .. }
            | VoicemailEvent::MailboxCreated { .. }
            | VoicemailEvent::MailboxUpdated { .. }
            | VoicemailEvent::MailboxDeleted { .. } => None,
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            VoicemailEvent::MessageRecorded { timestamp, .. }
            | VoicemailEvent::MessageTranscribed { timestamp, .. }
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
            | ConferenceEvent::ActiveSpeakerChanged { timestamp, .. }
            | ConferenceEvent::HandRaised { timestamp, .. }
            | ConferenceEvent::HandLowered { timestamp, .. }
            | ConferenceEvent::StageJoined { timestamp, .. }
            | ConferenceEvent::StageLeft { timestamp, .. }
            | ConferenceEvent::QuestionAsked { timestamp, .. }
            | ConferenceEvent::QuestionPublished { timestamp, .. }
            | ConferenceEvent::QuestionAnswered { timestamp, .. }
            | ConferenceEvent::QuestionDismissed { timestamp, .. }
            | ConferenceEvent::ChatDeleted { timestamp, .. }
            | ConferenceEvent::ChatMuted { timestamp, .. }
            | ConferenceEvent::WebinarLive { timestamp, .. }
            | ConferenceEvent::WebinarEnded { timestamp, .. }
            | ConferenceEvent::RtmpLive { timestamp, .. }
            | ConferenceEvent::RtmpFailed { timestamp, .. } => *timestamp,
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
            | PresenceEvent::PublishExpired { timestamp, .. }
            | PresenceEvent::AppearanceChanged { timestamp, .. }
            | PresenceEvent::UserChanged { timestamp, .. }
            | PresenceEvent::UserCall { timestamp, .. } => *timestamp,
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
/// a `cdr_id` they have seen. `Archived` is announced once a tenant's day
/// is in the archive, confirmed (`cdr.archived`); a day given late records
/// is written again under the same key with a higher `generation`.
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
        /// The site the record came up from, at the cloud
        /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site_id: Option<String>,
        /// When the record was written.
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
    Archived {
        tenant_id: TenantId,
        /// The UTC day, `YYYY-MM-DD`.
        day: String,
        /// Where the object is: the target (`s3://bucket` or a directory)
        /// and the key within it.
        target: String,
        object_key: String,
        /// Records in the object, and its size compressed.
        records: u64,
        bytes: u64,
        /// Hex SHA-256 of the object.
        sha256: String,
        /// How many times the day has been written.
        generation: u64,
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
        let tenant = super::TenantId::new();
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

/// Compliance events. A legal hold placed or released
/// (`compliance.hold_placed`, `compliance.hold_released`) and a subject
/// request finished (`compliance.subject_request_completed`): what an
/// eDiscovery or records system subscribes to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComplianceEvent {
    HoldPlaced {
        hold_id: String,
        tenant_id: TenantId,
        matter: String,
        /// Who placed it.
        actor: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
    HoldReleased {
        hold_id: String,
        tenant_id: TenantId,
        matter: String,
        actor: String,
        reason: String,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
    /// A subject request finished (`docs/COMPLIANCE_FOLLOWUPS.md` §6.2).
    SubjectRequestCompleted {
        request_id: String,
        tenant_id: TenantId,
        /// `export` or `erasure`.
        kind: String,
        /// `completed` or `failed`.
        status: String,
        /// Whether it finished after the day it was due.
        overdue: bool,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
}

#[cfg(test)]
mod compliance_event_tests {
    use super::*;

    #[test]
    fn a_hold_event_is_named_and_shaped_as_documented() {
        let event = PlatformEvent::Compliance(ComplianceEvent::HoldPlaced {
            hold_id: "hold-1".into(),
            tenant_id: TenantId::nil(),
            matter: "Case 42".into(),
            actor: "counsel".into(),
            timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_705_584_000),
        });
        assert_eq!(event.event_type_name(), "compliance.hold_placed");
        assert_eq!(event.tenant_id(), Some(&TenantId::nil()));
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event_type"], "compliance");
        assert_eq!(json["type"], "hold_placed");
        assert_eq!(json["hold_id"], "hold-1");
        assert_eq!(json["timestamp"], 1_705_584_000);
    }

    #[test]
    fn a_finished_subject_request_is_named_for_its_tenant() {
        let event = PlatformEvent::Compliance(ComplianceEvent::SubjectRequestCompleted {
            request_id: "sreq-1".into(),
            tenant_id: TenantId::nil(),
            kind: "export".into(),
            status: "completed".into(),
            overdue: false,
            timestamp: SystemTime::UNIX_EPOCH,
        });
        assert_eq!(
            event.event_type_name(),
            "compliance.subject_request_completed"
        );
        assert_eq!(event.tenant_id(), Some(&TenantId::nil()));
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "subject_request_completed");
        assert_eq!(json["kind"], "export");
    }
}

#[cfg(test)]
mod event_type_list {
    use super::PlatformEvent;
    use std::collections::BTreeSet;

    /// The names `event_type_name` writes, read from its source.
    fn named_in_source() -> BTreeSet<String> {
        let src = include_str!("types.rs");
        let start = src.find("pub fn event_type_name").expect("event_type_name");
        let end = start + src[start..].find("\n    }\n").expect("its end");
        src[start..end]
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|s| {
                s.contains('.')
                    && s.chars()
                        .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.')
            })
            .map(str::to_string)
            .collect()
    }

    /// A transcription names its tenant, so the tenant's webhooks get it
    /// (`docs/DEVELOPER_SURFACE.md` phase 6).
    #[test]
    fn a_transcription_reaches_its_tenant() {
        let tenant = super::TenantId::new();
        let event = PlatformEvent::Voicemail(super::VoicemailEvent::MessageTranscribed {
            tenant_id: tenant,
            mailbox_id: "mbx".into(),
            message_id: "msg".into(),
            transcription: "call me back".into(),
            confidence: Some(0.9),
            timestamp: std::time::SystemTime::now(),
        });
        assert_eq!(event.event_type_name(), "voicemail.transcribed");
        assert_eq!(event.tenant_id(), Some(&tenant));
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["transcription"], "call me back", "{json}");
        assert_eq!(json["confidence"], 0.9, "{json}");
    }

    /// A call's and a flow's events name the call's tenant, so the tenant's
    /// webhooks get them; one from an older node, without it, still reads.
    #[test]
    fn call_and_flow_events_reach_their_tenant() {
        let tenant = super::TenantId::new();
        let call_id = fcp_core::CallId::new();
        let ringing = PlatformEvent::Call(super::CallEvent::Ringing {
            call_id,
            tenant_id: Some(tenant),
            timestamp: std::time::SystemTime::now(),
            node_id: None,
        });
        assert_eq!(ringing.tenant_id(), Some(&tenant));
        let completed = PlatformEvent::Flow(super::FlowEvent::Completed {
            call_id,
            tenant_id: Some(tenant),
            flow_id: "flow-1".into(),
            outcome: "hangup".into(),
            timestamp: std::time::SystemTime::now(),
        });
        assert_eq!(completed.tenant_id(), Some(&tenant));

        let mut older = serde_json::to_value(&ringing).unwrap();
        assert!(older["tenant_id"].is_string(), "{older}");
        older.as_object_mut().unwrap().remove("tenant_id");
        let older: PlatformEvent = serde_json::from_value(older).unwrap();
        assert_eq!(older.tenant_id(), None);
        assert_eq!(older.call_id(), Some(&call_id));
    }

    /// A new mailbox names its tenant; one published before, with the
    /// tenant as a bare UUID string, still reads as that tenant.
    #[test]
    fn a_new_mailbox_reaches_its_tenant() {
        let tenant = super::TenantId::new();
        let older = serde_json::json!({
            "event_type": "voicemail",
            "type": "mailbox_created",
            "mailbox_id": "mbx",
            "user_id": "user-1",
            "extension": "1001",
            "tenant_id": tenant.as_uuid().to_string(),
            "timestamp": 1_791_220_983u64,
        });
        let event: PlatformEvent = serde_json::from_value(older).unwrap();
        assert_eq!(event.tenant_id(), Some(&tenant));
    }

    #[test]
    fn every_event_type_is_listed() {
        let listed: BTreeSet<String> = PlatformEvent::EVENT_TYPES
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            listed.len(),
            PlatformEvent::EVENT_TYPES.len(),
            "a name listed twice"
        );
        assert_eq!(
            listed,
            named_in_source(),
            "EVENT_TYPES and event_type_name disagree"
        );
    }
}
