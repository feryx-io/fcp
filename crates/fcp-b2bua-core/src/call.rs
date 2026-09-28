//! Call state management for B2BUA.
//!
//! This module handles the state of calls in a Back-to-Back User Agent (B2BUA).
//! Each call has two legs: A-leg (caller) and B-leg (callee), which are managed
//! as separate SIP dialogs but logically connected.

use sip_core::Request;
use sip_core::Response;
use sip_dialog::Dialog;
use sip_transaction::TransactionKey;
use sip_uac::integrated::CallHandle;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Notify;
use uuid::Uuid;

/// Unique identifier for a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallId(Uuid);

impl CallId {
    /// Generate a new random call ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// The call this UUID names, for looking one up from a value that
    /// arrived as text — a transfer origin, a record id.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for CallId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CallId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// State of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// Initial state - INVITE received/sent but not yet processed
    Initiated,

    /// Waiting for routing decision
    RoutingPending,

    /// Routing approved, looking up destination
    LookingUpDestination,

    /// Sending INVITE to destination
    Inviting,

    /// Received provisional response (e.g., 180 Ringing)
    Proceeding,

    /// Call established (200 OK received/sent, ACK sent/received)
    Established,

    /// Call parked in a parking orbit (A-leg on hold, B-leg terminated)
    Parked,

    /// Call termination in progress (BYE sent/received)
    Terminating,

    /// Call ended normally
    Terminated,

    /// Call failed (4xx, 5xx, 6xx response or timeout)
    Failed,
}

impl CallState {
    /// Check if the call is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminated | Self::Failed)
    }

    /// Check if the call is active (can carry media).
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Established | Self::Parked)
    }
}

/// Direction of a call leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    /// Inbound call leg (from caller)
    Inbound,

    /// Outbound call leg (to callee)
    Outbound,
}

/// A call leg represents one side of a B2BUA call.
#[derive(Clone)]
pub struct CallLeg {
    /// Direction of this leg
    pub direction: CallDirection,

    /// SIP dialog for this leg
    pub dialog: Option<Dialog>,

    /// Outbound call handle (B-leg) for transaction control (CANCEL, etc.)
    /// Since await_final() and cancel() both take &self, we only need Arc (no Mutex or Option)
    pub outbound_handle: Option<Arc<CallHandle>>,

    /// Reliable provisional responses by RSeq (for PRACK forwarding)
    pub reliable_provisionals: Vec<Response>,

    /// Remote party URI (From/To)
    pub remote_uri: String,

    /// Remote party display name
    pub remote_display_name: Option<String>,

    /// Contact address (where to send requests)
    pub contact: Option<SocketAddr>,

    /// SDP body (if present)
    pub sdp: Option<String>,

    /// Time when this leg was created
    pub created_at: SystemTime,

    /// RFC 5626 flow token for TLS connection reuse (used for BYE via flow)
    pub flow_token: Option<String>,
}

impl CallLeg {
    /// Create a new inbound call leg from an INVITE request.
    pub fn from_invite(request: &Request, peer: SocketAddr) -> Self {
        // Extract From header
        let from = sip_parse::header(request.headers(), "From")
            .map(|s| s.as_str())
            .unwrap_or("");
        let (remote_uri, remote_display_name) = parse_name_addr(from);

        // Extract SDP body if present
        // The SDP alone or the SDP part of a multipart body.
        let sdp = crate::body::request_sdp(request);

        Self {
            direction: CallDirection::Inbound,
            dialog: None,
            outbound_handle: None,
            reliable_provisionals: Vec::new(),
            remote_uri,
            remote_display_name,
            contact: Some(peer),
            sdp,
            created_at: SystemTime::now(),
            flow_token: None,
        }
    }

    /// Create a new outbound call leg (to be filled in when sending INVITE).
    pub fn new_outbound(remote_uri: String, remote_display_name: Option<String>) -> Self {
        Self {
            direction: CallDirection::Outbound,
            dialog: None,
            outbound_handle: None,
            reliable_provisionals: Vec::new(),
            remote_uri,
            remote_display_name,
            contact: None,
            sdp: None,
            created_at: SystemTime::now(),
            flow_token: None,
        }
    }
}

/// Emergency-call context attached to a [`Call`] when the dialed number is
/// an emergency number for the tenant.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EmergencyCallInfo {
    pub dialed: String,
    pub kind: fcp_core::e911::EmergencyKind,
    pub callback_number: Option<String>,
    pub location: Option<fcp_core::e911::DispatchableLocation>,
    /// The emergency leg's outcome, set by a path that places that leg
    /// itself rather than as the call's B-leg (a flow's `Dial` node, whose
    /// call was answered before it dialed).
    #[serde(default)]
    pub leg: Option<EmergencyLeg>,
    /// How the location went out on the last attempt: the delivery methods
    /// of the trunk it was sent to (`[[trunks]] emergency_location`).
    #[serde(default)]
    pub delivered: Vec<fcp_core::e911::LocationDelivery>,
    /// The `Geolocation-Error` (or 424) a trunk answered the location with.
    #[serde(default)]
    pub geolocation_error: Option<String>,
}

/// What became of an emergency leg placed apart from the call's B-leg.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EmergencyLeg {
    /// `answered`, `busy`, `no_answer`, `failed`, `error`.
    pub outcome: String,
    pub sip_status: Option<u16>,
    pub answered_at: Option<SystemTime>,
}

impl EmergencyCallInfo {
    /// How the location travelled with the call: the last attempt's
    /// delivery methods joined with `+` (`elin+pidf_lo`); `elin` when no
    /// attempt recorded any and there is a callback number.
    pub fn delivery(&self) -> Option<String> {
        use fcp_core::e911::LocationDelivery;
        let mut methods: Vec<String> = self
            .delivered
            .iter()
            .filter(|d| **d != LocationDelivery::Elin || self.callback_number.is_some())
            .filter(|d| **d != LocationDelivery::PidfLo || self.location.is_some())
            .map(|d| d.to_string())
            .collect();
        if self.delivered.is_empty() && self.callback_number.is_some() {
            methods.push(LocationDelivery::Elin.to_string());
        }
        methods.sort();
        methods.dedup();
        (!methods.is_empty()).then(|| methods.join("+"))
    }
}

/// How a leg takes DTMF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtmfWay {
    /// RFC 2833 telephone-event, negotiated in its SDP.
    Rfc2833,
    /// SIP INFO (`application/dtmf-relay`).
    Info,
    /// In the audio, or not at all.
    Inband,
}

/// How each leg of a trunk call takes DTMF, settled when the trunk answers
/// (`docs/TENANT_TRUNKS.md` §6), so a digit one leg sends reaches the other
/// the way that leg takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DtmfCarriage {
    /// The caller's leg.
    pub a: DtmfWay,
    /// The trunk's leg.
    pub b: DtmfWay,
}

/// One outbound route and the trunk that carries it — the target as the
/// fork sees it, and the trunk itself, which says what it does to the call
/// (`docs/TENANT_TRUNKS.md` §§4, 6) and under which id its health, its
/// capacity and its record are kept.
#[derive(Debug, Clone)]
pub struct RouteTrunk {
    pub target: String,
    pub trunk: std::sync::Arc<fcp_core::Trunk>,
}

impl RouteTrunk {
    /// The trunk's name, for the log and the record.
    pub fn name(&self) -> String {
        self.trunk.name.clone()
    }

    /// The trunk's id.
    pub fn id(&self) -> fcp_core::TrunkId {
        self.trunk.trunk_id()
    }
}

/// One of a call's two legs, by slot: A is the leg the call arrived on,
/// B the leg it was placed on, until a transfer or a replacement moves
/// another dialog into a slot (`docs/ENTERPRISE_CALLING.md` §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegSide {
    A,
    B,
}

/// The appearance of a shared line a call is (`docs/ENTERPRISE_CALLING.md`
/// §3.9): the line, the index its phones show it at, and the call's leg
/// that is the line's — A for a call a phone placed from it, B for a call
/// to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallAppearance {
    pub tenant_id: fcp_core::TenantId,
    pub line_binding_id: fcp_core::LineBindingId,
    /// The line's number, E.164.
    pub number: String,
    pub index: u8,
    pub side: LegSide,
    /// What a retrieve forwarded from another node must carry
    /// (`X-FCP-Node-Handoff`).
    pub handoff: String,
}

impl LegSide {
    /// The side of a leg: `true` for A.
    pub fn of(is_a_leg: bool) -> Self {
        if is_a_leg {
            Self::A
        } else {
            Self::B
        }
    }

    /// The other leg.
    pub fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }
}

impl Call {
    /// The users this call counts against: its parties that are users,
    /// while it is up (established or parked).
    /// The state of the call's appearance, as the line's phones should
    /// show it; `None` when it is not one, or no longer occupies its index
    /// (ended, or parked away from the line).
    pub fn appearance_state(&self) -> Option<fcp_core::sca::AppearanceState> {
        use fcp_core::sca::AppearanceState as S;
        let appearance = self.appearance.as_ref()?;
        match self.state {
            CallState::Initiated
            | CallState::RoutingPending
            | CallState::LookingUpDestination
            | CallState::Inviting
            | CallState::Proceeding => Some(match appearance.side {
                LegSide::A => S::Progressing,
                LegSide::B => S::Alerting,
            }),
            // The line's phone holding its far party lights the key held
            // on every phone; the far party holding the line does not.
            CallState::Established if self.held == Some(appearance.side.other()) => Some(S::Held),
            CallState::Established => Some(S::Active),
            CallState::Parked
            | CallState::Terminating
            | CallState::Terminated
            | CallState::Failed => None,
        }
    }

    pub fn counted_users(&self) -> Vec<fcp_core::UserId> {
        if !matches!(self.state, CallState::Established | CallState::Parked) {
            return Vec::new();
        }
        let mut users: Vec<fcp_core::UserId> = [self.caller_user_id, self.callee_user_id]
            .into_iter()
            .flatten()
            .collect();
        users.dedup();
        users
    }
}

/// A B2BUA call with two legs.
#[derive(Clone)]
pub struct Call {
    /// Unique call identifier
    pub id: CallId,

    /// Tenant ID for multi-tenancy (defaults to nil for MVP single-tenant)
    pub tenant_id: fcp_core::TenantId,

    /// Current call state
    pub state: CallState,

    /// A-leg (caller side)
    pub a_leg: CallLeg,

    /// Server transaction key for the inbound INVITE (used to send final responses)
    pub a_leg_invite_tx: Option<TransactionKey>,

    /// A-leg To-tag (RFC 3261 §12.1.1: must be consistent across all responses)
    /// Generated when first provisional response (180 Ringing) is sent
    pub a_leg_to_tag: Option<String>,

    /// B-leg (callee side), may not exist yet
    pub b_leg: Option<CallLeg>,

    /// Original INVITE request from caller
    pub original_invite: Option<Request>,

    /// Media session ID (if media is anchored on server)
    pub media_session_id: Option<fcp_core::CallId>,

    /// Whether media is anchored for this call
    pub media_anchored: bool,

    /// The conference room this call was routed to, when the B-leg is a
    /// conference node: what lets a REFER from that node be executed as
    /// a migration rather than relayed to the caller
    /// (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5b).
    pub conference_room: Option<String>,

    /// Time when call was created
    pub created_at: SystemTime,

    /// Time when call was established (200 OK)
    pub established_at: Option<SystemTime>,

    /// Time when call ended
    pub ended_at: Option<SystemTime>,

    /// Reason for call failure (if failed)
    pub failure_reason: Option<String>,

    /// Why the call ended, as the handler that ended it knows; what the
    /// record's disposition, cause and hangup party are derived from.
    pub termination: Option<fcp_cdr::TerminationReason>,

    /// RTP counters captured from the media session before it was torn
    /// down; on the record when present.
    pub rtp_stats: Option<fcp_cdr::RtpStats>,

    /// The call this one came from: the call a transferred caller left, the
    /// ringing call a pickup took, the parked call a retrieve brought back
    /// (`docs/CDR_PIPELINE.md` §5).
    pub correlation_id: Option<fcp_core::CallId>,

    /// Where the call's recording is being written, when it is recorded.
    pub recording_path: Option<String>,

    /// What the parties were told about the call's recording, once a
    /// recording was to start; on the record
    /// (`docs/COMPLIANCE_FOLLOWUPS.md` §8).
    pub recording_consent: Option<fcp_core::RecordingConsent>,

    /// Why the call is to be recorded without anyone asking — a user's
    /// `record_all_calls`, a number's `recording_enabled`, the node's
    /// `enable_recording` — decided when the INVITE arrives, started once
    /// the call is answered.
    pub auto_recording: Option<fcp_core::recording::CallRecordingSource>,

    /// Each outbound route's target and trunk, primary first, so the trunk
    /// that answered can be named on the record rather than the primary.
    pub route_trunks: Vec<RouteTrunk>,

    /// How each leg takes DTMF, once a trunk has answered on an anchored
    /// call; `None` leaves digits as each leg sends them.
    pub dtmf_carriage: Option<DtmfCarriage>,

    /// The trunk a carrier's call arrived on (`docs/TENANT_TRUNKS.md` §7).
    pub arrived_on_trunk: Option<fcp_core::TrunkId>,
    /// The site whose link this call came up, at the cloud: the call is
    /// that site's tenant's own, placed by a phone the site authenticated
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.6), and not a carrier's.
    pub link_site: Option<fcp_core::SiteId>,

    /// Where a tenant's `route_to_trunk` rule confined this call: when it
    /// goes out to a carrier, only this trunk or trunk group may carry it
    /// (`docs/TENANT_TRUNKS.md` §12, phase 6b).
    pub trunk_target: Option<fcp_core::TrunkTarget>,

    /// Flow ID if this call is executing a flow
    pub flow_id: Option<String>,

    /// Current node ID in the flow execution
    pub flow_current_node: Option<String>,

    /// Name of the outbound trunk (if routed via trunk).
    pub trunk_name: Option<String>,

    /// Which trunk that is (`docs/TENANT_TRUNKS.md` §4): two tenants may
    /// both have a "carrier-a", so the record keeps the id.
    pub trunk_id: Option<fcp_core::TrunkId>,

    /// Allowed codecs from the outbound trunk's codec config (preference order).
    /// When set, B-leg SDP is filtered to these codecs.
    pub outbound_allowed_codecs: Option<Vec<String>>,

    /// Notify handle to signal cancellation to the parallel forking task.
    /// When the A-leg sends CANCEL before any B-leg answers, the CANCEL handler
    /// notifies this to wake up the forking task so it can CANCEL all B-legs immediately.
    pub cancel_notify: Option<Arc<Notify>>,

    /// Cap on the answered duration imposed by balance enforcement; the
    /// handler hangs the call up when it is reached.
    pub max_duration: Option<std::time::Duration>,

    /// Arbitrary custom data attached to this call (flows through to CDR).
    /// Used by PBX features such as CMC/FAC billing codes.
    pub custom_data: Option<serde_json::Value>,

    /// Parking orbit number (set when call is in Parked state).
    pub parked_orbit: Option<u16>,

    /// Extension of the user who parked this call.
    pub parker_extension: Option<String>,

    /// Line binding ID for the callee (used for CFNA lookup).
    pub line_binding_id: Option<fcp_core::LineBindingId>,

    /// Per-user ring-no-answer timeout from the line binding.
    /// When set, overrides the global B2BUA forking timeout.
    pub no_answer_timeout: Option<Duration>,

    /// Original callee identifier (phone number or extension) before routing resolution.
    pub original_callee: Option<String>,

    /// The user the call is for, when it is for a user: whose ring plan
    /// decided the legs and the treatments (`docs/ENTERPRISE_CALLING.md`
    /// §3.1).
    pub callee_user_id: Option<fcp_core::UserId>,

    /// What the plan decided for busy and for no answer, applied when
    /// every leg has failed. `None` on a call whose legs were not a
    /// user's plan (a device, a trunk, a forward's own legs).
    pub treatments: Option<fcp_core::Treatments>,

    /// The user who placed the call, when a user of the tenant did: with
    /// `callee_user_id`, who the call counts against while it is up
    /// (`docs/ENTERPRISE_CALLING.md` §3.4). Cleared when that party leaves
    /// the call (a transfer, a park).
    pub caller_user_id: Option<fcp_core::UserId>,

    /// Find-me steps still to ring once the legs ringing now have failed
    /// (`docs/ENTERPRISE_CALLING.md` §3.6).
    pub find_me_steps: Vec<fcp_core::FindMeStep>,

    /// The target URIs of the legs ringing now whose answer must be
    /// confirmed (press 1) before the caller is connected, and the name
    /// the prompt says the call is for.
    pub confirm_targets: Vec<String>,
    pub confirm_for: Option<String>,

    /// The leg that is on hold: its far party sent `sendonly` or
    /// `inactive`, or the node held it (a parked party).
    pub held: Option<LegSide>,

    /// The appearance of a shared line this call is, and which of its legs
    /// is the line's (`docs/ENTERPRISE_CALLING.md` §3.9).
    pub appearance: Option<CallAppearance>,

    /// The executive whose assistants this call rings first
    /// (`docs/ENTERPRISE_CALLING.md` §3.10): what an assistant's push
    /// reads, and what spares the pushed call the screening.
    pub executive_for: Option<fcp_core::UserId>,

    /// What the legs this call rings carry: an `Alert-Info` for a distinct
    /// ring, and the caller's name as shown ("… for <executive>").
    pub leg_alert_info: Option<String>,
    pub leg_display_name: Option<String>,

    /// The executive an assistant placed this call for: whose caller ID it
    /// leaves with (§3.10).
    pub on_behalf_of: Option<fcp_core::UserId>,

    /// Caller's phone number or extension.
    pub caller_number: Option<String>,

    /// Set when the dialed number is an emergency number: what was dialed,
    /// the callback number used as caller ID, and the dispatchable
    /// location resolved for the caller.
    pub emergency: Option<EmergencyCallInfo>,
}

impl fmt::Debug for CallLeg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallLeg")
            .field("direction", &self.direction)
            .field("dialog", &self.dialog)
            .field("has_outbound_handle", &self.outbound_handle.is_some())
            .field("reliable_prov_count", &self.reliable_provisionals.len())
            .field("remote_uri", &self.remote_uri)
            .field("remote_display_name", &self.remote_display_name)
            .field("contact", &self.contact)
            .field("sdp", &self.sdp)
            .field("created_at", &self.created_at)
            .finish()
    }
}

impl fmt::Debug for Call {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Call")
            .field("id", &self.id)
            .field("tenant_id", &self.tenant_id)
            .field("state", &self.state)
            .field("a_leg", &self.a_leg)
            .field("a_leg_invite_tx", &self.a_leg_invite_tx)
            .field("b_leg", &self.b_leg)
            .field("original_invite", &self.original_invite.is_some())
            .field("media_session_id", &self.media_session_id)
            .field("media_anchored", &self.media_anchored)
            .field("created_at", &self.created_at)
            .field("established_at", &self.established_at)
            .field("ended_at", &self.ended_at)
            .field("failure_reason", &self.failure_reason)
            .field("trunk_name", &self.trunk_name)
            .field("trunk_id", &self.trunk_id)
            .field("custom_data", &self.custom_data)
            .field("parked_orbit", &self.parked_orbit)
            .field("parker_extension", &self.parker_extension)
            .finish()
    }
}

impl Call {
    /// Create a new call from an inbound INVITE.
    ///
    /// # Arguments
    /// * `request` - The incoming INVITE request
    /// * `peer` - Socket address of the peer sending the INVITE
    /// * `tenant_id` - Tenant ID for multi-tenant isolation (use TenantId::nil() for default tenant)
    pub fn from_invite(request: Request, peer: SocketAddr, tenant_id: fcp_core::TenantId) -> Self {
        let a_leg = CallLeg::from_invite(&request, peer);

        Self {
            id: CallId::new(),
            tenant_id,
            state: CallState::Initiated,
            a_leg,
            a_leg_invite_tx: None,
            a_leg_to_tag: None,
            b_leg: None,
            original_invite: Some(request),
            media_session_id: None,
            media_anchored: false,
            conference_room: None,
            created_at: SystemTime::now(),
            established_at: None,
            ended_at: None,
            failure_reason: None,
            termination: None,
            rtp_stats: None,
            correlation_id: None,
            recording_path: None,
            recording_consent: None,
            auto_recording: None,
            route_trunks: Vec::new(),
            dtmf_carriage: None,
            arrived_on_trunk: None,
            link_site: None,
            trunk_target: None,
            flow_id: None,
            flow_current_node: None,
            trunk_name: None,
            trunk_id: None,
            outbound_allowed_codecs: None,
            cancel_notify: None,
            max_duration: None,
            custom_data: None,
            parked_orbit: None,
            parker_extension: None,
            line_binding_id: None,
            no_answer_timeout: None,
            original_callee: None,
            callee_user_id: None,
            treatments: None,
            held: None,
            appearance: None,
            executive_for: None,
            leg_alert_info: None,
            leg_display_name: None,
            on_behalf_of: None,
            caller_user_id: None,
            find_me_steps: Vec::new(),
            confirm_targets: Vec::new(),
            confirm_for: None,
            caller_number: None,
            emergency: None,
        }
    }

    /// Get the call duration (if established).
    pub fn duration(&self) -> Option<std::time::Duration> {
        if let Some(established_at) = self.established_at {
            if let Some(ended_at) = self.ended_at {
                ended_at.duration_since(established_at).ok()
            } else {
                SystemTime::now().duration_since(established_at).ok()
            }
        } else {
            None
        }
    }

    /// Mark the call as established.
    pub fn establish(&mut self) {
        self.state = CallState::Established;
        self.established_at = Some(SystemTime::now());
    }

    /// Mark the call as terminated.
    pub fn terminate(&mut self) {
        self.state = CallState::Terminated;
        self.ended_at = Some(SystemTime::now());
    }

    /// Park the call in a parking orbit.
    pub fn park(&mut self, orbit: u16, parker_ext: String) {
        self.state = CallState::Parked;
        self.parked_orbit = Some(orbit);
        self.parker_extension = Some(parker_ext);
    }

    /// Unpark the call (return to Established state).
    pub fn unpark(&mut self) {
        self.state = CallState::Established;
        self.parked_orbit = None;
        self.parker_extension = None;
    }

    /// Mark the call as failed with a reason.
    pub fn fail(&mut self, reason: String) {
        self.state = CallState::Failed;
        self.ended_at = Some(SystemTime::now());
        self.failure_reason = Some(reason);
    }

    /// Create a new call originated via API (click-to-call).
    ///
    /// Both legs are outbound. The A-leg targets the `from` party (whose phone
    /// rings first) and the B-leg targets the `to` party.
    pub fn new_originated(
        from_uri: String,
        from_display_name: Option<String>,
        to_uri: String,
        tenant_id: fcp_core::TenantId,
    ) -> Self {
        let a_leg = CallLeg::new_outbound(from_uri, from_display_name);
        let b_leg = CallLeg::new_outbound(to_uri, None);

        Self {
            id: CallId::new(),
            tenant_id,
            state: CallState::Initiated,
            a_leg,
            a_leg_invite_tx: None,
            a_leg_to_tag: None,
            b_leg: Some(b_leg),
            original_invite: None,
            media_session_id: None,
            media_anchored: false,
            conference_room: None,
            created_at: SystemTime::now(),
            established_at: None,
            ended_at: None,
            failure_reason: None,
            termination: None,
            rtp_stats: None,
            correlation_id: None,
            recording_path: None,
            recording_consent: None,
            auto_recording: None,
            route_trunks: Vec::new(),
            dtmf_carriage: None,
            arrived_on_trunk: None,
            link_site: None,
            trunk_target: None,
            flow_id: None,
            flow_current_node: None,
            trunk_name: None,
            trunk_id: None,
            outbound_allowed_codecs: None,
            cancel_notify: None,
            max_duration: None,
            custom_data: None,
            parked_orbit: None,
            parker_extension: None,
            line_binding_id: None,
            no_answer_timeout: None,
            original_callee: None,
            callee_user_id: None,
            treatments: None,
            held: None,
            appearance: None,
            executive_for: None,
            leg_alert_info: None,
            leg_display_name: None,
            on_behalf_of: None,
            caller_user_id: None,
            find_me_steps: Vec::new(),
            confirm_targets: Vec::new(),
            confirm_for: None,
            caller_number: None,
            emergency: None,
        }
    }

    /// Get the remote party identifier (from A-leg).
    pub fn caller(&self) -> &str {
        &self.a_leg.remote_uri
    }

    /// Get the destination party identifier (from B-leg or original INVITE).
    pub fn callee(&self) -> Option<&str> {
        if let Some(b_leg) = &self.b_leg {
            Some(&b_leg.remote_uri)
        } else if let Some(invite) = &self.original_invite {
            // Parse To header from original INVITE
            sip_parse::header(invite.headers(), "To").map(|s| s.as_str())
        } else {
            None
        }
    }
}

/// Parse a name-addr format like "Alice <sip:alice@example.com>"
/// Returns (uri, display_name)
fn parse_name_addr(header: &str) -> (String, Option<String>) {
    let header = header.trim();

    // Check if there's a display name
    if let Some(start) = header.find('<') {
        if let Some(end) = header.find('>') {
            let display_name = header[..start].trim().trim_matches('"');
            let uri = header[start + 1..end].trim();

            return (
                uri.to_string(),
                if display_name.is_empty() {
                    None
                } else {
                    Some(display_name.to_string())
                },
            );
        }
    }

    // No angle brackets, just a URI
    (header.to_string(), None)
}

#[cfg(test)]
#[allow(unused_must_use, unused_imports)]
mod tests {
    use super::*;

    #[test]
    fn test_call_id_generation() {
        let id1 = CallId::new();
        let id2 = CallId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_call_state_terminal() {
        assert!(CallState::Terminated.is_terminal());
        assert!(CallState::Failed.is_terminal());
        assert!(!CallState::Established.is_terminal());
        assert!(!CallState::Parked.is_terminal());
    }

    #[test]
    fn test_call_state_active() {
        assert!(CallState::Established.is_active());
        assert!(CallState::Parked.is_active());
        assert!(!CallState::Terminated.is_active());
        assert!(!CallState::Initiated.is_active());
    }

    #[test]
    fn test_park_and_unpark() {
        use bytes::Bytes;
        use sip_core::{Headers, Method, RequestLine};
        use smol_str::SmolStr;

        let peer: std::net::SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let mut headers = Headers::new();
        headers
            .push(
                SmolStr::new("From"),
                SmolStr::new("Alice <sip:alice@example.com>"),
            )
            .unwrap();
        headers
            .push(
                SmolStr::new("To"),
                SmolStr::new("Bob <sip:bob@example.com>"),
            )
            .unwrap();
        headers
            .push(SmolStr::new("Call-ID"), SmolStr::new("test-park"))
            .unwrap();

        let request = sip_core::Request::new(
            RequestLine::new(
                Method::Invite,
                sip_core::SipUri::parse("sip:bob@example.com").unwrap(),
            ),
            headers,
            Bytes::new(),
        )
        .unwrap();

        let mut call = Call::from_invite(request, peer, fcp_core::TenantId::nil());
        call.establish();
        assert_eq!(call.state, CallState::Established);
        assert!(call.parked_orbit.is_none());
        assert!(call.parker_extension.is_none());

        // Park
        call.park(701, "100".to_string());
        assert_eq!(call.state, CallState::Parked);
        assert_eq!(call.parked_orbit, Some(701));
        assert_eq!(call.parker_extension.as_deref(), Some("100"));

        // Unpark
        call.unpark();
        assert_eq!(call.state, CallState::Established);
        assert!(call.parked_orbit.is_none());
        assert!(call.parker_extension.is_none());
    }

    #[test]
    fn test_parse_name_addr() {
        let (uri, name) = parse_name_addr("\"Alice\" <sip:alice@example.com>");
        assert_eq!(uri, "sip:alice@example.com");
        assert_eq!(name, Some("Alice".to_string()));

        let (uri, name) = parse_name_addr("sip:bob@example.com");
        assert_eq!(uri, "sip:bob@example.com");
        assert_eq!(name, None);

        let (uri, name) = parse_name_addr("Charlie <sip:charlie@example.com>");
        assert_eq!(uri, "sip:charlie@example.com");
        assert_eq!(name, Some("Charlie".to_string()));
    }
}
