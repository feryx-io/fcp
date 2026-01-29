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
use std::time::SystemTime;
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
        matches!(self, Self::Established)
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
        let sdp = if !request.body().is_empty() {
            Some(String::from_utf8_lossy(&request.body()).to_string())
        } else {
            None
        };

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
        }
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

    /// Time when call was created
    pub created_at: SystemTime,

    /// Time when call was established (200 OK)
    pub established_at: Option<SystemTime>,

    /// Time when call ended
    pub ended_at: Option<SystemTime>,

    /// Reason for call failure (if failed)
    pub failure_reason: Option<String>,

    /// Flow ID if this call is executing a flow
    pub flow_id: Option<String>,

    /// Current node ID in the flow execution
    pub flow_current_node: Option<String>,

    /// Notify handle to signal cancellation to the parallel forking task.
    /// When the A-leg sends CANCEL before any B-leg answers, the CANCEL handler
    /// notifies this to wake up the forking task so it can CANCEL all B-legs immediately.
    pub cancel_notify: Option<Arc<Notify>>,
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
            created_at: SystemTime::now(),
            established_at: None,
            ended_at: None,
            failure_reason: None,
            flow_id: None,
            flow_current_node: None,
            cancel_notify: None,
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

    /// Mark the call as failed with a reason.
    pub fn fail(&mut self, reason: String) {
        self.state = CallState::Failed;
        self.ended_at = Some(SystemTime::now());
        self.failure_reason = Some(reason);
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
