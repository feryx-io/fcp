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
    /// How the location travels with the call: `elin` when there is a
    /// callback number for the carrier to map to an address.
    pub fn delivery(&self) -> Option<&'static str> {
        self.callback_number.as_ref().map(|_| "elin")
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
    pub route_trunks: Vec<(String, String)>,

    /// Flow ID if this call is executing a flow
    pub flow_id: Option<String>,

    /// Current node ID in the flow execution
    pub flow_current_node: Option<String>,

    /// Name of the outbound trunk (if routed via trunk).
    pub trunk_name: Option<String>,

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
            flow_id: None,
            flow_current_node: None,
            trunk_name: None,
            outbound_allowed_codecs: None,
            cancel_notify: None,
            max_duration: None,
            custom_data: None,
            parked_orbit: None,
            parker_extension: None,
            line_binding_id: None,
            no_answer_timeout: None,
            original_callee: None,
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
            flow_id: None,
            flow_current_node: None,
            trunk_name: None,
            outbound_allowed_codecs: None,
            cancel_notify: None,
            max_duration: None,
            custom_data: None,
            parked_orbit: None,
            parker_extension: None,
            line_binding_id: None,
            no_answer_timeout: None,
            original_callee: None,
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
