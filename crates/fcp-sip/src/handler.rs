//! Message handler for routing inbound SIP messages.
//!
//! # SUBSCRIBE/NOTIFY Event Notification Flow (RFC 3265)
//!
//! This B2BUA implementation provides transparent proxying of event subscriptions
//! between two SIP dialogs, enabling event notifications to flow through the B2BUA
//! while maintaining proper subscription state management.
//!
//! ## Overview
//!
//! When a SIP endpoint subscribes to events (e.g., call transfer status via REFER),
//! the B2BUA forwards the SUBSCRIBE request to the opposite dialog leg and manages
//! the resulting subscription state. Subsequent NOTIFY requests for that subscription
//! are automatically forwarded to the original subscriber.
//!
//! ## Detailed Flow
//!
//! ### Initial SUBSCRIBE (A → B2BUA → B)
//!
//! 1. **Inbound SUBSCRIBE received** (from A-leg dialog)
//!    - Method: `handle_subscribe()`
//!    - Lookup call by dialog ID (Call-ID, From-tag, To-tag)
//!    - Determine which dialog leg received the SUBSCRIBE (A-leg or B-leg)
//!    - Acquire mid-dialog lock to prevent race conditions
//!
//! 2. **Forward SUBSCRIBE to opposite dialog** (to B-leg)
//!    - Clone target dialog (opposite leg)
//!    - Extract Event header (e.g., "refer", "message-summary")
//!    - Extract Expires header (subscription duration, defaults to 3600s)
//!    - Call `uac.send_subscribe(&mut target_dialog, event, expires)`
//!    - Transaction manager handles request/response transaction
//!
//! 3. **Process SUBSCRIBE response from B**
//!    - On success (200 OK):
//!      - Create Subscription object with SubscriptionId (Call-ID, tags, event)
//!      - Store subscription in SubscriptionManager (global subscription registry)
//!      - Index subscription by (DialogId, Event) for fast NOTIFY lookup
//!      - Persist updated target dialog state (incremented CSeq)
//!    - On failure (481, 403, etc.):
//!      - Cleanup dialog state if peer indicates dialog doesn't exist (481)
//!      - Return error response to original subscriber
//!
//! 4. **Return response to A**
//!    - Build response based on B's response code
//!    - Copy relevant headers (Expires, Contact)
//!    - Maintain proper Via headers for response routing
//!
//! ### Subsequent NOTIFY (B → B2BUA → A)
//!
//! 1. **Inbound NOTIFY received** (from B-leg dialog)
//!    - Method: `handle_notify()`
//!    - Lookup call by dialog ID
//!    - Determine which leg sent the NOTIFY (typically B-leg)
//!    - Acquire mid-dialog lock
//!
//! 2. **Lookup subscription**
//!    - Extract Event header from NOTIFY
//!    - Call `find_subscription(&target_dialog_id, event)`
//!    - Uses subscription_index: `DashMap<(DialogId, Event), SubscriptionId>`
//!    - Fast O(1) lookup via index
//!    - Fallback to linear scan if index miss occurs
//!
//! 3. **Forward NOTIFY to opposite dialog** (to A-leg)
//!    - Extract Subscription-State header (active/pending/terminated)
//!    - Extract body (e.g., message/sipfrag for REFER status)
//!    - Body size check (max bytes configured via settings)
//!    - Call `uac.send_notify(&subscription, state, body)`
//!
//! 4. **Handle subscription termination**
//!    - If Subscription-State: terminated
//!    - Remove subscription from SubscriptionManager
//!    - Remove from subscription_index
//!    - Automatic cleanup via `spawn_subscription_expiry_monitor()`
//!
//! 5. **Return 200 OK to B**
//!    - NOTIFY always receives 200 OK if successfully processed
//!    - Build response preserving dialog context
//!
//! ## Data Structures
//!
//! ### SubscriptionId (from sip-dialog crate)
//! ```ignore
//! pub struct SubscriptionId {
//!     call_id: SmolStr,
//!     from_tag: SmolStr,
//!     to_tag: SmolStr,
//!     event: SmolStr,  // e.g., "refer", "message-summary"
//! }
//! ```
//!
//! ### Subscription (from sip-dialog crate)
//! ```ignore
//! pub struct Subscription {
//!     id: SubscriptionId,
//!     state: SubscriptionState,  // Active, Pending, Terminated
//!     expires: Instant,
//!     contact: SipUri,
//!     route_set: Vec<SipUri>,
//!     local_cseq: u32,
//!     remote_cseq: u32,
//! }
//! ```
//!
//! ### Subscription Index (SipMessageHandler field)
//! ```ignore
//! subscription_index: DashMap<(DialogId, SmolStr), SubscriptionId>
//! ```
//! - Key: (DialogId, Event)
//! - Value: SubscriptionId
//! - Purpose: Fast NOTIFY routing lookup
//! - Populated when SUBSCRIBE succeeds (2xx response)
//! - Cleaned up when dialog terminates (BYE, 481, etc.)
//!
//! ## Common Event Packages
//!
//! - **refer** (RFC 3515): Call transfer status notifications
//!   - NOTIFY body: message/sipfrag (e.g., "SIP/2.0 100 Trying")
//!   - Use case: Transferor monitors transfer progress
//!
//! - **message-summary** (RFC 3842): Voicemail waiting indicator
//!   - NOTIFY body: application/simple-message-summary
//!   - Use case: Phone displays MWI light
//!
//! - **dialog** (RFC 4235): Dialog state (used by BLF)
//!   - NOTIFY body: application/dialog-info+xml
//!   - Use case: Busy Lamp Field (BLF) on phone
//!
//! - **presence** (RFC 3856): Presence status
//!   - NOTIFY body: application/pidf+xml
//!   - Use case: IM client presence indicators
//!
//! ## Cleanup and Memory Management
//!
//! Subscription state is cleaned up in the following scenarios:
//!
//! 1. **Dialog termination (BYE)**: `terminate_dialog_with_cleanup()`
//!    - Removes all subscriptions for dialog from subscription_index
//!    - Uses iterator filter to find all `(dialog_id, *)` entries
//!
//! 2. **Forward failure (481, transport error)**: `cleanup_after_forward_failure()`
//!    - Same cleanup as BYE
//!    - Prevents memory leaks when peer dialog no longer exists
//!
//! 3. **Subscription expiry**: Implemented via periodic expiry monitor task
//!    - Monitors `subscription.expires` timestamp every 30 seconds
//!    - Automatically removes expired subscriptions
//!    - Also calls `cleanup_terminated()` for state-based cleanup
//!
//! ## Rate Limiting
//!
//! Currently no rate limiting on SUBSCRIBE/NOTIFY (future enhancement).
//! Consider adding limits similar to INFO/UPDATE:
//! - `subscribe_rate_limit`: Max SUBSCRIBE per dialog per window
//! - `notify_rate_limit`: Max NOTIFY per subscription per window
//!
//! ## Metrics
//!
//! - `fcp_sip_mid_dialog_forward_total{method="SUBSCRIBE", result}` - Counter
//! - `fcp_sip_mid_dialog_forward_ms{method="SUBSCRIBE", result}` - Histogram
//! - `fcp_sip_notify_body_bytes{event}` - Histogram (body size distribution)
//!
//! ## Error Handling
//!
//! - Missing Event header → 400 Bad Request
//! - Subscription not found (NOTIFY) → 481 Call/Transaction Does Not Exist
//! - Body size exceeds limit → 413 Request Entity Too Large
//! - Forward failure → 481 + cleanup
//!
//! ## Future Enhancements
//!
//! 1. Subscription expiry monitoring task (similar to session timer monitor)
//! 2. Rate limiting for SUBSCRIBE/NOTIFY
//! 3. Event package validation (only allow configured packages)
//! 4. Subscription metrics (active subscriptions gauge by event type)
//! 5. Automatic NOTIFY retransmission handling
//! 6. Fork detection (multiple 2xx to SUBSCRIBE)

use anyhow::{Context as _, Result};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use sip_dialog::session_timer_manager::{SessionTimerEvent, SessionTimerManager};
use sip_sdp::parse::parse_sdp;
use smol_str::SmolStr;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::metrics::MetricsHandle;
use crate::prelude::*;
use crate::registrar::FcpRegistrar;
use crate::settings::ServerSettings;
use fcp_b2bua_core::B2buaConfig;
use fcp_core::storage::QueueConfigStore;
use fcp_core::storage::{
    CredentialStore, DeviceStore, ExtensionStore, LineBindingStore, PhoneNumberStore, TenantStore,
    TrunkStore,
};
use fcp_core::CallingStore;

/// Result of outbound route resolution.
#[derive(Debug, Clone, Default)]
pub struct OutboundRoute {
    /// Target SIP URI to INVITE.
    pub target_uri: String,
    /// The trunk this route goes out on, when a trunk chose it. What the
    /// trunk does to the call it carries — its identity, its codecs, its
    /// session timer, where it sends the INVITE — is read from here and
    /// nowhere else (`docs/TENANT_TRUNKS.md` §6).
    pub trunk: Option<Arc<fcp_core::Trunk>>,
}

impl OutboundRoute {
    /// A route to `target_uri` that no trunk chose — a registered contact.
    pub fn to(target_uri: impl Into<String>) -> Self {
        Self {
            target_uri: target_uri.into(),
            trunk: None,
        }
    }

    /// The trunk's name, for the log and the record.
    pub fn trunk_name(&self) -> Option<String> {
        self.trunk.as_ref().map(|t| t.name.clone())
    }

    /// The trunk's id, which is what its health, capacity and record are
    /// kept under: two tenants may both have a `carrier-a`
    /// (`docs/TENANT_TRUNKS.md` §4). A platform trunk's id is derived from
    /// its name.
    pub fn trunk_id(&self) -> Option<fcp_core::TrunkId> {
        self.trunk.as_ref().map(|t| t.trunk_id())
    }

    /// Allowed codecs in the trunk's preference order; `None` when it
    /// restricts none.
    pub fn allowed_codecs(&self) -> Option<Vec<String>> {
        self.trunk.as_ref().and_then(|t| {
            (!t.codec.allowed_codecs.is_empty()).then(|| t.codec.allowed_codecs.clone())
        })
    }

    /// How an emergency call's location travels on this route
    /// (`[[trunks]] emergency_location`); ELIN alone when the trunk says
    /// nothing, as a route with no trunk does.
    pub fn emergency_location(&self) -> Vec<fcp_core::e911::LocationDelivery> {
        match &self.trunk {
            Some(trunk) if !trunk.emergency_location.is_empty() => trunk.emergency_location.clone(),
            _ => vec![fcp_core::e911::LocationDelivery::Elin],
        }
    }
}

/// What a node found when it looked for a trunk to carry a call
/// (`docs/TENANT_TRUNKS.md` §5).
enum TrunkBindings {
    /// The ordered routes to try, primary first.
    Routed(Vec<sip_registrar::Binding>),
    /// Trunks are configured for this destination, but none can take the
    /// call now — unhealthy, at capacity, over CPS (503).
    Unavailable,
    /// Nothing is configured to carry it; the caller answers as it did
    /// before trunks were consulted.
    NoRoute,
}

/// Outcome of classified route resolution (the reachability stage).
///
/// Distinguishes *why* no route was produced so the caller can answer with
/// the right SIP status: 404 when nothing is configured to carry the
/// destination, 503 when routes exist but none is currently available
/// (unhealthy, at capacity, over CPS).
#[derive(Debug, Clone)]
pub enum RouteResolution {
    /// Ordered routes for failover; never empty.
    Routes(Vec<OutboundRoute>),
    /// No trunk/route is configured for this destination → 404.
    NoRoute,
    /// Routes are configured but all are currently unavailable → 503.
    Unavailable,
}

/// Provides fallback outbound routing when registrar lookup fails.
///
/// Used by the SBC to route calls to trunks when the destination
/// is not locally registered.
/// Picks the conference node a new participant should be sent to.
///
/// Implemented by binaries with cluster membership (the call manager uses
/// the conference registry and the least-loaded conference node). Returns
/// `host[:port]` of the chosen node's SIP address, or `None` to fall back
/// to the configured `conference_server_uri`.
#[async_trait::async_trait]
pub trait ConferenceNodeSelector: Send + Sync {
    /// `video` carries the meeting's video codecs when it allows video, so
    /// the selector can prefer a node with video capacity for them.
    async fn select_conference_node(
        &self,
        tenant_id: &fcp_core::TenantId,
        room_number: &str,
        video: Option<&ConferenceVideoNeed>,
    ) -> Option<String>;

    /// Whether `host` (a `host:port` this selector placed a caller on) was
    /// a conference node that has since left the cluster: killed, or its
    /// heartbeat gone. A caller still on it hears nothing and is moved
    /// (`docs/DEEP_TESTING.md` D5b). `false` for an address it never
    /// placed on, such as the configured fallback server.
    async fn conference_node_gone(&self, host: &str) -> bool {
        let _ = host;
        false
    }
}

/// What a video meeting asks of a conference node: the SDP names of the
/// codecs it allows (`H264`, `VP8`, …). Built from a `Meeting` with
/// [`ConferenceVideoNeed::of`]; `None` for an audio-only meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConferenceVideoNeed {
    pub codecs: Vec<String>,
}

impl ConferenceVideoNeed {
    pub fn of(meeting: &fcp_core::Meeting) -> Option<Self> {
        meeting.video_enabled.then(|| Self {
            codecs: meeting
                .video_codecs
                .iter()
                .map(|c| c.sdp_name().to_string())
                .collect(),
        })
    }
}

#[async_trait::async_trait]
pub trait OutboundRouteProvider: Send + Sync {
    /// The route for `destination_uri` on behalf of `tenant` — the caller's
    /// tenant, whose own trunks carry its calls before the platform's
    /// (`docs/TENANT_TRUNKS.md` §5). `None` is a call the platform itself
    /// is placing, which uses the node's own trunks.
    async fn resolve_outbound(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
    ) -> Option<OutboundRoute>;

    /// Return an ordered list of routes for failover. Default wraps single route.
    async fn resolve_outbound_ordered(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
    ) -> Vec<OutboundRoute> {
        self.resolve_outbound(tenant, destination_uri)
            .await
            .into_iter()
            .collect()
    }

    /// Classified resolution (reachability stage). The default cannot tell
    /// "nothing configured" from "nothing available" and reports `NoRoute`
    /// for any empty result; providers with a route table should override.
    /// Routes for an emergency call, best first. Providers that know about
    /// emergency-capable trunks put them ahead of the normal choice; the
    /// default is the ordinary ordered resolution so the call still goes out.
    async fn resolve_emergency_ordered(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
    ) -> Vec<OutboundRoute> {
        self.resolve_outbound_ordered(tenant, destination_uri).await
    }

    async fn resolve_outbound_classified(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
    ) -> RouteResolution {
        let routes = self.resolve_outbound_ordered(tenant, destination_uri).await;
        if routes.is_empty() {
            RouteResolution::NoRoute
        } else {
            RouteResolution::Routes(routes)
        }
    }

    /// Classified resolution for a call a tenant's `route_to_trunk` rule
    /// has confined to `target` (`docs/TENANT_TRUNKS.md` §12, phase 6b):
    /// only what the rule names may carry it, and nothing else is tried
    /// when none of that can. The default keeps the ordinary routes that
    /// match; a provider that knows its trunks should look past the
    /// ordinary choice, which may not have ranked the named trunk at all.
    async fn resolve_targeted_classified(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
        target: &fcp_core::TrunkTarget,
    ) -> RouteResolution {
        let routes: Vec<OutboundRoute> = self
            .resolve_outbound_ordered(tenant, destination_uri)
            .await
            .into_iter()
            .filter(|r| {
                r.trunk
                    .as_ref()
                    .is_some_and(|t| crate::handler::trunk_is_target(t, target))
            })
            .collect();
        if routes.is_empty() {
            RouteResolution::Unavailable
        } else {
            RouteResolution::Routes(routes)
        }
    }
}

/// Whether `trunk` is what `target` names: the trunk itself, or a member of
/// the group.
pub fn trunk_is_target(trunk: &fcp_core::Trunk, target: &fcp_core::TrunkTarget) -> bool {
    match target {
        fcp_core::TrunkTarget::Trunk(id) => &trunk.trunk_id() == id,
        fcp_core::TrunkTarget::Group(group) => trunk.trunk_group.as_deref() == Some(group.as_str()),
    }
}
use sip_transaction::ServerTransactionHandle;

/// A target contact for B-leg INVITEs with optional flow token for TLS connection reuse.
#[derive(Debug, Clone)]
struct TargetContact {
    /// The SIP URI to send the INVITE to
    uri: String,
    /// Optional flow token for RFC 5626 TLS connection reuse
    flow_token: Option<String>,
    /// What the trunk carrying this leg makes of the INVITE — its
    /// identity, its session timer (`docs/TENANT_TRUNKS.md` §6). Nothing
    /// for a registered contact, which no trunk carries.
    egress: crate::trunk_egress::TrunkEgress,
    /// Reached through the node holding its flow (`docs/PUSH.md` §2.4):
    /// that node's SIP address and the flow's token.
    relay: Option<(String, String)>,
}

/// Legs a fork gains while it runs: the apps it woke, as they come back
/// (`docs/PUSH.md` §2.2), each sent the INVITE the others were.
struct LateLegs {
    contacts: tokio::sync::mpsc::Receiver<crate::push::LateContact>,
    uac: Arc<SipUserAgentClient>,
    sdp: Option<String>,
    headers: Option<sip_core::Headers>,
}

/// The emergency routes a call has not tried yet, and what each attempt
/// carries: an emergency call goes to its routes one at a time, in order,
/// never to two carriers at once (`docs/E911.md`).
struct EmergencyFallback {
    /// The routes not tried yet, in order.
    attempts: std::collections::VecDeque<crate::emergency::EmergencyAttempt>,
    /// The route the call is on now.
    current: Option<crate::emergency::EmergencyAttempt>,
    /// Whether the current route's trunk refused the location
    /// (`Geolocation-Error` or 424).
    location_refused: bool,
    sdp: Option<String>,
    from: Option<sip_core::SipUri>,
    display_name: Option<String>,
    headers: Option<sip_core::Headers>,
}

/// The trunk routes an ordinary call has not tried yet
/// (`docs/TENANT_TRUNKS.md` §5): a call to carriers goes to one at a time,
/// the next when a carrier fails — never to two carriers at once, which
/// would ring the callee twice.
struct TrunkFallback {
    /// The routes not tried yet, in order.
    remaining: std::collections::VecDeque<TargetContact>,
    sdp: Option<String>,
    headers: Option<sip_core::Headers>,
    from: Option<sip_core::SipUri>,
    display_name: Option<String>,
}

/// Whether a final response says the carrier failed rather than the
/// callee answered: what the next carrier might do better. A busy, a
/// decline, a number that does not exist or a callee who is away is the
/// callee's answer, and another carrier would only deliver it again.
fn carrier_failed(code: u16) -> bool {
    matches!(code, 401 | 403 | 407 | 408 | 488 | 500..=599)
}

/// What became of a notification call (`docs/E911_BLOCK_2.md` §8.1).
/// Only [`AnnouncementOutcome::Answered`] told anybody anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnouncementOutcome {
    /// Someone answered and heard the notice.
    Answered,
    /// It rang out, or was cancelled before it was answered.
    NoAnswer,
    /// The extension was busy or rejected the call.
    Busy,
    /// A final response that is neither: its status.
    Failed(u16),
}

impl std::fmt::Display for AnnouncementOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnnouncementOutcome::Answered => f.write_str("answered"),
            AnnouncementOutcome::NoAnswer => f.write_str("no_answer"),
            AnnouncementOutcome::Busy => f.write_str("busy"),
            AnnouncementOutcome::Failed(status) => write!(f, "failed_{status}"),
        }
    }
}

/// Routes inbound SIP messages to the transaction manager.
#[derive(Clone)]
pub struct SipMessageHandler {
    /// Transaction manager for handling SIP transactions
    transaction_manager: Arc<TransactionManager>,

    /// Dialog manager for managing call state
    dialog_manager: Arc<DialogManager>,

    /// User Agent Server helper for request handling
    uas: Arc<sip_uas::UserAgentServer>,

    /// Media profile for SDP generation
    media_profile: Option<sip_sdp::profiles::MediaProfileBuilder>,

    /// Local IP address for SDP
    local_addr: String,

    /// RTP port for audio
    rtp_port: u16,

    /// Optional registrar for handling REGISTER requests
    registrar: Option<Arc<FcpRegistrar>>,

    /// Optional routing service for call authorization
    routing_service: Option<Arc<fcp_routing::RoutingService>>,

    /// Call manager for B2BUA functionality
    call_manager: Arc<fcp_b2bua_core::CallManager>,

    /// Billing engine for balance admission (402) and mid-call cut-off.
    /// Defaults to the call manager's engine; `None` disables enforcement.
    billing: Option<Arc<fcp_billing::BillingEngine>>,

    /// User Agent Client for making outbound calls (B-leg)
    uac: Option<Arc<SipUserAgentClient>>,

    /// Optional storage for line binding lookups
    storage: Option<Arc<fcp_core::DynStorage>>,

    /// Explicit default tenant for deployments without tenant-specific SIP domains.
    /// When unset, inbound requests must derive a tenant from SIP addressing.
    default_tenant_id: Option<fcp_core::TenantId>,
    /// Each user's calls, for call waiting (`docs/ENTERPRISE_CALLING.md` §3.4).
    user_calls: Option<Arc<crate::user_calls::UserCalls>>,
    /// Hoteling sessions, for `*88` and `*89` (§3.8).
    hoteling: Option<Arc<crate::hoteling::Hoteling>>,
    /// The publisher's keys, for waking phone apps (`docs/PUSH.md`).
    push: Option<Arc<fcp_push::PushSender>>,
    /// Calls waiting for the apps they woke (`docs/PUSH.md` §2.2).
    push_waits: Arc<crate::push::PushWaits>,
    /// Shared lines' appearances (§3.9).
    sca: Option<Arc<crate::sca::Sca>>,
    mwi_subscriptions: Option<Arc<crate::mwi_subscriptions::MwiSubscriptions>>,
    /// Calls with a supervisor (§3.11).
    supervisions: Option<Arc<crate::supervision::Supervisions>>,
    /// Retrieves sent to the node holding the call: the call here → that
    /// node's SIP URI, the one destination its B-leg is sent to, and the
    /// handoff token the INVITE carries.
    sca_handoff_targets: Arc<dashmap::DashMap<fcp_b2bua_core::CallId, (String, String)>>,
    /// Relayed legs' destinations: the phone's contact and the flow
    /// (`docs/PUSH.md` §2.4).
    flow_relay_targets: Arc<dashmap::DashMap<fcp_b2bua_core::CallId, (String, String)>>,
    /// Where each node takes SIP, by node id: what a call to a phone whose
    /// flow another node holds is relayed to (`docs/PUSH.md` §2.4).
    node_sip_address: Option<NodeSipAddress>,
    /// Per call: the bindings whose flow another node holds, by contact,
    /// with that node and the flow.
    flow_relays: Arc<dashmap::DashMap<fcp_b2bua_core::CallId, FlowRelays>>,
    /// Edges whose caller headers are believed (`[e911] trusted_edges`).
    trusted_edges: crate::emergency::TrustedEdges,
    /// What reaches this node's SIP port, and what a flood of it costs
    /// (`docs/EDGE_SECURITY.md` §2.4). Absent leaves the handler as it was
    /// before there was a guard: whatever arrives is parsed.
    edge_guard: Option<Arc<crate::edge::EdgeGuard>>,
    /// Whether the store answers (`docs/DEEP_TESTING.md` D5). While it
    /// says no, a REGISTER or a new INVITE is refused at once with a 503
    /// rather than left waiting on a database that is not there. Absent is
    /// always yes.
    store_up: Option<StoreUp>,
    store_writable: Option<StoreUp>,
    /// Whether this node tells the next hop where its caller is
    /// (`X-FCP-Client-Address`): an SBC in front of phones does.
    vouch_client_address: bool,

    /// Media manager for RTP handling
    media_manager: Option<Arc<fcp_media::MediaManager>>,

    /// Media configuration
    media_config: Option<fcp_core::config::MediaConfig>,

    /// What starts, stops and ends call recordings, through the tenant's
    /// consent step (`docs/COMPLIANCE_FOLLOWUPS.md` §8).
    call_recorder: Arc<crate::recording::CallRecorder>,

    /// Per-dialog lock to serialize mid-dialog requests (re-INVITE/UPDATE) to avoid glare
    mid_dialog_lock: Arc<dashmap::DashMap<sip_dialog::DialogId, Arc<tokio::sync::Mutex<()>>>>,

    /// Subscription manager for REFER/NOTIFY tracking
    subscription_manager: Arc<sip_dialog::SubscriptionManager>,
    /// Index of subscriptions by dialog/event for quick lookup
    subscription_index: Arc<
        dashmap::DashMap<(sip_dialog::DialogId, smol_str::SmolStr), sip_dialog::SubscriptionId>,
    >,

    /// Timestamps tracking when subscriptions were created, for expiry monitoring
    subscription_created_at: Arc<dashmap::DashMap<sip_dialog::SubscriptionId, Instant>>,

    /// Session timer manager for RFC 4028 enforcement
    session_timer_manager: Arc<SessionTimerManager>,

    /// Metrics sink
    metrics: MetricsHandle,

    /// Optional transcript directory for SIP message capture
    _transcript_dir: Option<PathBuf>,

    /// Channel for async transcript writes (background task)
    transcript_tx: Option<mpsc::Sender<TranscriptEntry>>,

    /// Settings
    settings: ServerSettings,

    /// B2BUA policy configuration (topology hiding, media anchoring, forking, transfer)
    b2bua_config: B2buaConfig,

    /// Runtime-reloadable voicemail server URI override.
    /// The config poll loop updates this when the DB config changes, so the
    /// running process picks up new values without a restart.
    voicemail_server_uri_override: Arc<std::sync::RwLock<Option<String>>>,

    /// Runtime override for conference_server_uri (hot-reloadable via config poll).
    conference_server_uri_override: Arc<std::sync::RwLock<Option<String>>>,

    /// Per-dialog rate limits
    rate_limits: Arc<dashmap::DashMap<sip_dialog::DialogId, RateWindow>>,

    /// Flow call handler for executing programmable flows
    flow_handler: Option<Arc<crate::flow_handler::FlowCallHandler>>,

    /// Conference call handler for routing INVITEs to conference rooms
    conference_handler: Option<Arc<dyn crate::conference::ConferenceHandler>>,

    /// Queue/ACD handler for routing INVITEs to ACD queues
    queue_handler: Option<Arc<dyn crate::queue::QueueHandler>>,

    /// Optional outbound route provider for trunk routing fallback.
    /// When the registrar has no binding for the destination, this provider
    /// is consulted before returning 480 Temporarily Unavailable.
    outbound_route_provider: Option<Arc<dyn OutboundRouteProvider>>,

    /// Optional PBX handler for hunt groups, parking, pickup, intercom, CMC/FAC.
    pbx_handler: Option<Arc<fcp_pbx::PbxHandler>>,

    /// Optional presence handler for SUBSCRIBE/PUBLISH/NOTIFY (RFC 3856/3903/4235).
    presence_handler: Option<Arc<dyn crate::presence::PresenceHandler>>,

    /// Optional cluster-wide parking store for cross-node park/pickup.
    cluster_parking: Option<Arc<dyn fcp_core::ClusterParkingStore>>,

    /// Tenants' parking lots as last read, briefly (`docs/PARKING.md`):
    /// every dialed number is checked against their park numbers.
    parking_lots: dashmap::DashMap<
        fcp_core::TenantId,
        (std::time::Instant, Arc<Vec<fcp_core::parking::ParkingLot>>),
    >,

    /// Cluster-aware conference placement (None = static conference_server_uri).
    conference_node_selector: Option<Arc<dyn ConferenceNodeSelector>>,

    /// This node's identifier (for cluster parking writes).
    node_id: String,

    /// Flow registry for RFC 5626 TLS connection reuse.
    /// Maps flow tokens to connection writers for NAT'd clients.
    flow_registry: Arc<crate::flow::FlowRegistry>,

    /// Channel for SIP message tracing (raw message capture to PostgreSQL).
    sip_trace_tx: Option<mpsc::Sender<fcp_core::SipTraceMessage>>,

    /// Map SIP Call-ID header values → FCP internal CallId for trace correlation.
    /// Populated at call creation (A-leg) and B-leg INVITE (B-leg).
    sip_call_id_map: Arc<dashmap::DashMap<String, String>>,
    /// Emergency calls' untried routes (see [`EmergencyFallback`]).
    emergency_fallbacks: Arc<dashmap::DashMap<fcp_b2bua_core::CallId, EmergencyFallback>>,
    /// CANCELs that arrived before their INVITE's call existed, by the
    /// INVITE's Via branch (RFC 3261 §9.2 matches a CANCEL to the INVITE's
    /// transaction, not to a call): the INVITE handler, having made the
    /// call, finds its branch here and answers 487 instead of ringing
    /// anyone (`docs/DEEP_TESTING.md` D2). Kept no longer than an INVITE
    /// transaction may last.
    early_cancels: Arc<dashmap::DashMap<String, std::time::Instant>>,
    /// Ordinary calls' trunk routes not tried yet (`docs/TENANT_TRUNKS.md`
    /// §5).
    trunk_fallbacks: Arc<dashmap::DashMap<fcp_b2bua_core::CallId, TrunkFallback>>,
    /// The SRTP this node offered on a call's B-leg, kept until the answer
    /// names the key the far end sends with (`docs/TENANT_TRUNKS.md` §6).
    b_leg_srtp: Arc<dashmap::DashMap<fcp_b2bua_core::CallId, fcp_media::SrtpOffer>>,
    /// The key each call's B-leg was keyed with, once answered: what this
    /// node answers that leg's later offers with.
    b_leg_keys: Arc<dashmap::DashMap<fcp_b2bua_core::CallId, fcp_media::SrtpOffer>>,
    /// The header manipulation rules a trunk may name
    /// (`docs/TENANT_TRUNKS.md` §6); `None` on a node with none configured.
    header_rules: Option<Arc<dyn crate::trunk_egress::TrunkHeaderRules>>,
    /// What signs the calls a trunk asks to have signed (RFC 8224);
    /// `None` on a node with no signing key.
    call_signer: Option<Arc<dyn crate::trunk_egress::TrunkCallSigner>>,
    /// The trunks this node takes carriers' calls on, which decide whose
    /// call each is (`docs/TENANT_TRUNKS.md` §7); `None` on a node that
    /// admits by its tenants' stored trunks as before.
    inbound_trunks: Option<Arc<dyn crate::trunk_ingress::InboundTrunks>>,
    /// The sites bound at this cloud by their links' registrations
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
    link_peers: Arc<crate::link_peers::LinkPeers>,
    /// Every tenant's own routing rules (`docs/TENANT_TRUNKS.md` §12,
    /// phase 6b); `None` on a node that decides by its YAML policy alone.
    tenant_rules: Option<Arc<fcp_policy::TenantRuleSet>>,
    /// What each trunk carries on this node, against its limits (§7);
    /// `None` on a node that does not count.
    trunk_calls: Option<Arc<dyn crate::trunk_ingress::TrunkCallCounter>>,
    /// What this node will carry, and what it refuses
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5); `None` on a node that
    /// keeps no limits of its own.
    admission: Option<Arc<dyn crate::admission::NodeAdmission>>,
    /// Conference calls whose B-leg is being moved to another conference
    /// node (phase 5b), so a second REFER while one is in flight is refused.
    conference_migrations: Arc<dashmap::DashMap<fcp_b2bua_core::CallId, ()>>,
    /// 2xx responses a flow sent that the caller has not ACKed; the
    /// flow's BYE waits for the ACK (RFC 3261 §15).
    ack_waits: Arc<crate::ack_wait::AckWaits>,
    /// Calls being redirected into a flow by the API, until their
    /// dial-back arrives (`docs/DEVELOPER_SURFACE.md` phase 8).
    redirect_tickets: Arc<crate::call_control::RedirectTickets>,
    /// This handler as it serves requests, set by the first one. Built in
    /// `new` and finished by its builders, it exists whole only from then:
    /// the session timer listener, started in `new`, ends an expired call
    /// through it, both legs and the media as any hangup does. Never
    /// dropped, like the handler itself.
    serving: Arc<std::sync::OnceLock<SipMessageHandler>>,
}

#[derive(Clone)]
pub struct RateWindow {
    pub window_start: Instant,
    pub counts: HashMap<&'static str, u32>,
}

/// Entry for async transcript logging
struct TranscriptEntry {
    call_id: String,
    timestamp: std::time::SystemTime,
    direction: String,
    method_or_status: String,
    dialog_id_str: String,
    cseq: String,
    peer: Option<String>,
    transport: Option<String>,
    record_route: String,
    route: String,
    body_hash: Option<String>,
}

/// Helper function to add response headers for dialog-establishing responses
/// (Used by flow Answer callback which doesn't have access to self)
fn add_response_headers(
    request: &sip_core::Request,
    response: &mut sip_core::Response,
    uas: &sip_uas::UserAgentServer,
) {
    use smol_str::SmolStr;
    use uuid::Uuid;

    const ALLOW: &str =
        "INVITE, ACK, CANCEL, BYE, UPDATE, PRACK, INFO, REFER, SUBSCRIBE, NOTIFY, OPTIONS";
    const SUPPORTED: &str = "100rel, timer";

    // Ensure To-tag present for dialog-establishing responses
    let status_code = response.start_line().code();
    if status_code >= 101 {
        if let Some(to) = response.headers().get("To") {
            if !to.contains(";tag=") {
                let tag = Uuid::new_v4().simple().to_string();
                let new_to = SmolStr::new(format!("{};tag={}", to, tag));
                response.headers_mut().remove("To");
                let _ = response.headers_mut().push(SmolStr::new("To"), new_to);
            }
        }
    }

    // INVITE-related responses should carry Contact/Record-Route
    if request.method() == &sip_core::Method::Invite {
        if response.headers().get("Contact").is_none() {
            let _ = response.headers_mut().push(
                SmolStr::new("Contact"),
                SmolStr::new(format!("<{}>", uas.contact_uri.as_str())),
            );
        }
        if response.headers().get("Record-Route").is_none() {
            let _ = response.headers_mut().push(
                SmolStr::new("Record-Route"),
                SmolStr::new(format!("<{};lr>", uas.contact_uri.as_str())),
            );
        }
    }

    // Capability headers
    if response.headers().get("Allow").is_none() {
        let _ = response
            .headers_mut()
            .push(SmolStr::new("Allow"), SmolStr::new(ALLOW));
    }
    if response.headers().get("Supported").is_none() {
        let _ = response
            .headers_mut()
            .push(SmolStr::new("Supported"), SmolStr::new(SUPPORTED));
    }

    // Ensure Content-Length matches body
    let body_len = response.body().len();
    response.headers_mut().remove("Content-Length");
    let _ = response.headers_mut().push(
        SmolStr::new("Content-Length"),
        SmolStr::new(body_len.to_string()),
    );
}

impl SipMessageHandler {
    /// Create a new message handler.
    ///
    /// # Arguments
    /// * `transaction_manager` - Transaction manager for SIP transactions
    /// * `dialog_manager` - Dialog manager for call state
    /// * `local_uri` - Local SIP URI for Contact/From headers
    /// * `media_profile` - Optional SDP media profile for call setup
    /// * `local_addr` - Local IP address for SDP
    /// * `rtp_port` - RTP port for audio (default: 10000)
    /// * `registrar` - Optional registrar for REGISTER request handling
    /// * `routing_service` - Optional routing service for call authorization
    /// * `call_manager` - Call manager for B2BUA functionality
    /// * `uac` - Optional User Agent Client for outbound calls (B-leg forwarding)
    /// * `storage` - Optional storage for line binding lookups
    /// * `media_manager` - Optional media manager for RTP handling
    /// * `media_config` - Optional media configuration
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        dialog_manager: Arc<DialogManager>,
        local_uri: sip_core::SipUri,
        media_profile: Option<sip_sdp::profiles::MediaProfileBuilder>,
        local_addr: String,
        rtp_port: u16,
        registrar: Option<Arc<FcpRegistrar>>,
        routing_service: Option<Arc<fcp_routing::RoutingService>>,
        call_manager: Arc<fcp_b2bua_core::CallManager>,
        uac: Option<Arc<SipUserAgentClient>>,
        storage: Option<Arc<fcp_core::DynStorage>>,
        media_manager: Option<Arc<fcp_media::MediaManager>>,
        media_config: Option<fcp_core::config::MediaConfig>,
        flow_handler: Option<Arc<crate::flow_handler::FlowCallHandler>>,
        b2bua_config: B2buaConfig,
        outbound_route_provider: Option<Arc<dyn OutboundRouteProvider>>,
        pbx_handler: Option<Arc<fcp_pbx::PbxHandler>>,
        conference_handler: Option<Arc<dyn crate::conference::ConferenceHandler>>,
        queue_handler: Option<Arc<dyn crate::queue::QueueHandler>>,
        presence_handler: Option<Arc<dyn crate::presence::PresenceHandler>>,
        cluster_parking: Option<Arc<dyn fcp_core::ClusterParkingStore>>,
        node_id: String,
    ) -> anyhow::Result<Self> {
        let settings = ServerSettings::from_env();
        Self::new_with_settings(
            transaction_manager,
            dialog_manager,
            local_uri,
            media_profile,
            local_addr,
            rtp_port,
            registrar,
            routing_service,
            call_manager,
            uac,
            storage,
            media_manager,
            media_config,
            flow_handler,
            settings,
            b2bua_config,
            outbound_route_provider,
            pbx_handler,
            conference_handler,
            queue_handler,
            presence_handler,
            cluster_parking,
            node_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_settings(
        transaction_manager: Arc<TransactionManager>,
        dialog_manager: Arc<DialogManager>,
        local_uri: sip_core::SipUri,
        media_profile: Option<sip_sdp::profiles::MediaProfileBuilder>,
        local_addr: String,
        rtp_port: u16,
        registrar: Option<Arc<FcpRegistrar>>,
        routing_service: Option<Arc<fcp_routing::RoutingService>>,
        call_manager: Arc<fcp_b2bua_core::CallManager>,
        uac: Option<Arc<SipUserAgentClient>>,
        storage: Option<Arc<fcp_core::DynStorage>>,
        media_manager: Option<Arc<fcp_media::MediaManager>>,
        media_config: Option<fcp_core::config::MediaConfig>,
        flow_handler: Option<Arc<crate::flow_handler::FlowCallHandler>>,
        settings: ServerSettings,
        b2bua_config: B2buaConfig,
        outbound_route_provider: Option<Arc<dyn OutboundRouteProvider>>,
        pbx_handler: Option<Arc<fcp_pbx::PbxHandler>>,
        conference_handler: Option<Arc<dyn crate::conference::ConferenceHandler>>,
        queue_handler: Option<Arc<dyn crate::queue::QueueHandler>>,
        presence_handler: Option<Arc<dyn crate::presence::PresenceHandler>>,
        cluster_parking: Option<Arc<dyn fcp_core::ClusterParkingStore>>,
        node_id: String,
    ) -> anyhow::Result<Self> {
        // Validate settings early to catch configuration errors at startup
        settings
            .validate()
            .map_err(|e| anyhow::anyhow!("Invalid ServerSettings: {}", e))?;

        // A call that ends never answered lets its media session go
        // (`docs/DEEP_TESTING.md` D8): one refused before anything rang
        // held its session and two ports for as long as the node ran.
        if let Some(media) = media_manager.as_ref() {
            let media = Arc::downgrade(media);
            call_manager.set_on_unanswered_media(Arc::new(move |session| {
                let Some(media) = media.upgrade() else {
                    return;
                };
                tokio::spawn(async move {
                    // Already ended by its own path: nothing to do.
                    if media.terminate_session(&session).await.is_ok() {
                        debug!(media_session_id = %session, "Ended the media session of an unanswered call");
                    }
                });
            }));
        }

        // Create User Agent Server helper with shared dialog manager
        let mut uas = sip_uas::UserAgentServer::new(local_uri.clone(), local_uri);
        uas.dialog_manager = Arc::clone(&dialog_manager);

        let subscription_manager = Arc::new(sip_dialog::SubscriptionManager::new());
        let session_timer_manager = Arc::new(SessionTimerManager::new());

        // Initialize transcript background writer if directory configured
        let transcript_tx = if let Some(ref dir) = settings.transcript_dir {
            // Use bounded channel for backpressure (max 1000 pending entries)
            let (tx, mut rx) = mpsc::channel::<TranscriptEntry>(1000);
            let transcript_dir = dir.clone();
            let max_bytes = settings.transcript_max_bytes;

            tokio::spawn(async move {
                while let Some(entry) = rx.recv().await {
                    let filename = format!("{}.log", entry.call_id.replace(':', "_"));
                    let path = transcript_dir.join(&filename);

                    // Check file size and rotate if needed (async)
                    if let Some(max) = max_bytes {
                        match tokio::fs::metadata(&path).await {
                            Ok(meta) if meta.len() > max => {
                                let ts = chrono::Utc::now().timestamp();
                                let new_path = transcript_dir.join(format!(
                                    "{}.{}.log",
                                    entry.call_id.replace(':', "_"),
                                    ts
                                ));
                                if let Err(e) = tokio::fs::rename(&path, new_path).await {
                                    tracing::warn!(
                                        call_id = %entry.call_id,
                                        error = %e,
                                        "Failed to rotate transcript file"
                                    );
                                }
                            }
                            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                                tracing::warn!(
                                    call_id = %entry.call_id,
                                    error = %e,
                                    "Failed to check transcript file size"
                                );
                            }
                            _ => {}
                        }
                    }

                    // Open and write to file (async)
                    match tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .await
                    {
                        Ok(mut file) => {
                            use tokio::io::AsyncWriteExt;

                            let entry_ts =
                                chrono::DateTime::<chrono::Utc>::from(entry.timestamp).to_rfc3339();
                            let line = format!(
                                "{} {} {} dialog={} cseq={} peer={} transport={} rr={} route={} body_hash={}\n",
                                entry_ts,
                                entry.direction,
                                entry.method_or_status,
                                entry.dialog_id_str,
                                entry.cseq,
                                entry.peer.as_deref().unwrap_or(""),
                                entry.transport.as_deref().unwrap_or(""),
                                entry.record_route,
                                entry.route,
                                entry.body_hash.as_deref().unwrap_or("")
                            );

                            if let Err(e) = file.write_all(line.as_bytes()).await {
                                tracing::error!(
                                    call_id = %entry.call_id,
                                    error = %e,
                                    "Failed to write transcript entry"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                call_id = %entry.call_id,
                                path = ?path,
                                error = %e,
                                "Failed to open transcript file"
                            );
                        }
                    }
                }

                tracing::info!("Transcript writer task exiting");
            });

            Some(tx)
        } else {
            None
        };

        let call_recorder = Arc::new(crate::recording::CallRecorder::new(
            Arc::clone(&call_manager),
            media_manager.clone(),
            storage.clone(),
        ));
        let this = Self {
            transaction_manager,
            dialog_manager,
            uas: Arc::new(uas),
            media_profile,
            local_addr,
            rtp_port,
            registrar,
            routing_service,
            billing: call_manager.billing().cloned(),
            call_manager,
            uac,
            storage,
            default_tenant_id: None,
            user_calls: None,
            hoteling: None,
            push: None,
            push_waits: Arc::new(crate::push::PushWaits::default()),
            sca: None,
            mwi_subscriptions: None,
            supervisions: None,
            sca_handoff_targets: Arc::new(dashmap::DashMap::new()),
            flow_relay_targets: Arc::new(dashmap::DashMap::new()),
            node_sip_address: None,
            flow_relays: Arc::new(dashmap::DashMap::new()),
            trusted_edges: Default::default(),
            edge_guard: None,
            store_up: None,
            store_writable: None,
            vouch_client_address: false,
            media_manager,
            media_config,
            call_recorder,
            mid_dialog_lock: Arc::new(dashmap::DashMap::new()),
            subscription_manager,
            subscription_index: Arc::new(dashmap::DashMap::new()),
            subscription_created_at: Arc::new(dashmap::DashMap::new()),
            session_timer_manager,
            metrics: settings.metrics.clone(),
            _transcript_dir: settings.transcript_dir.clone(),
            transcript_tx,
            voicemail_server_uri_override: Arc::new(std::sync::RwLock::new(
                b2bua_config.voicemail_server_uri.clone(),
            )),
            conference_server_uri_override: Arc::new(std::sync::RwLock::new(
                b2bua_config.conference_server_uri.clone(),
            )),
            settings,
            b2bua_config,
            rate_limits: Arc::new(dashmap::DashMap::new()),
            flow_handler,
            conference_handler,
            queue_handler,
            outbound_route_provider,
            pbx_handler,
            presence_handler,
            cluster_parking,
            parking_lots: dashmap::DashMap::new(),
            conference_node_selector: None,
            node_id,
            flow_registry: Arc::new(crate::flow::FlowRegistry::new()),
            sip_trace_tx: None,
            sip_call_id_map: Arc::new(dashmap::DashMap::new()),
            emergency_fallbacks: Arc::new(dashmap::DashMap::new()),
            early_cancels: Arc::new(dashmap::DashMap::new()),
            trunk_fallbacks: Arc::new(dashmap::DashMap::new()),
            b_leg_srtp: Arc::new(dashmap::DashMap::new()),
            b_leg_keys: Arc::new(dashmap::DashMap::new()),
            header_rules: None,
            inbound_trunks: None,
            link_peers: Arc::new(crate::link_peers::LinkPeers::new()),
            tenant_rules: None,
            trunk_calls: None,
            admission: None,
            call_signer: None,
            conference_migrations: Arc::new(dashmap::DashMap::new()),
            ack_waits: Arc::new(crate::ack_wait::AckWaits::new()),
            redirect_tickets: Arc::new(crate::call_control::RedirectTickets::default()),
            serving: Arc::new(std::sync::OnceLock::new()),
        };

        // Start session timer event loop
        this.spawn_session_timer_listener();

        // Start session timer health monitor
        this.spawn_session_timer_monitor();

        // Start subscription expiry monitor
        this.spawn_subscription_expiry_monitor();

        Ok(this)
    }

    /// Set the explicit default tenant used when SIP addressing carries no tenant domain.
    /// Route conference calls through a cluster-aware node selector.
    pub fn with_conference_node_selector(
        mut self,
        selector: Arc<dyn ConferenceNodeSelector>,
    ) -> Self {
        self.conference_node_selector = Some(selector);
        self
    }

    /// `host[:port]` of the conference server that should receive a new
    /// participant of `room_number`: the cluster selector first, then the
    /// hot-reloadable override, then the static config.
    async fn conference_target_host(
        &self,
        tenant_id: &fcp_core::TenantId,
        room_number: &str,
        video: Option<&ConferenceVideoNeed>,
    ) -> Option<String> {
        if let Some(selector) = &self.conference_node_selector {
            if let Some(host) = selector
                .select_conference_node(tenant_id, room_number, video)
                .await
            {
                return Some(host);
            }
        }
        let configured = {
            let guard = self.conference_server_uri_override.read().unwrap();
            guard
                .clone()
                .or_else(|| self.b2bua_config.conference_server_uri.clone())
        }?;
        let host = configured
            .strip_prefix("sip:")
            .or_else(|| configured.strip_prefix("sips:"))
            .unwrap_or(&configured)
            .split('@')
            .next_back()
            .unwrap_or(&configured)
            .to_string();
        Some(host)
    }

    /// Share a call recorder with the flow handler and the API, so a call is
    /// recorded once whoever asks.
    pub fn with_call_recorder(mut self, recorder: Arc<crate::recording::CallRecorder>) -> Self {
        self.call_recorder = recorder;
        self
    }

    /// The recorder every call recording starts through.
    pub fn call_recorder(&self) -> &Arc<crate::recording::CallRecorder> {
        &self.call_recorder
    }

    /// Believe the caller a trusted edge names for its INVITEs
    /// (`docs/E911_BLOCK_2.md` §4, [`crate::emergency`]).
    /// The engine that runs the header manipulation rules a trunk names
    /// (`docs/TENANT_TRUNKS.md` §6).
    pub fn with_header_rules(
        mut self,
        rules: Option<Arc<dyn crate::trunk_egress::TrunkHeaderRules>>,
    ) -> Self {
        self.header_rules = rules;
        self
    }

    /// The trunks this node takes carriers' calls on
    /// (`docs/TENANT_TRUNKS.md` §7).
    pub fn with_inbound_trunks(
        mut self,
        trunks: Option<Arc<dyn crate::trunk_ingress::InboundTrunks>>,
    ) -> Self {
        self.inbound_trunks = trunks;
        self
    }

    /// The sites bound at this node, and the flows to reach them on
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.6), for a status page.
    pub fn link_peers(
        &self,
    ) -> (
        Arc<crate::link_peers::LinkPeers>,
        Arc<crate::flow::FlowRegistry>,
    ) {
        (
            Arc::clone(&self.link_peers),
            Arc::clone(&self.flow_registry),
        )
    }

    /// Every tenant's own routing rules, decided on this node
    /// (`docs/TENANT_TRUNKS.md` §12, phase 6b).
    pub fn with_tenant_rules(mut self, rules: Option<Arc<fcp_policy::TenantRuleSet>>) -> Self {
        self.tenant_rules = rules;
        self
    }

    /// What counts the calls each trunk carries against its limits
    /// (`docs/TENANT_TRUNKS.md` §7).
    pub fn with_trunk_calls(
        mut self,
        calls: Option<Arc<dyn crate::trunk_ingress::TrunkCallCounter>>,
    ) -> Self {
        self.trunk_calls = calls;
        self
    }

    /// What this node will carry: its own limits and its tenants'
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5).
    pub fn with_admission(
        mut self,
        admission: Option<Arc<dyn crate::admission::NodeAdmission>>,
    ) -> Self {
        self.admission = admission;
        self
    }

    /// What signs the calls a trunk asks to have signed
    /// (`docs/TENANT_TRUNKS.md` §6).
    pub fn with_call_signer(
        mut self,
        signer: Option<Arc<dyn crate::trunk_egress::TrunkCallSigner>>,
    ) -> Self {
        self.call_signer = signer;
        self
    }

    /// The defences at this node's SIP port (`docs/EDGE_SECURITY.md` §2.4).
    pub fn with_edge_guard(mut self, guard: Arc<crate::edge::EdgeGuard>) -> Self {
        self.edge_guard = Some(guard);
        self
    }

    /// What says whether the store answers (`docs/DEEP_TESTING.md` D5).
    pub fn with_store_gate(mut self, store_up: StoreUp) -> Self {
        self.store_up = Some(store_up);
        self
    }

    /// What says whether the store takes writes: false while the primary's
    /// address leads to a standby (`docs/DEEP_TESTING.md` D5c).
    pub fn with_store_writable(mut self, writable: StoreUp) -> Self {
        self.store_writable = Some(writable);
        self
    }

    /// Whether work that needs the store may start.
    fn store_answers(&self) -> bool {
        self.store_up.as_ref().is_none_or(|up| up())
    }

    /// Whether work that writes to the store may start.
    fn store_writes(&self) -> bool {
        self.store_answers() && self.store_writable.as_ref().is_none_or(|w| w())
    }

    /// The 503 for a request the store is needed for while it is away:
    /// the phone tries again, here or at another node, in a few seconds.
    fn store_unavailable(&self, request: &sip_core::Request) -> sip_core::Response {
        self.metrics.inc_counter(
            "fcp_sip_store_refused_total",
            &[("method", request.method().as_str())],
        );
        let reason = if self.store_answers() {
            "Service Unavailable - store read-only"
        } else {
            "Service Unavailable - store unreachable"
        };
        let mut response = self.create_simple_response(request, 503, reason);
        let _ = response.headers_mut().push(
            SmolStr::new("Retry-After"),
            SmolStr::new(STORE_RETRY_AFTER_SECS.to_string()),
        );
        response
    }

    pub fn with_trusted_edges(mut self, edges: crate::emergency::TrustedEdges) -> Self {
        self.trusted_edges = edges;
        self
    }

    /// Tell the next hop where each caller is (`X-FCP-Client-Address`),
    /// for a node that stands in front of phones (`docs/E911_BLOCK_2.md`
    /// §6.2). The next hop believes it only from its trusted edges.
    pub fn with_client_address_vouching(mut self, vouch: bool) -> Self {
        self.vouch_client_address = vouch;
        self
    }

    /// Take supervisors onto calls, and hear them hang up (§3.11).
    pub fn with_supervisions(
        mut self,
        supervisions: Arc<crate::supervision::Supervisions>,
    ) -> Self {
        self.supervisions = Some(supervisions);
        self
    }

    /// Keep shared lines' appearances (`call-info`, `line-seize`,
    /// `docs/ENTERPRISE_CALLING.md` §3.9).
    pub fn with_sca(mut self, sca: Arc<crate::sca::Sca>) -> Self {
        self.sca = Some(sca);
        self
    }

    /// Answer `message-summary` subscriptions (RFC 3842).
    pub fn with_mwi_subscriptions(
        mut self,
        subs: Arc<crate::mwi_subscriptions::MwiSubscriptions>,
    ) -> Self {
        self.mwi_subscriptions = Some(subs);
        self
    }

    /// Tell a device's phone to fetch its configuration again
    /// (`docs/PROVISIONING.md` §8): how many of its contacts were sent the
    /// NOTIFY, or `None` when this node cannot send one.
    ///
    /// `flowless`: also send to contacts with no flow (one node of the
    /// cluster does); contacts on a flow this node holds are sent to down
    /// it, the rest left to the nodes holding them.
    pub async fn resync_device(&self, device: &fcp_core::Device, flowless: bool) -> Option<usize> {
        let (Some(uac), Some(registrar), Some(storage)) =
            (&self.uac, &self.registrar, &self.storage)
        else {
            return None;
        };
        Some(
            crate::device_sync::resync_device(
                uac,
                registrar,
                storage,
                device,
                Some(&self.flow_registry),
                flowless,
            )
            .await,
        )
    }

    /// Where each node takes SIP, by node id (the cluster's members): what
    /// a call to a phone whose flow another node holds is relayed to
    /// (`docs/PUSH.md` §2.4).
    pub fn with_node_sip_address(mut self, address: NodeSipAddress) -> Self {
        self.node_sip_address = Some(address);
        self
    }

    /// Wake phone apps with these keys (`[push]`, `docs/PUSH.md`).
    pub fn with_push(mut self, push: Option<Arc<fcp_push::PushSender>>) -> Self {
        self.push = push;
        self
    }

    /// How each of `contacts` is reached (`docs/PUSH.md` §2.2, §2.4):
    /// rung (a live flow here, no flow, or no way to wake it); rung through
    /// the node holding its flow, recorded for the call; or woken by push —
    /// at once, or (its flow on another node) only if that node says the
    /// flow is gone. The rung, and the bindings to wake with whether each
    /// is deferred.
    async fn classify_bindings(
        &self,
        call_id: fcp_b2bua_core::CallId,
        tenant_id: &fcp_core::TenantId,
        contacts: Vec<sip_registrar::Binding>,
    ) -> (
        Vec<sip_registrar::Binding>,
        Vec<(fcp_core::storage::Registration, bool)>,
    ) {
        let Some(registrar) = &self.registrar else {
            return (contacts, Vec::new());
        };
        let mut rows: std::collections::HashMap<String, Vec<fcp_core::storage::Registration>> =
            std::collections::HashMap::new();
        let mut ring = Vec::new();
        let mut wake = Vec::new();
        let mut relays = std::collections::HashMap::new();
        for binding in contacts {
            let flow_here = binding
                .flow_token()
                .is_some_and(|t| self.flow_registry.is_valid(t));
            // The stored row is wanted only for a binding that may be woken
            // or reached through another node.
            let elsewhere =
                !flow_here && binding.flow_token().is_some() && self.node_sip_address.is_some();
            if flow_here || (self.push.is_none() && !elsewhere) {
                ring.push(binding);
                continue;
            }
            let aor = binding.aor().to_string();
            if !rows.contains_key(&aor) {
                let found = registrar
                    .get_registration_rows_in_tenant(tenant_id, &aor)
                    .await
                    .unwrap_or_default();
                rows.insert(aor.clone(), found);
            }
            let row = rows[&aor]
                .iter()
                .find(|r| crate::nat::reachable_contact(r) == binding.contact())
                .cloned();
            let wakeable = self.push.is_some() && row.as_ref().is_some_and(|r| r.push.is_some());
            // Its flow on another node this node can reach.
            let relay = match (&row, binding.flow_token(), &self.node_sip_address) {
                (Some(row), Some(token), Some(address)) => row
                    .flow_node
                    .as_deref()
                    .filter(|node| *node != self.node_id)
                    .and_then(|node| address(node))
                    .map(|at| (at.to_string(), token.to_string())),
                _ => None,
            };
            match (relay, row) {
                (Some(relay), row) => {
                    relays.insert(binding.contact().to_string(), relay);
                    if let (true, Some(row)) = (wakeable, row) {
                        wake.push((row, true));
                    }
                    ring.push(binding);
                }
                (None, Some(row)) if wakeable => wake.push((row, false)),
                _ => ring.push(binding),
            }
        }
        if !relays.is_empty() {
            self.flow_relays.insert(call_id, relays);
        }
        (ring, wake)
    }

    /// What a call's push says: the call, and who calls (the phone's call
    /// screen shows it before the app has done anything).
    fn push_message(
        request: &sip_core::Request,
        call_id: fcp_b2bua_core::CallId,
        tenant_id: &fcp_core::TenantId,
    ) -> fcp_push::PushMessage {
        let from = request.headers().get("From").unwrap_or("");
        let name = from
            .split('<')
            .next()
            .map(|n| n.trim().trim_matches('"').trim().to_string())
            .filter(|n| !n.is_empty());
        fcp_push::PushMessage::new(
            fcp_push::PushKind::Call,
            call_id.to_string(),
            tenant_id.to_string(),
        )
        .caller(caller_number(request), name)
    }

    /// Push to the bindings a call is to push now: how many the providers
    /// took. One not taken will not come back.
    async fn push_waiting(&self, call_id: fcp_b2bua_core::CallId) -> usize {
        let (Some(sender), Some(registrar)) = (&self.push, &self.registrar) else {
            return 0;
        };
        let Some((targets, message)) = self.push_waits.take_to_push(&call_id) else {
            return 0;
        };
        if targets.is_empty() {
            return 0;
        }
        let rows: Vec<_> = targets.iter().map(|(_, row)| row.clone()).collect();
        let results = crate::push::push_bindings(sender, registrar, &rows, &message, None).await;
        let sent = results.iter().filter(|r| r.outcome == "sent").count();
        let failed: Vec<usize> = results
            .iter()
            .zip(&targets)
            .filter(|(r, _)| r.outcome != "sent")
            .map(|(_, (i, _))| *i)
            .collect();
        self.push_waits.abandon(&call_id, &failed);
        info!(
            call_id = %call_id,
            pushed = rows.len(),
            sent,
            "Woke the callee's sleeping apps"
        );
        sent
    }

    /// A REGISTER somewhere in the cluster (`registration.registered`): an
    /// app a call woke has come back, and its new binding goes to the call
    /// (`docs/PUSH.md` §2.2).
    pub async fn note_registered(&self, tenant_id: &fcp_core::TenantId, aor: &str) {
        let calls = self.push_waits.waiting_at(tenant_id, aor);
        if calls.is_empty() {
            return;
        }
        let Some(registrar) = &self.registrar else {
            return;
        };
        // From the store: this node's cache learns of the REGISTER from
        // the same event, perhaps after this.
        registrar.forget_registration(tenant_id, aor);
        let rows = registrar
            .get_registration_rows_in_tenant(tenant_id, aor)
            .await
            .unwrap_or_default();
        for call in calls {
            let n = self.push_waits.deliver(&call, &rows);
            if n > 0 {
                info!(call_id = %call, aor, woken = n, "A woken app is back: ringing it");
            }
        }
    }

    /// A call that woke apps is over: those that never came back are told
    /// so (a cancel push), so they stop ringing.
    fn end_push_wait(&self, call_id: fcp_b2bua_core::CallId) {
        let Some((missing, cancel)) = self.push_waits.end(&call_id) else {
            return;
        };
        let (Some(sender), Some(registrar)) = (self.push.clone(), self.registrar.clone()) else {
            return;
        };
        if missing.is_empty() {
            return;
        }
        tokio::spawn(async move {
            crate::push::push_bindings(&sender, &registrar, &missing, &cancel, None).await;
        });
    }

    /// A site's push, sent with this node's keys (`docs/PUSH.md` §2.4):
    /// `None` when this node has no keys (no `[push]`, or it relays its own).
    /// A gone token is the site's to take off its binding; the outcome says
    /// so.
    pub async fn send_relayed_push(
        &self,
        push: &fcp_core::push::RelayedPush,
    ) -> Option<Result<fcp_push::PushOutcome, String>> {
        let sender = self.push.as_ref().filter(|s| !s.relays())?;
        let message = match serde_json::from_value::<fcp_push::PushMessage>(push.message.clone()) {
            Ok(m) => m,
            Err(e) => return Some(Err(format!("the message: {e}"))),
        };
        Some(Ok(sender.send(&push.to, &message).await))
    }

    /// An administrator's test push to a device's apps: what each push did,
    /// or `None` when this node has no `[push]` (or no registrar).
    pub async fn push_test_device(
        &self,
        device: &fcp_core::Device,
    ) -> Option<Vec<crate::push::PushResult>> {
        let (Some(push), Some(registrar), Some(storage)) =
            (&self.push, &self.registrar, &self.storage)
        else {
            return None;
        };
        Some(crate::push::test_device(push, registrar, storage, device).await)
    }

    /// Log users in and out of host phones (`*88`, `*89`).
    pub fn with_hoteling(mut self, hoteling: Arc<crate::hoteling::Hoteling>) -> Self {
        self.hoteling = Some(hoteling);
        self
    }

    /// Count each user's calls, for call waiting.
    pub fn with_user_calls(mut self, user_calls: Arc<crate::user_calls::UserCalls>) -> Self {
        self.user_calls = Some(user_calls);
        self
    }

    pub fn with_default_tenant_id(mut self, tenant_id: fcp_core::TenantId) -> Self {
        self.default_tenant_id = Some(tenant_id);
        self
    }

    /// Override the billing engine used for balance admission and cut-off
    /// (`None` disables enforcement at the edge; settlement still happens
    /// in the call manager).
    pub fn with_billing(mut self, billing: Option<Arc<fcp_billing::BillingEngine>>) -> Self {
        self.billing = billing;
        self
    }

    /// Hang the call up once its affordable duration is exhausted.
    ///
    /// Waits for the call to be answered (giving up if it fails or is torn
    /// down first), sleeps until `established_at + max_duration`, then sends
    /// BYE on both legs if the call is still up. Only calls admitted with a
    /// finite cap get a watchdog.
    fn spawn_balance_watchdog(
        &self,
        call_id: fcp_b2bua_core::CallId,
        tenant_id: fcp_core::TenantId,
        max_duration: Duration,
    ) {
        use fcp_b2bua_core::CallState;
        const POLL: Duration = Duration::from_secs(1);
        // Longest we wait for an answer before assuming the call will not
        // be established (ringing timers are far shorter).
        const MAX_WAIT_FOR_ANSWER: Duration = Duration::from_secs(600);

        let handler = self.clone();
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let established_at = loop {
                match handler.call_manager.get_call(&call_id) {
                    None => return,
                    Some(call) => match call.state {
                        CallState::Terminating | CallState::Terminated | CallState::Failed => {
                            return
                        }
                        _ => {
                            if let Some(at) = call.established_at {
                                break at;
                            }
                        }
                    },
                }
                if started.elapsed() > MAX_WAIT_FOR_ANSWER {
                    return;
                }
                tokio::time::sleep(POLL).await;
            };

            let deadline = established_at + max_duration;
            if let Ok(remaining) = deadline.duration_since(std::time::SystemTime::now()) {
                tokio::time::sleep(remaining).await;
            }

            let still_up = handler
                .call_manager
                .get_call(&call_id)
                .map(|c| matches!(c.state, CallState::Established | CallState::Parked))
                .unwrap_or(false);
            if !still_up {
                return;
            }
            if let Err(e) = handler.hangup_call_by_id_with(
                &call_id,
                fcp_cdr::TerminationReason::system(
                    fcp_cdr::SystemCause::BalanceExhausted,
                    "Balance exhausted: affordable duration reached",
                ),
            ) {
                warn!(call_id = %call_id, error = %e, "Balance cut-off hangup failed");
                return;
            }
            if let Some(billing) = &handler.billing {
                billing
                    .report_cut_off(
                        &tenant_id,
                        fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        max_duration,
                    )
                    .await;
            }
        });
    }

    /// Get a reference to the B2BUA policy configuration.
    pub fn b2bua_config(&self) -> &B2buaConfig {
        &self.b2bua_config
    }

    /// Get a reference to the runtime-reloadable voicemail server URI override.
    pub fn voicemail_server_uri_override(&self) -> &Arc<std::sync::RwLock<Option<String>>> {
        &self.voicemail_server_uri_override
    }

    /// Get a reference to the runtime-reloadable conference server URI override.
    pub fn conference_server_uri_override(&self) -> &Arc<std::sync::RwLock<Option<String>>> {
        &self.conference_server_uri_override
    }

    /// Get a reference to the rate limits map for debugging/monitoring.
    pub fn rate_limits(&self) -> &Arc<dashmap::DashMap<sip_dialog::DialogId, RateWindow>> {
        &self.rate_limits
    }

    /// Get the count of active rate limit entries.
    pub fn rate_limit_count(&self) -> usize {
        self.rate_limits.len()
    }

    /// Get a reference to the subscription creation timestamps.
    pub fn subscription_created_at(
        &self,
    ) -> &Arc<dashmap::DashMap<sip_dialog::SubscriptionId, Instant>> {
        &self.subscription_created_at
    }

    /// Get a reference to the flow registry for RFC 5626 connection reuse.
    pub fn flow_registry(&self) -> &Arc<crate::flow::FlowRegistry> {
        &self.flow_registry
    }

    /// Set the SIP trace channel for raw message capture to PostgreSQL.
    pub fn set_sip_trace_tx(&mut self, tx: mpsc::Sender<fcp_core::SipTraceMessage>) {
        self.sip_trace_tx = Some(tx);
    }

    /// Extract DID (phone number) from a SIP URI.
    ///
    /// Extracts the username portion from URIs like:
    /// - "sip:+15551234567@domain" -> Some("+15551234567")
    /// - "<sip:+15551234567@domain>" -> Some("+15551234567")
    /// - "Alice <sip:alice@domain>" -> Some("alice")
    fn extract_did_from_uri(uri_str: &str) -> Option<String> {
        // Extract URI between < > if present
        let uri_part = if let Some(start) = uri_str.find('<') {
            if let Some(end) = uri_str.find('>') {
                &uri_str[start + 1..end]
            } else {
                uri_str
            }
        } else {
            uri_str
        };

        // Extract username from sip:username@domain or tel:+15551234567
        if let Some(user_start) = uri_part.find("sip:") {
            let after_sip = &uri_part[user_start + 4..];
            if let Some(at_pos) = after_sip.find('@') {
                return Some(after_sip[..at_pos].to_string());
            }
        } else if let Some(tel_start) = uri_part.find("tel:") {
            let after_tel = &uri_part[tel_start + 4..];
            // Remove any parameters after semicolon
            let number = if let Some(semi_pos) = after_tel.find(';') {
                &after_tel[..semi_pos]
            } else {
                after_tel
            };
            return Some(number.to_string());
        }

        None
    }

    /// The tenant whose call this is, for choosing the trunks its calls may
    /// leave on (`docs/TENANT_TRUNKS.md` §5).
    /// Whether an answer honours the SRTP the trunk requires: a carrier
    /// that answers a `required` trunk in the clear has not answered it
    /// (`docs/TENANT_TRUNKS.md` §6). `true` for every other trunk, which
    /// is free to answer either way.
    fn srtp_answer_honoured(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        answer_sdp: Option<&str>,
    ) -> bool {
        let required = self
            .call_manager
            .get_call(call_id)
            .and_then(|c| c.route_trunks.first().map(|r| r.trunk.srtp.mode))
            .is_some_and(|mode| mode == fcp_core::trunk::SrtpMode::Required);
        if !required {
            return true;
        }
        answer_sdp.is_some_and(|sdp| !fcp_media::sdp::parse_sdes_crypto(sdp).is_empty())
    }

    /// End a leg this node cannot use: a carrier that answered a trunk
    /// requiring SRTP without any (`docs/TENANT_TRUNKS.md` §6). The leg is
    /// answered, so it takes a BYE.
    async fn hang_up_unencrypted_leg(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        leg: &crate::uac::OutboundCall,
    ) {
        let Some(uac) = &self.uac else { return };
        let dialog = leg.dialog.read().await.clone();
        match uac.bye(&dialog).await {
            Ok(response) => {
                self.dialog_manager.remove(dialog.id());
                debug!(
                    call_id = %call_id,
                    code = response.start_line().code(),
                    "Sent BYE to the leg that answered without the SRTP its trunk requires"
                );
            }
            Err(e) => warn!(call_id = %call_id, error = %e, "BYE to the unencrypted leg failed"),
        }
    }

    /// The offer for a device registered over TLS: `sdp` with the call's
    /// device key (made on first use), when the call is anchored here and
    /// goes to no trunk; `sdp` unchanged otherwise.
    fn secure_device_offer(&self, call_id: &fcp_b2bua_core::CallId, sdp: String) -> String {
        let Some(call) = self.call_manager.get_call(call_id) else {
            return sdp;
        };
        if !(call.media_anchored && call.media_session_id.is_some())
            || !call.route_trunks.is_empty()
        {
            return sdp;
        }
        let offer = match self.b_leg_srtp.entry(*call_id) {
            dashmap::mapref::entry::Entry::Occupied(e) => e.get().sdp.clone(),
            dashmap::mapref::entry::Entry::Vacant(e) => match fcp_media::SrtpOffer::generate() {
                Ok(offer) => e.insert(offer).sdp.clone(),
                Err(err) => {
                    warn!(call_id = %call_id, error = %err, "Could not generate SRTP key material for a TLS device");
                    return sdp;
                }
            },
        };
        fcp_media::with_sdes(&sdp, &offer)
    }

    /// The SRTP this node offered on the call's B-leg, taken out of the map
    /// now that the answer is here, and kept as the leg's key for later
    /// answers (a hold, a re-INVITE) while the call lasts.
    fn take_b_leg_srtp(&self, call_id: &fcp_b2bua_core::CallId) -> Option<fcp_media::SrtpOffer> {
        let offer = self.b_leg_srtp.remove(call_id).map(|(_, offer)| offer)?;
        self.b_leg_keys
            .retain(|id, _| self.call_manager.get_call(id).is_some());
        self.b_leg_keys.insert(*call_id, offer.clone());
        Some(offer)
    }

    /// The key this node answers `side` with when that leg is encrypted: the
    /// caller's as the media backend last answered it, the callee's as this
    /// node offered it. `None` for a leg this node does not key.
    async fn leg_srtp(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        side: fcp_b2bua_core::LegSide,
        session: &fcp_media::MediaSession,
    ) -> Option<fcp_media::SrtpSdpContext> {
        match side {
            fcp_b2bua_core::LegSide::A => {
                fcp_media::srtp_context_of(&session.local_sdp().await.unwrap_or_default())
            }
            fcp_b2bua_core::LegSide::B => self.b_leg_keys.get(call_id).map(|o| o.sdp.clone()),
        }
    }

    /// Take a remote offer on a call's B-leg: an encrypted one is keyed
    /// from a fresh key of ours under its tag — kept as the leg's key, and
    /// returned for the answer — rather than one the answer would not name;
    /// a plain one as before.
    async fn key_b_leg_from_offer(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        session_id: &fcp_core::CallId,
        media_manager: &Arc<fcp_media::MediaManager>,
        offer: &str,
    ) -> fcp_media::Result<Option<fcp_media::SrtpSdpContext>> {
        let answering = fcp_media::parse_sdes_crypto(offer)
            .first()
            .and_then(|line| fcp_media::SrtpOffer::answering(line).ok());
        match answering {
            Some(ours) => {
                media_manager
                    .update_session_b_leg_offered(session_id, offer.to_string(), Some(ours.clone()))
                    .await?;
                let key = ours.sdp.clone();
                self.b_leg_keys
                    .retain(|id, _| self.call_manager.get_call(id).is_some());
                self.b_leg_keys.insert(*call_id, ours);
                Ok(Some(key))
            }
            None => {
                media_manager
                    .update_session_b_leg(session_id, offer.to_string())
                    .await?;
                Ok(None)
            }
        }
    }

    /// Count the call against the trunk that answered it, if a trunk did.
    async fn count_answering_trunk(&self, call_id: &fcp_b2bua_core::CallId) {
        let Some(counter) = &self.trunk_calls else {
            return;
        };
        if let Some(trunk) = self
            .call_manager
            .get_call(call_id)
            .as_ref()
            .and_then(carrying_trunk)
        {
            counter.carried(&call_id.to_string(), &trunk).await;
        }
    }

    /// Settle how each leg of an anchored trunk call takes DTMF, once the
    /// trunk has answered with `answer` (`docs/TENANT_TRUNKS.md` §6), and
    /// arrange for a digit one leg sends to reach the other that way.
    ///
    /// Telephone-event is relayed only to a leg that negotiated it. A
    /// digit the caller sends as telephone-event to a trunk that takes
    /// SIP INFO is sent on as INFO; the other direction — INFO to a leg
    /// that takes telephone-event — is [`Self::convert_info_dtmf`]'s.
    async fn arrange_trunk_dtmf(&self, call_id: &fcp_b2bua_core::CallId, answer: &str) {
        let Some(media) = self.media_manager.as_ref() else {
            return;
        };
        let Some(call) = self.call_manager.get_call(call_id) else {
            return;
        };
        let (true, Some(session_id)) = (call.media_anchored, call.media_session_id) else {
            return;
        };
        let Some(trunk) = carrying_trunk(&call) else {
            return;
        };
        let a_offer = call
            .original_invite
            .as_ref()
            .and_then(crate::body::request_sdp);
        let carriage = crate::trunk_egress::dtmf_carriage(&trunk, a_offer.as_deref(), answer);
        let _ = self.call_manager.update_call(call_id, |call| {
            call.dtmf_carriage = Some(carriage);
        });
        debug!(call_id = %call_id, trunk = %trunk.name, ?carriage, "DTMF per leg");

        for (leg, way) in [
            (fcp_media::MediaLeg::A, carriage.a),
            (fcp_media::MediaLeg::B, carriage.b),
        ] {
            let relay = way == fcp_b2bua_core::DtmfWay::Rfc2833;
            if let Err(e) = media.set_rfc2833_relay_to(&session_id, leg, relay) {
                warn!(call_id = %call_id, ?leg, error = %e, "Could not steer RFC 2833 relay");
            }
        }

        if carriage.b != fcp_b2bua_core::DtmfWay::Info {
            return;
        }
        let (Some(mut digits), Some(uac)) = (media.subscribe_digits(&session_id), self.uac.clone())
        else {
            warn!(
                call_id = %call_id,
                trunk = %trunk.name,
                "The trunk takes DTMF as SIP INFO and this node cannot convert the caller's"
            );
            return;
        };
        let call_manager = Arc::clone(&self.call_manager);
        let dialog_manager = Arc::clone(&self.dialog_manager);
        let locks = Arc::clone(&self.mid_dialog_lock);
        let call_id = *call_id;
        tokio::spawn(async move {
            while let Some(digit) = digits.recv().await {
                // A digit heard in the audio is in what the trunk hears
                // already; only telephone-event needs carrying.
                if digit.leg != fcp_media::MediaLeg::A || digit.in_band {
                    continue;
                }
                let Some(dialog_id) = call_manager.get_call(&call_id).map(|c| {
                    c.b_leg
                        .as_ref()
                        .and_then(|b| b.dialog.as_ref().map(|d| d.id().clone()))
                }) else {
                    break;
                };
                let Some(dialog_id) = dialog_id else {
                    continue;
                };
                // Serialised with every other request on the trunk's
                // dialog, and the dialog read under the lock, so the CSeq
                // is the next one.
                let lock = Arc::clone(
                    locks
                        .entry(dialog_id)
                        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                        .value(),
                );
                let _guard = lock.lock_owned().await;
                let Some(mut dialog) = call_manager
                    .get_call(&call_id)
                    .and_then(|c| c.b_leg.and_then(|b| b.dialog))
                else {
                    break;
                };
                let body = crate::trunk_egress::dtmf_relay_body(digit.digit, digit.duration_ms);
                match uac
                    .send_info(&mut dialog, crate::trunk_egress::DTMF_RELAY, &body)
                    .await
                {
                    Ok(resp) => debug!(
                        call_id = %call_id,
                        digit = %digit.digit,
                        status = resp.start_line().code(),
                        "Sent the caller's digit to the trunk as SIP INFO"
                    ),
                    Err(e) => warn!(
                        call_id = %call_id,
                        digit = %digit.digit,
                        error = %e,
                        "Could not send the caller's digit to the trunk as SIP INFO"
                    ),
                }
                let _ = call_manager.update_call(&call_id, |call| {
                    if let Some(b) = &mut call.b_leg {
                        b.dialog = Some(dialog.clone());
                    }
                });
                let _ = dialog_manager.insert(dialog);
            }
        });
    }

    /// A digit arriving as SIP INFO for a leg that takes telephone-event
    /// instead: played to that leg, and the INFO answered here. `None`
    /// when the INFO goes on as it came.
    async fn convert_info_dtmf(
        &self,
        request: &sip_core::Request,
        call: &fcp_b2bua_core::Call,
        is_a_leg: bool,
    ) -> Option<sip_core::Response> {
        let carriage = call.dtmf_carriage?;
        let content_type = sip_parse::header(request.headers(), "Content-Type")?;
        if !content_type
            .as_str()
            .trim()
            .eq_ignore_ascii_case(crate::trunk_egress::DTMF_RELAY)
        {
            return None;
        }
        let (to, way) = if is_a_leg {
            (fcp_media::MediaLeg::B, carriage.b)
        } else {
            (fcp_media::MediaLeg::A, carriage.a)
        };
        if way != fcp_b2bua_core::DtmfWay::Rfc2833 {
            return None;
        }
        let (digit, duration_ms) =
            crate::trunk_egress::parse_dtmf_relay(&String::from_utf8_lossy(request.body()))?;
        let session_id = call.media_session_id.as_ref()?;
        let media = self.media_manager.as_ref()?;
        match media.send_dtmf(session_id, to, digit, duration_ms).await {
            Ok(()) => {
                debug!(call_id = %call.id, %digit, ?to, "Played a SIP INFO digit as RFC 2833");
                Some(self.create_simple_response(request, 200, "OK"))
            }
            Err(e) => {
                warn!(
                    call_id = %call.id,
                    %digit,
                    error = %e,
                    "Could not play a SIP INFO digit as RFC 2833; forwarding the INFO"
                );
                None
            }
        }
    }

    fn call_tenant(&self, call_id: &fcp_b2bua_core::CallId) -> Option<fcp_core::TenantId> {
        self.call_manager
            .get_call(call_id)
            .map(|call| call.tenant_id)
    }

    /// The trunk routes a call to `destination` takes when this platform
    /// serves the destination itself no longer — an outbound call to a
    /// number that is nobody's DID here (`docs/TENANT_TRUNKS.md` §5).
    /// Records the chosen trunk on the call, and the whole ordered list so
    /// failover can name the one that answered.
    /// What the tenant's own rules decide about this INVITE
    /// (`docs/TENANT_TRUNKS.md` §12, phase 6b): `Ok` with the trunk target
    /// a `route_to_trunk` rule confined it to, if one did, or `Err` with the
    /// reason a `deny` rule refused it. Kept synchronous and out of
    /// `handle_invite`, whose future is deep enough already: nothing here
    /// lives across an await.
    fn tenant_rule_decision(
        &self,
        tenant_id: &fcp_core::TenantId,
        request: &sip_core::Request,
        peer: SocketAddr,
    ) -> std::result::Result<Option<fcp_core::TrunkTarget>, String> {
        let Some(rules) = &self.tenant_rules else {
            return Ok(None);
        };
        let caller = sip_parse::header(request.headers(), "From")
            .map(|from| {
                let from = from.as_str();
                from.split('<')
                    .nth(1)
                    .and_then(|s| s.split('>').next())
                    .unwrap_or(from)
                    .split(';')
                    .next()
                    .unwrap_or(from)
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        let destination = request.uri().as_str().to_string();
        let facts = fcp_policy::CallFacts {
            caller: &caller,
            destination: &destination,
            source_ip: peer.ip(),
        };
        match rules.evaluate(tenant_id, &facts) {
            fcp_policy::RuleOutcome::Deny { rule, reason } => {
                info!(
                    tenant_id = %tenant_id,
                    rule = %rule,
                    destination = %destination,
                    "Call denied by the tenant's routing rule"
                );
                Err(reason)
            }
            fcp_policy::RuleOutcome::Route { rule, target } => {
                debug!(
                    tenant_id = %tenant_id,
                    rule = %rule,
                    target = %target,
                    "The tenant's routing rule confines this call to a trunk"
                );
                Ok(Some(target))
            }
            fcp_policy::RuleOutcome::Allow { rule } => {
                debug!(tenant_id = %tenant_id, rule = %rule, "Call allowed by the tenant's routing rule");
                Ok(None)
            }
            fcp_policy::RuleOutcome::NoMatch => Ok(None),
        }
    }

    /// The trunk routes for a call to `destination`, classified: the
    /// ordinary choice, or only what a tenant's `route_to_trunk` rule named
    /// when one matched the call (`docs/TENANT_TRUNKS.md` §12, phase 6b).
    async fn classified_routes(
        &self,
        provider: &Arc<dyn OutboundRouteProvider>,
        call_id: &fcp_b2bua_core::CallId,
        destination: &str,
    ) -> RouteResolution {
        let (tenant, target) = match self.call_manager.get_call(call_id) {
            Some(call) => (Some(call.tenant_id), call.trunk_target.clone()),
            None => (None, None),
        };
        // A star code that reached the trunks is not one of ours, and is
        // no carrier's number either (D8: `*9` went to Twilio, which said
        // 400).
        let dialed = crate::class_of_service::uri_user(destination);
        if dialed.starts_with('*') || dialed.starts_with('#') {
            info!(call_id = %call_id, dialed = %dialed, "Not a feature code: no trunk takes it");
            return RouteResolution::NoRoute;
        }
        let resolution = self
            .unclassified_routes(provider, call_id, tenant, target, destination)
            .await;
        match (resolution, tenant) {
            (RouteResolution::Routes(routes), Some(tenant)) => {
                RouteResolution::Routes(self.in_e164_where_wanted(routes, &tenant, dialed).await)
            }
            (resolution, _) => resolution,
        }
    }

    /// The routes as the trunks give them: confined to the trunk a rule
    /// named, else every trunk that may carry the call.
    async fn unclassified_routes(
        &self,
        provider: &Arc<dyn OutboundRouteProvider>,
        call_id: &fcp_b2bua_core::CallId,
        tenant: Option<fcp_core::TenantId>,
        target: Option<fcp_core::TrunkTarget>,
        destination: &str,
    ) -> RouteResolution {
        match target {
            Some(target) => {
                let resolution = provider
                    .resolve_targeted_classified(tenant, destination, &target)
                    .await;
                if !matches!(resolution, RouteResolution::Routes(_)) {
                    warn!(
                        call_id = %call_id,
                        destination = %destination,
                        target = %target,
                        "The trunk a routing rule names cannot take this call; nothing else may"
                    );
                }
                resolution
            }
            None => {
                provider
                    .resolve_outbound_classified(tenant, destination)
                    .await
            }
        }
    }

    /// Each route on a trunk that takes E.164 (`Trunk::send_e164`) with
    /// the number as the tenant's country reads `dialed`, then the trunk's
    /// digit plan; the rest as they are, and every route as it is when
    /// `dialed` is not a number.
    async fn in_e164_where_wanted(
        &self,
        mut routes: Vec<OutboundRoute>,
        tenant: &fcp_core::TenantId,
        dialed: &str,
    ) -> Vec<OutboundRoute> {
        if !routes
            .iter()
            .any(|r| r.trunk.as_ref().is_some_and(|t| t.send_e164))
        {
            return routes;
        }
        let Some(storage) = &self.storage else {
            return routes;
        };
        let max_digits = usize::from(self.b2bua_config.max_extension_digits);
        let Some(e164) =
            crate::class_of_service::classify_for_tenant(storage, tenant, dialed, max_digits)
                .await
                .e164
        else {
            return routes;
        };
        for route in &mut routes {
            let Some(trunk) = route.trunk.as_ref().filter(|t| t.send_e164) else {
                continue;
            };
            let number = match &trunk.egress_number_transform {
                Some(transform) => transform.apply(&e164),
                None => e164.clone(),
            };
            route.target_uri = with_uri_user(&route.target_uri, &number);
        }
        routes
    }

    async fn trunk_bindings(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        destination: &str,
    ) -> TrunkBindings {
        let Some(provider) = &self.outbound_route_provider else {
            return TrunkBindings::NoRoute;
        };
        let routes = match self.classified_routes(provider, call_id, destination).await {
            RouteResolution::Routes(routes) => routes,
            RouteResolution::Unavailable => return TrunkBindings::Unavailable,
            RouteResolution::NoRoute => return TrunkBindings::NoRoute,
        };
        let Some(route) = routes.first() else {
            return TrunkBindings::NoRoute;
        };
        info!(
            call_id = %call_id,
            destination = %destination,
            target = %route.target_uri,
            trunk = ?route.trunk_name(),
            routes = routes.len(),
            "Routing via outbound trunk"
        );
        let trunk_name = route.trunk_name();
        let trunk_id = route.trunk_id();
        let allowed_codecs = route.allowed_codecs();
        let trunks = route_trunks(&routes);
        let _ = self.call_manager.update_call(call_id, |call| {
            call.trunk_name = trunk_name;
            call.trunk_id = trunk_id;
            call.outbound_allowed_codecs = allowed_codecs;
            call.route_trunks = trunks;
        });

        let mut bindings = Vec::with_capacity(routes.len());
        for r in &routes {
            // In angle brackets, so the URI's own parameters — a TLS
            // trunk's `;transport=tls` — stay with it rather than being
            // read as contact parameters and dropped.
            match sip_registrar::Binding::new(
                SmolStr::new(destination),
                SmolStr::new(format!("<{}>", r.target_uri)),
                Duration::from_secs(3600),
            ) {
                Ok(binding) => bindings.push(binding),
                Err(e) => {
                    warn!(call_id = %call_id, target = %r.target_uri, error = %e, "Skipping route: failed to create binding")
                }
            }
        }
        if bindings.is_empty() {
            return TrunkBindings::Unavailable;
        }
        TrunkBindings::Routed(bindings)
    }

    /// The tenant of the registration this INVITE's caller placed it from:
    /// the From AOR's bindings, one of them at the peer's address. A
    /// registered, digest-checked caller's call is its registration's
    /// tenant's, whatever host the phone dialled — a softphone configured
    /// with the node's address dials `sip:1002@<address>`, which names no
    /// tenant (found by the nightly stack, `docs/NIGHTLY_STACK.md` §4).
    /// `None` when the caller is not registered here.
    async fn registered_caller_tenant(
        &self,
        request: &sip_core::Request,
        peer_ip: std::net::IpAddr,
    ) -> Option<fcp_core::TenantId> {
        if let Some(registrar) = &self.registrar {
            let from_header = sip_parse::header(request.headers(), "From");
            let from_aor = from_header.and_then(|h| {
                let s = h.as_str();
                // Extract URI from name-addr: "Name" <sip:user@host> or sip:user@host
                let uri = if let Some(start) = s.find('<') {
                    let end = s.find('>').unwrap_or(s.len());
                    &s[start + 1..end]
                } else {
                    s.split(';').next().unwrap_or(s)
                };
                let cleaned = uri.trim_start_matches("sip:").trim_start_matches("sips:");
                if cleaned.contains('@') {
                    Some(format!("sip:{}", cleaned))
                } else {
                    None
                }
            });
            if let Some(aor) = from_aor {
                let rows = match registrar.get_registration_rows(&aor).await {
                    Ok(rows) => rows,
                    Err(e) => {
                        debug!(aor = %aor, error = %e, "No registrations to place this INVITE's caller");
                        return None;
                    }
                };
                let matched = rows
                    .iter()
                    .find(|reg| crate::registrar::registration_is_at(reg, peer_ip))
                    .map(|reg| reg.tenant_id);
                if matched.is_none() && !rows.is_empty() {
                    warn!(
                        aor = %aor,
                        source = %peer_ip,
                        bindings = rows.len(),
                        "INVITE claims a registered user that is registered somewhere else"
                    );
                }
                matched
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Whether an INVITE arrived on a trunk this node takes calls on
    /// (`docs/TENANT_TRUNKS.md` §7).
    ///
    /// A trunk is proof of its own kind: a carrier holds no user's
    /// password, so a call that arrived on one is never challenged for a
    /// digest (`docs/EDGE_SECURITY.md` §2.1). Looked up only where the
    /// answer is needed — an authenticated caller never reaches the
    /// store.
    async fn invite_on_trunk(
        &self,
        tenant_id: &fcp_core::TenantId,
        peer_ip: std::net::IpAddr,
        inbound_trunk: bool,
    ) -> bool {
        if inbound_trunk {
            return true;
        }
        if self.inbound_trunks.is_some() {
            return false;
        }
        // A node without a trunk set admits by its tenants' stored
        // trunks, as it always has.
        let Some(storage) = &self.storage else {
            return false;
        };
        let peer_ip_str = peer_ip.to_string();
        match storage.list_trunks(tenant_id).await {
            Ok(trunks) => trunks.iter().any(|t| {
                t.enabled
                    && t.hosts.iter().any(|h| {
                        let host = h.split(':').next().unwrap_or(h);
                        host == peer_ip_str
                    })
            }),
            Err(e) => {
                warn!(error = %e, tenant_id = %tenant_id, "Failed to query trunks for admission check, denying");
                false
            }
        }
    }

    /// The trunk a carrier's INVITE arrived on, and whose call that makes it
    /// (`docs/TENANT_TRUNKS.md` §7); `None` when this node has no trunk set.
    /// A REGISTER that bound a site's link account
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.6): the site is bound from
    /// `peer`, reachable down `flow_token`, for as long as it asked; one
    /// that asked for nothing is unbound.
    fn note_link_peer_registration(
        &self,
        request: &sip_core::Request,
        peer: SocketAddr,
        flow_token: Option<&str>,
    ) {
        let Some(trunks) = &self.inbound_trunks else {
            return;
        };
        let Some(user) = crate::link_peers::to_user(request) else {
            return;
        };
        let Some(trunk) = trunks.link_peer_by_account(&user) else {
            return;
        };
        let expiry = crate::link_peers::requested_expiry(request);
        if expiry == 0 {
            self.link_peers.unbind(&trunk.key());
            return;
        }
        if flow_token.is_none() {
            warn!(
                trunk = %trunk.name,
                peer = %peer,
                "A site's link registered over a transport that leaves no flow; its calls are sent to where it registered from"
            );
        }
        self.link_peers.bind(
            trunk,
            peer,
            flow_token.map(str::to_string),
            Duration::from_secs(u64::from(expiry)),
        );
    }

    /// A number answered at a site other than this deployment
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.6): the binding the call goes
    /// down, which is the site's registration at this node, with the
    /// number as the target's user. `None` when the number is not at
    /// another site; an error when it is and the site cannot be reached.
    async fn route_to_site(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        normalized_aor: &str,
    ) -> Option<std::result::Result<Vec<sip_registrar::Binding>, String>> {
        let trunks = self.inbound_trunks.as_ref()?;
        let storage = self.storage.as_ref()?;
        if !is_e164_number(normalized_aor) {
            return None;
        }
        let number = normalized_aor
            .strip_prefix("sip:")
            .or_else(|| normalized_aor.strip_prefix("sips:"))
            .unwrap_or(normalized_aor)
            .split('@')
            .next()
            .unwrap_or(normalized_aor);
        let owned = storage
            .get_phone_number_by_e164(number)
            .await
            .ok()
            .flatten()?;
        let site = owned.site_id?;
        if trunks.site() == Some(site) {
            return None;
        }
        let Some(peer) = trunks.link_peer_for_site(site) else {
            return Some(Err(format!(
                "number {number} is at site {site}, which has no link trunk here"
            )));
        };
        let Some(bound) = self.link_peers.for_trunk(&peer.key(), &self.flow_registry) else {
            return Some(Err(format!(
                "number {number} is at site {site}, which is not registered"
            )));
        };
        let contact = format!("<sip:{number}@{};transport=tls>", bound.peer);
        let binding = match sip_registrar::Binding::new(
            SmolStr::new(normalized_aor),
            SmolStr::new(&contact),
            bound.remaining().max(Duration::from_secs(1)),
        ) {
            Ok(binding) => binding,
            Err(e) => return Some(Err(format!("the site's binding is not usable: {e}"))),
        };
        let binding = match &bound.flow_token {
            Some(token) => binding.with_flow_token(SmolStr::new(token)),
            None => binding,
        };
        info!(
            call_id = %call_id,
            number = %number,
            site_id = %site,
            peer = %bound.peer,
            via_flow = bound.flow_token.is_some(),
            "Routing a call for a number at a site down the site's link"
        );
        let _ = self.call_manager.update_call(call_id, |call| {
            call.trunk_name = Some(peer.name.clone());
            call.trunk_id = Some(peer.trunk_id());
        });
        // The site's media address is not what its SDP will say.
        self.latch_leg(call_id, fcp_media::backend::Leg::B, bound.peer.ip())
            .await;
        Some(Ok(vec![binding]))
    }

    /// Learn one leg's RTP address from its first packet from `source`,
    /// rather than its SDP, on an anchored call.
    async fn latch_leg(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        leg: fcp_media::backend::Leg,
        source: std::net::IpAddr,
    ) {
        let Some(media_manager) = &self.media_manager else {
            return;
        };
        let Some(session) = self
            .call_manager
            .get_call(call_id)
            .and_then(|c| c.media_session_id)
        else {
            return;
        };
        if let Err(e) = media_manager.latch_leg(&session, leg, source).await {
            debug!(call_id = %call_id, ?leg, error = %e, "Could not set the leg to learn its address");
        }
    }

    /// A call that came up a site's link has media from wherever the site
    /// is, not from the address in its SDP.
    async fn latch_link_peer_a_leg(&self, call_id: &fcp_b2bua_core::CallId) {
        let Some(peer) = self
            .call_manager
            .get_call(call_id)
            .filter(|c| c.link_site.is_some())
            .and_then(|c| c.a_leg.contact)
        else {
            return;
        };
        self.latch_leg(call_id, fcp_media::backend::Leg::A, peer.ip())
            .await;
    }

    async fn inbound_trunk(
        &self,
        request: &sip_core::Request,
        peer: SocketAddr,
    ) -> Option<crate::trunk_ingress::InboundChoice> {
        let trunks = self.inbound_trunks.as_ref()?;
        // A site bound here is known by where it registered from
        // (`docs/HYBRID_CONTROL_PLANE.md` §3.6): its calls are its
        // tenant's own, before any carrier's address is looked at.
        if let Some(trunk) = self.link_peers.from_peer(peer, &self.flow_registry) {
            if let Some(tenant) = trunk.tenant_id {
                return Some(crate::trunk_ingress::InboundChoice::Tenant { trunk, tenant });
            }
        }
        let candidates = trunks.inbound_from(peer.ip());
        if candidates.is_empty() {
            return Some(crate::trunk_ingress::InboundChoice::NoTrunk);
        }
        let request_user = request
            .uri()
            .as_sip()
            .and_then(|u| u.user().map(str::to_string));
        // The dialled number is the Request-URI's user, or the To's when the
        // carrier addresses the account a trunk registered.
        let to_user = sip_parse::header(request.headers(), "To").and_then(|to| {
            let to = to.as_str();
            let uri = to
                .split_once('<')
                .and_then(|(_, rest)| rest.split_once('>').map(|(u, _)| u))
                .unwrap_or(to);
            let uri = uri.trim_start_matches("sip:").trim_start_matches("sips:");
            uri.split_once('@').map(|(u, _)| u.to_string())
        });
        let mut number_owner = None;
        if let Some(storage) = &self.storage {
            let max_digits = self.b2bua_config.max_extension_digits;
            for user in request_user.iter().chain(to_user.iter()) {
                let number = normalize_dialed_number(user, max_digits);
                if !is_e164_number(&number) {
                    continue;
                }
                if let Ok(Some(owned)) = storage.get_phone_number_by_e164(&number).await {
                    number_owner = Some(owned.tenant_id);
                    break;
                }
            }
        }
        Some(crate::trunk_ingress::choose_inbound(
            &candidates,
            request_user.as_deref(),
            number_owner,
        ))
    }

    async fn resolve_runtime_tenant(
        &self,
        request: &sip_core::Request,
        call_id: &fcp_b2bua_core::CallId,
    ) -> Result<fcp_core::TenantId> {
        if let Some(call) = self.call_manager.get_call(call_id) {
            return Ok(call.tenant_id);
        }

        if let Some(domain) = extract_tenant_from_invite(request) {
            if fcp_core::is_host_literal(&domain) {
                return self
                    .default_tenant_fallback()
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("tenant context required"));
            }

            if let Some(storage) = &self.storage {
                if let Some(tenant) = storage.get_tenant_by_sip_domain(&domain).await? {
                    return Ok(tenant.id);
                }
                return match storage.get_tenant_by_name(&domain).await {
                    Ok(Some(tenant)) => Ok(tenant.id),
                    Ok(None) => Err(anyhow::anyhow!("unknown tenant domain '{}'", domain)),
                    Err(e) => Err(e).context("failed to resolve tenant from SIP domain"),
                };
            }

            return self.default_tenant_fallback().await?.ok_or_else(|| {
                anyhow::anyhow!("tenant store unavailable for domain '{}'", domain)
            });
        }

        self.default_tenant_fallback()
            .await?
            .ok_or_else(|| anyhow::anyhow!("tenant context required"))
    }

    async fn default_tenant_fallback(&self) -> Result<Option<fcp_core::TenantId>> {
        let Some(default_tenant_id) = self.default_tenant_id else {
            return Ok(None);
        };
        let Some(storage) = &self.storage else {
            return Ok(Some(default_tenant_id));
        };

        let tenants = storage
            .list_tenants()
            .await
            .context("failed to list tenants for default tenant fallback")?;
        if tenants.is_empty() || (tenants.len() == 1 && tenants[0].id == default_tenant_id) {
            Ok(Some(default_tenant_id))
        } else {
            Ok(None)
        }
    }

    /// Extract display name from a SIP From/To header value.
    ///
    /// Handles formats like:
    /// - `"Alice Smith" <sip:alice@domain>` -> Some("Alice Smith")
    /// - `Alice <sip:alice@domain>` -> Some("Alice")
    /// - `<sip:alice@domain>` -> None
    /// - `sip:alice@domain` -> None
    fn extract_display_name(header_value: &str) -> Option<String> {
        let trimmed = header_value.trim();
        // Must have angle brackets for display name to be present
        let bracket_pos = trimmed.find('<')?;
        if bracket_pos == 0 {
            return None;
        }
        let name_part = trimmed[..bracket_pos].trim();
        if name_part.is_empty() {
            return None;
        }
        // Strip surrounding quotes if present
        let name = if name_part.starts_with('"') && name_part.ends_with('"') && name_part.len() >= 2
        {
            &name_part[1..name_part.len() - 1]
        } else {
            name_part
        };
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    }

    /// Handle an incoming SIP request and generate a response.
    ///
    /// Supports:
    /// - INVITE: Creates dialog and sends 200 OK
    /// - ACK: Confirms dialog establishment (no response needed)
    /// - BYE: Terminates dialog and sends 200 OK
    /// - OPTIONS: Sends 200 OK (keepalive/ping)
    /// - REGISTER: Handles endpoint registration (if registrar configured)
    /// - Other methods: Sends 501 Not Implemented
    async fn handle_request(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        peer: SocketAddr,
        transport: sip_transaction::TransportKind,
        stream: Option<mpsc::Sender<Bytes>>,
        peer_identity: Option<Arc<sip_transport::PeerIdentity>>,
    ) -> Result<()> {
        use sip_core::Method;

        self.serving.get_or_init(|| self.clone());
        match request.method() {
            Method::Invite => {
                // If this is an in-dialog re-INVITE, handle separately
                if self.dialog_manager.find_by_request(request).is_some()
                    || self.is_b_leg_reinvite(request)
                {
                    // Check conference handler first — conference participants
                    // don't have B2BUA call objects
                    if let Some(ref conf) = self.conference_handler {
                        if let Ok(resp) = conf.handle_reinvite(request).await {
                            handle.send_final(resp).await;
                            return Ok(());
                        }
                    }
                    // Fall through to B2BUA re-INVITE
                    let response = self.handle_reinvite(request).await?;
                    handle.send_final(response).await;
                } else if sip_parse::header(request.headers(), "To")
                    .and_then(|to| crate::sca::tag_of(to.as_str()))
                    .is_some()
                {
                    // A re-INVITE for a dialog this node does not hold.
                    // Taken as a new call, it rang the callee and answered
                    // with the request's To tag and a second one of its own
                    // (`docs/DEEP_TESTING.md` D2c). RFC 3261 §12.2.2: 481,
                    // the node not recreating the dialog.
                    debug!(peer = %peer, "Refusing an INVITE for a dialog that does not exist");
                    let response = self.create_simple_response(
                        request,
                        481,
                        "Call/Transaction Does Not Exist",
                    );
                    handle.send_final(response).await;
                } else {
                    // One source may not spend the whole node's budget
                    // (`docs/EDGE_SECURITY.md` §2.4). Counted only where a
                    // call starts: a re-INVITE is bounded per dialog, and
                    // counting it here would refuse a long call for being
                    // put on hold twice.
                    if let Some(guard) = &self.edge_guard {
                        if let crate::edge::EdgeVerdict::Drop(why) = guard.admit_invite(peer.ip()) {
                            self.metrics
                                .inc_counter("fcp_sip_edge_dropped_total", &[("reason", why)]);
                            let mut response = self.create_simple_response(
                                request,
                                503,
                                "Service Unavailable - source is starting calls too fast",
                            );
                            let _ = response
                                .headers_mut()
                                .push(SmolStr::new("Retry-After"), SmolStr::new("1"));
                            handle.send_final(response).await;
                            return Ok(());
                        }
                    }

                    if !self.store_answers() {
                        info!(peer = %peer, "Refusing a call: the store does not answer");
                        handle.send_final(self.store_unavailable(request)).await;
                        return Ok(());
                    }

                    // Spawn INVITE handling in separate task to avoid blocking server transaction
                    // This prevents B2BUA from blocking for 32 seconds waiting for B-leg responses
                    let handler = self.clone();
                    let request_clone = request.clone();
                    let handle_clone = handle.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handler
                            .handle_invite(&request_clone, &handle_clone, peer, peer_identity)
                            .await
                        {
                            error!("INVITE handling failed: {}", e);
                        }
                    });
                }
            }
            Method::Cancel => {
                self.handle_cancel(request, handle).await?;
            }
            Method::Ack => {
                // ACK for 2xx responses is outside transaction layer (RFC 3261 §17.1.1.3)
                self.handle_ack(request)?;
                // No response needed for ACK
            }
            Method::Bye => {
                // A supervisor hanging up leaves the call they supervise
                // (`docs/ENTERPRISE_CALLING.md` §3.11).
                if let Some(supervisions) = &self.supervisions {
                    let sip_call_id = sip_parse::header(request.headers(), "Call-ID")
                        .map(|c| c.trim().to_string())
                        .unwrap_or_default();
                    if supervisions.is_supervisor(&sip_call_id) {
                        supervisions.hung_up(&sip_call_id).await;
                        let response = self.create_simple_response(request, 200, "OK");
                        handle.send_final(response).await;
                        return Ok(());
                    }
                }
                let response = self.handle_bye(request)?;
                handle.send_final(response).await;
            }
            Method::Update => {
                let response = self.handle_update(request).await?;
                handle.send_final(response).await;
            }
            Method::Prack => {
                let response = self.handle_prack(request).await?;
                handle.send_final(response).await;
            }
            Method::Info => {
                // Check conference handler first
                if let Some(ref conf) = self.conference_handler {
                    if let Ok(resp) = conf.handle_info(request) {
                        handle.send_final(resp).await;
                        return Ok(());
                    }
                }
                let response = self.handle_info(request).await?;
                handle.send_final(response).await;
            }
            Method::Refer => {
                // Check conference handler first
                if let Some(ref conf) = self.conference_handler {
                    if let Ok(resp) = conf.handle_refer(request) {
                        handle.send_final(resp).await;
                        return Ok(());
                    }
                }
                let response = self.handle_refer(request).await?;
                handle.send_final(response).await;
            }
            Method::Subscribe => {
                // Message waiting (RFC 3842), in or out of dialog.
                if let Some(subs) = &self.mwi_subscriptions {
                    if crate::mwi_subscriptions::MwiSubscriptions::handles_event(request) {
                        if let Err(e) = subs.handle_subscribe(request, handle).await {
                            warn!(error = %e, "A message-summary SUBSCRIBE failed");
                            let response =
                                self.create_simple_response(request, 500, "Server Internal Error");
                            handle.send_final(response).await;
                        }
                        return Ok(());
                    }
                }
                // A shared line's appearances (`call-info`, `line-seize`,
                // `docs/ENTERPRISE_CALLING.md` §3.9), in or out of dialog.
                if let Some(sca) = &self.sca {
                    if crate::sca::Sca::handles_event(request) {
                        if let Err(e) = sca.handle_subscribe(request, handle).await {
                            warn!(error = %e, "A shared-line SUBSCRIBE failed");
                            let response =
                                self.create_simple_response(request, 500, "Server Internal Error");
                            handle.send_final(response).await;
                        }
                        return Ok(());
                    }
                }
                // Check presence handler first (RFC 3856 presence, RFC 4235 dialog/BLF)
                if let Some(ref pres) = self.presence_handler {
                    if let Ok(()) = pres.handle_subscribe(request, handle).await {
                        return Ok(());
                    }
                }
                // Check conference handler (RFC 4575 conference event package)
                if let Some(ref conf) = self.conference_handler {
                    if let Ok(()) = conf.handle_subscribe(request, handle).await {
                        return Ok(());
                    }
                }
                // Outside a dialog, an event no handler here takes: 489
                // (RFC 6665 §4.2.2), not the in-dialog forward's error.
                let in_dialog = sip_parse::header(request.headers(), "To")
                    .is_some_and(|to| to.to_ascii_lowercase().contains(";tag="));
                if !in_dialog {
                    let mut response = self.create_simple_response(request, 489, "Bad Event");
                    let mut allowed: Vec<&str> = Vec::new();
                    if self.mwi_subscriptions.is_some() {
                        allowed.push("message-summary");
                    }
                    if self.presence_handler.is_some() {
                        allowed.extend(["presence", "dialog"]);
                    }
                    if self.sca.is_some() {
                        allowed.extend(["call-info", "line-seize"]);
                    }
                    if !allowed.is_empty() {
                        let _ = response.headers_mut().push(
                            SmolStr::new("Allow-Events"),
                            SmolStr::new(allowed.join(", ")),
                        );
                    }
                    handle.send_final(response).await;
                    return Ok(());
                }
                let response = self.handle_subscribe(request).await?;
                handle.send_final(response).await;
            }
            Method::Notify => {
                // A conference node's own sessions first: the outcome of a
                // REFER it sent (phase 5 migration).
                if let Some(ref conf) = self.conference_handler {
                    if let Ok(resp) = conf.handle_notify(request) {
                        handle.send_final(resp).await;
                        return Ok(());
                    }
                }
                let response = self.handle_notify(request).await?;
                handle.send_final(response).await;
            }
            Method::Publish => {
                // RFC 3903 — presence PUBLISH
                if let Some(ref pres) = self.presence_handler {
                    if let Ok(()) = pres.handle_publish(request, handle).await {
                        return Ok(());
                    }
                }
                // No handler claimed it — 501
                let response = self.create_simple_response(request, 501, "Not Implemented");
                handle.send_final(response).await;
            }
            Method::Options => {
                debug!("Responding to OPTIONS with 200 OK");
                let response = self.create_simple_response(request, 200, "OK");
                handle.send_final(response).await;
            }
            Method::Register => {
                // A registration is a write: with the store away it cannot
                // be taken, and one already under way when it went is
                // answered within the deadline (`docs/DEEP_TESTING.md` D5).
                let response = if !self.store_writes() {
                    self.store_unavailable(request)
                } else {
                    match tokio::time::timeout(
                        REGISTER_DEADLINE,
                        self.handle_register_internal(request, transport, stream, peer),
                    )
                    .await
                    {
                        Ok(response) => response,
                        Err(_) => {
                            warn!(peer = %peer, "A REGISTER outran its deadline; refused with 503");
                            self.store_unavailable(request)
                        }
                    }
                };
                handle.send_final(response).await;
            }
            _ => {
                debug!(
                    method = ?request.method(),
                    "Responding with 501 Not Implemented"
                );
                let response = self.create_simple_response(request, 501, "Not Implemented");
                handle.send_final(response).await;
            }
        }

        Ok(())
    }

    /// Handle CANCEL: acknowledge and mark call terminated.
    async fn handle_cancel(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
    ) -> Result<()> {
        debug!("Handling CANCEL request");

        // Per RFC 3261, respond 200 OK to CANCEL itself
        let ok = self.create_simple_response(request, 200, "OK");
        handle.send_final(ok).await;

        // Check conference handler first — conference INVITEs don't use CallManager
        if let Some(ref conf) = self.conference_handler {
            if let Some(call_id_hdr) = sip_parse::header(request.headers(), "Call-ID") {
                if let Ok(true) = conf.handle_cancel(call_id_hdr.as_str()) {
                    debug!(call_id = %call_id_hdr, "Conference handler processed CANCEL");
                    return Ok(());
                }
            }
        }

        // Check queue handler — queue INVITEs don't use CallManager
        if let Some(ref queue) = self.queue_handler {
            if let Some(call_id_hdr) = sip_parse::header(request.headers(), "Call-ID") {
                if let Ok(true) = queue.handle_cancel(call_id_hdr.as_str()) {
                    debug!(call_id = %call_id_hdr, "Queue handler processed CANCEL");
                    return Ok(());
                }
            }
        }

        // Try to locate the related call via INVITE Call-ID and terminate it
        if let Some(call_id_hdr) = sip_parse::header(request.headers(), "Call-ID") {
            if let Some(call) = self
                .call_manager
                .find_by_invite_call_id(call_id_hdr.as_str())
            {
                info!(call_id = %call.id, "CANCEL matched call; marking as failed");

                // One update marks the call cancelled and takes what the
                // forking task left on it: its wake-up and the leg it chose.
                // The forking task stores each in an update that reads the
                // mark, so whichever runs second sees the other — a fork
                // never starts unwoken, and an answer that crossed this
                // CANCEL is ended by exactly one side (`docs/DEEP_TESTING.md`
                // D2).
                //
                // An answered call is past CANCEL (RFC 3261 §9.2: a CANCEL
                // for a request that has its final response has no effect):
                // the forking task marks it established before it sends the
                // caller the 200, in an update that also reads the mark, so
                // the caller either has, or will be sent, the answer and
                // hangs up with BYE.
                let dialog_manager = Arc::clone(&self.dialog_manager);
                let mut notify = None;
                let mut chosen = None;
                let mut answered = false;
                let _ = self.call_manager.update_call(&call.id, |c| {
                    if c.state == fcp_b2bua_core::CallState::Established {
                        answered = true;
                        return;
                    }
                    c.caller_cancelled = true;
                    notify = c.cancel_notify.clone();
                    if let Some(b) = &mut c.b_leg {
                        if let Some(dialog) = b.dialog.take() {
                            info!(
                                call_id = %c.id,
                                dialog_id = ?dialog.id(),
                                "Removing stored B-leg dialog due to CANCEL"
                            );
                            dialog_manager.remove(dialog.id());
                            self.session_timer_manager.stop_timer(dialog.id());
                        }
                        chosen = b
                            .outbound_handle
                            .take()
                            .map(|handle| (handle, b.remote_uri.clone()));
                    }
                });

                if answered {
                    info!(call_id = %call.id, "CANCEL after the call was answered: no effect");
                    return Ok(());
                }

                // Wake the forking task, which CANCELs every leg it owns
                // (or BYEs one that answered).
                if let Some(notify) = notify {
                    info!(call_id = %call.id, "Signaling forking task to CANCEL all B-legs via cancel_notify");
                    notify.notify_one();
                } else {
                    debug!(
                        call_id = %call.id,
                        call_state = ?call.state,
                        "CANCEL before any B-leg was rung; the forking step will see the mark"
                    );
                }

                // The leg the forking task already chose: CANCEL it, or BYE
                // it if it answered.
                if let Some((outbound, target)) = chosen {
                    info!(call_id = %call.id, "Ending the chosen B-leg: CANCEL, or BYE if it answered");
                    tokio::spawn(Self::cancel_or_bye_fork_leg_with(
                        self.uac.clone(),
                        Arc::clone(&self.dialog_manager),
                        call.id,
                        target,
                        0,
                        outbound,
                        "Caller CANCEL".to_string(),
                    ));
                }

                // Send 487 to the original INVITE transaction on A-leg if we still have it
                if let (Some(tx_key), Some(invite_req)) =
                    (&call.a_leg_invite_tx, &call.original_invite)
                {
                    let response =
                        self.create_cancelled_response(invite_req, call.a_leg_to_tag.as_deref());
                    self.transaction_manager.send_final(tx_key, response).await;
                }

                stop_timers_for_call(&self.session_timer_manager, &call);
                self.end_call_after_media(
                    &call,
                    fcp_cdr::TerminationReason::Cancelled,
                    true,
                    "CANCEL",
                );
            } else {
                // Its INVITE may still be on its way to a call: the 100 is
                // sent first, then the caller is authenticated and routed.
                // That INVITE answers 487 when it finds this.
                let now = std::time::Instant::now();
                self.early_cancels
                    .retain(|_, at| now.duration_since(*at) < Duration::from_secs(32));
                // Strays (a CANCEL for no INVITE) are kept as long; a flood
                // of them stops being kept rather than growing this.
                if self.early_cancels.len() < 10_000 {
                    self.early_cancels
                        .insert(handle.key().branch().to_string(), now);
                }
                debug!("CANCEL arrived before its call; kept for its INVITE");
            }
        } else {
            debug!("CANCEL missing Call-ID header; cannot match call");
        }

        Ok(())
    }

    /// Handle an INVITE request as a B2BUA.
    ///
    /// Flow:
    /// 1. Send 100 Trying to A-leg (caller)
    /// 2. Check routing policy
    /// 3. Create Call object and store in CallManager
    /// 4. TODO: Look up destination endpoint
    /// 5. TODO: Create outbound INVITE to B-leg (callee)
    /// 6. TODO: Relay responses from B-leg to A-leg
    async fn handle_invite(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        peer: SocketAddr,
        peer_identity: Option<Arc<sip_transport::PeerIdentity>>,
    ) -> Result<()> {
        debug!("Handling INVITE request (B2BUA mode)");

        // Send 100 Trying immediately
        let mut trying = sip_uas::UserAgentServer::create_trying(request);
        self.ensure_response_hygiene(request, &mut trying);
        handle.send_provisional(trying).await;

        // Validate incoming Session-Expires per RFC 4028 §7
        if self.b2bua_config.session_timer.enabled {
            if let Err(reject) = sip_uas::UserAgentServer::validate_session_timer(
                request,
                Some(Duration::from_secs(
                    self.b2bua_config.session_timer.min_se_secs as u64,
                )),
            ) {
                debug!("Rejecting INVITE with 422: Session-Expires below Min-SE");
                handle.send_final(reject).await;
                return Ok(());
            }
        }

        // A retrieve another node sent here for a held appearance of a call
        // this node holds (`docs/ENTERPRISE_CALLING.md` §3.9). The handoff
        // token is the appearance's own, which only a node that read the
        // table knows; it stands for the phone's credentials, which that
        // node checked. The INVITE takes the line's leg of the call and
        // nothing else, so no tenant, route or admission is looked at.
        if let Some(token) = sip_parse::header(request.headers(), crate::sca::NODE_HANDOFF_HEADER) {
            let held = self
                .sca
                .as_ref()
                .and_then(|sca| sca.handoff_call(token.as_str()))
                .and_then(|id| self.call_manager.get_call(&id));
            return match held {
                Some(call) => {
                    info!(call_id = %call.id, source = %peer, "A retrieve handed over from another node");
                    Box::pin(self.retrieve_appearance(
                        request,
                        handle,
                        &fcp_b2bua_core::CallId::new(),
                        &call,
                    ))
                    .await
                }
                None => {
                    info!(source = %peer, "Refusing a handoff that names no appearance held here");
                    let response = self.create_simple_response(request, 481, "No Such Appearance");
                    handle.send_final(response).await;
                    Ok(())
                }
            };
        }

        // What this peer may claim about the call
        // (`docs/EDGE_SECURITY.md` §2.3). Taken away before anything
        // reads it, so a reader added later is gated by construction
        // rather than by somebody remembering. A trusted edge keeps what
        // it sends; so does a transfer origin naming a call this node
        // actually has, which is how a transferred phone carries one.
        let knows_call = |id: &str| {
            id.parse::<fcp_core::CallId>()
                .ok()
                .map(|id| fcp_b2bua_core::CallId::from_uuid(*id.as_uuid()))
                .is_some_and(|id| self.call_manager.get_call(&id).is_some())
        };
        // A relay's call id, read before it is stripped: believed below only
        // with a flow token this node issued, which only a node has.
        let relay_call_id = sip_parse::header(request.headers(), crate::push::FLOW_RELAY_HEADER)
            .and_then(|_| sip_parse::header(request.headers(), fcp_core::CALL_ID_HEADER))
            .and_then(|v| v.as_str().trim().parse::<fcp_core::CallId>().ok());
        let unclaimed = crate::claims::Claims::from(crate::claims::PeerTrust::of(
            self.trusted_edges.contains(peer.ip()),
        ))
        .knowing_calls(&knows_call)
        .strip(request);
        let request = unclaimed.as_ref().unwrap_or(request);

        // Resolve the tenant for this INVITE using the tiered strategy:
        //   1. Hostname domain → tenants.sip_domains allowlist
        //   2. Hostname domain → legacy tenant-name fallback (with warning)
        //   3. IP / localhost → default tenant only when the edge is effectively single-tenant
        //   4. No domain → default tenant only when the edge is effectively single-tenant
        // A real hostname that matches nothing still fails closed with 403.
        // A trusted edge placing a call for one of a tenant's users names
        // them; that is the call's tenant and caller.
        let edge_caller = self.trusted_edges.caller(request, peer.ip());
        // A leg this node placed to itself, continuing one of its calls (a
        // transfer or a forward): the call's tenant's, and admitted without
        // a digest — the node checked the caller when it placed the leg.
        // A call another node relays down a phone's flow this node holds
        // (`docs/PUSH.md` §2.4). The flow's token is this node's own, which
        // only a node that read the binding knows; it stands for the
        // phone's registration, which this node took. The call is that
        // registration's tenant's and goes to the phone and nowhere else; a
        // flow gone is RFC 5626's 430, so the sender can wake the phone.
        let flow_relay = match sip_parse::header(request.headers(), crate::push::FLOW_RELAY_HEADER)
        {
            None => None,
            Some(token) => {
                let contact =
                    sip_parse::header(request.headers(), crate::push::FLOW_RELAY_CONTACT_HEADER)
                        .map(|c| c.as_str().trim().to_string());
                let live = self
                    .flow_registry
                    .lookup(token.as_str())
                    .filter(|f| !f.stream.is_closed())
                    .and_then(|f| f.owner);
                match (live, contact) {
                    (Some((tenant, _aor)), Some(contact)) => Some(crate::push::FlowRelay {
                        tenant,
                        token: token.to_string(),
                        contact,
                        for_call: relay_call_id,
                    }),
                    _ => {
                        info!(source = %peer, "Refusing a relay down a flow this node does not hold");
                        let response = self.create_simple_response(request, 430, "Flow Failed");
                        handle.send_final(response).await;
                        return Ok(());
                    }
                }
            }
        };
        let own_leg_tenant = if let Some(relay) = &flow_relay {
            Some(relay.tenant)
        } else if edge_caller.is_none() {
            self.own_leg_tenant(request, peer)
        } else {
            None
        };
        // A registered device is itself, whoever else shares its address;
        // otherwise a carrier's call is whose the trunk it arrived on says
        // (`docs/TENANT_TRUNKS.md` §7), before the domain is looked at.
        let registered_tenant = if own_leg_tenant.is_some() {
            own_leg_tenant
        } else if edge_caller.is_none() {
            self.registered_caller_tenant(request, peer.ip()).await
        } else {
            None
        };
        let is_registered_device = registered_tenant.is_some();
        let inbound = if edge_caller.is_none() && !is_registered_device {
            self.inbound_trunk(request, peer).await
        } else {
            None
        };
        if let Some(crate::trunk_ingress::InboundChoice::Ambiguous) = &inbound {
            info!(
                source = %peer,
                "Rejecting INVITE: several tenants' trunks take calls from this source and the \
                 number is none of theirs"
            );
            let response = self.create_simple_response(request, 404, "Not Found");
            handle.send_final(response).await;
            return Ok(());
        }
        let inbound_trunk = inbound.as_ref().and_then(|c| c.trunk().cloned());
        // The rules the trunk names run on what it brought in; the call
        // sees the request as they leave it (`docs/TENANT_TRUNKS.md` §7).
        let rewritten = inbound_trunk.as_ref().and_then(|trunk| {
            crate::trunk_ingress::apply_ingress_rules(self.header_rules.as_ref(), trunk, request)
        });
        let request = rewritten.as_ref().unwrap_or(request);
        let trunk_tenant = match &inbound {
            Some(crate::trunk_ingress::InboundChoice::Tenant { tenant, .. }) => Some(*tenant),
            _ => None,
        };
        let tenant_id = if let Some(edge) = &edge_caller {
            debug!(
                source = %peer,
                tenant_id = %edge.tenant_id,
                caller = ?edge.username,
                site = ?edge.site_id,
                "INVITE placed by a trusted edge for a tenant user"
            );
            edge.tenant_id
        } else if let Some(tenant) = registered_tenant {
            debug!(
                source = %peer,
                tenant_id = %tenant,
                "INVITE placed by a registered device; the call is its registration's tenant's"
            );
            tenant
        } else if let (Some(tenant), Some(trunk)) = (trunk_tenant, &inbound_trunk) {
            debug!(
                source = %peer,
                tenant_id = %tenant,
                trunk = %trunk.name,
                "INVITE arrived on a trunk; the call is the trunk's tenant's"
            );
            tenant
        } else if let Some(domain) = extract_tenant_from_invite(request) {
            if fcp_core::is_host_literal(&domain) {
                // IP literal or localhost — use the default tenant directly.
                // Internal service-to-service INVITEs (e.g. call-manager →
                // conference server) legitimately use IP addresses in the
                // Request-URI.  Trunk admission control below still enforces
                // per-IP access for external sources.
                if let Some(default_tenant_id) = self.default_tenant_id {
                    debug!(
                        domain = %domain,
                        tenant_id = %default_tenant_id,
                        "Host literal in INVITE; using default tenant"
                    );
                    default_tenant_id
                } else {
                    warn!(
                        domain = %domain,
                        "Rejecting INVITE: host literal domain and no default tenant configured"
                    );
                    let response =
                        self.create_simple_response(request, 403, "Tenant context required");
                    handle.send_final(response).await;
                    return Ok(());
                }
            } else if let Some(storage) = &self.storage {
                // Tier 1: sip_domains allowlist.
                match storage.get_tenant_by_sip_domain(&domain).await {
                    Ok(Some(tenant)) => {
                        debug!(
                            domain = %domain,
                            tenant_id = %tenant.id,
                            tenant_name = %tenant.name,
                            "Resolved tenant from INVITE sip_domain"
                        );
                        tenant.id
                    }
                    Ok(None) => {
                        // Tier 2: legacy tenant-name fallback.
                        match storage.get_tenant_by_name(&domain).await {
                            Ok(Some(tenant)) => {
                                warn!(
                                    domain = %domain,
                                    tenant_id = %tenant.id,
                                    "INVITE tenant resolved via legacy name match; add '{}' to the tenant's sip_domains",
                                    domain
                                );
                                tenant.id
                            }
                            Ok(None) => {
                                warn!(
                                    domain = %domain,
                                    "Rejecting INVITE: unknown tenant domain"
                                );
                                let response = self.create_simple_response(
                                    request,
                                    403,
                                    "Unknown tenant domain",
                                );
                                handle.send_final(response).await;
                                return Ok(());
                            }
                            Err(e) => {
                                warn!(
                                    domain = %domain,
                                    error = %e,
                                    "Rejecting INVITE: tenant lookup failed"
                                );
                                let response = self.create_simple_response(
                                    request,
                                    503,
                                    "Tenant lookup unavailable",
                                );
                                handle.send_final(response).await;
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            domain = %domain,
                            error = %e,
                            "Rejecting INVITE: tenant lookup failed"
                        );
                        let response =
                            self.create_simple_response(request, 503, "Tenant lookup unavailable");
                        handle.send_final(response).await;
                        return Ok(());
                    }
                }
            } else if let Some(default_tenant_id) = self.default_tenant_fallback().await? {
                debug!(
                    domain = %domain,
                    tenant_id = %default_tenant_id,
                    "No tenant store configured; using default tenant for INVITE domain"
                );
                default_tenant_id
            } else {
                warn!(
                    domain = %domain,
                    "Rejecting INVITE: tenant domain present but no tenant store configured"
                );
                let response =
                    self.create_simple_response(request, 503, "Tenant resolution unavailable");
                handle.send_final(response).await;
                return Ok(());
            }
        } else if let Some(default_tenant_id) = self.default_tenant_fallback().await? {
            default_tenant_id
        } else {
            warn!("Rejecting INVITE: no tenant context could be derived from SIP addressing");
            let response = self.create_simple_response(request, 403, "Tenant context required");
            handle.send_final(response).await;
            return Ok(());
        };

        // Who placed this INVITE (`docs/EDGE_SECURITY.md` §2.1).
        //
        // A trusted edge vouches with proof of its own, and a carrier
        // arriving on a trunk holds no user's password, so neither is
        // challenged. Everything else must carry a digest where
        // `[registrar] auth_invites` is on — loopback included, since an
        // internal component is either a trusted edge or holds credentials
        // of its own. The digest then decides *instead of* the source
        // address: proving the password is stronger than arriving from a
        // plausible place, and a phone that has moved has not stopped
        // being itself.
        let peer_ip = peer.ip();
        let is_trusted_edge = edge_caller.is_some()
            || own_leg_tenant.is_some()
            || crate::own_leg::proves(request)
            || self.trusted_edges.contains(peer_ip);
        let invite_auth = match (&self.registrar, is_trusted_edge) {
            (Some(registrar), false) => registrar.authenticate_invite(request).await,
            _ => crate::registrar::InviteAuth::NotRequired,
        };
        match invite_auth {
            crate::registrar::InviteAuth::Admitted { username } => {
                debug!(source = %peer, username = %username, "INVITE authenticated");
            }
            crate::registrar::InviteAuth::WrongUser {
                authenticated,
                claimed,
            } => {
                // Real credentials wearing somebody else's From. A retry
                // cannot fix that, so it is refused rather than challenged.
                warn!(
                    source = %peer,
                    tenant_id = %tenant_id,
                    authenticated = %authenticated,
                    claimed = %claimed,
                    "Rejecting INVITE: the credentials are not the caller's"
                );
                let response = self.create_simple_response(
                    request,
                    403,
                    "Forbidden - credentials do not match the caller",
                );
                handle.send_final(response).await;
                return Ok(());
            }
            crate::registrar::InviteAuth::Challenge { stale } => {
                if self
                    .invite_on_trunk(&tenant_id, peer_ip, inbound_trunk.is_some())
                    .await
                {
                    debug!(source = %peer, tenant_id = %tenant_id, "INVITE admitted via trunk");
                } else {
                    info!(
                        source = %peer,
                        tenant_id = %tenant_id,
                        stale,
                        "Challenging INVITE: it carries no usable credentials"
                    );
                    let response = match self
                        .registrar
                        .as_ref()
                        .map(|r| r.invite_challenge(request, stale))
                    {
                        Some(Ok(response)) => response,
                        other => {
                            if let Some(Err(e)) = other {
                                error!(error = %e, "Failed to build an INVITE challenge");
                            }
                            self.create_simple_response(request, 500, "Internal Server Error")
                        }
                    };
                    handle.send_final(response).await;
                    return Ok(());
                }
            }
            // Nothing to prove: `auth_invites` is off, or this node has no
            // credentials to check one against. Admission is by source
            // address, as it was before (`docs/TENANT_TRUNKS.md` §7).
            crate::registrar::InviteAuth::NotRequired => {
                if self.storage.is_some() {
                    let local_ip: std::net::IpAddr = self
                        .local_addr
                        .parse()
                        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
                    let is_local = peer_ip == local_ip || peer_ip.is_loopback();

                    if !is_local && edge_caller.is_none() && !is_registered_device {
                        if !self
                            .invite_on_trunk(&tenant_id, peer_ip, inbound_trunk.is_some())
                            .await
                        {
                            info!(
                                source = %peer,
                                tenant_id = %tenant_id,
                                "Rejecting INVITE: source IP does not match any enabled trunk"
                            );
                            let response = self.create_simple_response(
                                request,
                                403,
                                "Forbidden - no trunk configured for source",
                            );
                            handle.send_final(response).await;
                            return Ok(());
                        }
                        debug!(source = %peer, tenant_id = %tenant_id, "INVITE admitted via trunk");
                    }
                }
            }
        }

        // Emergency calls (911 and the tenant's emergency/test numbers) are
        // detected before any admission control: no routing policy or rule
        // applies to them, they are never rejected for balance, and they
        // carry the caller's dispatchable location.
        let emergency = self
            .detect_emergency(&tenant_id, request, peer, edge_caller.as_ref())
            .await;
        if let Some(em) = &emergency {
            info!(
                tenant_id = %tenant_id,
                dialed = %em.dialed,
                kind = %em.kind,
                callback = ?em.callback_number,
                location = ?em.location.as_ref().map(|l| l.summary()),
                "Emergency call detected"
            );
        }

        // Check routing policy if routing service is configured: the
        // node's own, the platform's.
        if let (Some(routing_service), None) = (&self.routing_service, &emergency) {
            match self
                .evaluate_routing_policy(
                    request,
                    routing_service,
                    peer,
                    &tenant_id,
                    edge_caller.as_ref().and_then(|e| e.username.as_deref()),
                    // A site's call was placed by a phone the site already
                    // authenticated: no user of the cloud's to check.
                    inbound_trunk.is_some() || edge_caller.as_ref().is_some_and(|e| e.is_site()),
                )
                .await
            {
                Ok(RoutingVerdict::Refused(reason)) => {
                    info!(
                        tenant_id = %tenant_id,
                        reason = %reason,
                        "Call refused by the routing engine: the caller is not one of the tenant's users"
                    );
                    let response = self.create_simple_response(request, 403, &reason);
                    handle.send_final(response).await;
                    return Ok(());
                }
                Ok(RoutingVerdict::Decision(decision)) => {
                    if !decision.action.is_allow() {
                        let matched_rule_str = format!("{:?}", decision.matched_rule);
                        info!(
                            reason = %decision.reason,
                            matched_rule = %matched_rule_str,
                            "Call denied by routing policy"
                        );
                        let response = self.create_simple_response(request, 403, &decision.reason);
                        handle.send_final(response).await;
                        return Ok(());
                    }
                    let matched_rule_str = format!("{:?}", decision.matched_rule);
                    debug!(
                        reason = %decision.reason,
                        matched_rule = %matched_rule_str,
                        eval_time_us = decision.evaluation_time_us,
                        "Call allowed by routing policy"
                    );
                }
                Err(e) => {
                    error!("Routing policy evaluation failed: {}", e);
                    // Fail closed: deny the call on evaluation errors
                    let response = self.create_simple_response(
                        request,
                        500,
                        "Routing policy evaluation error",
                    );
                    handle.send_final(response).await;
                    return Ok(());
                }
            }
        }

        // Then the tenant's own rules (`docs/TENANT_TRUNKS.md` §12, phase
        // 6b): a deny refuses the call, a route_to_trunk confines it to the
        // trunk or group it names. Never an emergency call's.
        let trunk_target = match emergency {
            None => match self.tenant_rule_decision(&tenant_id, request, peer) {
                Ok(target) => target,
                Err(reason) => {
                    let response = self.create_simple_response(request, 403, &reason);
                    handle.send_final(response).await;
                    return Ok(());
                }
            },
            Some(_) => None,
        };

        // Balance enforcement: refuse calls the tenant cannot pay for, and
        // remember how long an affordable call may last. An emergency call
        // is never refused for balance.

        let mut billing_max_duration: Option<Duration> = None;
        // (A relayed leg is its call's node's to charge for.)
        if let (Some(billing), None, None) = (&self.billing, &emergency, &flow_relay) {
            let destination = sip_parse::header(request.headers(), "To")
                .map(|to| {
                    let to = to.as_str();
                    to.split('<')
                        .nth(1)
                        .and_then(|s| s.split('>').next())
                        .unwrap_or(to)
                        .trim()
                        .to_string()
                })
                .unwrap_or_default();
            match billing.admit(&tenant_id, &destination).await {
                fcp_billing::AdmissionDecision::Allow { max_duration } => {
                    billing_max_duration = max_duration;
                }
                fcp_billing::AdmissionDecision::Deny {
                    available,
                    required,
                    currency,
                } => {
                    info!(
                        tenant_id = %tenant_id,
                        destination = %destination,
                        available = %available,
                        required = %required,
                        currency = %currency,
                        "Rejecting INVITE: insufficient balance"
                    );
                    let response = self.create_simple_response(request, 402, "Payment Required");
                    handle.send_final(response).await;
                    return Ok(());
                }
            }
        }

        // Create Call object from the incoming INVITE with tenant context
        let mut call = fcp_b2bua_core::Call::from_invite(request.clone(), peer, tenant_id);
        call.state = fcp_b2bua_core::CallState::RoutingPending;
        call.a_leg_invite_tx = Some(handle.key().clone());
        call.max_duration = billing_max_duration;
        call.emergency = emergency.clone();
        call.trunk_target = trunk_target;
        // The record names the trunk an inbound call arrived on.
        if let Some(trunk) = &inbound_trunk {
            call.trunk_name = Some(trunk.name.clone());
            call.trunk_id = Some(trunk.trunk_id());
            call.arrived_on_trunk = Some(trunk.trunk_id());
            // Up a site's link, the call is the site's own, not a carrier's;
            // and the site is reached down the connection it came on, so
            // what this call sends the site — its BYE, a re-INVITE — goes
            // down that flow rather than to a contact behind NAT.
            call.link_site = trunk.site_id.filter(|_| trunk.is_link_peer());
            if call.link_site.is_some() {
                call.a_leg.flow_token = self
                    .link_peers
                    .flow_token_for_peer(peer, &self.flow_registry);
            }
        }
        // A caller transferred here carries the call it left, from the
        // Refer-To this B2BUA relayed (see handle_refer).
        call.correlation_id = sip_parse::header(request.headers(), TRANSFERRED_FROM_HEADER)
            .and_then(|v| v.as_str().trim().parse::<fcp_core::CallId>().ok());
        // A leg this node placed to itself for a forward names the call it
        // forwards for (believed only then, `claims`): its `Diversion`.
        call.forwarded_for = sip_parse::header(request.headers(), FORWARDED_FOR_HEADER)
            .and_then(|v| v.as_str().trim().parse::<fcp_core::CallId>().ok())
            .map(|id| fcp_b2bua_core::CallId::from_uuid(*id.as_uuid()));
        // A relayed leg: its call's node records and announces the call.
        if let Some(relay) = &flow_relay {
            call.relay_for = Some(relay.for_call.unwrap_or_default());
        }

        // Store call in CallManager
        let call_id = self.call_manager.create_call(call);
        // A CANCEL from now on finds the call; one that came before it was
        // kept by its branch.
        if self.early_cancels.remove(handle.key().branch()).is_some() {
            info!(call_id = %call_id, "The caller cancelled before the call was set up");
            let response = self.create_cancelled_response(request, None);
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call_with(&call_id, fcp_cdr::TerminationReason::Cancelled);
            return Ok(());
        }
        if let Some(relay) = &flow_relay {
            info!(call_id = %call_id, for_call = ?relay.for_call, "A call relayed down a flow this node holds");
            self.flow_relay_targets
                .insert(call_id, (relay.contact.clone(), relay.token.clone()));
        }

        // The caller's class of service (`docs/ENTERPRISE_CALLING.md` §3.7):
        // what a user may dial, and how many calls they may be on. Not an
        // emergency call's, not a carrier's or a site's (the site checks its
        // own), and not a leg this node placed to itself for a forward — the
        // forwarding user's class was checked before it was placed.
        let forwarded_leg = request.headers().contains(FORWARDED_FOR_HEADER);
        if emergency.is_none()
            && inbound_trunk.is_none()
            && !forwarded_leg
            && flow_relay.is_none()
            && !edge_caller.as_ref().is_some_and(|e| e.is_site())
        {
            let caller = edge_caller
                .as_ref()
                .and_then(|e| e.username.clone())
                .or_else(|| {
                    extract_from_uri(request)
                        .ok()
                        .and_then(|u| u.user().map(str::to_string))
                });
            if let Some(refusal) = self
                .class_of_service_refusal(request, &tenant_id, caller.as_deref())
                .await
            {
                info!(
                    call_id = %call_id,
                    tenant_id = %tenant_id,
                    caller = ?caller,
                    class = %refusal.class_name,
                    "Refusing INVITE: {}",
                    refusal.detail()
                );
                let mut response = self.create_simple_response(request, 403, "Forbidden");
                let _ = response.headers_mut().push(
                    SmolStr::new("Reason"),
                    SmolStr::new(refusal.reason_header()),
                );
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call_with(
                    &call_id,
                    fcp_cdr::TerminationReason::Failed {
                        status: Some(403),
                        detail: refusal.detail(),
                    },
                );
                return Ok(());
            }
        }

        // What this node and this tenant will carry
        // (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). Checked before the
        // work of a call, and never for an emergency call.
        if let (Some(admission), None, None) = (&self.admission, &emergency, &flow_relay) {
            if let Err(refusal) = admission.admit(&call_id.to_string(), &tenant_id).await {
                info!(
                    call_id = %call_id,
                    tenant_id = %tenant_id,
                    source = %peer,
                    reason = %refusal.as_str(),
                    "Refusing INVITE: {}",
                    refusal.detail()
                );
                let mut response = self.create_simple_response(request, 503, "Service Unavailable");
                let _ = response.headers_mut().push(
                    SmolStr::new("Retry-After"),
                    SmolStr::new(admission.retry_after_secs().to_string()),
                );
                let _ = response.headers_mut().push(
                    SmolStr::new("Reason"),
                    SmolStr::new(format!(
                        "SIP;cause=503;text=\"{}\"",
                        refusal.detail().to_ascii_lowercase()
                    )),
                );
                handle.send_final(response).await;
                let _ = self
                    .call_manager
                    .terminate_call(&call_id, Some(refusal.detail().to_string()));
                return Ok(());
            }
        }

        // A carrier's call counts against the trunk it arrived on, which may
        // be at its limit (`docs/TENANT_TRUNKS.md` §7). An emergency call is
        // never refused for it.
        if let (Some(trunk), Some(counter), None) = (&inbound_trunk, &self.trunk_calls, &emergency)
        {
            if let Err(limit) = counter.admit(&call_id.to_string(), trunk).await {
                let (reason, detail) = match limit {
                    crate::trunk_ingress::TrunkLimit::Concurrency => {
                        ("at its call limit", "Trunk at its concurrent call limit")
                    }
                    crate::trunk_ingress::TrunkLimit::CallsPerSecond => (
                        "over its calls per second",
                        "Trunk over its calls per second",
                    ),
                };
                info!(
                    call_id = %call_id,
                    trunk = %trunk.name,
                    source = %peer,
                    "Rejecting INVITE: the trunk it arrived on is {reason}"
                );
                let mut response = self.create_simple_response(request, 503, "Service Unavailable");
                let _ = response
                    .headers_mut()
                    .push(SmolStr::new("Retry-After"), SmolStr::new("1"));
                handle.send_final(response).await;
                let _ = self
                    .call_manager
                    .terminate_call(&call_id, Some(detail.to_string()));
                return Ok(());
            }
        }
        if let Some(max_duration) = billing_max_duration {
            self.spawn_balance_watchdog(call_id, tenant_id, max_duration);
        }
        if let Some(em) = &emergency {
            self.record_emergency_call(call_id, tenant_id, request, em, edge_caller.as_ref())
                .await;
        }

        // Register A-leg SIP Call-ID → FCP CallId mapping for trace correlation
        if let Some(sip_cid) = sip_parse::header(request.headers(), "Call-ID") {
            self.sip_call_id_map
                .insert(sip_cid.as_str().to_string(), call_id.to_string());
        }

        // Re-log the initial INVITE with the correct FCP CallId (the one logged in
        // handle_inbound used the SIP Call-ID because the call didn't exist yet)
        let call_id_str = call_id.to_string();
        let raw_invite_bytes = sip_parse::serialize_request(request);
        let raw_invite = String::from_utf8_lossy(&raw_invite_bytes);
        self.log_sip_message(
            "inbound",
            request.method().as_str(),
            Some(&call_id_str),
            None,
            sip_parse::header(request.headers(), "CSeq").map(|s| s.as_str()),
            Some(peer),
            Some("tcp"),
            Some(request.headers()),
            Some(request.body()),
            Some(&raw_invite),
        );

        // Log the outbound 100 Trying (reconstruct from request since `trying` was consumed)
        let trying_raw = format!(
            "SIP/2.0 100 Trying\r\n{}\r\nContent-Length: 0\r\n\r\n",
            request
                .headers()
                .iter()
                .filter(|h| {
                    let n = h.name();
                    n.eq_ignore_ascii_case("Via")
                        || n.eq_ignore_ascii_case("From")
                        || n.eq_ignore_ascii_case("To")
                        || n.eq_ignore_ascii_case("Call-ID")
                        || n.eq_ignore_ascii_case("CSeq")
                })
                .map(|h| format!("{}: {}", h.name(), h.value().trim()))
                .collect::<Vec<_>>()
                .join("\r\n")
        );
        self.log_sip_message(
            "outbound",
            "100",
            Some(&call_id_str),
            None,
            sip_parse::header(request.headers(), "CSeq").map(|s| s.as_str()),
            Some(peer),
            Some("tcp"),
            Some(request.headers()),
            None,
            Some(&trying_raw),
        );

        // Extract caller/callee for logging (to avoid non-Send tracing issues)
        let caller_str = sip_parse::header(request.headers(), "From")
            .map(|h| h.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let callee_str = sip_parse::header(request.headers(), "To")
            .map(|h| h.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        info!(
            call_id = %call_id,
            tenant_id = %tenant_id,
            caller = %caller_str,
            callee = %callee_str,
            "Created B2BUA call with tenant context"
        );

        // Evaluate media anchoring decision
        self.evaluate_media_anchoring(request, &call_id).await;

        // The user placing the call, if one of the tenant's is: the call
        // counts against them while it is up (`docs/ENTERPRISE_CALLING.md`
        // §3.4).
        // A relayed leg goes down its flow, and is nothing else here
        // (`docs/PUSH.md` §2.4).
        if flow_relay.is_some() {
            if let Some(uac) = &self.uac {
                return Box::pin(self.forward_invite_to_b_leg(
                    request,
                    handle,
                    call_id,
                    uac.clone(),
                ))
                .await;
            }
        }
        self.note_caller_user(request, &call_id).await;

        // A call the API redirected into a flow, dialed back by this node
        // with the redirect's one-time ticket (`docs/DEVELOPER_SURFACE.md`
        // phase 8): the flow runs for it, as the call's tenant's.
        let dialed = request.uri().to_string();
        let dialed_user = crate::class_of_service::uri_user(&dialed);
        if dialed_user.starts_with(crate::call_control::REDIRECT_PREFIX) {
            let Some(redirect) = self.redirect_tickets.redeem(dialed_user) else {
                let response = self.create_simple_response(request, 404, "Not Found");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call_with(
                    &call_id,
                    fcp_cdr::TerminationReason::failed("an unknown redirect".to_string()),
                );
                return Ok(());
            };
            let _ = self
                .call_manager
                .update_call(&call_id, |c| c.tenant_id = redirect.tenant_id);
            return self
                .start_flow_for_call(request, handle, call_id, redirect.flow_id, "")
                .await;
        }

        // An INVITE that replaces a leg of a call this node holds: an
        // attended transfer's second half, or a phone taking over a call
        // (RFC 3891, `docs/ENTERPRISE_CALLING.md` §3.2).
        if let Some(replaces) = sip_parse::header(request.headers(), "Replaces") {
            return self
                .handle_replaces(request, handle, &call_id, replaces.as_str())
                .await;
        }

        // A call from a shared line's phone takes an appearance of it, and
        // one naming a held appearance picks that call up (§3.9). A leg
        // this node placed to itself for a forward or a transfer is not a
        // phone's.
        if let Some(sca) = &self.sca {
            let own_leg = sip_parse::header(request.headers(), FORWARDED_FOR_HEADER).is_some()
                || sip_parse::header(request.headers(), TRANSFERRED_FROM_HEADER).is_some();
            if !own_leg {
                match Box::pin(sca.outbound(request, call_id)).await {
                    crate::sca::Outbound::NotShared => {}
                    crate::sca::Outbound::Claimed(appearance) => {
                        debug!(call_id = %call_id, line = %appearance.number, index = appearance.index, "A call from a shared line");
                        let _ = self
                            .call_manager
                            .update_call(&call_id, |c| c.appearance = Some(appearance));
                    }
                    crate::sca::Outbound::RetrieveHere(held) => {
                        return match self.call_manager.get_call(&held) {
                            Some(call) => {
                                Box::pin(self.retrieve_appearance(request, handle, &call_id, &call))
                                    .await
                            }
                            None => {
                                let response = self.create_simple_response(
                                    request,
                                    481,
                                    "Call/Transaction Does Not Exist",
                                );
                                handle.send_final(response).await;
                                let _ = self
                                    .call_manager
                                    .terminate_call(&call_id, Some("retrieve: no call".into()));
                                Ok(())
                            }
                        };
                    }
                    crate::sca::Outbound::RetrieveThere { node_uri, handoff } => {
                        if let Some(uac) = &self.uac {
                            info!(call_id = %call_id, node = %node_uri, "Retrieving a held appearance on another node");
                            self.sca_handoff_targets
                                .insert(call_id, (node_uri, handoff));
                            return Box::pin(self.forward_invite_to_b_leg(
                                request,
                                handle,
                                call_id,
                                uac.clone(),
                            ))
                            .await;
                        }
                    }
                    crate::sca::Outbound::Refuse(code, reason) => {
                        info!(call_id = %call_id, code, reason, "A call from a shared line refused");
                        let response = self.create_simple_response(request, code, reason);
                        handle.send_final(response).await;
                        let _ = self.call_manager.terminate_call_with(
                            &call_id,
                            fcp_cdr::TerminationReason::rejected(code, reason),
                        );
                        return Ok(());
                    }
                }
            }
        }

        // An emergency test call the tenant has this platform answer: the
        // node reads back what a live call would carry, and no carrier is
        // called (`docs/E911.md` "Test calls").
        // Boxed and apart: this future is deep, and what a branch holds
        // across an await here overflowed a debug worker's stack.
        if let Some(done) =
            Box::pin(self.read_back_local_test(request, handle, &call_id, tenant_id)).await
        {
            return done;
        }

        // Check if this is a PBX feature code (e.g., *8 pickup, *70 park, *81 intercom)
        if let Some(pbx) = &self.pbx_handler {
            if pbx.is_enabled() {
                // Extract the dialed string from the Request-URI
                let dialed = request.start_line().uri().to_string();
                if let Some(action) = pbx.parse_feature_code(&dialed) {
                    info!(
                        call_id = %call_id,
                        action = ?action,
                        "PBX feature code detected"
                    );
                    return self
                        .handle_pbx_feature_code(request, handle, &call_id, action)
                        .await;
                }
                // A lot's park number or one of its slots (`docs/PARKING.md`
                // §3). Boxed and apart, as the test-call read-back is: this
                // future is deep, and what it holds across an await here
                // overflowed the SBC's worker stack (fcp-link's calls test).
                if let Some(done) =
                    Box::pin(self.park_by_number(request, handle, &call_id, tenant_id, &dialed))
                        .await
                {
                    return done;
                }
            }
        }

        // Check if a conference handler wants this call (before flow routing)
        if let Some(conf_handler) = &self.conference_handler {
            if conf_handler.can_handle(request, peer).await {
                debug!(call_id = %call_id, "Routing to conference handler");
                // The room keeps its own record of the leg; this call object
                // would otherwise outlive it.
                self.call_manager.discard_call(&call_id);
                return conf_handler
                    .handle_invite(request, handle, peer, peer_identity)
                    .await;
            }
        }

        // Check if a queue handler wants this call (before flow routing)
        if let Some(queue_handler) = &self.queue_handler {
            if queue_handler.can_handle(request).await {
                // The queue's hours, decided here where a closed caller can
                // be sent on (`docs/ENTERPRISE_CALLING.md` §3.5).
                if let Some((state, target)) = self.queue_schedule_divert(request, &call_id).await {
                    let a_leg = Arc::new(handle.clone());
                    let callee = request.start_line().uri().to_string();
                    return self
                        .divert_for_schedule(request, &a_leg, call_id, state, target, &callee)
                        .await;
                }
                debug!(call_id = %call_id, "Routing to queue handler");
                let result = queue_handler
                    .handle_invite(request, handle, peer, call_id)
                    .await;
                if let Err(e) = &result {
                    // The queue ends the call when it refuses or finishes;
                    // an error left it to us.
                    let _ = self.call_manager.terminate_call_with(
                        &call_id,
                        fcp_cdr::TerminationReason::failed(format!("Queue: {e}")),
                    );
                }
                return result;
            }
        }

        // Check if there's a flow configured for this DID
        if let Some(flow_handler) = &self.flow_handler {
            // Extract To URI (DID) from request
            if let Some(to_header) = sip_parse::header(request.headers(), "To") {
                // Extract DID from To: <sip:+15551234567@domain>
                let did = Self::extract_did_from_uri(to_header.as_str());

                if let Some(did) = did {
                    debug!(call_id = %call_id, did = %did, "Checking for flow routing");

                    // Try To header DID first, then Request-URI user part, then _default
                    let flow_id_opt = if let Some(fid) = flow_handler.get_flow_for_did(&did).await {
                        Some(fid)
                    } else {
                        // Try Request-URI user part (for CFNA→voicemail routing where
                        // the callee is in the R-URI but not necessarily in To)
                        let ruri_did = Self::extract_did_from_uri(&request.uri().to_string());
                        let ruri_match = match ruri_did {
                            Some(ref user) if user != &did => {
                                flow_handler.get_flow_for_did(user).await
                            }
                            _ => None,
                        };
                        // Fall back to _default route
                        match ruri_match {
                            Some(fid) => Some(fid),
                            None => flow_handler.get_flow_for_did("_default").await,
                        }
                    };

                    if let Some(flow_id) = flow_id_opt {
                        info!(
                            call_id = %call_id,
                            did = %did,
                            flow_id = %flow_id,
                            "Starting flow execution for inbound call"
                        );

                        // Flows need media anchored for prompt playback
                        // and DTMF detection. Ensure a session exists.
                        if let Some(call_entry) = self.call_manager.get_call(&call_id) {
                            if call_entry.media_session_id.is_none() {
                                if let Some(ref media_manager) = self.media_manager {
                                    let fcp_call_id = fcp_core::CallId::new();
                                    let fcp_dialog_id = fcp_core::DialogId::new();
                                    match media_manager
                                        .create_session_default(fcp_call_id, fcp_dialog_id)
                                        .await
                                    {
                                        Ok(_session) => {
                                            if let Some(a_leg_sdp) =
                                                crate::body::request_sdp(request)
                                            {
                                                if let Err(e) = media_manager
                                                    .update_session(&fcp_call_id, a_leg_sdp)
                                                    .await
                                                {
                                                    warn!(call_id = %call_id, error = %e, "Failed to set A-leg media endpoint for flow");
                                                }
                                            }
                                            let _ =
                                                self.call_manager.update_call(&call_id, |call| {
                                                    call.media_session_id = Some(fcp_call_id);
                                                    call.media_anchored = true;
                                                });
                                            info!(call_id = %call_id, "Created media session for flow (was not anchored)");
                                        }
                                        Err(e) => {
                                            error!(call_id = %call_id, error = %e, "Failed to create media session for flow");
                                        }
                                    }
                                }
                            }
                        }

                        // Build initial variables from call context
                        let mut initial_variables = std::collections::HashMap::new();

                        // Extract From URI (caller number and display name)
                        if let Some(from_header) = sip_parse::header(request.headers(), "From") {
                            let from_str = from_header.as_str();
                            if let Some(caller) = Self::extract_did_from_uri(from_str) {
                                initial_variables
                                    .insert("caller_number".to_string(), caller.clone());
                                initial_variables.insert("caller".to_string(), caller);
                            }
                            // Extract display name: "Display Name" <sip:...>
                            if let Some(display_name) = Self::extract_display_name(from_str) {
                                initial_variables.insert("caller_name".to_string(), display_name);
                            }
                        }

                        initial_variables.insert("called_number".to_string(), did.clone());

                        // Extract Call-ID for correlation
                        if let Some(sip_call_id) = sip_parse::header(request.headers(), "Call-ID") {
                            initial_variables
                                .insert("sip_call_id".to_string(), sip_call_id.to_string());
                        }

                        // Extract reason parameter from Request-URI for busy/no-answer greeting selection
                        let ruri_str = request.uri().to_string();
                        if let Some(pos) = ruri_str.find(";reason=") {
                            let val = &ruri_str[pos + 8..];
                            let val = val.split(';').next().unwrap_or(val);
                            initial_variables
                                .insert("is_busy".to_string(), (val == "busy").to_string());
                        }

                        // Create callbacks for Answer and Hangup nodes
                        let answer_handle = handle.clone();
                        let answer_request = request.clone();
                        let answer_call_id = call_id;
                        let answer_call_manager = Arc::clone(&self.call_manager);
                        let answer_media_manager = self.media_manager.clone();
                        let answer_uas = self.uas.clone();
                        let answer_dialog_manager = Arc::clone(&self.dialog_manager);
                        let answer_ack_waits = Arc::clone(&self.ack_waits);
                        let answer_callback: fcp_flow::AnswerCallback = Arc::new(move || {
                            let handle = answer_handle.clone();
                            let request = answer_request.clone();
                            let call_id = answer_call_id;
                            let call_manager = Arc::clone(&answer_call_manager);
                            let media_manager = answer_media_manager.clone();
                            let uas = answer_uas.clone();
                            let dialog_manager = Arc::clone(&answer_dialog_manager);
                            let ack_waits = Arc::clone(&answer_ack_waits);
                            Box::pin(async move {
                                info!(call_id = %call_id, "Answer callback executing - creating 200 OK");
                                // Create base 200 OK response
                                let mut response =
                                    sip_uas::UserAgentServer::create_response(&request, 200, "OK");

                                // Add required dialog-establishing headers (To tag, Contact, Record-Route)
                                add_response_headers(&request, &mut response, &uas);

                                // If media is anchored, we need to add SDP to the response
                                if let Some(media_mgr) = media_manager {
                                    // Get the call to retrieve media_session_id
                                    if let Some(call) = call_manager.get_call(&call_id) {
                                        // If we have a media session, negotiate SDP
                                        if let Some(ref media_session_id) = call.media_session_id {
                                            // Extract remote SDP from INVITE body
                                            if let Some(remote_sdp) =
                                                crate::body::request_sdp(&request)
                                            {
                                                // Update media session with remote SDP and get local SDP
                                                match media_mgr
                                                    .update_session(media_session_id, remote_sdp)
                                                    .await
                                                {
                                                    Ok(local_sdp) => {
                                                        // Rebuild response with SDP body
                                                        let sdp_bytes =
                                                            bytes::Bytes::from(local_sdp);

                                                        // Clone headers and add Content-Type and Content-Length
                                                        let mut headers =
                                                            response.headers().clone();
                                                        let _ = headers.push(
                                                            smol_str::SmolStr::new("Content-Type"),
                                                            smol_str::SmolStr::new(
                                                                "application/sdp",
                                                            ),
                                                        );
                                                        let _ = headers.push(
                                                            smol_str::SmolStr::new(
                                                                "Content-Length",
                                                            ),
                                                            smol_str::SmolStr::new(
                                                                sdp_bytes.len().to_string(),
                                                            ),
                                                        );

                                                        // Create new response with SDP
                                                        response = sip_core::Response::new(
                                                            response.start_line().clone(),
                                                            headers,
                                                            sdp_bytes,
                                                        )
                                                        .expect(
                                                            "Failed to create response with SDP",
                                                        );

                                                        info!(call_id = %call_id, "Added SDP to 200 OK response");
                                                    }
                                                    Err(e) => {
                                                        error!(call_id = %call_id, error = %e, "Failed to update media session with SDP");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // The dialog before the 200 OK: the caller may hang up
                                // the moment it sees the answer, and a BYE that arrives
                                // before the dialog is on the call finds nothing to end
                                // (`find_by_dialog_call_id`) — the call is left for the
                                // flow to trip over and its record is never written.
                                if let Some(a_leg_dialog) = sip_dialog::Dialog::new_uas(
                                    &request,
                                    &response,
                                    uas.local_uri.clone(),
                                    extract_from_uri(&request).unwrap_or_else(|_| {
                                        sip_core::SipUri::parse("sip:unknown@unknown").unwrap()
                                    }),
                                ) {
                                    let dialog_id_str = format!("{:?}", a_leg_dialog.id());
                                    info!(call_id = %call_id, dialog_id = %dialog_id_str, "Created A-leg dialog");

                                    // Insert dialog into dialog manager
                                    let _ = dialog_manager.insert(a_leg_dialog.clone());

                                    // Update call with dialog. `establish` and not the
                                    // state alone: the answer time is what makes the
                                    // record an answered call with a duration, and a
                                    // flow answering is an answer like any other.
                                    let _ = call_manager.update_call(&call_id, |call| {
                                        call.a_leg.dialog = Some(a_leg_dialog.clone());
                                        call.establish();
                                    });

                                    info!(call_id = %call_id, "A-leg dialog stored in call");
                                } else {
                                    error!(call_id = %call_id, "Failed to create A-leg dialog from 200 OK");
                                }

                                // Send 200 OK
                                ack_waits.expect(call_id);
                                handle.send_final(response.clone()).await;

                                Ok(())
                            })
                        });

                        let hangup_call_id = call_id;
                        let hangup_call_manager = Arc::clone(&self.call_manager);
                        let hangup_uac = self.uac.clone();
                        let hangup_dialog_manager = Arc::clone(&self.dialog_manager);
                        let hangup_ack_waits = Arc::clone(&self.ack_waits);
                        let hangup_media_manager = self.media_manager.clone();
                        let hangup_callback: fcp_flow::HangupCallback = Arc::new(move |reason| {
                            let call_id = hangup_call_id;
                            let call_manager = Arc::clone(&hangup_call_manager);
                            let uac_opt = hangup_uac.clone();
                            let dialog_manager = Arc::clone(&hangup_dialog_manager);
                            let ack_waits = Arc::clone(&hangup_ack_waits);
                            let media_manager = hangup_media_manager.clone();
                            Box::pin(async move {
                                // Not before the caller has ACKed the answer (RFC 3261 §15).
                                ack_waits.wait(&call_id, crate::ack_wait::ACK_WAIT).await;
                                info!(call_id = %call_id, reason = ?reason, "Hangup callback executing");
                                // Get the dialog for this call
                                if let Some(call_entry) = call_manager.get_call(&call_id) {
                                    info!(call_id = %call_id, "Found call entry, checking for dialog");

                                    // Terminate media session if anchored
                                    if call_entry.media_anchored {
                                        if let (Some(ref media_session_id), Some(ref media_mgr)) =
                                            (&call_entry.media_session_id, &media_manager)
                                        {
                                            match media_mgr
                                                .terminate_session(media_session_id)
                                                .await
                                            {
                                                Ok(()) => {
                                                    info!(
                                                        call_id = %call_id,
                                                        media_session_id = %media_session_id,
                                                        "Terminated media session (hangup callback)"
                                                    );
                                                }
                                                Err(e) => {
                                                    error!(
                                                        call_id = %call_id,
                                                        media_session_id = %media_session_id,
                                                        error = %e,
                                                        "Failed to terminate media session (hangup callback)"
                                                    );
                                                }
                                            }
                                        }
                                    }

                                    if let Some(dialog) = call_entry.a_leg.dialog.clone() {
                                        let dialog_id_str = format!("{:?}", dialog.id());
                                        info!(call_id = %call_id, dialog_id = %dialog_id_str, "Found A-leg dialog, sending BYE");
                                        // Send BYE request if UAC is available
                                        if let Some(uac) = uac_opt {
                                            match uac.bye(&dialog).await {
                                                Ok(_) => {
                                                    info!(call_id = %call_id, "BYE sent successfully");
                                                    // Remove dialog from manager
                                                    dialog_manager.remove(dialog.id());
                                                }
                                                Err(e) => {
                                                    error!(call_id = %call_id, error = %e, "Failed to send BYE");
                                                }
                                            }
                                        } else {
                                            warn!(call_id = %call_id, "UAC not available, cannot send BYE");
                                        }
                                    } else {
                                        warn!(call_id = %call_id, "No A-leg dialog found, cannot send BYE");
                                    }
                                } else {
                                    warn!(call_id = %call_id, "Call entry not found in call manager");
                                }

                                // Terminate the call normally (Ended, not Failed).
                                // Flow-level hangup reasons are application logic,
                                // not SIP failures — use None so CallEvent::Ended
                                // fires and presence is cleaned up reliably.
                                let _ = call_manager.terminate_call(&call_id, None);
                                Ok(())
                            })
                        });

                        // Get mutable call reference and start flow
                        if let Some(mut call_entry) = self.call_manager.get_call(&call_id) {
                            match flow_handler
                                .start_flow(
                                    &mut call_entry,
                                    flow_id,
                                    initial_variables,
                                    answer_callback,
                                    hangup_callback,
                                )
                                .await
                            {
                                Ok(()) => {
                                    info!(call_id = %call_id, "Flow execution started successfully");
                                    // Flow will handle the call - don't forward to B-leg
                                    // Send 183 Session Progress to indicate flow is starting
                                    let mut progress = sip_uas::UserAgentServer::create_response(
                                        request,
                                        183,
                                        "Session Progress",
                                    );
                                    self.ensure_response_hygiene(request, &mut progress);
                                    handle.send_provisional(progress).await;
                                    return Ok(());
                                }
                                Err(e) => {
                                    error!(
                                        call_id = %call_id,
                                        error = %e,
                                        "Failed to start flow execution, falling back to normal routing"
                                    );
                                    // Fall through to normal B2BUA routing
                                }
                            }
                        }
                    }
                }
            }
        }

        // If UAC is available, forward to B-leg; otherwise use UAS mode
        if let Some(uac) = &self.uac {
            // Full B2BUA mode: Forward INVITE to B-leg
            self.forward_invite_to_b_leg(request, handle, call_id, uac.clone())
                .await
        } else {
            // Fallback UAS mode: Accept call directly without B-leg forwarding
            self.handle_invite_uas_mode(request, handle, call_id).await
        }
    }

    /// Evaluate whether to anchor media for this call.
    ///
    /// This method analyzes the call's requirements and makes a decision on whether
    /// RTP should flow through the server (anchored) or directly between endpoints (pass-through).
    ///
    /// Decision factors:
    /// - Global media.enabled flag
    /// - Codec compatibility between endpoints
    /// - Recording requirements
    /// - DTMF detection/injection needs
    /// - Conference call requirements
    async fn evaluate_media_anchoring(
        &self,
        request: &sip_core::Request,
        call_id: &fcp_b2bua_core::CallId,
    ) {
        // The caller's preferences, and whether the call is to be recorded
        // without anyone asking: it must be anchored for that, and records
        // once it is answered (docs/COMPLIANCE_FOLLOWUPS.md §8.1).
        let user_prefs = if let Some(ref registrar) = self.registrar {
            let from_aor = sip_parse::header(request.headers(), "From")
                .and_then(|from| crate::presence::extract_aor(from.as_str()));

            if let Some(aor) = from_aor {
                registrar.get_user_media_preferences(&aor).await
            } else {
                fcp_core::MediaPreferences::default()
            }
        } else {
            fcp_core::MediaPreferences::default()
        };
        let auto_recording = self
            .automatic_recording(request, call_id, &user_prefs)
            .await;
        if let Some(source) = auto_recording {
            info!(call_id = %call_id, source = source.as_str(), "Call to be recorded once answered");
            let _ = self
                .call_manager
                .update_call(call_id, |call| call.auto_recording = Some(source));
        }

        // Policy short-circuit: B2buaConfig media anchoring overrides
        if self.b2bua_config.media_anchoring.force_anchor() {
            info!(call_id = %call_id, "Media anchoring forced by B2BUA policy (Always mode)");
            // Mark call as media-anchored and create a media session
            if let (Some(media_manager), Some(_)) = (&self.media_manager, &self.media_config) {
                let fcp_call_id = fcp_core::CallId::new();
                let fcp_dialog_id = fcp_core::DialogId::new();
                match media_manager
                    .create_session_default(fcp_call_id, fcp_dialog_id)
                    .await
                {
                    Ok(_session) => {
                        // Set A-leg remote endpoint from INVITE SDP
                        if let Some(a_leg_sdp) = crate::body::request_sdp(request) {
                            if let Err(e) =
                                media_manager.update_session(&fcp_call_id, a_leg_sdp).await
                            {
                                warn!(call_id = %call_id, error = %e, "Failed to set A-leg media endpoint");
                            }
                        }

                        let _ = self.call_manager.update_call(call_id, |call| {
                            call.media_session_id = Some(fcp_call_id);
                            call.media_anchored = true;
                        });
                    }
                    Err(e) => {
                        error!(
                            call_id = %call_id,
                            error = %e,
                            "Failed to create media session for forced anchoring, falling back"
                        );
                    }
                }
            } else {
                // No media manager — just mark call as anchored
                let _ = self.call_manager.update_call(call_id, |call| {
                    call.media_anchored = true;
                });
            }
            return;
        }
        if self.b2bua_config.media_anchoring.force_passthrough() {
            debug!(call_id = %call_id, "Media pass-through forced by B2BUA policy (Never mode)");
            if let Some(source) = auto_recording {
                warn!(
                    call_id = %call_id,
                    source = source.as_str(),
                    "Call not recorded: media anchoring is off (Never mode), so there is nothing to record"
                );
            }
            return;
        }
        // Conditional mode — fall through to existing logic

        // Check if media manager and config are available
        let (media_manager, media_config) = match (&self.media_manager, &self.media_config) {
            (Some(mm), Some(mc)) => (mm, mc),
            _ => {
                debug!(call_id = %call_id, "Media manager not available, using pass-through");
                return;
            }
        };

        // Where a caller behind NAT was seen: its RTP is learned from there.
        let caller_behind_nat = self
            .call_manager
            .get_call(call_id)
            .and_then(|c| c.a_leg.contact)
            .map(|peer| peer.ip())
            .filter(|peer| {
                crate::body::request_sdp(request)
                    .is_some_and(|sdp| crate::nat::learns_media_address(&sdp, *peer))
            });

        // Extract codecs from A-leg SDP
        let a_leg_codecs = if let Some(sdp) = crate::body::request_sdp(request) {
            fcp_media::extract_codecs_from_sdp(&sdp)
        } else {
            Vec::new()
        };

        debug!(
            call_id = %call_id,
            codecs = ?a_leg_codecs,
            "Extracted codecs from A-leg SDP"
        );

        // Build decision context using user preferences. The node's
        // `enable_recording` is one of the reasons a call is recorded, so
        // it does not gate the others.
        let ctx = fcp_media::MediaDecisionContext {
            media_enabled: media_config.enabled,
            transcoding_enabled: media_config.enable_transcoding,
            recording_enabled: true,
            dtmf_enabled: media_config.enable_dtmf,
            a_leg_codecs,
            b_leg_codecs: None, // B-leg not known yet
            recording_requested: auto_recording.is_some(),
            dtmf_requested: user_prefs.enable_dtmf_detection || user_prefs.force_media_anchoring,
            conference_call: false, // TODO: Detect conference scenarios
            hold_music: media_config.hold_music,
            emergency_call: self
                .call_manager
                .get_call(call_id)
                .is_some_and(|c| c.emergency.is_some()),
            caller_behind_nat: caller_behind_nat.is_some(),
        };

        // Make anchoring decision
        let decision = fcp_media::should_anchor_media(&ctx);

        match decision {
            fcp_media::MediaAnchoringDecision::PassThrough { reason } => {
                info!(
                    call_id = %call_id,
                    reason = %reason,
                    "Media decision: Pass-through (RTP flows directly)"
                );
                // No action needed - existing behavior continues
            }
            fcp_media::MediaAnchoringDecision::Anchored {
                reason,
                requires_transcoding,
            } => {
                info!(
                    call_id = %call_id,
                    reason = %reason,
                    transcoding = requires_transcoding,
                    "Media decision: Anchored (RTP flows through server)"
                );

                // Create FCP-internal CallId for media session
                // Note: This is different from SIP Call-ID
                let fcp_call_id = fcp_core::CallId::new();

                // Create FCP DialogId for the session
                let fcp_dialog_id = fcp_core::DialogId::new();

                // Create media session
                match media_manager
                    .create_session_default(fcp_call_id, fcp_dialog_id)
                    .await
                {
                    Ok(session) => {
                        let session_state = session.state().await;
                        info!(
                            call_id = %call_id,
                            media_session_id = %fcp_call_id,
                            state = ?session_state,
                            "Created media session"
                        );

                        // Set A-leg remote endpoint from INVITE SDP
                        if let Some(a_leg_sdp) = crate::body::request_sdp(request) {
                            if let Err(e) =
                                media_manager.update_session(&fcp_call_id, a_leg_sdp).await
                            {
                                warn!(call_id = %call_id, error = %e, "Failed to set A-leg media endpoint");
                            }
                        }

                        // Update call object to track media session
                        let _ = self.call_manager.update_call(call_id, |call| {
                            call.media_session_id = Some(fcp_call_id);
                            call.media_anchored = true;
                        });
                        if let Some(peer) = caller_behind_nat {
                            self.latch_leg(call_id, fcp_media::backend::Leg::A, peer)
                                .await;
                        }
                    }
                    Err(e) => {
                        error!(
                            call_id = %call_id,
                            error = %e,
                            "Failed to create media session, falling back to pass-through"
                        );
                        // Fall back to pass-through on error
                    }
                }
            }
        }
    }

    /// Why a call is to be recorded without anyone asking
    /// (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1): `record_all_calls` on either
    /// party's user, `recording_enabled` on the dialled or calling number,
    /// or the node's `enable_recording`, in that order. The callee is the
    /// user or number the INVITE dialled — by name, extension or number —
    /// not whoever the call is later forwarded to.
    async fn automatic_recording(
        &self,
        request: &sip_core::Request,
        call_id: &fcp_b2bua_core::CallId,
        caller_prefs: &fcp_core::MediaPreferences,
    ) -> Option<fcp_core::recording::CallRecordingSource> {
        use fcp_core::recording::CallRecordingSource;

        let platform = self
            .media_config
            .as_ref()
            .is_some_and(|m| m.enable_recording);
        let tenant = self.call_manager.get_call(call_id)?.tenant_id;
        let header_user = |name: &str| {
            sip_parse::header(request.headers(), name)
                .and_then(|h| crate::presence::extract_aor(h.as_str()))
                .and_then(|aor| aor.split('@').next().map(str::to_string))
        };
        if caller_prefs.record_all_calls {
            return Some(CallRecordingSource::User);
        }
        let Some(storage) = &self.storage else {
            return platform.then_some(CallRecordingSource::Platform);
        };
        let max_digits = self.b2bua_config.max_extension_digits;
        let number_in_tenant =
            |number: fcp_core::PhoneNumber| (number.tenant_id == tenant).then_some(number);
        let by_e164 = |user: String| {
            let storage = Arc::clone(storage);
            async move {
                let normalized = normalize_dialed_number(&user, max_digits);
                if !is_e164_number(&normalized) {
                    return None;
                }
                storage
                    .get_phone_number_by_e164(&normalized)
                    .await
                    .ok()
                    .flatten()
            }
        };

        // The dialled user and number.
        let mut dialled_user: Option<fcp_core::storage::User> = None;
        let mut dialled_number: Option<fcp_core::PhoneNumber> = None;
        if let Some(to_user) = header_user("To") {
            if let Ok(Some(user)) = storage.get_user_by_sip_name(&to_user, &tenant).await {
                dialled_user = Some(user);
            } else if let Ok(Some(extension)) =
                storage.get_extension_by_number(&tenant, &to_user).await
            {
                match &extension.target {
                    fcp_core::ExtensionTarget::User(id) => {
                        dialled_user = storage.get_user_by_id(id).await.ok().flatten();
                    }
                    fcp_core::ExtensionTarget::Device(id) => {
                        if let Ok(Some(device)) = storage.get_device(id).await {
                            if let Some(owner) = device.owner_user_id {
                                dialled_user = storage.get_user_by_id(&owner).await.ok().flatten();
                            }
                        }
                    }
                    fcp_core::ExtensionTarget::PhoneNumber(id) => {
                        dialled_number = storage.get_phone_number(id).await.ok().flatten();
                    }
                    fcp_core::ExtensionTarget::Service(_) => {}
                }
            } else {
                dialled_number = by_e164(to_user).await;
            }
        }
        let dialled_number = dialled_number.and_then(number_in_tenant);
        if dialled_user.is_none() {
            if let Some(number) = &dialled_number {
                if let Ok(Some(binding)) =
                    storage.get_line_binding_by_phone_number(&number.id).await
                {
                    if let Some(owner) = binding.owner_user_id {
                        dialled_user = storage.get_user_by_id(&owner).await.ok().flatten();
                    }
                }
            }
        }
        if dialled_user
            .filter(|u| u.tenant_id == tenant)
            .is_some_and(|u| u.media_preferences.record_all_calls)
        {
            return Some(CallRecordingSource::User);
        }
        if dialled_number.is_some_and(|n| n.recording_enabled) {
            return Some(CallRecordingSource::Number);
        }
        if let Some(from_user) = header_user("From") {
            if by_e164(from_user)
                .await
                .and_then(number_in_tenant)
                .is_some_and(|n| n.recording_enabled)
            {
                return Some(CallRecordingSource::Number);
            }
        }
        platform.then_some(CallRecordingSource::Platform)
    }

    /// Name the call in the answer to a caller who is one of the tenant's
    /// users, as the INVITE to a phone names it: an app asks for its own
    /// call by that id (`/me/calls/{id}/recording`, `docs/RECORD_AND_CONTACTS.md`
    /// §2). A carrier is not told.
    fn name_call_for_caller(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        response: &mut sip_core::Response,
    ) {
        let named = self
            .call_manager
            .get_call(call_id)
            .filter(|c| c.caller_user_id.is_some())
            .map(|c| {
                c.relay_for
                    .map(|r| r.as_uuid().to_string())
                    .unwrap_or_else(|| c.id.to_string())
            });
        if let Some(named) = named {
            response.headers_mut().remove(fcp_core::CALL_ID_HEADER);
            let _ = response
                .headers_mut()
                .push(SmolStr::new(fcp_core::CALL_ID_HEADER), SmolStr::new(named));
        }
    }

    fn ensure_response_hygiene(
        &self,
        request: &sip_core::Request,
        response: &mut sip_core::Response,
    ) {
        const ALLOW: &str =
            "INVITE, ACK, CANCEL, BYE, UPDATE, PRACK, INFO, REFER, SUBSCRIBE, NOTIFY, OPTIONS";
        const SUPPORTED: &str = "100rel, timer";

        // Ensure To-tag present for dialog-establishing responses
        // RFC 3261 §12.1.2: Only responses >= 101 establish dialogs (100 Trying does not)
        let status_code = response.start_line().code();
        if status_code >= 101 {
            if let Some(to) = response.headers().get("To") {
                if !to.contains(";tag=") {
                    let tag = Uuid::new_v4().simple().to_string();
                    let new_to = SmolStr::new(format!("{};tag={}", to, tag));
                    response.headers_mut().remove("To");
                    let _ = response.headers_mut().push(SmolStr::new("To"), new_to);
                }
            }
        }

        // INVITE-related responses should carry Contact/Record-Route
        if request.method() == &sip_core::Method::Invite {
            if response.headers().get("Contact").is_none() {
                let _ = response.headers_mut().push(
                    SmolStr::new("Contact"),
                    SmolStr::new(format!("<{}>", self.uas.contact_uri.as_str())),
                );
            }
            if response.headers().get("Record-Route").is_none() {
                let _ = response.headers_mut().push(
                    SmolStr::new("Record-Route"),
                    SmolStr::new(format!("<{};lr>", self.uas.contact_uri.as_str())),
                );
            }
        }

        // Capability headers
        if response.headers().get("Allow").is_none() {
            let _ = response
                .headers_mut()
                .push(SmolStr::new("Allow"), SmolStr::new(ALLOW));
        }
        if response.headers().get("Supported").is_none() {
            let _ = response
                .headers_mut()
                .push(SmolStr::new("Supported"), SmolStr::new(SUPPORTED));
        }

        // Ensure Content-Length matches body
        let body_len = response.body().len();
        response.headers_mut().remove("Content-Length");
        let _ = response.headers_mut().push(
            SmolStr::new("Content-Length"),
            SmolStr::new(body_len.to_string()),
        );
    }

    fn build_local_extension_uri(&self, extension: &str) -> String {
        format!("sip:{}@{}", extension, self.uas.local_uri.host())
    }

    fn collect_outbound_intercom_headers(&self, request: &sip_core::Request) -> sip_core::Headers {
        let mut headers = sip_core::Headers::new();
        for name in ["Alert-Info", "Call-Info", "X-FCP-Intercom-Mode"] {
            for value in request.headers().get_all(name) {
                // A shared line's appearance is the line's business, not
                // the far party's (§3.9).
                if name == "Call-Info" && value.to_ascii_lowercase().contains("appearance-index") {
                    continue;
                }
                let _ = headers.push(SmolStr::new(name), value);
            }
        }
        headers
    }

    fn is_oneway_intercom_request(request: &sip_core::Request) -> bool {
        request
            .headers()
            .get("X-FCP-Intercom-Mode")
            .map(|value| value.eq_ignore_ascii_case("oneway"))
            .unwrap_or(false)
    }

    /// Rewrite the direction attribute (`a=sendrecv` / `a=sendonly` / `a=recvonly`
    /// / `a=inactive`) on every audio and video section of an SDP.
    ///
    /// - Hold and resume apply to the whole call, so every RTP section is
    ///   rewritten; other media (`m=application`, …) keep their direction.
    /// - A section with no explicit direction attribute gets one inserted
    ///   immediately after its `m=` line.
    /// - Line endings are preserved: if the input uses CRLF we emit CRLF,
    ///   otherwise we emit LF.
    /// - If no audio or video section exists the SDP is returned unchanged.
    fn rewrite_sdp_direction(sdp: &str, direction: &str) -> String {
        // Detect the line ending used by the input. SIP mandates CRLF but we
        // tolerate LF-only bodies that some stacks produce.
        let eol: &str = if sdp.contains("\r\n") { "\r\n" } else { "\n" };
        let target = format!("a={}", direction);

        // Split preserving empty trailing element so we can rejoin faithfully.
        let lines: Vec<&str> = sdp.split(eol).collect();
        let mut out: Vec<String> = Vec::with_capacity(lines.len() + 2);

        // `open` is the index in `out` of the current eligible section's m=
        // line while that section has not shown a direction attribute yet.
        let mut open: Option<usize> = None;
        let mut eligible = false;

        for line in lines.iter() {
            if let Some(rest) = line.strip_prefix("m=") {
                // Entering a new media section: give the previous one its
                // direction if it never carried one.
                if let Some(idx) = open.take() {
                    out.insert(idx + 1, target.clone());
                }
                let media_type = rest.split_whitespace().next().unwrap_or("");
                eligible = matches!(media_type, "audio" | "video");
                if eligible {
                    open = Some(out.len());
                }
                out.push((*line).to_string());
                continue;
            }

            if eligible {
                let trimmed = line.trim();
                if trimmed == "a=sendrecv"
                    || trimmed == "a=sendonly"
                    || trimmed == "a=recvonly"
                    || trimmed == "a=inactive"
                {
                    out.push(target.clone());
                    open = None;
                    continue;
                }
            }

            out.push((*line).to_string());
        }

        // The SDP ended inside an eligible section with no direction line.
        if let Some(idx) = open {
            out.insert(idx + 1, target.clone());
        }

        out.join(eol)
    }

    fn build_feature_route_request(
        &self,
        request: &sip_core::Request,
        target_extension: &str,
        extra_headers: &[(String, String)],
    ) -> Result<sip_core::Request> {
        let mut routed = request.clone();
        let target_uri = sip_core::SipUri::parse(&self.build_local_extension_uri(target_extension))
            .map_err(|e| anyhow::anyhow!("invalid intercom target URI: {}", e))?;
        let to_header = format!("<{}>", target_uri.as_str());

        routed.set_uri(target_uri.clone());
        routed.headers_mut().remove("To");
        routed
            .headers_mut()
            .push(SmolStr::new("To"), SmolStr::new(to_header))
            .map_err(|e| anyhow::anyhow!("failed to rewrite To header: {}", e))?;

        for (name, value) in extra_headers {
            routed.headers_mut().remove(name.as_str());
            routed
                .headers_mut()
                .push(SmolStr::new(name.as_str()), SmolStr::new(value.as_str()))
                .map_err(|e| anyhow::anyhow!("failed to append intercom header {}: {}", name, e))?;
        }

        Ok(routed)
    }

    fn build_final_ok_response(
        &self,
        request: &sip_core::Request,
        to_tag: Option<&str>,
        sdp_body: Option<&str>,
    ) -> Result<sip_core::Response> {
        let mut response = self.uas.create_ok(request, sdp_body)?;

        if let Some(tag) = to_tag {
            if let Some(to) = request.headers().get("To") {
                let to_with_tag = format!("{};tag={}", to, tag);
                response.headers_mut().retain(|h| h.name() != "To");
                let _ = response
                    .headers_mut()
                    .push(SmolStr::new("To"), SmolStr::new(to_with_tag));
            }
        }

        self.ensure_response_hygiene(request, &mut response);
        inject_session_timer_headers(&self.b2bua_config, request, &mut response);
        Ok(response)
    }

    fn build_inbound_dialog(
        &self,
        request: &sip_core::Request,
        response: &sip_core::Response,
    ) -> Result<sip_dialog::Dialog> {
        sip_dialog::Dialog::new_uas(
            request,
            response,
            self.uas.local_uri.clone(),
            extract_from_uri(request)
                .unwrap_or_else(|_| sip_core::SipUri::parse("sip:unknown@unknown").unwrap()),
        )
        .ok_or_else(|| anyhow::anyhow!("failed to create UAS dialog"))
    }

    fn build_pickup_leg(
        &self,
        request: &sip_core::Request,
        dialog: sip_dialog::Dialog,
    ) -> fcp_b2bua_core::CallLeg {
        let remote_uri = extract_from_uri(request)
            .map(|uri| uri.as_str().to_string())
            .unwrap_or_else(|_| "sip:unknown@unknown".to_string());
        let remote_display_name = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_display_name(from.as_str()));

        let mut leg = fcp_b2bua_core::CallLeg::new_outbound(remote_uri, remote_display_name);
        leg.dialog = Some(dialog);
        leg.sdp = crate::body::request_sdp(request);
        leg
    }

    async fn anchor_pickup_media(
        &self,
        ringing_call_id: &fcp_b2bua_core::CallId,
        original_offer: Option<&str>,
        picker_offer: Option<&str>,
        label: &str,
    ) -> Result<(fcp_core::CallId, Option<String>, Option<String>)> {
        let media_manager = self
            .media_manager
            .as_ref()
            .context("media manager unavailable")?;
        let media_session_id = fcp_core::CallId::new();
        let dialog_id = fcp_core::DialogId::new();

        media_manager
            .create_session_default(media_session_id, dialog_id)
            .await
            .with_context(|| format!("{} failed: could not create media session", label))?;

        let original_answer = match original_offer {
            Some(offer) => Some(
                media_manager
                    .update_session(&media_session_id, offer.to_string())
                    .await
                    .with_context(|| {
                        format!(
                            "{} failed: could not set original caller media endpoint",
                            label
                        )
                    })?,
            ),
            None => None,
        };

        // An encrypted picker is answered with a key of ours for its leg.
        let picker_srtp = match picker_offer {
            Some(offer) => self
                .key_b_leg_from_offer(ringing_call_id, &media_session_id, media_manager, offer)
                .await
                .with_context(|| {
                    format!("{} failed: could not set picker media endpoint", label)
                })?,
            None => None,
        };

        let session = media_manager
            .get_session(&media_session_id)
            .await
            .with_context(|| format!("{} failed: could not load media session", label))?;
        // The original caller is leg A, the picker leg B: each is told its
        // own leg's port.
        let (a_port, _) = session.ports_for(fcp_media::backend::Leg::A);
        let (b_port, _) = session.ports_for(fcp_media::backend::Leg::B);
        let picker_answer = picker_offer
            .map(|offer| {
                fcp_media::construct_answer_for_a_leg(
                    offer,
                    &self.local_addr,
                    b_port,
                    picker_srtp.as_ref(),
                )
            })
            .or_else(|| Some(generate_basic_sdp(&self.local_addr, b_port)));
        let original_answer =
            original_answer.or_else(|| Some(generate_basic_sdp(&self.local_addr, a_port)));

        info!(
            call_id = %ringing_call_id,
            media_session_id = %media_session_id,
            "Pickup dynamically anchored media to generate valid SDP for both legs"
        );

        Ok((media_session_id, original_answer, picker_answer))
    }

    /// Cancel a forked B-leg, falling back to BYE when the leg has already
    /// crossed into a confirmed dialog.
    ///
    /// Per RFC 3261 §9.1, a CANCEL that arrives after the UAS has sent a
    /// final 2xx response to the INVITE is simply ineffective — the CANCEL
    /// transaction itself completes normally (so `cancel()` returns `Ok`)
    /// but the INVITE remains answered and the callee still believes it is
    /// on a live call. To guarantee cleanup we therefore:
    ///
    /// 1. Send the CANCEL, unless the leg has already answered.
    /// 2. Wait for the INVITE's own final response: the CANCEL's answer
    ///    can come first, and the callee's 200 after it.
    /// 3. If the INVITE was answered, send BYE to tear it down — whether
    ///    the CANCEL returned `Ok` or `Err`.
    ///
    /// This fixes a race window where pickup rebinds the call the instant a
    /// fork target answers: the arbiter's `cancel()` returns `Ok`, but the
    /// target phone would otherwise remain "in a call" until the user hangs
    /// up manually.
    async fn cancel_or_bye_fork_leg(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        target: &str,
        leg_index: usize,
        b_leg_handle: &Arc<crate::uac::OutboundCall>,
        context: &str,
    ) {
        Self::cancel_or_bye_fork_leg_with(
            self.uac.clone(),
            Arc::clone(&self.dialog_manager),
            *call_id,
            target.to_string(),
            leg_index,
            Arc::clone(b_leg_handle),
            context.to_string(),
        )
        .await
    }

    /// [`cancel_or_bye_fork_leg`](Self::cancel_or_bye_fork_leg) on its own,
    /// so a fork race can spawn it: cancelling a leg waits on that leg's
    /// CANCEL transaction, and a contact that is gone — a phone that
    /// crashed, a registration a restore brought back — answers nothing,
    /// so a winner relayed only after every loser was cancelled reached
    /// the caller a transaction timeout late (found by the nightly stack,
    /// `docs/NIGHTLY_STACK.md` §4).
    async fn cancel_or_bye_fork_leg_with(
        uac: Option<Arc<SipUserAgentClient>>,
        dialog_manager: Arc<DialogManager>,
        call_id: fcp_b2bua_core::CallId,
        target: String,
        leg_index: usize,
        b_leg_handle: Arc<crate::uac::OutboundCall>,
        context: String,
    ) {
        // A leg that has already answered is past CANCEL (RFC 3261 §9.1:
        // a client does not cancel a request that has its final response);
        // it is ended with a BYE alone.
        let confirmed =
            b_leg_handle.dialog.read().await.state() == sip_dialog::DialogStateType::Confirmed;
        let cancel_result = if confirmed {
            Ok(())
        } else {
            b_leg_handle.cancel().await.map(|_| ())
        };
        if let Err(ref e) = cancel_result {
            warn!(
                call_id = %call_id,
                target = %target,
                error = %e,
                "{}: CANCEL failed for fork {}",
                context,
                leg_index
            );
        }

        // A CANCEL answered 200 says nothing of the INVITE: the callee's 200
        // may have crossed it, and may reach this node after the CANCEL's
        // own answer (`docs/DEEP_TESTING.md` D2). So wait for the INVITE's
        // outcome, for as long as its transaction may last (Timer B). A
        // 2xx is its outcome only once it is ACKed, so the BYE below never
        // overtakes the ACK — which a confirmed dialog alone does not
        // promise.
        let outcome =
            tokio::time::timeout(Duration::from_secs(32), b_leg_handle.invite_outcome()).await;
        let answered = matches!(outcome, Ok(sip_uac::InviteOutcome::Final(code)) if (200..300).contains(&code));
        if !answered {
            if cancel_result.is_err() {
                debug!(
                    call_id = %call_id,
                    target = %target,
                    outcome = ?outcome,
                    "{}: fork {} did not answer, skipping BYE cleanup",
                    context,
                    leg_index
                );
            }
            return;
        }
        let dialog = b_leg_handle.dialog.read().await.clone();

        // Dialog is confirmed — the callee already answered 200 OK. CANCEL is
        // ineffective (or was already ineffective when we sent it); send BYE.
        let Some(uac) = &uac else {
            warn!(
                call_id = %call_id,
                target = %target,
                "{}: confirmed fork {} needs BYE cleanup but UAC is unavailable",
                context,
                leg_index
            );
            return;
        };

        match uac.bye(&dialog).await {
            Ok(response) => {
                dialog_manager.remove(dialog.id());
                info!(
                    call_id = %call_id,
                    target = %target,
                    code = response.start_line().code(),
                    cancel_ok = cancel_result.is_ok(),
                    "{}: sent BYE to confirmed fork {} (CANCEL raced too late)",
                    context,
                    leg_index
                );
            }
            Err(bye_error) => {
                warn!(
                    call_id = %call_id,
                    target = %target,
                    error = %bye_error,
                    "{}: failed to send BYE to confirmed fork {}",
                    context,
                    leg_index
                );
            }
        }
    }

    /// Apply topology hiding transformations to a response.
    ///
    /// When active, rewrites or strips headers that reveal internal topology
    /// (Via, Record-Route, Contact, User-Agent) based on the `TopologyHidingConfig`.
    fn apply_topology_hiding(&self, response: &mut sip_core::Response) {
        let config = &self.b2bua_config.topology_hiding;
        if !config.is_active() {
            return;
        }

        let external_domain = config.local_domain.as_deref().unwrap_or(&self.local_addr);

        debug!(
            mode = ?config.mode,
            domain = %external_domain,
            "Applying topology hiding"
        );

        let headers = response.headers_mut();

        // Strip Via headers (keep only top Via for response routing)
        if config.hide_via {
            let via_values: Vec<String> = headers.get_all("Via").map(|v| v.to_string()).collect();
            if via_values.len() > 1 {
                headers.remove("Via");
                // Re-add only the top Via (first one — the one for the outermost hop)
                if let Some(top_via) = via_values.first() {
                    let _ = headers.push("Via", top_via.as_str());
                }
            }
        }

        // Rewrite Record-Route headers with external domain
        if config.hide_record_route {
            let rr_count = headers.get_all("Record-Route").count();
            if rr_count > 0 {
                headers.remove("Record-Route");
                let rr_value = format!("<sip:{};lr>", external_domain);
                let _ = headers.push("Record-Route", &rr_value);
            }
        }

        // Rewrite Contact header with external domain
        if config.hide_contact && headers.get("Contact").is_some() {
            headers.remove("Contact");
            let contact_value = format!("<sip:{}>", external_domain);
            let _ = headers.push("Contact", &contact_value);
        }

        // Strip or replace User-Agent/Server headers
        if config.hide_user_agent {
            headers.remove("User-Agent");
            headers.remove("Server");
            if let Some(ref custom_ua) = config.custom_user_agent {
                let _ = headers.push("Server", custom_ua.as_str());
            }
        }
    }

    /// Forward INVITE to B-leg in full B2BUA mode.
    ///
    /// This method implements the complete B2BUA call flow:
    /// 1. Look up destination in registrar
    /// 2. Create outbound INVITE to B-leg using UAC
    /// 3. Relay provisional responses (100, 180) to A-leg
    /// 4. Relay final response (200 OK) with SDP to A-leg
    /// 5. Create both A-leg and B-leg dialogs
    async fn forward_invite_to_b_leg(
        &self,
        request: &sip_core::Request,
        a_leg_handle: &ServerTransactionHandle,
        call_id: fcp_b2bua_core::CallId,
        uac: Arc<SipUserAgentClient>,
    ) -> Result<()> {
        let a_leg_handle_arc = Arc::new(a_leg_handle.clone());
        self.latch_link_peer_a_leg(&call_id).await;

        // Extract To header to get destination AOR
        let to = sip_parse::header(request.headers(), "To")
            .ok_or_else(|| anyhow::anyhow!("Missing To header"))?;

        // Extract destination URI
        let destination_uri = to
            .split('<')
            .nth(1)
            .and_then(|s| s.split('>').next())
            .unwrap_or(to)
            .trim();

        // Normalize AOR by stripping URI parameters (e.g., ;transport=UDP)
        // The AOR for registrar lookup should be just user@host
        let destination_aor = destination_uri
            .split(';')
            .next()
            .unwrap_or(destination_uri)
            .trim();

        // Normalize the dialed number (e.g., 15185556789 -> +15185556789)
        let normalized_aor =
            normalize_dialed_number(destination_aor, self.b2bua_config.max_extension_digits);

        debug!(
            call_id = %call_id,
            destination_uri = %destination_uri,
            destination_aor = %destination_aor,
            normalized_aor = %normalized_aor,
            "Looking up destination in registrar"
        );

        // Update call state to LookingUpDestination
        let _ = self.call_manager.update_call(&call_id, |call| {
            call.state = fcp_b2bua_core::CallState::LookingUpDestination;
        });

        // Check if destination is a phone number (E.164 format)
        // If so, look up line binding to find which user/devices to route to
        // Emergency calls skip extension/DID lookup entirely: an extension
        // named "911" must never capture them. They go to emergency-capable
        // trunks first, then whatever the provider would normally choose.
        let emergency_ctx = self
            .call_manager
            .get_call(&call_id)
            .and_then(|call| call.emergency.clone());
        let (emergency_bindings, emergency_routes) = if emergency_ctx.is_some() {
            let routes = match &self.outbound_route_provider {
                Some(provider) => {
                    provider
                        .resolve_emergency_ordered(self.call_tenant(&call_id), destination_aor)
                        .await
                }
                None => Vec::new(),
            };
            if let Some(route) = routes.first() {
                info!(
                    call_id = %call_id,
                    target = %route.target_uri,
                    trunk = ?route.trunk_name(),
                    routes = routes.len(),
                    "Routing emergency call via trunk"
                );
                let trunk_name = route.trunk_name();
                let trunk_id = route.trunk_id();
                let allowed_codecs = route.allowed_codecs();
                let trunks = route_trunks(&routes);
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.trunk_name = trunk_name;
                    call.trunk_id = trunk_id;
                    call.outbound_allowed_codecs = allowed_codecs;
                    call.route_trunks = trunks;
                });
            }
            let mut bindings = Vec::with_capacity(routes.len());
            for r in &routes {
                match sip_registrar::Binding::new(
                    SmolStr::new(destination_aor),
                    SmolStr::new(format!("<{}>", r.target_uri)),
                    Duration::from_secs(3600),
                ) {
                    Ok(binding) => bindings.push(binding),
                    Err(e) => {
                        warn!(call_id = %call_id, target = %r.target_uri, error = %e, "Skipping emergency route: failed to create binding");
                    }
                }
            }
            if bindings.is_empty() {
                error!(
                    call_id = %call_id,
                    destination = %destination_aor,
                    "EMERGENCY CALL COULD NOT BE ROUTED: no upstream trunk available"
                );
                let response = self.create_simple_response(request, 503, "Service Unavailable");
                a_leg_handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    &call_id,
                    Some("No trunk available for emergency call".to_string()),
                );
                return Ok(());
            }
            (Some(bindings), routes)
        } else {
            (None, Vec::new())
        };

        let destination_contacts = if let Some((_, (contact, token))) =
            self.flow_relay_targets.remove(&call_id)
        {
            // A relayed leg goes down the flow it names
            // (`docs/PUSH.md` §2.4), to the phone and nowhere else.
            match sip_registrar::Binding::new(
                SmolStr::new(destination_aor),
                SmolStr::new(format!("<{contact}>")),
                Duration::from_secs(3600),
            ) {
                Ok(binding) => vec![binding.with_flow_token(SmolStr::new(token))],
                Err(e) => {
                    warn!(call_id = %call_id, contact = %contact, error = %e, "A relay's contact is not a SIP URI");
                    let response = self.create_simple_response(request, 430, "Flow Failed");
                    a_leg_handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call(&call_id, Some("relay: bad contact".into()));
                    return Ok(());
                }
            }
        } else if let Some(node_uri) = self.sca_handoff_targets.get(&call_id).map(|t| t.0.clone()) {
            // A retrieve of an appearance held on another node goes there,
            // carrying its handoff (§3.9).
            match sip_registrar::Binding::new(
                SmolStr::new(destination_aor),
                SmolStr::new(format!("<{node_uri}>")),
                Duration::from_secs(3600),
            ) {
                Ok(binding) => vec![binding],
                Err(e) => {
                    warn!(call_id = %call_id, node = %node_uri, error = %e, "A retrieve's node is not a SIP URI");
                    self.sca_handoff_targets.remove(&call_id);
                    let response =
                        self.create_simple_response(request, 480, "Temporarily Unavailable");
                    a_leg_handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call(&call_id, Some("retrieve: bad node URI".into()));
                    return Ok(());
                }
            }
        } else if let Some(bindings) = emergency_bindings {
            bindings
        } else if let Some(at_site) = self.route_to_site(&call_id, &normalized_aor).await {
            // A number answered at a site goes down that site's link
            // (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
            match at_site {
                Ok(bindings) => bindings,
                Err(reason) => {
                    info!(call_id = %call_id, destination = %normalized_aor, reason = %reason, "A call for a number at a site that cannot be reached");
                    let response =
                        self.create_simple_response(request, 480, "Temporarily Unavailable");
                    a_leg_handle.send_final(response).await;
                    let _ = self.call_manager.terminate_call(&call_id, Some(reason));
                    return Ok(());
                }
            }
        } else if is_e164_number(&normalized_aor) {
            debug!(
                call_id = %call_id,
                destination = %normalized_aor,
                "Destination is a phone number, looking up line binding"
            );

            // Extract E.164 number from SIP URI (e.g., sip:+15551234567@host -> +15551234567)
            let e164_number = normalized_aor
                .strip_prefix("sip:")
                .or_else(|| normalized_aor.strip_prefix("sips:"))
                .unwrap_or(&normalized_aor)
                .split('@')
                .next()
                .unwrap_or(&normalized_aor);

            // Try to look up line binding for this phone number
            if let Some(storage) = &self.storage {
                // First, get the phone number by E.164
                match storage.get_phone_number_by_e164(e164_number).await {
                    Ok(Some(phone_number)) => {
                        // The number's own screening (§3.7): anonymous
                        // callers, its allow list, its block list.
                        let screening = phone_number.screen(caller_number(request).as_deref());
                        if screening != fcp_core::Screening::Allowed {
                            // RFC 5079: 433 says anonymity is what was refused.
                            let (code, phrase) = match screening {
                                fcp_core::Screening::Anonymous => (433, "Anonymity Disallowed"),
                                _ => (603, "Decline"),
                            };
                            info!(
                                call_id = %call_id,
                                number = %phone_number.e164,
                                screening = screening.as_str(),
                                "Refusing the call: the number screens its caller out"
                            );
                            let response = self.create_simple_response(request, code, phrase);
                            a_leg_handle_arc.send_final(response).await;
                            let _ = self.call_manager.terminate_call_with(
                                &call_id,
                                fcp_cdr::TerminationReason::rejected(code, phrase),
                            );
                            return Ok(());
                        }
                        // Then get the line binding by phone number ID
                        match storage
                            .get_line_binding_by_phone_number(&phone_number.id)
                            .await
                        {
                            Ok(Some(line_binding)) => {
                                debug!(
                                    call_id = %call_id,
                                    phone_number = %destination_aor,
                                    binding_type = %line_binding.binding_type,
                                    "Found line binding for phone number"
                                );

                                // The number's line binding, for its mailbox and
                                // the record.
                                {
                                    let lb_id = line_binding.id;
                                    let callee = destination_aor.to_string();
                                    let _ = self.call_manager.update_call(&call_id, |call| {
                                        call.line_binding_id = Some(lb_id);
                                        call.number_binding_id = Some(lb_id);
                                        call.original_callee = Some(callee);
                                    });
                                }

                                // The number's hours (`docs/ENTERPRISE_CALLING.md`
                                // §3.5): closed, the call goes where the
                                // number says.
                                if let Some(schedule_id) = line_binding.schedule_id {
                                    if let Some((state, target)) = self
                                        .schedule_divert(
                                            &schedule_id,
                                            &line_binding.tenant_id,
                                            line_binding.closed_target.clone(),
                                            line_binding.holiday_target.clone(),
                                        )
                                        .await
                                    {
                                        return self
                                            .divert_for_schedule(
                                                request,
                                                &a_leg_handle_arc,
                                                call_id,
                                                state,
                                                target,
                                                destination_aor,
                                            )
                                            .await;
                                    }
                                }

                                // Handle different binding types
                                match line_binding.binding_type {
                                    fcp_core::BindingType::User => {
                                        // The owner's plan rings the binding's devices
                                        // (`docs/ENTERPRISE_CALLING.md` §3.1).
                                        let contacts = match &self.registrar {
                                            Some(registrar) => {
                                                crate::ring_plan::contacts_for_binding(
                                                    storage,
                                                    registrar,
                                                    &line_binding,
                                                    None,
                                                )
                                                .await
                                            }
                                            None => Vec::new(),
                                        };
                                        match self
                                            .ring_by_plan(
                                                request,
                                                &a_leg_handle_arc,
                                                call_id,
                                                &line_binding.tenant_id,
                                                line_binding.owner_user_id,
                                                contacts,
                                                destination_aor,
                                            )
                                            .await?
                                        {
                                            Some(contacts) => {
                                                if !self
                                                    .take_line_appearance(
                                                        request,
                                                        &a_leg_handle_arc,
                                                        call_id,
                                                        &line_binding,
                                                        &phone_number.e164,
                                                    )
                                                    .await
                                                {
                                                    return Ok(());
                                                }
                                                contacts
                                            }
                                            None => return Ok(()),
                                        }
                                    }
                                    fcp_core::BindingType::Ivr => {
                                        // IVR routing - execute a flow
                                        let flow_id = line_binding.flow_id.clone();
                                        if let (Some(flow_id), Some(_)) =
                                            (flow_id, &self.flow_handler)
                                        {
                                            return self
                                                .start_flow_for_call(
                                                    request,
                                                    a_leg_handle,
                                                    call_id,
                                                    flow_id,
                                                    destination_aor,
                                                )
                                                .await;
                                        } else {
                                            let detail = if line_binding.flow_id.is_none() {
                                                "No flow configured for IVR binding"
                                            } else {
                                                "Flow handler not available"
                                            };
                                            warn!(call_id = %call_id, "{}", detail);
                                            let response = self.create_simple_response(
                                                request,
                                                501,
                                                "Not Implemented",
                                            );
                                            a_leg_handle.send_final(response).await;
                                            let _ = self
                                                .call_manager
                                                .terminate_call(&call_id, Some(detail.to_string()));
                                            return Ok(());
                                        }
                                    }
                                    fcp_core::BindingType::Queue => {
                                        let response = self.create_simple_response(
                                            request,
                                            404,
                                            "Queue Route Not Found",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some(
                                                "Queue binding present but no matching queue extension is configured"
                                                    .to_string(),
                                            ),
                                        );
                                        return Ok(());
                                    }
                                    fcp_core::BindingType::ConferenceBridge => {
                                        use fcp_core::storage::ConferenceStore;

                                        let room_number = destination_aor
                                            .split('@')
                                            .next()
                                            .unwrap_or(destination_aor);
                                        let meeting_result = storage
                                            .get_meeting_by_room_number(
                                                &line_binding.tenant_id,
                                                room_number,
                                            )
                                            .await;

                                        match meeting_result {
                                            Ok(Some(meeting)) if meeting.enabled => {
                                                let conf_host = self
                                                    .conference_target_host(
                                                        &meeting.tenant_id,
                                                        room_number,
                                                        ConferenceVideoNeed::of(&meeting).as_ref(),
                                                    )
                                                    .await;
                                                if let Some(host) = conf_host.as_deref() {
                                                    let target_uri =
                                                        format!("sip:{}@{}", room_number, host);
                                                    self.mark_conference_call(
                                                        &call_id,
                                                        room_number,
                                                        host,
                                                    );
                                                    match sip_registrar::Binding::new(
                                                        SmolStr::new(destination_aor),
                                                        SmolStr::new(&target_uri),
                                                        std::time::Duration::from_secs(3600),
                                                    ) {
                                                        Ok(binding) => {
                                                            info!(
                                                                call_id = %call_id,
                                                                room_number = %room_number,
                                                                meeting_name = %meeting.name,
                                                                target_uri = %target_uri,
                                                                "Routing conference line binding to conference server"
                                                            );
                                                            vec![binding]
                                                        }
                                                        Err(e) => {
                                                            warn!(call_id = %call_id, error = %e, "Failed to create conference binding");
                                                            let response = self
                                                                .create_simple_response(
                                                                    request,
                                                                    500,
                                                                    "Internal Server Error",
                                                                );
                                                            a_leg_handle.send_final(response).await;
                                                            let _ = self.call_manager.terminate_call(
                                                                &call_id,
                                                                Some(format!(
                                                                    "Failed to create conference binding: {}",
                                                                    e
                                                                )),
                                                            );
                                                            return Ok(());
                                                        }
                                                    }
                                                } else {
                                                    let response = self.create_simple_response(
                                                        request,
                                                        503,
                                                        "Conference Service Unavailable",
                                                    );
                                                    a_leg_handle.send_final(response).await;
                                                    let _ = self.call_manager.terminate_call(
                                                        &call_id,
                                                        Some(
                                                            "Conference room found but conference_server_uri is not configured"
                                                                .to_string(),
                                                        ),
                                                    );
                                                    return Ok(());
                                                }
                                            }
                                            Ok(Some(_)) | Ok(None) => {
                                                let response = self.create_simple_response(
                                                    request,
                                                    404,
                                                    "Conference Room Not Found",
                                                );
                                                a_leg_handle.send_final(response).await;
                                                let _ = self.call_manager.terminate_call(
                                                    &call_id,
                                                    Some(format!(
                                                        "No enabled conference room found for {}",
                                                        room_number
                                                    )),
                                                );
                                                return Ok(());
                                            }
                                            Err(e) => {
                                                error!(call_id = %call_id, room_number = %room_number, error = %e, "Failed to look up conference room");
                                                let response = self.create_simple_response(
                                                    request,
                                                    500,
                                                    "Internal Server Error",
                                                );
                                                a_leg_handle.send_final(response).await;
                                                let _ = self.call_manager.terminate_call(
                                                    &call_id,
                                                    Some(format!(
                                                        "Failed to look up conference room: {}",
                                                        e
                                                    )),
                                                );
                                                return Ok(());
                                            }
                                        }
                                    }
                                    fcp_core::BindingType::Voicemail => {
                                        info!(
                                            call_id = %call_id,
                                            binding_type = %line_binding.binding_type,
                                            "Routing voicemail line binding to voicemail server"
                                        );
                                        match self
                                            .route_cfna(
                                                request,
                                                &a_leg_handle_arc,
                                                call_id,
                                                "voicemail",
                                                Some(destination_aor),
                                                None,
                                            )
                                            .await
                                        {
                                            Ok(()) => return Ok(()),
                                            Err(e) => {
                                                error!(call_id = %call_id, error = %e, "Voicemail routing failed");
                                                let response = self.create_simple_response(
                                                    request,
                                                    480,
                                                    "Temporarily Unavailable",
                                                );
                                                a_leg_handle.send_final(response).await;
                                                let _ = self.call_manager.terminate_call(
                                                    &call_id,
                                                    Some(format!(
                                                        "Voicemail routing failed: {}",
                                                        e
                                                    )),
                                                );
                                                return Ok(());
                                            }
                                        }
                                    }
                                    _ => {
                                        info!(call_id = %call_id, binding_type = %line_binding.binding_type, "Binding type not supported");
                                        let response = self.create_simple_response(
                                            request,
                                            501,
                                            "Not Implemented",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some(format!(
                                                "Binding type {} not supported",
                                                line_binding.binding_type
                                            )),
                                        );
                                        return Ok(());
                                    }
                                }
                            }
                            Ok(None) => {
                                // No line binding for this phone number - fall through to direct registrar lookup
                                debug!(
                                    call_id = %call_id,
                                    phone_number = %destination_aor,
                                    "No line binding found for phone number, trying direct registrar lookup"
                                );

                                // A carrier's call to a number of ours that
                                // nothing answers is not sent back out to a
                                // carrier, which would send it straight back.
                                let from_a_carrier =
                                    self.call_manager.get_call(&call_id).is_some_and(|c| {
                                        c.arrived_on_trunk.is_some() && c.link_site.is_none()
                                    });
                                if let Some(registrar) = &self.registrar {
                                    match registrar.get_registrations(destination_aor).await {
                                        Ok(bindings) if !bindings.is_empty() => bindings,
                                        _ if from_a_carrier => {
                                            info!(call_id = %call_id, phone_number = %destination_aor, "A carrier's call to a number of ours that nothing answers");
                                            let response = self.create_simple_response(
                                                request,
                                                404,
                                                "Not Found",
                                            );
                                            a_leg_handle.send_final(response).await;
                                            let _ = self.call_manager.terminate_call(
                                                &call_id,
                                                Some("Number has nothing bound to it".to_string()),
                                            );
                                            return Ok(());
                                        }
                                        // Nobody here answers this number: it is an
                                        // outbound call, and goes out on a trunk.
                                        _ => match self
                                            .trunk_bindings(&call_id, destination_aor)
                                            .await
                                        {
                                            TrunkBindings::Routed(bindings) => bindings,
                                            TrunkBindings::Unavailable => {
                                                warn!(call_id = %call_id, destination = %destination_aor, "Routes configured but no trunk currently available");
                                                let response = self.create_simple_response(
                                                    request,
                                                    503,
                                                    "Service Unavailable",
                                                );
                                                a_leg_handle.send_final(response).await;
                                                let _ = self.call_manager.terminate_call(
                                                    &call_id,
                                                    Some("All trunks unavailable".to_string()),
                                                );
                                                return Ok(());
                                            }
                                            TrunkBindings::NoRoute => {
                                                info!(call_id = %call_id, destination = %destination_aor, "No line binding or registration found");
                                                let response = self.create_simple_response(
                                                    request,
                                                    480,
                                                    "Temporarily Unavailable",
                                                );
                                                a_leg_handle.send_final(response).await;
                                                let _ = self.call_manager.terminate_call(
                                                    &call_id,
                                                    Some("Destination not found".to_string()),
                                                );
                                                return Ok(());
                                            }
                                        },
                                    }
                                } else {
                                    error!(call_id = %call_id, "No registrar available");
                                    let response = self.create_simple_response(
                                        request,
                                        500,
                                        "Service Unavailable",
                                    );
                                    a_leg_handle.send_final(response).await;
                                    let _ = self.call_manager.terminate_call(
                                        &call_id,
                                        Some("No registrar configured".to_string()),
                                    );
                                    return Ok(());
                                }
                            }
                            Err(e) => {
                                error!(
                                    call_id = %call_id,
                                    phone_number = %destination_aor,
                                    error = %e,
                                    "Failed to look up line binding"
                                );
                                let response = self.create_simple_response(
                                    request,
                                    500,
                                    "Internal Server Error",
                                );
                                a_leg_handle.send_final(response).await;
                                let _ = self.call_manager.terminate_call(
                                    &call_id,
                                    Some(format!("Failed to lookup line binding: {}", e)),
                                );
                                return Ok(());
                            }
                        }
                    }
                    Ok(None) => {
                        // Phone number not found in database - fall through to direct registrar lookup
                        debug!(
                            call_id = %call_id,
                            phone_number = %destination_aor,
                            "Phone number not in database, trying direct registrar lookup"
                        );

                        if let Some(registrar) = &self.registrar {
                            match registrar.get_registrations(destination_aor).await {
                                Ok(bindings) if !bindings.is_empty() => bindings,
                                // This number is nobody's here: the call is
                                // outbound, and goes out on a trunk — the
                                // caller's tenant's, then the platform's.
                                _ => match self.trunk_bindings(&call_id, destination_aor).await {
                                    TrunkBindings::Routed(bindings) => bindings,
                                    TrunkBindings::Unavailable => {
                                        warn!(call_id = %call_id, destination = %destination_aor, "Routes configured but no trunk currently available");
                                        let response = self.create_simple_response(
                                            request,
                                            503,
                                            "Service Unavailable",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("All trunks unavailable".to_string()),
                                        );
                                        return Ok(());
                                    }
                                    TrunkBindings::NoRoute => {
                                        info!(call_id = %call_id, destination = %destination_aor, "Phone number not found");
                                        let response = self.create_simple_response(
                                            request,
                                            480,
                                            "Temporarily Unavailable",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("Destination not found".to_string()),
                                        );
                                        return Ok(());
                                    }
                                },
                            }
                        } else {
                            error!(call_id = %call_id, "No registrar available");
                            let response =
                                self.create_simple_response(request, 500, "Service Unavailable");
                            a_leg_handle.send_final(response).await;
                            let _ = self.call_manager.terminate_call(
                                &call_id,
                                Some("No registrar configured".to_string()),
                            );
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        error!(
                            call_id = %call_id,
                            phone_number = %destination_aor,
                            error = %e,
                            "Failed to look up phone number"
                        );
                        let response =
                            self.create_simple_response(request, 500, "Internal Server Error");
                        a_leg_handle.send_final(response).await;
                        let _ = self.call_manager.terminate_call(
                            &call_id,
                            Some(format!("Failed to lookup phone number: {}", e)),
                        );
                        return Ok(());
                    }
                }
            } else {
                // No storage available - fall back to direct registrar lookup
                debug!(call_id = %call_id, "No storage available, using direct registrar lookup");
                if let Some(registrar) = &self.registrar {
                    match registrar.get_registrations(destination_aor).await {
                        Ok(bindings) if !bindings.is_empty() => bindings,
                        _ => {
                            // Try outbound route provider with failover before giving up.
                            // Classified resolution (reachability stage): 404 when
                            // nothing is configured for the destination, 503 when
                            // routes exist but no trunk is currently available.
                            if let Some(provider) = &self.outbound_route_provider {
                                let routes = match self
                                    .classified_routes(provider, &call_id, destination_aor)
                                    .await
                                {
                                    RouteResolution::Routes(routes) => routes,
                                    RouteResolution::Unavailable => {
                                        warn!(call_id = %call_id, destination = %destination_aor, "Routes configured but no trunk currently available");
                                        let response = self.create_simple_response(
                                            request,
                                            503,
                                            "Service Unavailable",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("All trunks unavailable".to_string()),
                                        );
                                        return Ok(());
                                    }
                                    RouteResolution::NoRoute => {
                                        info!(call_id = %call_id, destination = %destination_aor, "Destination not registered and no route configured");
                                        let response =
                                            self.create_simple_response(request, 404, "Not Found");
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("No route to destination".to_string()),
                                        );
                                        return Ok(());
                                    }
                                };
                                if let Some(route) = routes.first() {
                                    info!(call_id = %call_id, target = %route.target_uri, trunk = ?route.trunk_name(), allowed_codecs = ?route.allowed_codecs(), routes = routes.len(), "Routing via outbound trunk (failover available)");
                                    // Store trunk name and allowed codecs on call for lifecycle tracking
                                    {
                                        let trunk_name = route.trunk_name();
                                        let trunk_id = route.trunk_id();
                                        let allowed_codecs = route.allowed_codecs();
                                        let route_trunks = route_trunks(&routes);
                                        let _ = self.call_manager.update_call(&call_id, |call| {
                                            call.trunk_name = trunk_name;
                                            call.trunk_id = trunk_id;
                                            call.outbound_allowed_codecs = allowed_codecs;
                                            call.route_trunks = route_trunks;
                                        });
                                    }
                                    // Build bindings for primary + failover routes
                                    let mut failover_bindings = Vec::with_capacity(routes.len());
                                    for r in &routes {
                                        match sip_registrar::Binding::new(
                                            SmolStr::new(destination_aor),
                                            SmolStr::new(format!("<{}>", r.target_uri)),
                                            Duration::from_secs(3600),
                                        ) {
                                            Ok(binding) => failover_bindings.push(binding),
                                            Err(e) => {
                                                warn!(call_id = %call_id, target = %r.target_uri, error = %e, "Skipping failover route: failed to create binding");
                                            }
                                        }
                                    }
                                    if failover_bindings.is_empty() {
                                        error!(call_id = %call_id, "All failover route bindings failed");
                                        let response = self.create_simple_response(
                                            request,
                                            500,
                                            "Internal Server Error",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("Failed to create trunk binding".to_string()),
                                        );
                                        return Ok(());
                                    }
                                    failover_bindings
                                } else {
                                    info!(call_id = %call_id, destination = %destination_aor, "Destination not registered, no outbound route");
                                    let response = self.create_simple_response(
                                        request,
                                        480,
                                        "Temporarily Unavailable",
                                    );
                                    a_leg_handle.send_final(response).await;
                                    let _ = self.call_manager.terminate_call(
                                        &call_id,
                                        Some("Destination not registered".to_string()),
                                    );
                                    return Ok(());
                                }
                            } else {
                                info!(call_id = %call_id, destination = %destination_aor, "Destination not registered");
                                let response = self.create_simple_response(
                                    request,
                                    480,
                                    "Temporarily Unavailable",
                                );
                                a_leg_handle.send_final(response).await;
                                let _ = self.call_manager.terminate_call(
                                    &call_id,
                                    Some("Destination not registered".to_string()),
                                );
                                return Ok(());
                            }
                        }
                    }
                } else {
                    error!(call_id = %call_id, "No registrar available");
                    let response = self.create_simple_response(request, 500, "Service Unavailable");
                    a_leg_handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call(&call_id, Some("No registrar configured".to_string()));
                    return Ok(());
                }
            }
        } else {
            // Not a phone number - try extension lookup first, then direct registrar lookup
            debug!(call_id = %call_id, destination = %normalized_aor, "Destination is not a phone number, checking extensions");

            // Extract extension number from URI (e.g., sip:5000@host -> 5000)
            let extension_number = normalized_aor
                .strip_prefix("sip:")
                .or_else(|| normalized_aor.strip_prefix("sips:"))
                .unwrap_or(&normalized_aor)
                .split('@')
                .next()
                .unwrap_or(&normalized_aor);

            // Check if this extension is a PBX hunt group
            if let Some(pbx) = &self.pbx_handler {
                if pbx.is_enabled() {
                    if let Some(plan) = pbx.resolve_hunt_group(extension_number) {
                        info!(
                            call_id = %call_id,
                            extension = %extension_number,
                            phases = plan.ring_phases.len(),
                            "Routing to hunt group"
                        );
                        return self
                            .execute_hunt_group_plan(request, a_leg_handle, call_id, plan, uac)
                            .await;
                    }
                }
            }

            // Try extension lookup if storage is available
            let mut extension_contacts = Vec::new();
            if let Some(storage) = &self.storage {
                let tenant_id = match self
                    .call_manager
                    .get_call(&call_id)
                    .map(|call| call.tenant_id)
                    .or(self.default_tenant_id)
                {
                    Some(tenant_id) => tenant_id,
                    None => {
                        warn!(
                            call_id = %call_id,
                            extension = %extension_number,
                            "Rejecting call: no tenant context available for extension lookup"
                        );
                        let response =
                            self.create_simple_response(request, 403, "Tenant context required");
                        a_leg_handle.send_final(response).await;
                        let _ = self
                            .call_manager
                            .terminate_call(&call_id, Some("Tenant context required".to_string()));
                        return Ok(());
                    }
                };

                // Extract the INVITE domain for device-lookup fallback.
                // When a device's sip_uri uses the server IP but the phone
                // registered under the tenant's SIP domain, this lets
                // get_registrations_for_device retry with the correct domain.
                let invite_domain = destination_aor.split('@').nth(1).unwrap_or("");

                debug!(
                    call_id = %call_id,
                    tenant_id = %tenant_id,
                    extension = %extension_number,
                    "Looking up extension (tenant-aware)"
                );

                match storage
                    .get_extension_by_number(&tenant_id, extension_number)
                    .await
                {
                    Ok(Some(extension)) => {
                        info!(
                            call_id = %call_id,
                            tenant_id = %tenant_id,
                            extension = %extension_number,
                            target = %extension.target,
                            "Found extension, routing to target"
                        );

                        // Route based on extension target type
                        match &extension.target {
                            fcp_core::ExtensionTarget::PhoneNumber(phone_number_id) => {
                                // Look up the phone number and its line bindings
                                match storage.get_phone_number(phone_number_id).await {
                                    Ok(Some(phone_number)) => {
                                        // Get line binding for this phone number
                                        match storage
                                            .get_line_binding_by_phone_number(&phone_number.id)
                                            .await
                                        {
                                            Ok(Some(line_binding)) => {
                                                {
                                                    let lb_id = line_binding.id;
                                                    let callee = destination_aor.to_string();
                                                    let _ = self.call_manager.update_call(
                                                        &call_id,
                                                        |call| {
                                                            call.line_binding_id = Some(lb_id);
                                                            call.number_binding_id = Some(lb_id);
                                                            call.original_callee = Some(callee);
                                                        },
                                                    );
                                                }

                                                // The number's hours (§3.5).
                                                if let Some(schedule_id) = line_binding.schedule_id
                                                {
                                                    if let Some((state, target)) = self
                                                        .schedule_divert(
                                                            &schedule_id,
                                                            &tenant_id,
                                                            line_binding.closed_target.clone(),
                                                            line_binding.holiday_target.clone(),
                                                        )
                                                        .await
                                                    {
                                                        return self
                                                            .divert_for_schedule(
                                                                request,
                                                                &a_leg_handle_arc,
                                                                call_id,
                                                                state,
                                                                target,
                                                                destination_aor,
                                                            )
                                                            .await;
                                                    }
                                                }

                                                // The owner's plan rings the binding's
                                                // devices (`docs/ENTERPRISE_CALLING.md` §3.1).
                                                let contacts = match &self.registrar {
                                                    Some(registrar) => {
                                                        crate::ring_plan::contacts_for_binding(
                                                            storage,
                                                            registrar,
                                                            &line_binding,
                                                            Some(invite_domain),
                                                        )
                                                        .await
                                                    }
                                                    None => Vec::new(),
                                                };
                                                match self
                                                    .ring_by_plan(
                                                        request,
                                                        &a_leg_handle_arc,
                                                        call_id,
                                                        &tenant_id,
                                                        line_binding.owner_user_id,
                                                        contacts,
                                                        destination_aor,
                                                    )
                                                    .await?
                                                {
                                                    Some(contacts) => {
                                                        if !self
                                                            .take_line_appearance(
                                                                request,
                                                                &a_leg_handle_arc,
                                                                call_id,
                                                                &line_binding,
                                                                &phone_number.e164,
                                                            )
                                                            .await
                                                        {
                                                            return Ok(());
                                                        }
                                                        extension_contacts.extend(contacts)
                                                    }
                                                    None => return Ok(()),
                                                }
                                            }
                                            _ => {
                                                warn!(call_id = %call_id, extension = %extension_number, "Extension target phone number has no line binding");
                                            }
                                        }
                                    }
                                    _ => {
                                        warn!(call_id = %call_id, extension = %extension_number, "Extension target phone number not found");
                                    }
                                }
                            }
                            fcp_core::ExtensionTarget::User(user_id) => {
                                info!(
                                    call_id = %call_id,
                                    user_id = %user_id,
                                    "Extension routes to user: their plan"
                                );
                                // The user's plan rings every device they own
                                // (`docs/ENTERPRISE_CALLING.md` §3.1).
                                let contacts = match &self.registrar {
                                    Some(registrar) => {
                                        crate::ring_plan::contacts_for_user(
                                            storage,
                                            registrar,
                                            &tenant_id,
                                            user_id,
                                            invite_domain,
                                        )
                                        .await
                                    }
                                    None => {
                                        warn!(call_id = %call_id, "No registrar available for user device lookup");
                                        Vec::new()
                                    }
                                };
                                match self
                                    .ring_by_plan(
                                        request,
                                        &a_leg_handle_arc,
                                        call_id,
                                        &tenant_id,
                                        Some(*user_id),
                                        contacts,
                                        destination_aor,
                                    )
                                    .await?
                                {
                                    Some(contacts) => extension_contacts.extend(contacts),
                                    None => return Ok(()),
                                }
                            }
                            fcp_core::ExtensionTarget::Device(device_id) => {
                                info!(
                                    call_id = %call_id,
                                    device_id = %device_id,
                                    "Extension routes to specific device"
                                );

                                // Get the specific device
                                match storage.get_device(device_id).await {
                                    Ok(Some(device)) => {
                                        if !device.enabled {
                                            warn!(call_id = %call_id, device_id = %device_id, "Target device is disabled");
                                            let response = self.create_simple_response(
                                                request,
                                                480,
                                                "Temporarily Unavailable",
                                            );
                                            a_leg_handle.send_final(response).await;
                                            let _ = self.call_manager.terminate_call(
                                                &call_id,
                                                Some("Target device is disabled".to_string()),
                                            );
                                            return Ok(());
                                        }

                                        if let Some(sip_uri) = &device.sip_uri {
                                            // Look up device registrations
                                            if let Some(registrar) = &self.registrar {
                                                match registrar
                                                    .get_registrations_for_device(
                                                        &tenant_id,
                                                        sip_uri,
                                                        invite_domain,
                                                    )
                                                    .await
                                                {
                                                    Ok(registrations)
                                                        if !registrations.is_empty() =>
                                                    {
                                                        debug!(
                                                            call_id = %call_id,
                                                            device_id = %device_id,
                                                            sip_uri = %sip_uri,
                                                            reg_count = registrations.len(),
                                                            "Found registrations for device"
                                                        );
                                                        // Rung with its effective user's
                                                        // treatment: its hoteling guest,
                                                        // else its owner (§3.8).
                                                        let session = crate::hoteling::session_of(
                                                            storage, &device,
                                                        )
                                                        .await;
                                                        let user =
                                                            fcp_core::hoteling::effective_user(
                                                                &device,
                                                                session.as_ref(),
                                                            );
                                                        match self
                                                            .ring_by_plan(
                                                                request,
                                                                &a_leg_handle_arc,
                                                                call_id,
                                                                &tenant_id,
                                                                user,
                                                                registrations,
                                                                destination_aor,
                                                            )
                                                            .await?
                                                        {
                                                            Some(contacts) => {
                                                                extension_contacts.extend(contacts)
                                                            }
                                                            None => return Ok(()),
                                                        }
                                                    }
                                                    Ok(_) => {
                                                        // No registration found — use
                                                        // the device's sip_uri directly
                                                        // as a synthetic contact.  This
                                                        // handles servers (IVR, conf)
                                                        // that receive INVITEs but don't
                                                        // REGISTER.
                                                        info!(
                                                            call_id = %call_id,
                                                            device_id = %device_id,
                                                            sip_uri = %sip_uri,
                                                            "Device not registered, using sip_uri as direct target"
                                                        );
                                                        match sip_registrar::Binding::new(
                                                            SmolStr::new(destination_aor),
                                                            SmolStr::new(sip_uri),
                                                            std::time::Duration::from_secs(3600),
                                                        ) {
                                                            Ok(binding) => {
                                                                extension_contacts.push(binding);

                                                                // The target is an unregistered
                                                                // server (IVR, voicemail, etc.)
                                                                // that anchors its own media.
                                                                // Disable media anchoring on this
                                                                // leg so RTP flows directly from
                                                                // the caller to the server —
                                                                // otherwise DTMF and playback
                                                                // won't work (the call-manager's
                                                                // media engine would consume
                                                                // RFC 2833 events before they
                                                                // reach the server).
                                                                if let Some(call_entry) = self
                                                                    .call_manager
                                                                    .get_call(&call_id)
                                                                {
                                                                    if call_entry.media_anchored {
                                                                        if let Some(
                                                                            ref session_id,
                                                                        ) = call_entry
                                                                            .media_session_id
                                                                        {
                                                                            if let Some(ref mm) =
                                                                                self.media_manager
                                                                            {
                                                                                let sid =
                                                                                    *session_id;
                                                                                let mm2 =
                                                                                    Arc::clone(mm);
                                                                                tokio::spawn(
                                                                                    async move {
                                                                                        if let Err(e) = mm2.terminate_session(&sid).await {
                                                                                        tracing::debug!(error = %e, "Failed to terminate pre-allocated media session for server target");
                                                                                    }
                                                                                    },
                                                                                );
                                                                            }
                                                                        }
                                                                        let _ = self.call_manager.update_call(&call_id, |call| {
                                                                            call.media_anchored = false;
                                                                            call.media_session_id = None;
                                                                        });
                                                                        info!(
                                                                            call_id = %call_id,
                                                                            sip_uri = %sip_uri,
                                                                            "Media anchoring disabled for server device — RTP flows directly to target"
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                            Err(e) => {
                                                                warn!(call_id = %call_id, error = %e, "Failed to create synthetic binding from device sip_uri");
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        error!(
                                                            call_id = %call_id,
                                                            device_id = %device_id,
                                                            error = %e,
                                                            "Failed to get device registrations"
                                                        );
                                                        let response = self.create_simple_response(
                                                            request,
                                                            500,
                                                            "Internal Server Error",
                                                        );
                                                        a_leg_handle.send_final(response).await;
                                                        let _ = self.call_manager.terminate_call(&call_id, Some(format!("Failed to get device registrations: {}", e)));
                                                        return Ok(());
                                                    }
                                                }
                                            } else {
                                                warn!(call_id = %call_id, "No registrar available for device lookup");
                                                let response = self.create_simple_response(
                                                    request,
                                                    500,
                                                    "Service Unavailable",
                                                );
                                                a_leg_handle.send_final(response).await;
                                                let _ = self.call_manager.terminate_call(
                                                    &call_id,
                                                    Some("No registrar configured".to_string()),
                                                );
                                                return Ok(());
                                            }
                                        } else {
                                            warn!(call_id = %call_id, device_id = %device_id, "Device has no SIP URI");
                                            let response = self.create_simple_response(
                                                request,
                                                480,
                                                "Temporarily Unavailable",
                                            );
                                            a_leg_handle.send_final(response).await;
                                            let _ = self.call_manager.terminate_call(
                                                &call_id,
                                                Some("Device has no SIP URI".to_string()),
                                            );
                                            return Ok(());
                                        }
                                    }
                                    Ok(None) => {
                                        warn!(call_id = %call_id, device_id = %device_id, "Device not found");
                                        let response =
                                            self.create_simple_response(request, 404, "Not Found");
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("Device not found".to_string()),
                                        );
                                        return Ok(());
                                    }
                                    Err(e) => {
                                        error!(call_id = %call_id, device_id = %device_id, error = %e, "Failed to get device");
                                        let response = self.create_simple_response(
                                            request,
                                            500,
                                            "Internal Server Error",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some(format!("Failed to get device: {}", e)),
                                        );
                                        return Ok(());
                                    }
                                }
                            }
                            fcp_core::ExtensionTarget::Service(service_id) => {
                                info!(call_id = %call_id, service_id = %service_id, extension = %extension_number, "Resolving legacy service-backed extension");

                                use fcp_core::storage::{ConferenceStore, VoicemailStore};

                                match storage
                                    .get_mailbox_by_extension(&tenant_id, extension_number)
                                    .await
                                {
                                    Ok(Some(mailbox)) if mailbox.enabled => {
                                        info!(
                                            call_id = %call_id,
                                            extension = %extension_number,
                                            mailbox_id = %mailbox.id,
                                            "Routing service-backed extension to voicemail server"
                                        );
                                        match self
                                            .route_cfna(
                                                request,
                                                &a_leg_handle_arc,
                                                call_id,
                                                "voicemail",
                                                Some(extension_number),
                                                None,
                                            )
                                            .await
                                        {
                                            Ok(()) => return Ok(()),
                                            Err(e) => {
                                                error!(call_id = %call_id, error = %e, "Voicemail routing failed");
                                                let response = self.create_simple_response(
                                                    request,
                                                    480,
                                                    "Temporarily Unavailable",
                                                );
                                                a_leg_handle.send_final(response).await;
                                                let _ = self.call_manager.terminate_call(
                                                    &call_id,
                                                    Some(format!(
                                                        "Voicemail routing failed: {}",
                                                        e
                                                    )),
                                                );
                                                return Ok(());
                                            }
                                        }
                                    }
                                    Ok(Some(_)) | Ok(None) => {}
                                    Err(e) => {
                                        error!(call_id = %call_id, extension = %extension_number, error = %e, "Failed to look up mailbox for service-backed extension");
                                        let response = self.create_simple_response(
                                            request,
                                            500,
                                            "Internal Server Error",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some(format!("Failed to look up mailbox: {}", e)),
                                        );
                                        return Ok(());
                                    }
                                }

                                let meeting_result = storage
                                    .get_meeting_by_room_number(&tenant_id, extension_number)
                                    .await;

                                match meeting_result {
                                    Ok(Some(meeting)) if meeting.enabled => {
                                        let conf_host = self
                                            .conference_target_host(
                                                &meeting.tenant_id,
                                                extension_number,
                                                ConferenceVideoNeed::of(&meeting).as_ref(),
                                            )
                                            .await;
                                        if let Some(host) = conf_host.as_deref() {
                                            let target_uri =
                                                format!("sip:{}@{}", extension_number, host);
                                            self.mark_conference_call(
                                                &call_id,
                                                extension_number,
                                                host,
                                            );
                                            info!(
                                                call_id = %call_id,
                                                room_number = %extension_number,
                                                meeting_name = %meeting.name,
                                                target_uri = %target_uri,
                                                "Routing service-backed extension to conference server"
                                            );
                                            match sip_registrar::Binding::new(
                                                SmolStr::new(destination_aor),
                                                SmolStr::new(&target_uri),
                                                std::time::Duration::from_secs(3600),
                                            ) {
                                                Ok(binding) => {
                                                    extension_contacts = vec![binding];
                                                }
                                                Err(e) => {
                                                    warn!(call_id = %call_id, error = %e, "Failed to create conference binding");
                                                    let response = self.create_simple_response(
                                                        request,
                                                        500,
                                                        "Internal Server Error",
                                                    );
                                                    a_leg_handle.send_final(response).await;
                                                    let _ = self.call_manager.terminate_call(
                                                        &call_id,
                                                        Some(format!(
                                                            "Failed to create conference binding: {}",
                                                            e
                                                        )),
                                                    );
                                                    return Ok(());
                                                }
                                            }
                                        } else {
                                            let response = self.create_simple_response(
                                                request,
                                                503,
                                                "Conference Service Unavailable",
                                            );
                                            a_leg_handle.send_final(response).await;
                                            let _ = self.call_manager.terminate_call(
                                                &call_id,
                                                Some(
                                                    "Conference room found but conference_server_uri is not configured"
                                                        .to_string(),
                                                ),
                                            );
                                            return Ok(());
                                        }
                                    }
                                    Ok(Some(_)) | Ok(None) => {
                                        let response = self.create_simple_response(
                                            request,
                                            404,
                                            "Service Route Not Found",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some(format!(
                                                "Service-backed extension {} has no routable queue, mailbox, or conference target (service_id: {})",
                                                extension_number, service_id
                                            )),
                                        );
                                        return Ok(());
                                    }
                                    Err(e) => {
                                        warn!(call_id = %call_id, extension = %extension_number, error = %e, "Failed to look up conference target for service-backed extension");
                                        let response = self.create_simple_response(
                                            request,
                                            500,
                                            "Internal Server Error",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some(format!(
                                                "Failed to look up service-backed conference target: {}",
                                                e
                                            )),
                                        );
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        // Extension not found — check if this is a conference room number.
                        // If so, forward the INVITE to the conference server via B2BUA.
                        if let Some(ref storage) = self.storage {
                            use fcp_core::storage::ConferenceStore;

                            // Look up meeting by room number only within the call's tenant.
                            let meeting_result = storage
                                .get_meeting_by_room_number(&tenant_id, extension_number)
                                .await;

                            match meeting_result {
                                Ok(Some(meeting)) if meeting.enabled => {
                                    // The same placement as the other conference
                                    // routes: the cluster's choice of node, else
                                    // the configured conference server.
                                    let conf_host = self
                                        .conference_target_host(
                                            &tenant_id,
                                            extension_number,
                                            ConferenceVideoNeed::of(&meeting).as_ref(),
                                        )
                                        .await;
                                    if let Some(host) = conf_host.as_deref() {
                                        let target_uri =
                                            format!("sip:{}@{}", extension_number, host);
                                        self.mark_conference_call(&call_id, extension_number, host);
                                        info!(
                                            call_id = %call_id,
                                            room_number = %extension_number,
                                            meeting_name = %meeting.name,
                                            target_uri = %target_uri,
                                            "Routing to conference server"
                                        );

                                        // Create a binding to the conference server
                                        match sip_registrar::Binding::new(
                                            SmolStr::new(destination_aor),
                                            SmolStr::new(&target_uri),
                                            std::time::Duration::from_secs(3600),
                                        ) {
                                            Ok(binding) => {
                                                extension_contacts = vec![binding];

                                                // Disable media anchoring for conference calls.
                                                // RTP must flow directly between the phone and
                                                // the conference server so that the conference
                                                // server's audio bridge handles mixing, DTMF
                                                // detection (PIN entry), and recording.
                                                let old_session = self
                                                    .call_manager
                                                    .get_call(&call_id)
                                                    .and_then(|c| {
                                                        if c.media_anchored {
                                                            c.media_session_id
                                                        } else {
                                                            None
                                                        }
                                                    });
                                                let _ = self.call_manager.update_call(
                                                    &call_id,
                                                    |call| {
                                                        call.media_anchored = false;
                                                        call.media_session_id = None;
                                                    },
                                                );
                                                // Tear down the pre-allocated media session
                                                if let (Some(session_id), Some(mm)) =
                                                    (old_session, &self.media_manager)
                                                {
                                                    if let Err(e) =
                                                        mm.terminate_session(&session_id).await
                                                    {
                                                        debug!(call_id = %call_id, error = %e, "Failed to terminate pre-allocated media session (may already be gone)");
                                                    }
                                                }
                                                info!(call_id = %call_id, "Media anchoring disabled for conference call — RTP flows directly to conference server");
                                            }
                                            Err(e) => {
                                                warn!(call_id = %call_id, error = %e, "Failed to create conference binding");
                                            }
                                        }
                                    } else {
                                        warn!(
                                            call_id = %call_id,
                                            room_number = %extension_number,
                                            "Conference room found but conference_server_uri is not configured"
                                        );
                                    }
                                }
                                Ok(Some(_meeting)) => {
                                    debug!(call_id = %call_id, room = %extension_number, "Conference room disabled");
                                }
                                Ok(None) => {
                                    debug!(call_id = %call_id, room = %extension_number, tenant_id = %tenant_id, "No conference room found");
                                }
                                Err(e) => {
                                    warn!(call_id = %call_id, room = %extension_number, error = %e, "Failed to look up conference room");
                                }
                            }
                        }
                        if extension_contacts.is_empty() {
                            debug!(call_id = %call_id, extension = %extension_number, "Extension not found, falling back to direct registrar lookup");
                        }
                    }
                    Err(e) => {
                        warn!(call_id = %call_id, extension = %extension_number, error = %e, "Failed to lookup extension, falling back to direct registrar lookup");
                    }
                }
            }

            // If we found contacts via extension routing, use them — or
            // none, when the user's only phones are apps being woken
            // (`docs/PUSH.md` §2.2): the fork waits for them.
            if !extension_contacts.is_empty() || self.push_waits.has(&call_id) {
                extension_contacts
            } else {
                // Fall back to direct registrar lookup (existing behavior)
                debug!(call_id = %call_id, destination = %destination_aor, "No extension found, using direct registrar lookup");

                if let Some(registrar) = &self.registrar {
                    let found = match registrar.get_registrations(destination_aor).await {
                        Ok(bindings) if !bindings.is_empty() => Ok(bindings),
                        other => {
                            let named = self
                                .registrations_of_named_user(&call_id, destination_aor)
                                .await;
                            if named.is_empty() {
                                other
                            } else {
                                Ok(named)
                            }
                        }
                    };
                    match found {
                        Ok(bindings) if !bindings.is_empty() => {
                            debug!(
                                call_id = %call_id,
                                destination = %destination_aor,
                                contacts = bindings.len(),
                                "Found registered contacts for destination"
                            );

                            // The address is a user's: their plan
                            // (`docs/ENTERPRISE_CALLING.md` §3.1).
                            let mut bindings = bindings;
                            if let Some(storage) = &self.storage {
                                let callee_username = destination_aor
                                    .strip_prefix("sip:")
                                    .or_else(|| destination_aor.strip_prefix("sips:"))
                                    .unwrap_or(destination_aor)
                                    .split('@')
                                    .next()
                                    .unwrap_or(destination_aor);
                                if let Some(tenant_id) = self
                                    .call_manager
                                    .get_call(&call_id)
                                    .map(|call| call.tenant_id)
                                    .or(self.default_tenant_id)
                                {
                                    if let Some(user_id) = crate::ring_plan::user_of(
                                        storage,
                                        &tenant_id,
                                        callee_username,
                                    )
                                    .await
                                    {
                                        match self
                                            .ring_by_plan(
                                                request,
                                                &a_leg_handle_arc,
                                                call_id,
                                                &tenant_id,
                                                Some(user_id),
                                                bindings,
                                                destination_aor,
                                            )
                                            .await?
                                        {
                                            Some(contacts) => bindings = contacts,
                                            None => return Ok(()),
                                        }
                                    }
                                }
                            }

                            bindings
                        }
                        Ok(_) => {
                            // Try outbound route provider with failover before giving up.
                            // Classified resolution (reachability stage): 404 when
                            // nothing is configured for the destination, 503 when
                            // routes exist but no trunk is currently available.
                            if let Some(provider) = &self.outbound_route_provider {
                                let routes = match self
                                    .classified_routes(provider, &call_id, destination_aor)
                                    .await
                                {
                                    RouteResolution::Routes(routes) => routes,
                                    RouteResolution::Unavailable => {
                                        warn!(call_id = %call_id, destination = %destination_aor, "Routes configured but no trunk currently available");
                                        let response = self.create_simple_response(
                                            request,
                                            503,
                                            "Service Unavailable",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("All trunks unavailable".to_string()),
                                        );
                                        return Ok(());
                                    }
                                    RouteResolution::NoRoute => {
                                        info!(call_id = %call_id, destination = %destination_aor, "Destination not registered and no route configured");
                                        let response =
                                            self.create_simple_response(request, 404, "Not Found");
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("No route to destination".to_string()),
                                        );
                                        return Ok(());
                                    }
                                };
                                if let Some(route) = routes.first() {
                                    info!(call_id = %call_id, target = %route.target_uri, trunk = ?route.trunk_name(), allowed_codecs = ?route.allowed_codecs(), routes = routes.len(), "Routing via outbound trunk (extension path, failover available)");
                                    // Store trunk name and allowed codecs on call for lifecycle tracking
                                    {
                                        let trunk_name = route.trunk_name();
                                        let trunk_id = route.trunk_id();
                                        let allowed_codecs = route.allowed_codecs();
                                        let route_trunks = route_trunks(&routes);
                                        let _ = self.call_manager.update_call(&call_id, |call| {
                                            call.trunk_name = trunk_name;
                                            call.trunk_id = trunk_id;
                                            call.outbound_allowed_codecs = allowed_codecs;
                                            call.route_trunks = route_trunks;
                                        });
                                    }
                                    // Build bindings for primary + failover routes
                                    let mut failover_bindings = Vec::with_capacity(routes.len());
                                    for r in &routes {
                                        match sip_registrar::Binding::new(
                                            SmolStr::new(destination_aor),
                                            SmolStr::new(format!("<{}>", r.target_uri)),
                                            Duration::from_secs(3600),
                                        ) {
                                            Ok(binding) => failover_bindings.push(binding),
                                            Err(e) => {
                                                warn!(call_id = %call_id, target = %r.target_uri, error = %e, "Skipping failover route: failed to create binding");
                                            }
                                        }
                                    }
                                    if failover_bindings.is_empty() {
                                        error!(call_id = %call_id, "All failover route bindings failed");
                                        let response = self.create_simple_response(
                                            request,
                                            500,
                                            "Internal Server Error",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("Failed to create trunk binding".to_string()),
                                        );
                                        return Ok(());
                                    }
                                    failover_bindings
                                } else {
                                    info!(
                                        call_id = %call_id,
                                        destination = %destination_aor,
                                        "Destination not registered (no bindings), no outbound route"
                                    );
                                    let response = self.create_simple_response(
                                        request,
                                        480,
                                        "Temporarily Unavailable",
                                    );
                                    a_leg_handle.send_final(response).await;
                                    let _ = self.call_manager.terminate_call(
                                        &call_id,
                                        Some("Destination not registered".to_string()),
                                    );
                                    return Ok(());
                                }
                            } else {
                                info!(
                                    call_id = %call_id,
                                    destination = %destination_aor,
                                    "Destination not registered (no bindings)"
                                );
                                let response = self.create_simple_response(
                                    request,
                                    480,
                                    "Temporarily Unavailable",
                                );
                                a_leg_handle.send_final(response).await;
                                let _ = self.call_manager.terminate_call(
                                    &call_id,
                                    Some("Destination not registered".to_string()),
                                );
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            error!(
                                call_id = %call_id,
                                destination = %destination_aor,
                                error = %e,
                                "Failed to lookup destination"
                            );
                            let response =
                                self.create_simple_response(request, 500, "Internal Server Error");
                            a_leg_handle.send_final(response).await;
                            let _ = self.call_manager.terminate_call(
                                &call_id,
                                Some(format!("Failed to lookup destination: {}", e)),
                            );
                            return Ok(());
                        }
                    }
                } else {
                    // No registrar configured - cannot forward call
                    error!(
                        call_id = %call_id,
                        "Cannot forward call: No registrar configured"
                    );
                    let response = self.create_simple_response(request, 500, "Service Unavailable");
                    a_leg_handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call(&call_id, Some("No registrar configured".to_string()));
                    return Ok(());
                }
            }
        };

        // Parallel forking: Extract URIs and flow tokens from all contacts
        let target_contacts: Vec<TargetContact> = destination_contacts
            .iter()
            .map(|contact: &sip_registrar::Binding| {
                let contact_str = contact.contact();
                // Extract URI from Contact header (strip angle brackets if present)
                // Contact format: <sip:user@host> or sip:user@host
                let uri = if let Some(start) = contact_str.find('<') {
                    if let Some(end) = contact_str.find('>') {
                        contact_str[start + 1..end].to_string()
                    } else {
                        contact_str.to_string()
                    }
                } else {
                    // No angle brackets - use as-is, but strip any contact-params after semicolon outside URI
                    contact_str
                        .split(';')
                        .next()
                        .unwrap_or(contact_str)
                        .to_string()
                };
                // Its flow on another node: rung through that node.
                let relay = self
                    .flow_relays
                    .get(&call_id)
                    .and_then(|r| r.get(contact_str).cloned());
                TargetContact {
                    uri,
                    flow_token: match relay {
                        Some(_) => None,
                        None => contact.flow_token().map(|s: &str| s.to_string()),
                    },
                    egress: crate::trunk_egress::TrunkEgress::none(),
                    relay,
                }
            })
            .collect();
        self.flow_relays.remove(&call_id);

        // Deduplicate contacts by host:port to avoid sending multiple INVITEs to
        // the same physical device. Softphones like Zoiper register multiple
        // contacts with different rinstance params but the same host:port.
        let mut target_contacts = dedup_target_contacts_by_hostport(target_contacts);

        info!(
            call_id = %call_id,
            target_count = target_contacts.len(),
            targets = ?target_contacts.iter().map(|t| &t.uri).collect::<Vec<_>>(),
            "Parallel forking INVITE to all registered devices"
        );

        // Update call state to Inviting and create B-leg
        let _ = self.call_manager.update_call(&call_id, |call| {
            call.state = fcp_b2bua_core::CallState::Inviting;

            // Create B-leg CallLeg if it doesn't exist
            if call.b_leg.is_none() {
                // Extract To header for callee info
                let to_header = request.headers().get("To").unwrap_or("");
                let (callee_uri, callee_display_name) = {
                    let parts: Vec<&str> = to_header.split(';').collect();
                    let name_addr = parts[0].trim();
                    if let Some(start) = name_addr.find('<') {
                        if let Some(end) = name_addr.find('>') {
                            let uri = name_addr[start + 1..end].to_string();
                            let display_name =
                                name_addr[..start].trim().trim_matches('"').to_string();
                            let display = if display_name.is_empty() {
                                None
                            } else {
                                Some(display_name)
                            };
                            (uri, display)
                        } else {
                            (name_addr.to_string(), None)
                        }
                    } else {
                        (name_addr.to_string(), None)
                    }
                };

                call.b_leg = Some(fcp_b2bua_core::CallLeg::new_outbound(
                    callee_uri.clone(),
                    callee_display_name.clone(),
                ));

                debug!(
                    call_id = %call_id,
                    callee_uri = %callee_uri,
                    callee_display_name = ?callee_display_name,
                    "Created B-leg CallLeg"
                );
            }
        });

        // Extract SDP from A-leg INVITE to forward to B-leg
        let mut a_leg_sdp = crate::body::request_sdp(request);

        // What the trunk carrying this leg says about SRTP
        // (`docs/TENANT_TRUNKS.md` §6). The node terminates the encryption,
        // so it can only offer SRTP on a leg whose media it anchors; a
        // trunk that requires SRTP on a call this node does not anchor has
        // no route it can honour, and says so rather than sending RTP in
        // the clear to a carrier expecting it encrypted.
        let srtp_mode = self
            .call_manager
            .get_call(&call_id)
            .and_then(|c| c.route_trunks.first().map(|r| r.trunk.srtp.mode))
            .unwrap_or(fcp_core::trunk::SrtpMode::Disabled);
        let anchored = self
            .call_manager
            .get_call(&call_id)
            .is_some_and(|c| c.media_anchored && c.media_session_id.is_some());
        let srtp_offer = match (srtp_mode, anchored) {
            (fcp_core::trunk::SrtpMode::Disabled, _) => None,
            (_, true) => match fcp_media::SrtpOffer::generate() {
                Ok(offer) => Some(offer),
                Err(e) => {
                    warn!(call_id = %call_id, error = %e, "Could not generate SRTP key material for the trunk");
                    None
                }
            },
            (fcp_core::trunk::SrtpMode::Required, false) => {
                warn!(
                    call_id = %call_id,
                    "The trunk requires SRTP and this call's media is not anchored here; \
                     refusing rather than sending it in the clear"
                );
                let response = self.create_simple_response(request, 488, "Not Acceptable Here");
                a_leg_handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    &call_id,
                    Some("Trunk requires SRTP on unanchored media".to_string()),
                );
                return Ok(());
            }
            (fcp_core::trunk::SrtpMode::Preferred, false) => {
                debug!(
                    call_id = %call_id,
                    "The trunk prefers SRTP but this call's media is not anchored here; \
                     offering plain RTP"
                );
                None
            }
        };
        if let Some(offer) = &srtp_offer {
            // An offer lives until its answer. One whose call ended without
            // an answer — cancelled, refused, timed out — is dropped here,
            // so the map holds only calls in flight.
            self.b_leg_srtp
                .retain(|id, _| self.call_manager.get_call(id).is_some());
            self.b_leg_srtp.insert(call_id, offer.clone());
        }

        // Rewrite SDP if media is anchored on server
        if let Some(ref original_sdp) = a_leg_sdp {
            // Check if this call has anchored media
            let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                if call.media_anchored {
                    call.media_session_id
                } else {
                    None
                }
            });

            if let (Some(media_session_id), Some(media_manager)) =
                (media_session_id, &self.media_manager)
            {
                // Get media session to extract leg B's RTP port
                match media_manager.get_session(&media_session_id).await {
                    Ok(session) => {
                        let (rtp_port, _rtcp_port) = session.ports_for(fcp_media::backend::Leg::B);

                        // Rewrite SDP to point to server's RTP endpoint, and
                        // offer the trunk's SRTP on it when it wants any.
                        let rewritten_sdp = fcp_media::rewrite_sdp_for_anchoring(
                            original_sdp,
                            &self.local_addr,
                            rtp_port,
                            srtp_offer.as_ref().map(|o| &o.sdp),
                        );

                        info!(
                            call_id = %call_id,
                            original_port = ?original_sdp.lines().find(|l| l.starts_with("m=audio")),
                            server_ip = %self.local_addr,
                            rtp_port = rtp_port,
                            "Rewrote SDP for media anchoring"
                        );

                        a_leg_sdp = Some(rewritten_sdp);
                    }
                    Err(e) => {
                        error!(
                            call_id = %call_id,
                            media_session_id = %media_session_id,
                            error = %e,
                            "Failed to get media session for SDP rewriting, using original SDP"
                        );
                    }
                }
            }
        }

        // Filter B-leg SDP based on trunk codec restrictions (if any) and A-leg codecs.
        //
        // When the trunk has `allowed_codecs`, use those in preference order. If
        // there is overlap with the A-leg codecs, intersect (trunk order); if no
        // overlap, use trunk codecs only — the media pipeline will transcode.
        //
        // When no trunk restriction is configured, fall back to A-leg preferred
        // codec + G.711 variants (original behaviour).
        if let Some(ref sdp) = a_leg_sdp {
            // Retrieve trunk-level codec restrictions from the call (set at routing time)
            let trunk_allowed: Option<Vec<String>> = self
                .call_manager
                .get_call(&call_id)
                .and_then(|c| c.outbound_allowed_codecs.clone());

            let a_leg_codecs = fcp_media::sdp::parse_sdp_codecs(sdp);

            let allowed_owned: Vec<String> = if let Some(ref trunk_codecs) = trunk_allowed {
                // Trunk has codec restrictions — intersect with A-leg (trunk preference order)
                let intersection: Vec<String> = trunk_codecs
                    .iter()
                    .filter(|tc| {
                        a_leg_codecs
                            .iter()
                            .any(|ac| ac.encoding_name.eq_ignore_ascii_case(tc))
                    })
                    .cloned()
                    .collect();

                if intersection.is_empty() {
                    // No overlap — use trunk codecs only; transcoding will bridge the gap
                    info!(
                        call_id = %call_id,
                        trunk_codecs = ?trunk_codecs,
                        a_leg_codecs = ?a_leg_codecs.iter().map(|c| &c.encoding_name).collect::<Vec<_>>(),
                        "No codec overlap between trunk and A-leg — transcoding required"
                    );
                    trunk_codecs.clone()
                } else {
                    info!(
                        call_id = %call_id,
                        intersection = ?intersection,
                        "Filtered B-leg SDP to trunk/A-leg codec intersection"
                    );
                    intersection
                }
            } else if let Some(preferred) = a_leg_codecs.first() {
                // No trunk restriction — use A-leg preferred + G.711 fallback
                let mut allowed: Vec<String> = vec![preferred.encoding_name.clone()];
                for codec in &a_leg_codecs {
                    let name = &codec.encoding_name;
                    if (name.eq_ignore_ascii_case("PCMU") || name.eq_ignore_ascii_case("PCMA"))
                        && !allowed.iter().any(|a| a.eq_ignore_ascii_case(name))
                    {
                        allowed.push(name.clone());
                    }
                }
                // With the media through the node, G.711 is offered even
                // when the caller did not: the node transcodes from the
                // caller's codec, so an Opus-only softphone reaches a
                // G.711-only desk phone (`docs/DEEP_TESTING.md` D2d). Only
                // for a codec the media pipeline transcodes.
                let transcodable = ["PCMU", "PCMA", "opus", "L16"]
                    .iter()
                    .any(|c| preferred.encoding_name.eq_ignore_ascii_case(c));
                if anchored && transcodable {
                    for g711 in ["PCMU", "PCMA"] {
                        if !allowed.iter().any(|a| a.eq_ignore_ascii_case(g711)) {
                            allowed.push(g711.to_string());
                        }
                    }
                }
                info!(
                    call_id = %call_id,
                    preferred_codec = %preferred.encoding_name,
                    allowed_codecs = ?allowed,
                    "Filtered B-leg SDP to match A-leg codecs (no trunk restriction)"
                );
                allowed
            } else {
                Vec::new()
            };

            if !allowed_owned.is_empty() {
                let allowed_refs: Vec<&str> = allowed_owned.iter().map(|s| s.as_str()).collect();
                // A codec chosen above that the caller did not offer (a
                // trunk's, or G.711 to transcode to) is added before the
                // filter, which only keeps.
                let offered = if anchored {
                    fcp_media::sdp::ensure_audio_codecs(sdp, &allowed_refs)
                } else {
                    sdp.clone()
                };
                let filtered = fcp_media::sdp::filter_sdp_codecs(&offered, &allowed_refs);
                a_leg_sdp = Some(filtered);
            }
        }

        // A trunk that takes DTMF as SIP INFO, or takes none, is not
        // offered RFC 2833 (`docs/TENANT_TRUNKS.md` §6): offering
        // `telephone-event` and then not sending it is how DTMF goes
        // missing on a carrier that believes the offer. The primary route's
        // trunk decides, as it does for codecs.
        if let Some(sdp) = a_leg_sdp.as_deref() {
            let takes_rfc2833 = self
                .call_manager
                .get_call(&call_id)
                .and_then(|c| c.route_trunks.first().cloned())
                .map(|route| {
                    matches!(route.trunk.dtmf.mode, fcp_core::trunk::TrunkDtmfMode::Relay)
                });
            if takes_rfc2833 == Some(false) {
                let without = fcp_media::sdp::drop_telephone_event(sdp);
                if without != sdp {
                    debug!(
                        call_id = %call_id,
                        "The trunk does not take RFC 2833; offering no telephone-event"
                    );
                    a_leg_sdp = Some(without);
                }
            }
        }

        // Extract caller's identity from A-leg for B2BUA caller ID preservation
        let peer_for_call = self
            .call_manager
            .get_call(&call_id)
            .and_then(|c| c.a_leg.contact);
        // A call an assistant takes for an executive shows whose it is
        // (`docs/ENTERPRISE_CALLING.md` §3.10).
        let caller_display_name = self
            .call_manager
            .get_call(&call_id)
            .and_then(|c| c.leg_display_name)
            .or_else(|| extract_from_display_name(request));
        let caller_from_uri = extract_from_uri(request).ok().map(normalize_from_uri);

        // Emergency calls go out with the location's callback number (ELIN)
        // as caller ID so the PSAP's ALI lookup and callback both work, and
        // carry P-Asserted-Identity + Priority: emergency for the trunk.
        let (caller_from_uri, emergency_headers) = match &emergency_ctx {
            Some(em) => {
                let (from, headers) =
                    crate::emergency::outbound_identity(em, caller_from_uri, &self.local_addr);
                (from, Some(headers))
            }
            None => (caller_from_uri, None),
        };
        // A call into a conference room names itself, so the room's record
        // of the leg shares this call's id, and says whose it is: the
        // conference server is reached at an address, which names no
        // tenant, and believes the tenant and caller from its trusted
        // edges (`docs/EDGE_SECURITY.md` §2.3). A call up a site's link
        // is named below, with its site.
        let emergency_headers = match self
            .call_manager
            .get_call(&call_id)
            .filter(|c| c.conference_room.is_some())
        {
            Some(call) => {
                let mut headers = emergency_headers.unwrap_or_default();
                let _ = headers.push(
                    SmolStr::new(fcp_core::CALL_ID_HEADER),
                    SmolStr::new(call_id.to_string()),
                );
                let caller = extract_from_uri(request)
                    .ok()
                    .and_then(|u| u.user().map(str::to_string));
                if let (None, Some(caller)) = (call.link_site, caller) {
                    let _ = headers.push(
                        SmolStr::new(fcp_core::CALLER_TENANT_HEADER),
                        SmolStr::new(call.tenant_id.to_string()),
                    );
                    let _ = headers.push(
                        SmolStr::new(fcp_core::CALLER_USER_HEADER),
                        SmolStr::new(caller),
                    );
                }
                Some(headers)
            }
            None => emergency_headers,
        };

        // An edge in front of phones says where its caller is, so the next
        // hop can place a device that moves (§6.2). The caller's own
        // address is the A-leg's source.
        let emergency_headers = match (self.vouch_client_address, peer_for_call) {
            (true, Some(client)) => {
                let mut headers = emergency_headers.unwrap_or_default();
                let _ = headers.push(
                    SmolStr::new(fcp_core::CLIENT_ADDRESS_HEADER),
                    SmolStr::new(client.ip().to_string()),
                );
                Some(headers)
            }
            _ => emergency_headers,
        };

        // A call that came up a site's link is the site's tenant's own; the
        // next hop is told so, and believes it from its trusted edges
        // (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
        let emergency_headers = match self
            .call_manager
            .get_call(&call_id)
            .and_then(|c| c.link_site.map(|site| (c.tenant_id, site)))
        {
            Some((tenant, site)) => {
                let mut headers = emergency_headers.unwrap_or_default();
                let _ = headers.push(
                    SmolStr::new(fcp_core::CALLER_TENANT_HEADER),
                    SmolStr::new(tenant.to_string()),
                );
                let _ = headers.push(
                    SmolStr::new(fcp_core::CALLER_SITE_HEADER),
                    SmolStr::new(site.to_string()),
                );
                Some(headers)
            }
            None => emergency_headers,
        };

        if let Some(ref name) = caller_display_name {
            debug!(call_id = %call_id, caller_name = %name, "Preserving caller display name for B-leg");
        }
        if let Some(ref uri) = caller_from_uri {
            debug!(call_id = %call_id, normalized_caller_uri = ?uri, "Preserving normalized caller From URI for B-leg");
        }

        // Set caller identity on UAC for this call (B2BUA caller ID preservation)
        uac.inner()
            .set_display_name(caller_display_name.clone())
            .await;
        uac.inner().set_from_uri(caller_from_uri.clone()).await;

        // What each trunk makes of the leg it carries: the identity it
        // presents, the session timer it asks for
        // (`docs/TENANT_TRUNKS.md` §6). A target no trunk carries — a
        // registered device — is left as the B2BUA built it.
        {
            let route_trunks = self
                .call_manager
                .get_call(&call_id)
                .map(|c| c.route_trunks)
                .unwrap_or_default();
            if !route_trunks.is_empty() {
                let emergency = emergency_ctx.is_some();
                let caller = caller_from_uri.as_ref().map(|u| u.as_str().to_string());
                // The caller's line and whether their class withholds it
                // (`docs/ENTERPRISE_CALLING.md` §3.7); an emergency call
                // presents its callback number whatever either says.
                let (line, withheld) = if emergency {
                    (None, false)
                } else {
                    self.caller_presentation(&call_id, caller.as_deref()).await
                };
                // A forward or a find-me step leaving by a trunk names the
                // number it was diverted from (§3.6, Q7).
                let diversion = if emergency {
                    None
                } else {
                    self.diversion_of(&call_id).await
                };
                for target in &mut target_contacts {
                    if let Some(route) = answering_trunk(&route_trunks, &target.uri) {
                        target.egress = crate::trunk_egress::TrunkEgress::of(
                            &route.trunk,
                            line.as_deref().or(caller.as_deref()),
                            emergency,
                        );
                        if let Some(line) = &line {
                            target.egress.present_as(line);
                        }
                        if withheld {
                            target.egress.withhold(caller.as_deref());
                        }
                        if let Some((number, reason)) = &diversion {
                            target.egress.divert(
                                &route.trunk,
                                number,
                                reason,
                                self.uas.local_uri.host(),
                            );
                        }
                        // The numbers the carrier will see are the ones the
                        // call is signed over (RFC 8224).
                        let orig_tn = tn_of(target.egress.from.as_deref().or(caller.as_deref()));
                        let dest_tn = tn_of(Some(target.uri.as_str()));
                        target.egress.sign(
                            &route.trunk,
                            self.call_signer.as_ref(),
                            &orig_tn,
                            &dest_tn,
                        );
                        debug!(
                            call_id = %call_id,
                            trunk = %route.name(),
                            target = %target.uri,
                            from = ?target.egress.from,
                            headers = target.egress.headers.len(),
                            "The trunk's own identity on this leg"
                        );
                    }
                }
            }
        }

        // Cap target list to configured max_parallel_legs
        let max_legs = self.b2bua_config.forking.max_parallel_legs;
        if target_contacts.len() > max_legs {
            info!(
                call_id = %call_id,
                total_targets = target_contacts.len(),
                max_parallel_legs = max_legs,
                "Capping parallel forking targets to configured maximum"
            );
            target_contacts.truncate(max_legs);
        }

        // Create cancel_notify BEFORE sending B-leg INVITEs so CANCEL handler can
        // signal the forking task even if CANCEL arrives while INVITEs are being sent.
        // This fixes a race condition where CANCEL could arrive during the INVITE loop.
        // Stored in the same update that reads whether the caller has
        // already cancelled: a CANCEL between the call's creation and here
        // answered the caller 487 and ended the call, and no one is rung.
        let cancel_notify = Arc::new(tokio::sync::Notify::new());
        let cancel_notify_clone = Arc::clone(&cancel_notify);
        let mut already_cancelled = false;
        match self.call_manager.update_call(&call_id, |call| {
            already_cancelled = call.caller_cancelled;
            call.cancel_notify = Some(cancel_notify_clone);
        }) {
            Ok(_) if already_cancelled => {
                info!(call_id = %call_id, "The caller cancelled before anyone was rung");
                return Ok(());
            }
            Ok(_) => {
                debug!(
                    call_id = %call_id,
                    "Stored cancel_notify in call before B-leg INVITEs"
                );
            }
            Err(e) => {
                info!(
                    call_id = %call_id,
                    error = %e,
                    "The call ended before anyone was rung"
                );
                return Ok(());
            }
        }

        // An emergency call tries its routes one at a time, never two
        // carriers at once, each with the location its trunk takes: the
        // first now, the rest as attempts fail (see handle_parallel_forking).
        if emergency_ctx.is_some() {
            self.emergency_fallbacks.insert(
                call_id,
                EmergencyFallback {
                    attempts: emergency_routes
                        .iter()
                        .map(crate::emergency::EmergencyAttempt::from_route)
                        .collect(),
                    current: None,
                    location_refused: false,
                    sdp: a_leg_sdp.clone(),
                    from: caller_from_uri.clone(),
                    display_name: caller_display_name.clone(),
                    headers: emergency_headers.clone(),
                },
            );
            uac.inner().set_display_name(None).await;
            uac.inner().set_from_uri(None).await;
            let Some((target, outbound)) = self.next_emergency_leg(call_id).await else {
                error!(
                    call_id = %call_id,
                    "EMERGENCY CALL COULD NOT BE PLACED: no emergency INVITE could be sent"
                );
                let response = self.create_simple_response(request, 503, "Service Unavailable");
                a_leg_handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    &call_id,
                    Some("503 No emergency INVITE could be sent".to_string()),
                );
                return Ok(());
            };
            let _ = self.call_manager.update_call(&call_id, |call| {
                call.state = fcp_b2bua_core::CallState::Proceeding;
            });
            let handler = self.clone();
            let request_clone = request.clone();
            let a_leg_handle_arc = Arc::new(a_leg_handle.clone());
            tokio::spawn(async move {
                if let Err(e) = handler
                    .handle_parallel_forking(
                        request_clone,
                        a_leg_handle_arc,
                        vec![(target, outbound, None)],
                        call_id,
                        cancel_notify,
                        None,
                    )
                    .await
                {
                    error!(call_id = %call_id, error = %e, "Error handling emergency call attempts");
                }
            });
            return Ok(());
        }

        // A call to carriers tries its trunk routes one at a time, the next
        // when a carrier fails (`docs/TENANT_TRUNKS.md` §5): ringing two
        // carriers at once rings the callee twice. A user's registered
        // devices still ring together.
        if target_contacts.len() > 1 && self.all_trunk_routes(&call_id, &target_contacts) {
            let remaining: std::collections::VecDeque<TargetContact> =
                target_contacts.drain(1..).collect();
            info!(
                call_id = %call_id,
                routes = remaining.len() + 1,
                "Trying the call's trunk routes one at a time"
            );
            self.trunk_fallbacks.insert(
                call_id,
                TrunkFallback {
                    remaining,
                    sdp: a_leg_sdp.clone(),
                    headers: emergency_headers.clone(),
                    from: caller_from_uri.clone(),
                    display_name: caller_display_name.clone(),
                },
            );
        }

        // Create outbound INVITEs to all target contacts in PARALLEL
        // This prevents TLS connection timeouts from blocking other B-legs
        // For TLS contacts with flow tokens (RFC 5626), use the existing connection
        let mut invite_futures = Vec::new();
        for target_contact in &target_contacts {
            invite_futures.push(self.send_b_leg_invite(
                Arc::clone(&uac),
                target_contact.clone(),
                a_leg_sdp.clone(),
                emergency_headers.clone(),
                call_id,
            ));
        }

        // The callee's sleeping apps (`docs/PUSH.md` §2.2): pushed now if
        // live phones ring too (with nothing else to ring, the plan pushed
        // already), and their fork takes each as it comes back.
        let push_now = self.push_waits.has_to_push(&call_id) && !target_contacts.is_empty();
        let relay_tokens: Vec<String> = target_contacts
            .iter()
            .filter_map(|t| t.relay.as_ref().map(|(_, token)| token.clone()))
            .collect();
        let late = if self.push_waits.has(&call_id) {
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            self.push_waits.attach(&call_id, tx);
            Some(LateLegs {
                contacts: rx,
                uac: Arc::clone(&uac),
                sdp: a_leg_sdp.clone(),
                headers: emergency_headers.clone(),
            })
        } else {
            None
        };

        // Wait for all INVITEs to complete (concurrently), and the pushes
        // with them: the race starts once both are out, so a call that is
        // answered at once is cancelled after its push, never before.
        // Each result includes (target_uri, call_handle, flow_token) for RFC 5626 BYE support
        let (results, _) = tokio::join!(futures::future::join_all(invite_futures), async {
            if push_now {
                self.push_waiting(call_id).await;
            }
        });
        let b_leg_calls: Vec<_> = results.into_iter().flatten().collect();
        // A relay that could not be sent: its node is not asked, and the
        // phone is not woken for it.
        for token in &relay_tokens {
            if !b_leg_calls
                .iter()
                .any(|(target, ..)| crate::push::relay_token(target) == Some(token.as_str()))
            {
                self.push_waits.forgo(&call_id, token);
            }
        }

        // Clear caller identity overrides after INVITEs sent
        uac.inner().set_display_name(None).await;
        uac.inner().set_from_uri(None).await;

        // Only apps that are being woken: the caller hears ringing while
        // they come back.
        if b_leg_calls.is_empty() && late.is_some() {
            let mut ringing = self.uas.create_ringing(request);
            self.ensure_response_hygiene(request, &mut ringing);
            if let Some(tag) = ringing
                .headers()
                .get("To")
                .map(SmolStr::new)
                .and_then(|to| extract_tag_value(Some(&to)))
            {
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.a_leg_to_tag = Some(tag);
                });
            }
            a_leg_handle.send_provisional(ringing).await;
        }

        // Check if we successfully created any B-legs
        if b_leg_calls.is_empty() && late.is_none() {
            error!(call_id = %call_id, "Failed to create any B-leg INVITEs");
            let response = self.create_simple_response(request, 500, "Failed to forward call");
            a_leg_handle.send_final(response).await;
            let _ = self.call_manager.terminate_call(
                &call_id,
                Some("Failed to create any B-leg INVITEs".to_string()),
            );
            return Ok(());
        }

        info!(
            call_id = %call_id,
            b_leg_count = b_leg_calls.len(),
            "All B-leg INVITEs sent, racing for first answer"
        );

        // Update call state to Proceeding (cancel_notify was already stored before the INVITE loop)
        let _ = self.call_manager.update_call(&call_id, |call| {
            call.state = fcp_b2bua_core::CallState::Proceeding;
        });

        // Spawn background task to race all B-legs
        let handler = self.clone();
        let request_clone = request.clone();
        let a_leg_handle_arc = Arc::new(a_leg_handle.clone());
        let call_id_clone = call_id;

        tokio::spawn(async move {
            if let Err(e) = handler
                .handle_parallel_forking(
                    request_clone,
                    a_leg_handle_arc,
                    b_leg_calls,
                    call_id_clone,
                    cancel_notify,
                    late,
                )
                .await
            {
                error!(call_id = %call_id_clone, error = %e, "Error handling parallel forking");
            }
        });

        // Return immediately - responses will be handled in background task
        info!(call_id = %call_id, "Parallel forking in progress, handling responses in background");
        Ok(())
    }

    /// Send one B-leg INVITE to `target_contact`: through its RFC 5626
    /// flow when it has one, with the extra headers the call carries and
    /// what its trunk makes of the leg. `None` when it could not be sent
    /// within five seconds.
    async fn send_b_leg_invite(
        &self,
        uac_clone: Arc<SipUserAgentClient>,
        target_contact: TargetContact,
        a_leg_sdp_clone: Option<String>,
        emergency_headers_clone: Option<sip_core::Headers>,
        call_id_clone: fcp_b2bua_core::CallId,
    ) -> Option<(String, Arc<crate::uac::OutboundCall>, Option<String>)> {
        // Reached through the node holding its flow (`docs/PUSH.md` §2.4):
        // sent to that node, naming the flow and the phone.
        let target_uri_clone = match &target_contact.relay {
            Some((at, token)) => format!("sip:{at};{}={token}", crate::push::FLOW_RELAY_URI_PARAM),
            None => target_contact.uri.clone(),
        };
        // A trunk leg to an address its host name resolved to keeps the
        // name in its Request-URI and is sent to the address.
        let (send_uri, destination) = crate::trunk_egress::take_destination(&target_uri_clone);
        let flow_token_clone = target_contact.flow_token.clone();
        let egress = target_contact.egress.clone();
        let header_rules = self.header_rules.clone();
        let flow_registry_clone = Arc::clone(&self.flow_registry);
        let sip_call_id_map_clone = Arc::clone(&self.sip_call_id_map);
        let sip_trace_tx_clone = self.sip_trace_tx.clone();
        let local_addr_clone = self.local_addr.clone();
        let node_id_clone = self.node_id.clone();

        debug!(
            call_id = %call_id_clone,
            target = %target_uri_clone,
            has_sdp = a_leg_sdp_clone.is_some(),
            has_flow_token = flow_token_clone.is_some(),
            trunk_identity = egress.from.is_some(),
            "Creating B-leg INVITE"
        );

        // The emergency headers, then what the trunk adds
        // (`docs/TENANT_TRUNKS.md` §6).
        let mut extra_headers = emergency_headers_clone.clone().unwrap_or_default();
        for (name, value) in &egress.headers {
            let _ = extra_headers.push(SmolStr::new(name), SmolStr::new(value));
        }
        // An executive's call rings its assistants differently (§3.10).
        if let Some(alert) = self
            .call_manager
            .get_call(&call_id_clone)
            .and_then(|c| c.leg_alert_info)
        {
            let _ = extra_headers.push(SmolStr::new("Alert-Info"), SmolStr::new(alert));
        }
        if let Some((_, token)) = &target_contact.relay {
            let _ = extra_headers.push(
                SmolStr::new(crate::push::FLOW_RELAY_HEADER),
                SmolStr::new(token.as_str()),
            );
            let _ = extra_headers.push(
                SmolStr::new(crate::push::FLOW_RELAY_CONTACT_HEADER),
                SmolStr::new(target_contact.uri.as_str()),
            );
        }
        // The phone is told which call this is: the one a push named, so
        // a woken app knows the INVITE for the call already on its screen
        // (`docs/PUSH.md` §2.5), as the push wrote it. A relayed leg names
        // the call it is for.
        if !extra_headers.contains(fcp_core::CALL_ID_HEADER) {
            let named = self
                .call_manager
                .get_call(&call_id_clone)
                .and_then(|c| c.relay_for)
                .map(|c| c.as_uuid().to_string())
                .unwrap_or_else(|| call_id_clone.to_string());
            let _ = extra_headers.push(SmolStr::new(fcp_core::CALL_ID_HEADER), SmolStr::new(named));
        }
        // A retrieve handed to the node holding the call carries the
        // appearance's handoff token (§3.9).
        if let Some((_, (_, token))) = self.sca_handoff_targets.remove(&call_id_clone) {
            let _ = extra_headers.push(
                SmolStr::new(crate::sca::NODE_HANDOFF_HEADER),
                SmolStr::new(token),
            );
        }
        // A softphone registered over TLS (it has a flow) on an anchored
        // call is offered SDES-SRTP with the node's key for the call — the
        // same key for every such device, kept for the answer like a
        // trunk's. Every other leg is offered what it was.
        let a_leg_sdp_clone = match (&flow_token_clone, a_leg_sdp_clone) {
            (Some(_), Some(sdp)) => Some(self.secure_device_offer(&call_id_clone, sdp)),
            (_, sdp) => sdp,
        };
        let invite_options = |sdp: Option<&str>| sip_uac::InviteOptions {
            body: sdp.map(sip_uac::InviteBody::sdp),
            extra_headers: extra_headers.clone(),
            from: egress.from_uri(),
            // The trunk's own header rules, run on the finished
            // INVITE (`docs/TENANT_TRUNKS.md` §6).
            filter: egress.request_filter(header_rules.as_ref()),
            destination: destination.clone(),
        };

        // Add 5-second timeout for each INVITE to prevent TLS hangs
        let invite_result = if let Some(ref flow_token) = flow_token_clone {
            // RFC 5626: Use existing TLS connection via flow token
            if let Some(flow_info) = flow_registry_clone.lookup(flow_token) {
                info!(
                    call_id = %call_id_clone,
                    target = %target_uri_clone,
                    flow_token = %flow_token,
                    peer_addr = %flow_info.peer_addr,
                    "Using RFC 5626 flow for TLS INVITE"
                );
                // The headers a routed target gets, a flow target
                // gets too (the conference call-id, …).
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    uac_clone.invite_via_flow_with_options(
                        &target_uri_clone,
                        invite_options(a_leg_sdp_clone.as_deref()),
                        flow_info.stream,
                        flow_info.peer_addr,
                    ),
                )
                .await
            } else {
                warn!(
                    call_id = %call_id_clone,
                    flow_token = %flow_token,
                    "Flow token not found or connection closed, falling back to normal INVITE"
                );
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    uac_clone.invite_with_options(
                        send_uri.as_str(),
                        invite_options(a_leg_sdp_clone.as_deref()),
                    ),
                )
                .await
            }
        } else {
            // Normal INVITE (UDP/TCP without flow)
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                uac_clone.invite_with_options(
                    send_uri.as_str(),
                    invite_options(a_leg_sdp_clone.as_deref()),
                ),
            )
            .await
        };

        match invite_result {
            Ok(Ok(call)) => {
                // Register B-leg SIP Call-ID → FCP CallId mapping for trace correlation
                let b_leg_invite = call.invite_request().await;
                if let Some(b_leg_cid) = sip_parse::header(b_leg_invite.headers(), "Call-ID") {
                    sip_call_id_map_clone
                        .insert(b_leg_cid.as_str().to_string(), call_id_clone.to_string());
                }

                // Log outbound B-leg INVITE for SIP trace
                if let Some(ref trace_tx) = sip_trace_tx_clone {
                    let b_req = call.invite_request().await;
                    let raw_bytes = sip_parse::serialize_request(&b_req);
                    let raw_str = String::from_utf8_lossy(&raw_bytes);
                    let from_uri = sip_parse::header(b_req.headers(), "From")
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_default();
                    let to_uri = sip_parse::header(b_req.headers(), "To")
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_default();
                    let sip_cid = sip_parse::header(b_req.headers(), "Call-ID")
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_default();
                    let cseq = sip_parse::header(b_req.headers(), "CSeq")
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_default();
                    let trace = fcp_core::SipTraceMessage {
                        id: 0,
                        call_id: call_id_clone.to_string(),
                        sip_call_id: sip_cid,
                        tenant_id: "_system".to_string(),
                        timestamp: std::time::SystemTime::now(),
                        direction: "outbound".to_string(),
                        method_or_status: "INVITE".to_string(),
                        cseq,
                        from_uri,
                        to_uri,
                        source_addr: local_addr_clone.clone(),
                        dest_addr: target_uri_clone.to_string(),
                        transport: if flow_token_clone.is_some() {
                            "tcp".to_string()
                        } else {
                            "udp".to_string()
                        },
                        raw_message: raw_str.to_string(),
                        node_id: Some(node_id_clone.clone()),
                    };
                    let _ = trace_tx.try_send(trace);
                }

                info!(
                    call_id = %call_id_clone,
                    target = %target_uri_clone,
                    via_flow = flow_token_clone.is_some(),
                    "B-leg INVITE sent"
                );
                Some((target_uri_clone, Arc::new(call), flow_token_clone))
            }
            Ok(Err(e)) => {
                warn!(
                    call_id = %call_id_clone,
                    target = %target_uri_clone,
                    error = %e,
                    "Failed to create B-leg INVITE"
                );
                None
            }
            Err(_) => {
                warn!(
                    call_id = %call_id_clone,
                    target = %target_uri_clone,
                    "B-leg INVITE timed out (5s)"
                );
                None
            }
        }
    }

    /// Handle B-leg responses in background task (spawned from forward_invite_to_b_leg)
    #[allow(dead_code)] // TODO: Integrate with B2BUA call flow
    async fn handle_b_leg_responses(
        &self,
        request: sip_core::Request,
        a_leg_handle: Arc<ServerTransactionHandle>,
        b_leg_handle: Arc<crate::uac::OutboundCall>,
        call_id: fcp_b2bua_core::CallId,
    ) -> Result<()> {
        // Track A-leg To-tag for dialog consistency (RFC 3261 §12.1.1)
        // Once we send a provisional with a To-tag, we must use the SAME tag for 200 OK
        let mut a_leg_to_tag: Option<String> = None;

        // Relay provisional responses from B-leg to A-leg until we get a 180 Ringing
        info!(call_id = %call_id, "Waiting for provisional responses from B-leg");
        loop {
            // await_provisional() takes &self, so we can call it directly on the Arc
            let provisional = match b_leg_handle.await_provisional().await {
                Some(p) => p,
                None => break, // No more provisionals
            };
            let status_code = provisional.start_line().code();
            debug!(
                call_id = %call_id,
                code = status_code,
                "Received provisional response from B-leg"
            );

            // Forward all 18x responses to A-leg (180, 183, 199, etc.)
            if (180..200).contains(&status_code) {
                info!(
                    call_id = %call_id,
                    code = status_code,
                    has_rseq = provisional.headers().get("RSeq").is_some(),
                    "Relaying provisional response to A-leg"
                );

                // Create provisional response using UAS helper to ensure proper To-tag generation
                // CRITICAL: We MUST use the same To-tag for both provisional and final responses (RFC 3261 §12.1.1)
                let response = if a_leg_to_tag.is_none() {
                    // First provisional - use UAS helper to generate proper To-tag
                    let initial_resp = self.uas.create_ringing(&request);

                    // Extract and save the To-tag for later use with 200 OK and 487
                    if let Some(to_header) = initial_resp.headers().get("To") {
                        if let Some(tag) = extract_tag_value(Some(&to_header.to_string().into())) {
                            a_leg_to_tag = Some(tag.to_string());
                            debug!(call_id = %call_id, a_leg_to_tag = %tag, "Generated A-leg To-tag from first provisional");

                            // Store tag in Call struct for consistency across all responses
                            let _ = self.call_manager.update_call(&call_id, |call| {
                                call.a_leg_to_tag = Some(tag.to_string());
                            });
                        }
                    }

                    // Determine final status and body
                    let final_status = if status_code != 180 { status_code } else { 180 };
                    let final_reason = if status_code != 180 {
                        provisional.start_line().reason()
                    } else {
                        initial_resp.start_line().reason()
                    };
                    let final_body = if !provisional.body().is_empty() {
                        provisional.body().clone()
                    } else {
                        initial_resp.body().clone()
                    };

                    // Build headers from initial response
                    let mut headers = sip_core::Headers::new();
                    for header in initial_resp.headers().iter() {
                        let _ =
                            headers.push(header.name_smol().clone(), header.value_smol().clone());
                    }

                    // Add Content-Type if we have a body from B-leg
                    if !provisional.body().is_empty() {
                        if let Some(ct) = provisional.headers().get("Content-Type") {
                            let _ = headers.push(SmolStr::new("Content-Type"), ct);
                        }
                        // Update Content-Length
                        headers.remove("Content-Length");
                        let _ = headers.push(
                            SmolStr::new("Content-Length"),
                            SmolStr::new(final_body.len().to_string()),
                        );
                    }

                    // Reconstruct response with desired status and body
                    sip_core::Response::new(
                        StatusLine::new(final_status, final_reason)
                            .expect("Failed to create status line"),
                        headers,
                        final_body,
                    )
                    .expect("Failed to create response")
                } else {
                    // Subsequent provisionals - reuse the established To-tag
                    self.create_provisional_from_b_leg(&request, &provisional)
                };

                info!(call_id = %call_id, code = status_code, "Sending provisional to A-leg");
                a_leg_handle.send_provisional(response).await;

                // Break after first 18x response to proceed to final response handling
                // This prevents hanging while waiting for more provisionals
                break;
            }
        }

        // Note: The CallHandle's dialog field will be updated with the correct
        // confirmed dialog (with proper To tag) when await_final() returns a 200 OK

        // Wait for final response from B-leg
        // Since await_final() takes &self, we can call it directly on the Arc
        // The CANCEL handler can call cancel() concurrently on the same Arc
        info!(call_id = %call_id, "Waiting for final response from B-leg");

        let b_leg_final = match b_leg_handle.await_final().await {
            Ok(response) => response,
            Err(e) => {
                // If call was cancelled, avoid sending 408 and just terminate.
                let cancelled = self
                    .call_manager
                    .get_call(&call_id)
                    .map(|c| c.state.is_terminal())
                    .unwrap_or(false);
                if cancelled {
                    info!(call_id = %call_id, "B-leg await_final aborted due to cancellation");
                    let _ = self
                        .call_manager
                        .terminate_call_with(&call_id, fcp_cdr::TerminationReason::Cancelled);
                } else {
                    error!(
                        call_id = %call_id,
                        error = %e,
                        "Failed to receive final response from B-leg"
                    );
                    let response = self.create_simple_response(&request, 408, "Request Timeout");
                    a_leg_handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call_with(&call_id, fcp_cdr::TerminationReason::NoAnswer);
                }
                return Ok(());
            }
        };

        let b_leg_status = b_leg_final.start_line().code();
        info!(
            call_id = %call_id,
            code = b_leg_status,
            "Received final response from B-leg"
        );

        // If call was already terminated (e.g., CANCEL), stop here
        let is_cancelled = self
            .call_manager
            .get_call(&call_id)
            .map(|c| c.state.is_terminal())
            .unwrap_or(false);
        if is_cancelled {
            info!(call_id = %call_id, "Call already terminated, skipping final relay");
            let _ = self
                .call_manager
                .terminate_call_with(&call_id, fcp_cdr::TerminationReason::Cancelled);
            return Ok(());
        }

        // Handle final response based on status code
        if b_leg_status == 200 {
            // Success! Relay 200 OK to A-leg
            // Extract SDP from B-leg response
            let mut b_leg_sdp = if !b_leg_final.body().is_empty() {
                Some(String::from_utf8_lossy(b_leg_final.body()).to_string())
            } else {
                None
            };
            // The callee's own answer, kept on its leg as the caller's offer
            // is on the A leg: a transfer of the callee re-offers it.
            let callee_sdp = b_leg_sdp.clone();

            // A carrier that answers a trunk requiring SRTP in the clear
            // has not answered it (`docs/TENANT_TRUNKS.md` §6).
            if !self.srtp_answer_honoured(&call_id, b_leg_sdp.as_deref()) {
                warn!(
                    call_id = %call_id,
                    "The trunk requires SRTP and the answer carried none; hanging up the leg"
                );
                self.hang_up_unencrypted_leg(&call_id, &b_leg_handle).await;
                let response = self.create_simple_response(&request, 488, "Not Acceptable Here");
                a_leg_handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    &call_id,
                    Some("Trunk requires SRTP; the answer carried none".to_string()),
                );
                return Ok(());
            }

            // The trunk that answered carries the call from now on
            // (`docs/TENANT_TRUNKS.md` §7).
            self.count_answering_trunk(&call_id).await;

            // Set B-leg remote endpoint on the media session (before SDP rewriting)
            if let Some(ref original_sdp) = b_leg_sdp {
                let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                    if call.media_anchored {
                        call.media_session_id
                    } else {
                        None
                    }
                });

                if let (Some(ref media_session_id), Some(media_manager)) =
                    (&media_session_id, &self.media_manager)
                {
                    // The key this node sends with is the one it offered
                    // (`docs/TENANT_TRUNKS.md` §6).
                    let offered = self.take_b_leg_srtp(&call_id);
                    if let Err(e) = media_manager
                        .update_session_b_leg_offered(
                            media_session_id,
                            original_sdp.clone(),
                            offered,
                        )
                        .await
                    {
                        warn!(
                            call_id = %call_id,
                            error = %e,
                            "Failed to set B-leg media endpoint"
                        );
                    }
                    self.arrange_trunk_dtmf(&call_id, original_sdp).await;
                }
            }

            // Rewrite B-leg SDP if media is anchored on server
            if let Some(ref original_sdp) = b_leg_sdp {
                // Check if this call has anchored media
                let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                    if call.media_anchored {
                        call.media_session_id
                    } else {
                        None
                    }
                });

                if let (Some(media_session_id), Some(media_manager)) =
                    (media_session_id, &self.media_manager)
                {
                    // Get media session to extract leg A's RTP port
                    match media_manager.get_session(&media_session_id).await {
                        Ok(session) => {
                            let (rtp_port, _rtcp_port) =
                                session.ports_for(fcp_media::backend::Leg::A);
                            // The caller's own key, as the media backend answered its offer: an
                            // encrypted caller is answered encrypted, whichever way the answer is
                            // built.
                            let a_leg_srtp = fcp_media::srtp_context_of(
                                &session.local_sdp().await.unwrap_or_default(),
                            );

                            // Construct answer for A-leg using A-leg's preferred codec
                            // This is critical for transcoding: A-leg sees its preferred codec,
                            // B-leg uses whatever it answered with, and the transcoder handles
                            // conversion in the middle transparently
                            let rewritten_sdp = if let Some(original_invite) = self
                                .call_manager
                                .get_call(&call_id)
                                .and_then(|c| c.original_invite.clone())
                            {
                                if let Some(a_leg_offer) =
                                    crate::body::request_sdp(&original_invite)
                                {
                                    let a_leg_codecs =
                                        fcp_media::sdp::parse_sdp_codecs(&a_leg_offer);
                                    if let Some(preferred) = a_leg_codecs.first() {
                                        // Construct answer using A-leg's preferred codec
                                        let constructed_sdp = fcp_media::construct_answer_for_a_leg(
                                            &a_leg_offer,
                                            &self.local_addr,
                                            rtp_port,
                                            a_leg_srtp.as_ref(),
                                        );
                                        info!(
                                            call_id = %call_id,
                                            a_leg_codec = %preferred.encoding_name,
                                            b_leg_codec = ?original_sdp.lines().find(|l| l.starts_with("m=audio")),
                                            "Constructed A-leg answer with preferred codec (transcoding enabled)"
                                        );
                                        constructed_sdp
                                    } else {
                                        // Fallback: just rewrite the SDP
                                        fcp_media::rewrite_sdp_for_anchoring(
                                            original_sdp,
                                            &self.local_addr,
                                            rtp_port,
                                            a_leg_srtp.as_ref(),
                                        )
                                    }
                                } else {
                                    // Fallback: just rewrite the SDP
                                    fcp_media::rewrite_sdp_for_anchoring(
                                        original_sdp,
                                        &self.local_addr,
                                        rtp_port,
                                        a_leg_srtp.as_ref(),
                                    )
                                }
                            } else {
                                // Fallback: just rewrite the SDP
                                fcp_media::rewrite_sdp_for_anchoring(
                                    original_sdp,
                                    &self.local_addr,
                                    rtp_port,
                                    a_leg_srtp.as_ref(),
                                )
                            };

                            info!(
                                call_id = %call_id,
                                server_ip = %self.local_addr,
                                rtp_port = rtp_port,
                                "Rewrote B-leg SDP for media anchoring"
                            );

                            b_leg_sdp = Some(rewritten_sdp);
                        }
                        Err(e) => {
                            error!(
                                call_id = %call_id,
                                media_session_id = %media_session_id,
                                error = %e,
                                "Failed to get media session for B-leg SDP rewriting, using original SDP"
                            );
                        }
                    }
                }
            }

            // Extract confirmed B-leg dialog from CallHandle
            // The CallHandle.dialog is now automatically updated when the 200 OK arrives
            // (fixed in siphon-rs commit ea120fd)
            let b_leg_dialog = b_leg_handle.dialog.read().await.clone();

            debug!(
                call_id = %call_id,
                confirmed_dialog_id = ?b_leg_dialog.id(),
                remote_tag = ?b_leg_dialog.id().remote_tag(),
                dialog_state = ?b_leg_dialog.state(),
                "Extracted confirmed B-leg dialog from CallHandle (automatically updated on 200 OK)"
            );

            // Register confirmed B-leg dialog in dialog_manager for in-dialog request routing
            let _ = self.dialog_manager.insert(b_leg_dialog.clone());
            self.start_session_timer(&b_leg_dialog);
            debug!(
                call_id = %call_id,
                "Registered confirmed B-leg dialog in dialog_manager"
            );

            // Create 200 OK response manually to ensure To-tag consistency
            // CRITICAL: Must reuse the To-tag from the provisional response (RFC 3261 §12.1.1)
            let mut a_leg_response = if let Some(ref saved_tag) = a_leg_to_tag {
                // Build 200 OK with the SAME To-tag as the provisional
                let mut resp = self.uas.create_ok(&request, b_leg_sdp.as_deref())?;

                // Replace To header with one containing our saved tag
                if let Some(to) = request.headers().get("To") {
                    let to_with_tag = format!("{};tag={}", to, saved_tag);
                    // Remove old To header and add new one with correct tag
                    resp.headers_mut().retain(|h| h.name() != "To");
                    let _ = resp
                        .headers_mut()
                        .push(SmolStr::new("To"), SmolStr::new(to_with_tag));
                    debug!(call_id = %call_id, a_leg_to_tag = %saved_tag, "Reused To-tag in 200 OK for dialog consistency");
                }

                resp
            } else {
                // Fallback: No provisional was sent (shouldn't happen in B2BUA)
                warn!(call_id = %call_id, "No A-leg To-tag saved, generating new one (dialog mismatch possible)");
                self.uas.create_ok(&request, b_leg_sdp.as_deref())?
            };

            // Inject RFC 4028 session timer headers into A-leg 200 OK
            inject_session_timer_headers(&self.b2bua_config, &request, &mut a_leg_response);
            self.name_call_for_caller(&call_id, &mut a_leg_response);

            // Create A-leg dialog from the 200 OK response
            let a_leg_dialog = match sip_dialog::Dialog::new_uas(
                &request,
                &a_leg_response,
                self.uas.local_uri.clone(),
                extract_from_uri(&request)
                    .unwrap_or_else(|_| sip_core::SipUri::parse("sip:unknown@unknown").unwrap()),
            ) {
                Some(dialog) => dialog,
                None => {
                    error!(call_id = %call_id, "Failed to create A-leg dialog from 200 OK");
                    let response =
                        self.create_simple_response(&request, 500, "Internal Server Error");
                    a_leg_handle.send_final(response).await;
                    let _ = self.call_manager.terminate_call(
                        &call_id,
                        Some("Failed to create A-leg dialog".to_string()),
                    );
                    return Ok(());
                }
            };

            info!(
                call_id = %call_id,
                "Call established - both legs connected"
            );

            // Register A-leg dialog in dialog_manager for in-dialog request routing
            let _ = self.dialog_manager.insert(a_leg_dialog.clone());
            self.start_session_timer(&a_leg_dialog);
            debug!(
                call_id = %call_id,
                a_leg_dialog_id = ?a_leg_dialog.id(),
                "Registered A-leg dialog in dialog_manager"
            );

            // Update call with both dialogs
            match self.call_manager.update_call(&call_id, |call| {
                call.a_leg.dialog = Some(a_leg_dialog.clone());
                // Store B-leg dialog information
                if let Some(b_leg) = &mut call.b_leg {
                    b_leg.dialog = Some(b_leg_dialog.clone());
                    b_leg.sdp = callee_sdp.clone();
                    debug!(
                        call_id = %call_id,
                        b_leg_dialog_call_id = ?b_leg_dialog.id().call_id(),
                        "Stored confirmed B-leg dialog in CallManager"
                    );
                }
                call.state = fcp_b2bua_core::CallState::Established;
                call.establish();
            }) {
                Ok(_) => debug!(call_id = %call_id, "Successfully updated call in CallManager"),
                Err(e) => {
                    error!(call_id = %call_id, error = %e, "Failed to update call in CallManager")
                }
            }

            // Send 200 OK to A-leg
            info!(call_id = %call_id, "Sending 200 OK to A-leg");
            a_leg_handle.send_final(a_leg_response).await;
            info!(call_id = %call_id, "200 OK sent to A-leg");

            // Note: ACK to B-leg is handled automatically by the UAC transaction layer
            debug!(call_id = %call_id, "ACK will be sent automatically to B-leg");
        } else {
            // B-leg rejected the call - forward rejection to A-leg
            info!(
                call_id = %call_id,
                code = b_leg_status,
                "B-leg rejected call, forwarding to A-leg"
            );

            // Map B-leg rejection to A-leg response
            let reason = b_leg_final.start_line().reason();
            let response = self.create_simple_response(&request, b_leg_status, reason);

            info!(call_id = %call_id, code = b_leg_status, "Sending rejection to A-leg");
            a_leg_handle.send_final(response).await;
            info!(call_id = %call_id, code = b_leg_status, "Rejection sent to A-leg");

            let _ = self.call_manager.terminate_call_with(
                &call_id,
                fcp_cdr::TerminationReason::rejected(b_leg_status, reason),
            );
        }

        Ok(())
    }

    /// Handle parallel forking: race multiple B-legs for first successful answer.
    ///
    /// Sends INVITE to all target contacts simultaneously and races for the first 2xx response.
    /// When a 2xx arrives, CANCELs all other pending B-legs and connects winner to A-leg.
    /// The flow_token (RFC 5626) is stored with the winning B-leg for later BYE via flow.
    async fn handle_parallel_forking(
        &self,
        request: sip_core::Request,
        a_leg_handle: Arc<ServerTransactionHandle>,
        b_leg_calls: Vec<(String, Arc<crate::uac::OutboundCall>, Option<String>)>,
        call_id: fcp_b2bua_core::CallId,
        cancel_notify: Arc<tokio::sync::Notify>,
        late: Option<LateLegs>,
    ) -> Result<()> {
        info!(
            call_id = %call_id,
            fork_count = b_leg_calls.len(),
            "Racing {} B-legs for first answer",
            b_leg_calls.len()
        );

        // Spawn dedicated tasks to listen for provisional responses and relay first 180/183
        // IMPORTANT: Send the full Response object (not just status code) to preserve SDP/headers
        let (provisional_tx, mut provisional_rx) =
            tokio::sync::mpsc::channel::<sip_core::Response>(10);
        let provisional_sent = Arc::new(tokio::sync::Mutex::new(false));

        let early_media_policy = self.b2bua_config.forking.early_media_policy;
        let dedup_provisionals = self.b2bua_config.forking.deduplicate_provisionals;

        for (i, (target, b_leg_handle, _flow_token)) in b_leg_calls.iter().enumerate() {
            spawn_provisional_listener(
                i,
                target.clone(),
                Arc::clone(b_leg_handle),
                provisional_tx.clone(),
                Arc::clone(&provisional_sent),
                early_media_policy,
                dedup_provisionals,
                call_id,
            );
        }

        // A leg that joins later (a woken app) relays its ringing too.
        let late_provisional_tx = late.as_ref().map(|_| provisional_tx.clone());
        // Drop original sender so channel closes when all provisional tasks complete
        drop(provisional_tx);

        // Spawn relay task to forward the actual B-leg provisional to A-leg
        // CRITICAL: This fixes two issues:
        // 1. Early media/183 with SDP is now forwarded (not dropped)
        // 2. To-tag is extracted and saved for reuse in 200 OK
        let call_id_clone = call_id;
        let a_leg_handle_clone = Arc::clone(&a_leg_handle);
        let request_clone = request.clone();
        let call_manager_clone = self.call_manager.clone();
        let handler_clone = self.clone();
        tokio::spawn(async move {
            if let Some(b_leg_provisional) = provisional_rx.recv().await {
                let already_rebound = call_manager_clone
                    .get_call(&call_id_clone)
                    .map(|call| call.state == fcp_b2bua_core::CallState::Established)
                    .unwrap_or(false);
                if already_rebound {
                    debug!(
                        call_id = %call_id_clone,
                        "Skipping provisional relay because call was already rebound"
                    );
                    return;
                }

                let code = b_leg_provisional.start_line().code();
                let has_body = !b_leg_provisional.body().is_empty();

                info!(
                    call_id = %call_id_clone,
                    code = code,
                    has_sdp = has_body,
                    "Forwarding actual B-leg provisional to A-leg (preserves early media/183)"
                );

                // Build provisional using B-leg response as template
                // This preserves: status code, SDP body, Require/RSeq headers, etc.
                let provisional =
                    handler_clone.create_provisional_from_b_leg(&request_clone, &b_leg_provisional);

                // Extract and save To-tag for reuse in 200 OK (RFC 3261 §12.1.1)
                if let Some(to_header) = provisional.headers().get("To") {
                    let to_smol = SmolStr::new(to_header);
                    if let Some(tag) = extract_tag_value(Some(&to_smol)) {
                        debug!(
                            call_id = %call_id_clone,
                            a_leg_to_tag = %tag,
                            "Saved A-leg To-tag from forked provisional for later 200 OK"
                        );

                        // Store tag in Call struct for 200 OK reuse
                        let _ = call_manager_clone.update_call(&call_id_clone, |call| {
                            call.a_leg_to_tag = Some(tag.to_string());
                        });
                    }
                }

                // Log outbound provisional relay to A-leg
                let code_str = code.to_string();
                let raw_prov_bytes = sip_parse::serialize_response(&provisional);
                let raw_prov = String::from_utf8_lossy(&raw_prov_bytes);
                let cid_str = call_id_clone.to_string();
                handler_clone.log_sip_message(
                    "outbound",
                    &code_str,
                    Some(&cid_str),
                    None,
                    sip_parse::header(provisional.headers(), "CSeq").map(|s| s.as_str()),
                    None,
                    None,
                    Some(provisional.headers()),
                    Some(provisional.body()),
                    Some(&raw_prov),
                );

                a_leg_handle_clone.send_provisional(provisional).await;
            }
        });

        // Create tasks to wait for FINAL responses from all B-legs (for racing)
        // Each task returns (index, target, code, response, handle, flow_token) for the winning leg
        let mut tasks: futures::stream::FuturesUnordered<_> = b_leg_calls
            .iter()
            .enumerate()
            .map(|(i, (target, b_leg_handle, flow_token))| {
                spawn_race_task(
                    i,
                    target.clone(),
                    Arc::clone(b_leg_handle),
                    flow_token.clone(),
                    call_id,
                )
            })
            .collect();
        // Legs that joined while the race ran: kept here so the CANCELs
        // below reach them whichever way it ends.
        let added_legs: std::sync::Mutex<Vec<BLeg>> = std::sync::Mutex::new(Vec::new());
        let first_index = b_leg_calls.len();
        let mut late = late;

        // Race all tasks and take the first successful 2xx response.
        // Also listen for A-leg cancellation so we can abort immediately.
        let mut winning_index = None;
        let mut winning_target = None;
        let mut winning_response = None;
        let mut winning_handle = None;
        let mut winning_flow_token: Option<String> = None;
        let mut cancelled_by_caller = false;
        let mut last_failure_code: Option<u16> = None;
        // Whether the race ended because nobody answered in time, for the
        // call's record (a timeout is no answer, not a failure).
        let mut fork_timed_out = false;

        debug!(call_id = %call_id, task_count = tasks.len(), "Starting race for first 2xx response");

        // Wrap the B-leg race into a single future so we can select against cancel
        let race_future = async {
            use futures::StreamExt;
            let mut race_failure_code: Option<u16> = None;
            loop {
                if tasks.is_empty() && late.is_none() {
                    break;
                }
                // Which happened: a leg finished, or a woken app came back
                // (`None`: every app it woke is accounted for).
                enum Step<R> {
                    Finished(R),
                    Late(Option<crate::push::LateContact>),
                }
                let has_late = late.is_some();
                let step = {
                    let next_late = async {
                        match late.as_mut() {
                            Some(late) => late.contacts.recv().await,
                            None => std::future::pending().await,
                        }
                    };
                    tokio::select! {
                        Some(result) = tasks.next(), if !tasks.is_empty() => Step::Finished(result),
                        contact = next_late, if has_late => Step::Late(contact),
                    }
                };
                let result = match step {
                    Step::Finished(result) => result,
                    Step::Late(None) => {
                        late = None;
                        continue;
                    }
                    // A woken app is back: it rings like the others.
                    Step::Late(Some(contact)) => {
                        let Some(ready) = late.as_ref() else { continue };
                        // Back at another node: rung through it.
                        let relay = match (
                            &contact.flow_node,
                            &contact.flow_token,
                            &self.node_sip_address,
                        ) {
                            (Some(node), Some(token), Some(address))
                                if *node != self.node_id && !self.flow_registry.is_valid(token) =>
                            {
                                address(node).map(|at| (at.to_string(), token.clone()))
                            }
                            _ => None,
                        };
                        let target = TargetContact {
                            uri: contact.uri,
                            flow_token: match relay {
                                Some(_) => None,
                                None => contact.flow_token,
                            },
                            egress: crate::trunk_egress::TrunkEgress::none(),
                            relay,
                        };
                        if let Some(leg) = self
                            .send_b_leg_invite(
                                Arc::clone(&ready.uac),
                                target,
                                ready.sdp.clone(),
                                ready.headers.clone(),
                                call_id,
                            )
                            .await
                        {
                            let mut added = added_legs.lock().unwrap_or_else(|e| e.into_inner());
                            let i = first_index + added.len();
                            info!(call_id = %call_id, target = %leg.0, "A woken app joins the race");
                            if let Some(tx) = &late_provisional_tx {
                                spawn_provisional_listener(
                                    i,
                                    leg.0.clone(),
                                    Arc::clone(&leg.1),
                                    tx.clone(),
                                    Arc::clone(&provisional_sent),
                                    early_media_policy,
                                    dedup_provisionals,
                                    call_id,
                                );
                            }
                            tasks.push(spawn_race_task(
                                i,
                                leg.0.clone(),
                                Arc::clone(&leg.1),
                                leg.2.clone(),
                                call_id,
                            ));
                            added.push(leg);
                        }
                        continue;
                    }
                };

                // The node holding a phone's flow says it is gone (430): an
                // app is woken instead (`docs/PUSH.md` §2.4).
                // Anything else from it: the phone was reached there, and
                // is not woken.
                if let Ok(Some((_, target, code, ..))) = &result {
                    if let Some(token) = crate::push::relay_token(target) {
                        if *code == 430 && self.push_waits.undefer(&call_id, token).is_some() {
                            info!(call_id = %call_id, "A phone's flow on another node is gone: waking it");
                            let handler = self.clone();
                            tokio::spawn(async move {
                                handler.push_waiting(call_id).await;
                            });
                        } else {
                            self.push_waits.forgo(&call_id, token);
                        }
                    }
                }
                match result {
                    Ok(Some((i, target, code, response, handle, flow_token))) => {
                        debug!(
                            call_id = %call_id,
                            b_leg_index = i,
                            target = %target,
                            code = code,
                            "B-leg {} completed with code {}", i, code
                        );

                        self.note_geolocation_error(call_id, &response);
                        if (200..300).contains(&code) {
                            info!(
                                call_id = %call_id,
                                winner = %target,
                                code = code,
                                has_flow_token = flow_token.is_some(),
                                "B-leg {} won the race with {}",
                                i, code
                            );
                            return (
                                Some((i, target, response, handle, flow_token)),
                                race_failure_code,
                            );
                        } else {
                            debug!(
                                call_id = %call_id,
                                code = code,
                                "B-leg {} returned non-2xx {}, continuing race", i, code
                            );
                            race_failure_code = Some(code);
                            // A 6xx is the callee's answer for every
                            // phone (RFC 3261 §16.7): the rest are
                            // CANCELled and the call goes to its busy
                            // treatment now. A decline on one phone kept
                            // the caller ringing out to the others' timeout
                            // (`docs/DEEP_TESTING.md` D8).
                            if code >= 600 {
                                info!(
                                    call_id = %call_id,
                                    code,
                                    "B-leg {} declined for every phone; ending the fork", i
                                );
                                return (None, race_failure_code);
                            }
                        }
                    }
                    Ok(None) => {
                        debug!(
                            call_id = %call_id,
                            "B-leg task returned None (likely failed or terminated)"
                        );
                    }
                    Err(e) => {
                        warn!(
                            call_id = %call_id,
                            error = ?e,
                            "B-leg task panicked"
                        );
                    }
                }
            }
            (None, race_failure_code) // No winner
        };

        tokio::pin!(race_future);

        let per_call_timeout = self
            .call_manager
            .get_call(&call_id)
            .and_then(|c| c.no_answer_timeout);
        let fork_timeout = per_call_timeout.unwrap_or(self.b2bua_config.forking.timeout);
        info!(
            call_id = %call_id,
            timeout_secs = fork_timeout.as_secs(),
            per_user = per_call_timeout.is_some(),
            b_leg_count = b_leg_calls.len(),
            "Starting B-leg race select! - listening for winner, cancel, or timeout"
        );

        tokio::select! {
            result = &mut race_future => {
                let (winner, failure_code) = result;
                last_failure_code = failure_code;
                if let Some((i, target, response, handle, flow_token)) = winner {
                    winning_index = Some(i);
                    winning_target = Some(target);
                    winning_response = Some(response);
                    winning_handle = Some(handle);
                    winning_flow_token = flow_token;
                }
            }
            _ = cancel_notify.notified() => {
                info!(
                    call_id = %call_id,
                    "cancel_notify.notified() triggered - caller cancelled, aborting B-leg race"
                );
                cancelled_by_caller = true;
            }
            _ = tokio::time::sleep(fork_timeout) => {
                warn!(
                    call_id = %call_id,
                    timeout_secs = fork_timeout.as_secs(),
                    "Forking timeout expired — no B-leg answered in time"
                );
                fork_timed_out = true;
            }
        }

        // Late legs join the rest for what follows (their CANCELs), and the
        // apps woken that never came back are told the call is over.
        let mut b_leg_calls = b_leg_calls;
        b_leg_calls.extend(
            added_legs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain(..),
        );
        self.end_push_wait(call_id);

        debug!(
            call_id = %call_id,
            has_winner = winning_index.is_some(),
            cancelled = cancelled_by_caller,
            "Race complete"
        );

        let already_rebound = self
            .call_manager
            .get_call(&call_id)
            .map(|call| {
                call.state == fcp_b2bua_core::CallState::Established
                    && call.a_leg.dialog.is_some()
                    && call
                        .b_leg
                        .as_ref()
                        .and_then(|leg| leg.dialog.as_ref())
                        .is_some()
            })
            .unwrap_or(false);
        if already_rebound {
            info!(
                call_id = %call_id,
                fork_count = b_leg_calls.len(),
                "Call was rebound while fork race was in flight; cancelling all forks"
            );
            // Cancel every outstanding fork leg so target phones stop ringing.
            // If a leg already raced pickup to 200 OK, the helper falls back to
            // sending BYE on the confirmed dialog to tear it down cleanly.
            for (i, (target, b_leg_handle, _flow_token)) in b_leg_calls.iter().enumerate() {
                info!(
                    call_id = %call_id,
                    target = %target,
                    "CANCELing fork {} after pickup rebind", i
                );
                self.cancel_or_bye_fork_leg(
                    &call_id,
                    target,
                    i,
                    b_leg_handle,
                    "Pickup rebind cleanup",
                )
                .await;
            }
            self.emergency_fallbacks.remove(&call_id);
            self.trunk_fallbacks.remove(&call_id);
            return Ok(());
        }

        if winning_index.is_some() || cancelled_by_caller {
            self.emergency_fallbacks.remove(&call_id);
            self.trunk_fallbacks.remove(&call_id);
        }

        // CANCEL all non-winning B-legs — each in its own task, so a leg
        // that answers nothing (a contact that is gone) holds up neither
        // the others nor the winner's answer to the caller.
        for (i, (target, b_leg_handle, _flow_token)) in b_leg_calls.iter().enumerate() {
            if Some(i) != winning_index {
                info!(
                    call_id = %call_id,
                    target = %target,
                    "CANCELing non-winning B-leg {}", i
                );
                tokio::spawn(Self::cancel_or_bye_fork_leg_with(
                    self.uac.clone(),
                    Arc::clone(&self.dialog_manager),
                    call_id,
                    target.clone(),
                    i,
                    Arc::clone(b_leg_handle),
                    "Fork race cleanup".to_string(),
                ));
            }
        }

        // A find-me leg that must be confirmed is connected only when its
        // answerer presses 1 (`docs/ENTERPRISE_CALLING.md` §3.6); otherwise
        // it is dropped and the call goes on as though nobody answered.
        let (winning_target, winning_response, winning_handle) = match (
            winning_target,
            winning_response,
            winning_handle,
        ) {
            (Some(target), Some(response), Some(handle))
                if self
                    .call_manager
                    .get_call(&call_id)
                    .is_some_and(|c| c.confirm_targets.contains(&target)) =>
            {
                if self.confirm_answer(&call_id, &response).await {
                    (Some(target), Some(response), Some(handle))
                } else {
                    info!(call_id = %call_id, target = %target, "Find-me answer not confirmed; dropping that leg");
                    let dialog = handle.dialog.read().await.clone();
                    if let Some(uac) = self.uac.clone() {
                        tokio::spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(10), uac.bye(&dialog))
                                .await;
                        });
                    }
                    last_failure_code = Some(603);
                    (None, None, None)
                }
            }
            other => other,
        };

        // If we have a winner, relay the response to A-leg
        if let (Some(target), Some(response), Some(handle)) =
            (winning_target, winning_response, winning_handle)
        {
            let code = response.start_line().code();
            info!(
                call_id = %call_id,
                winner = %target,
                code = code,
                "Relaying winning B-leg response to A-leg"
            );

            // Store the winning B-leg handle and flow token (RFC 5626)
            let winning_flow_token_clone = winning_flow_token.clone();
            let winner_uri = target.to_string();
            // Stored only if the caller has not cancelled: the CANCEL
            // handler sets the flag and takes the handle in one update, so
            // a cancel that crossed this answer is seen here or there.
            let mut caller_cancelled = true;
            let _ = self.call_manager.update_call(&call_id, |call| {
                if call.caller_cancelled {
                    return;
                }
                caller_cancelled = false;
                if let Some(b_leg) = &mut call.b_leg {
                    b_leg.outbound_handle = Some(Arc::clone(&handle));
                    b_leg.flow_token = winning_flow_token_clone;
                }
                call.state = fcp_b2bua_core::CallState::Established;
                // A failover route that answered names its own trunk.
                if let Some(answered) = answering_trunk(&call.route_trunks, &winner_uri) {
                    call.trunk_name = Some(answered.name());
                    call.trunk_id = Some(answered.id());
                }
            });
            if caller_cancelled {
                info!(
                    call_id = %call_id,
                    winner = %target,
                    "The callee answered as the caller cancelled; hanging up the answered leg"
                );
                tokio::spawn(Self::cancel_or_bye_fork_leg_with(
                    self.uac.clone(),
                    Arc::clone(&self.dialog_manager),
                    call_id,
                    target.to_string(),
                    winning_index.unwrap_or(0),
                    handle,
                    "Answer crossed the caller's CANCEL".to_string(),
                ));
                return Ok(());
            }

            // Extract SDP from winning B-leg response
            let mut b_leg_sdp = if !response.body().is_empty() {
                Some(String::from_utf8_lossy(response.body()).to_string())
            } else {
                None
            };
            // The callee's own answer, kept on its leg as the caller's offer
            // is on the A leg: a transfer of the callee re-offers it.
            let callee_sdp = b_leg_sdp.clone();

            // A carrier that answers a trunk requiring SRTP in the clear
            // has not answered it (`docs/TENANT_TRUNKS.md` §6).
            if !self.srtp_answer_honoured(&call_id, b_leg_sdp.as_deref()) {
                warn!(
                    call_id = %call_id,
                    target = %target,
                    "The trunk requires SRTP and the answer carried none; hanging up the leg"
                );
                self.hang_up_unencrypted_leg(&call_id, &handle).await;
                let response = self.create_simple_response(&request, 488, "Not Acceptable Here");
                a_leg_handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    &call_id,
                    Some("Trunk requires SRTP; the answer carried none".to_string()),
                );
                return Ok(());
            }

            // The trunk that answered carries the call from now on
            // (`docs/TENANT_TRUNKS.md` §7).
            self.count_answering_trunk(&call_id).await;

            // Set B-leg remote endpoint on the media session (before SDP rewriting)
            if let Some(ref original_sdp) = b_leg_sdp {
                let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                    if call.media_anchored {
                        call.media_session_id
                    } else {
                        None
                    }
                });

                if let (Some(ref media_session_id), Some(media_manager)) =
                    (&media_session_id, &self.media_manager)
                {
                    // The key this node sends with is the one it offered
                    // (`docs/TENANT_TRUNKS.md` §6).
                    let offered = self.take_b_leg_srtp(&call_id);
                    if let Err(e) = media_manager
                        .update_session_b_leg_offered(
                            media_session_id,
                            original_sdp.clone(),
                            offered,
                        )
                        .await
                    {
                        warn!(
                            call_id = %call_id,
                            error = %e,
                            "Failed to set B-leg media endpoint in parallel fork"
                        );
                    }
                    // A callee behind NAT answers from where it was rung.
                    if let Some(rung) = crate::nat::uri_address(&target.to_string())
                        .filter(|rung| crate::nat::learns_media_address(original_sdp, *rung))
                    {
                        self.latch_leg(&call_id, fcp_media::backend::Leg::B, rung)
                            .await;
                    }
                    self.arrange_trunk_dtmf(&call_id, original_sdp).await;
                }
            }

            // Rewrite B-leg SDP if media is anchored on server (for A-leg response)
            if let Some(ref original_sdp) = b_leg_sdp {
                // Check if this call has anchored media
                let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                    if call.media_anchored {
                        call.media_session_id
                    } else {
                        None
                    }
                });

                if let (Some(media_session_id), Some(media_manager)) =
                    (media_session_id, &self.media_manager)
                {
                    // Get media session to extract RTP port (A-leg port)
                    match media_manager.get_session(&media_session_id).await {
                        Ok(session) => {
                            let (rtp_port, _rtcp_port) =
                                session.ports_for(fcp_media::backend::Leg::A);
                            // The caller's own key, as the media backend answered its offer: an
                            // encrypted caller is answered encrypted, whichever way the answer is
                            // built.
                            let a_leg_srtp = fcp_media::srtp_context_of(
                                &session.local_sdp().await.unwrap_or_default(),
                            );

                            // Construct answer for A-leg using A-leg's preferred codec
                            // This is critical for transcoding: A-leg sees its preferred codec,
                            // B-leg uses whatever it answered with, and the transcoder handles
                            // conversion in the middle transparently
                            let rewritten_sdp = if let Some(original_invite) = self
                                .call_manager
                                .get_call(&call_id)
                                .and_then(|c| c.original_invite.clone())
                            {
                                if let Some(a_leg_offer) =
                                    crate::body::request_sdp(&original_invite)
                                {
                                    let a_leg_codecs =
                                        fcp_media::sdp::parse_sdp_codecs(&a_leg_offer);
                                    if let Some(preferred) = a_leg_codecs.first() {
                                        // Construct answer using A-leg's preferred codec
                                        let constructed_sdp = fcp_media::construct_answer_for_a_leg(
                                            &a_leg_offer,
                                            &self.local_addr,
                                            rtp_port,
                                            a_leg_srtp.as_ref(),
                                        );
                                        info!(
                                            call_id = %call_id,
                                            a_leg_codec = %preferred.encoding_name,
                                            b_leg_codec = ?original_sdp.lines().find(|l| l.starts_with("m=audio")),
                                            "Constructed A-leg answer with preferred codec (transcoding enabled)"
                                        );
                                        constructed_sdp
                                    } else {
                                        // Fallback: just rewrite the SDP
                                        fcp_media::rewrite_sdp_for_anchoring(
                                            original_sdp,
                                            &self.local_addr,
                                            rtp_port,
                                            a_leg_srtp.as_ref(),
                                        )
                                    }
                                } else {
                                    // Fallback: just rewrite the SDP
                                    fcp_media::rewrite_sdp_for_anchoring(
                                        original_sdp,
                                        &self.local_addr,
                                        rtp_port,
                                        a_leg_srtp.as_ref(),
                                    )
                                }
                            } else {
                                // Fallback: just rewrite the SDP
                                fcp_media::rewrite_sdp_for_anchoring(
                                    original_sdp,
                                    &self.local_addr,
                                    rtp_port,
                                    a_leg_srtp.as_ref(),
                                )
                            };

                            info!(
                                call_id = %call_id,
                                server_ip = %self.local_addr,
                                rtp_port = rtp_port,
                                "Rewrote winning B-leg SDP for media anchoring on A-leg"
                            );

                            b_leg_sdp = Some(rewritten_sdp);
                        }
                        Err(e) => {
                            error!(
                                call_id = %call_id,
                                media_session_id = %media_session_id,
                                error = %e,
                                "Failed to get media session for B-leg SDP rewriting, using original SDP"
                            );
                        }
                    }
                }
            }

            if Self::is_oneway_intercom_request(&request) {
                b_leg_sdp = b_leg_sdp
                    .as_deref()
                    .map(|sdp| Self::rewrite_sdp_direction(sdp, "recvonly"));
            }

            // Retrieve saved A-leg To-tag from provisional response
            let saved_tag = self
                .call_manager
                .get_call(&call_id)
                .and_then(|call| call.a_leg_to_tag.clone());

            // Create 200 OK response manually to ensure To-tag consistency
            // CRITICAL: Must reuse the To-tag from the provisional response (RFC 3261 §12.1.1)
            let mut ok_response = if let Some(ref tag) = saved_tag {
                // Build 200 OK with the SAME To-tag as the provisional
                let mut resp = self
                    .uas
                    .create_ok(&request, b_leg_sdp.as_deref())
                    .expect("Failed to create 200 OK");

                // Replace To header with one containing our saved tag
                if let Some(to) = request.headers().get("To") {
                    let to_with_tag = format!("{};tag={}", to, tag);
                    // Remove old To header and add new one with correct tag
                    resp.headers_mut().retain(|h| h.name() != "To");
                    let _ = resp
                        .headers_mut()
                        .push(SmolStr::new("To"), SmolStr::new(to_with_tag));
                    debug!(
                        call_id = %call_id,
                        a_leg_to_tag = %tag,
                        "Reused To-tag in forked 200 OK for dialog consistency"
                    );
                }

                resp
            } else {
                // The callee answered before any ringing reached the caller:
                // there is no early dialog, so the 200 OK starts it with a
                // tag of its own, and the A-leg dialog below is made from
                // this 200 OK. The provisional relay, finding the call
                // established, sends nothing after it.
                debug!(
                    call_id = %call_id,
                    "Answered before any ringing reached the caller; the 200 OK sets the To-tag"
                );
                self.uas
                    .create_ok(&request, b_leg_sdp.as_deref())
                    .expect("Failed to create 200 OK")
            };

            self.ensure_response_hygiene(&request, &mut ok_response);

            // Inject RFC 4028 session timer headers into forked 200 OK
            inject_session_timer_headers(&self.b2bua_config, &request, &mut ok_response);
            self.name_call_for_caller(&call_id, &mut ok_response);

            // Create A-leg dialog from the 200 OK response
            let a_leg_dialog = match sip_dialog::Dialog::new_uas(
                &request,
                &ok_response,
                self.uas.local_uri.clone(),
                extract_from_uri(&request)
                    .unwrap_or_else(|_| sip_core::SipUri::parse("sip:unknown@unknown").unwrap()),
            ) {
                Some(dialog) => dialog,
                None => {
                    error!(call_id = %call_id, "Failed to create A-leg dialog from 200 OK in parallel forking");
                    let response =
                        self.create_simple_response(&request, 500, "Internal Server Error");
                    a_leg_handle.send_final(response).await;
                    let _ = self.call_manager.terminate_call(
                        &call_id,
                        Some("Failed to create A-leg dialog".to_string()),
                    );
                    return Ok(());
                }
            };

            // Get B-leg dialog from the winning call handle
            let b_leg_dialog = {
                let dialog_guard = handle.dialog.read().await;
                Some(dialog_guard.clone())
            };

            debug!(
                call_id = %call_id,
                a_leg_dialog_id = ?a_leg_dialog.id(),
                b_leg_dialog_id = ?b_leg_dialog.as_ref().map(|d| d.id()),
                "Created dialogs for parallel forking winner"
            );

            // Register A-leg dialog in dialog_manager
            let _ = self.dialog_manager.insert(a_leg_dialog.clone());
            self.start_session_timer(&a_leg_dialog);

            // Update call with both dialogs
            match self.call_manager.update_call(&call_id, |call| {
                call.a_leg.dialog = Some(a_leg_dialog.clone());
                if let Some(b_leg) = &mut call.b_leg {
                    b_leg.dialog = b_leg_dialog.clone();
                    b_leg.sdp = callee_sdp.clone();
                }
                call.state = fcp_b2bua_core::CallState::Established;
                call.establish();
            }) {
                Ok(_) => debug!(call_id = %call_id, "Successfully stored dialogs in CallManager"),
                Err(e) => {
                    error!(call_id = %call_id, error = %e, "Failed to update call with dialogs")
                }
            }

            info!(call_id = %call_id, "Sending 200 OK to A-leg with media anchoring");
            let raw_ok_bytes = sip_parse::serialize_response(&ok_response);
            let raw_ok = String::from_utf8_lossy(&raw_ok_bytes);
            let cid_str = call_id.to_string();
            self.log_sip_message(
                "outbound",
                "200",
                Some(&cid_str),
                None,
                sip_parse::header(ok_response.headers(), "CSeq").map(|s| s.as_str()),
                None,
                None,
                Some(ok_response.headers()),
                Some(ok_response.body()),
                Some(&raw_ok),
            );
            a_leg_handle.send_final(ok_response).await;

            // A call recorded without anyone asking starts now, with both
            // parties on the line to hear the announcement.
            self.start_automatic_recording(&call_id);
        } else if cancelled_by_caller {
            // Caller already cancelled — handle_cancel sent 487 and terminated the call.
            // Just clean up; don't send another response to A-leg.
            info!(call_id = %call_id, "Forking task exiting after caller cancellation");
        } else {
            // An emergency call with routes left tries the next one
            // before the caller hears anything.
            if let Some((target, outbound)) = self.next_emergency_leg(call_id).await {
                return Box::pin(self.handle_parallel_forking(
                    request,
                    a_leg_handle,
                    vec![(target, outbound, None)],
                    call_id,
                    cancel_notify,
                    None,
                ))
                .await;
            }

            // So does a call to carriers whose carrier failed, as opposed
            // to the callee answering: a refusal of ours, a server error,
            // no response at all, or silence until the timeout. A carrier
            // that rang has delivered the call.
            let rang = *provisional_sent.lock().await;
            let failed = match last_failure_code {
                Some(code) => carrier_failed(code),
                None => !(fork_timed_out && rang),
            };
            if failed {
                if let Some(next) = self.next_trunk_leg(call_id).await {
                    return Box::pin(self.handle_parallel_forking(
                        request,
                        a_leg_handle,
                        vec![next],
                        call_id,
                        cancel_notify,
                        None,
                    ))
                    .await;
                }
            }
            self.trunk_fallbacks.remove(&call_id);

            // Find-me: the next step rings before any treatment (§3.6).
            if let Some(legs) = self.next_find_me_step(&request, call_id).await {
                let cancel_notify = Arc::new(tokio::sync::Notify::new());
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.cancel_notify = Some(Arc::clone(&cancel_notify));
                });
                return Box::pin(self.handle_parallel_forking(
                    request,
                    a_leg_handle,
                    legs,
                    call_id,
                    cancel_notify,
                    None,
                ))
                .await;
            }

            // All B-legs failed (not due to cancellation): the plan's
            // treatment for busy or for no answer, when the call was a
            // user's (`docs/ENTERPRISE_CALLING.md` §3.1).
            let call = self.call_manager.get_call(&call_id);
            let busy = crate::ring_plan::is_busy_code(last_failure_code);
            let treatment = call.as_ref().and_then(|c| c.treatments.as_ref()).map(|t| {
                if busy {
                    t.busy.clone()
                } else {
                    t.no_answer.clone()
                }
            });
            let callee = call.as_ref().and_then(|c| c.original_callee.clone());
            let reason = if busy { "busy" } else { "no-answer" };
            match treatment {
                Some(fcp_core::Treatment::Refuse { .. }) | None => {
                    // The callee's own refusal, when there was one (a single
                    // phone saying busy or declining is what the caller
                    // should hear), else 480.
                    let (code, reason) = match last_failure_code {
                        Some(code @ (400..=699)) if code != 408 => (code, reason_phrase(code)),
                        _ => (480, "Temporarily Unavailable"),
                    };
                    warn!(call_id = %call_id, code, "All B-legs failed, no treatment");
                    let response = self.create_simple_response(&request, code, reason);
                    a_leg_handle.send_final(response).await;
                    // The record says what the caller was told and why: the
                    // callee's refusal, nobody answering, or no leg that
                    // could be reached at all.
                    let termination = match last_failure_code {
                        Some(refused @ (400..=699)) if refused != 408 => {
                            fcp_cdr::TerminationReason::rejected(refused, reason)
                        }
                        Some(408) => fcp_cdr::TerminationReason::NoAnswer,
                        _ if fork_timed_out => fcp_cdr::TerminationReason::NoAnswer,
                        _ => fcp_cdr::TerminationReason::Failed {
                            status: Some(code),
                            detail: "All B-legs failed".to_string(),
                        },
                    };
                    let _ = self.call_manager.terminate_call_with(&call_id, termination);
                }
                Some(treatment) => {
                    info!(call_id = %call_id, ?treatment, reason, "All B-legs failed: applying the treatment");
                    self.apply_treatment(
                        &request,
                        &a_leg_handle,
                        call_id,
                        treatment,
                        callee.as_deref(),
                        reason,
                    )
                    .await;
                }
            }
        }

        Ok(())
    }

    /// Record a trunk's answer to the location an emergency call carried:
    /// its `Geolocation-Error`, or a 424, on the call (and so its record),
    /// and on the attempt, which decides what is tried next.
    fn note_geolocation_error(
        &self,
        call_id: fcp_b2bua_core::CallId,
        response: &sip_core::Response,
    ) {
        let Some(mut fallback) = self.emergency_fallbacks.get_mut(&call_id) else {
            return;
        };
        let Some(error) = crate::emergency::geolocation_error_of(response) else {
            return;
        };
        let refused = !(200..300).contains(&response.start_line().code());
        fallback.location_refused |= refused;
        let trunk = fallback.current.as_ref().and_then(|a| a.trunk.clone());
        drop(fallback);
        warn!(
            call_id = %call_id,
            trunk = ?trunk,
            code = response.start_line().code(),
            geolocation_error = %error,
            "Emergency trunk answered the location with an error"
        );
        let _ = self.call_manager.update_call(&call_id, |call| {
            if let Some(em) = call.emergency.as_mut() {
                em.geolocation_error = Some(error);
            }
        });
    }

    /// Whether every one of `targets` is one of the call's trunk routes.
    fn all_trunk_routes(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        targets: &[TargetContact],
    ) -> bool {
        let Some(call) = self.call_manager.get_call(call_id) else {
            return false;
        };
        !call.route_trunks.is_empty()
            && targets
                .iter()
                .all(|t| answering_trunk(&call.route_trunks, &t.uri).is_some())
    }

    /// The next trunk route for a call whose carrier failed, sent with the
    /// call's SDP, headers and caller identity; a route whose INVITE cannot
    /// be sent gives way to the one after it. `None` when nothing is left.
    async fn next_trunk_leg(
        &self,
        call_id: fcp_b2bua_core::CallId,
    ) -> Option<(String, Arc<crate::uac::OutboundCall>, Option<String>)> {
        let uac = Arc::clone(self.uac.as_ref()?);
        loop {
            let (target, sdp, headers, from, display_name) = {
                let mut fallback = self.trunk_fallbacks.get_mut(&call_id)?;
                match fallback.remaining.pop_front() {
                    Some(target) => (
                        target,
                        fallback.sdp.clone(),
                        fallback.headers.clone(),
                        fallback.from.clone(),
                        fallback.display_name.clone(),
                    ),
                    None => {
                        drop(fallback);
                        self.trunk_fallbacks.remove(&call_id);
                        return None;
                    }
                }
            };
            // The record names the trunk the call is on now; the answer,
            // if it comes, names it again.
            let trunk = self
                .call_manager
                .get_call(&call_id)
                .and_then(|c| answering_trunk(&c.route_trunks, &target.uri).cloned());
            info!(
                call_id = %call_id,
                target = %target.uri,
                trunk = ?trunk.as_ref().map(|t| t.trunk.name.clone()),
                "The carrier failed; trying the next trunk route"
            );
            if let Some(trunk) = trunk {
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.trunk_name = Some(trunk.trunk.name.clone());
                    call.trunk_id = Some(trunk.id());
                });
            }
            uac.inner().set_display_name(display_name).await;
            uac.inner().set_from_uri(from).await;
            let sent = self
                .send_b_leg_invite(Arc::clone(&uac), target, sdp, headers, call_id)
                .await;
            uac.inner().set_display_name(None).await;
            uac.inner().set_from_uri(None).await;
            if sent.is_some() {
                return sent;
            }
        }
    }

    /// The next emergency attempt for a call whose last one failed (or its
    /// first): the same route without the location by value when the trunk
    /// refused the location and also takes ELIN, else the next route. Sent
    /// with the call's SDP, the callback number as caller and the location
    /// the route's trunk takes. `None` when nothing is left to try.
    async fn next_emergency_leg(
        &self,
        call_id: fcp_b2bua_core::CallId,
    ) -> Option<(String, Arc<crate::uac::OutboundCall>)> {
        let uac = self.uac.as_ref()?;
        loop {
            let (attempt, sdp, from, display_name, headers) = {
                let mut fallback = self.emergency_fallbacks.get_mut(&call_id)?;
                let retry = fallback
                    .current
                    .take()
                    .filter(|_| fallback.location_refused)
                    .and_then(|a| a.without_pidf_lo());
                fallback.location_refused = false;
                match retry.or_else(|| fallback.attempts.pop_front()) {
                    Some(attempt) => {
                        fallback.current = Some(attempt.clone());
                        (
                            attempt,
                            fallback.sdp.clone(),
                            fallback.from.clone(),
                            fallback.display_name.clone(),
                            fallback.headers.clone(),
                        )
                    }
                    None => {
                        drop(fallback);
                        self.emergency_fallbacks.remove(&call_id);
                        return None;
                    }
                }
            };
            let call = self.call_manager.get_call(&call_id)?;
            let info = call.emergency.clone()?;
            // Up a site's link, the location goes on by value whatever the
            // next hop's trunk takes: the next hop is the cloud's call
            // manager, which places the call by it (§3.6 of
            // `docs/HYBRID_CONTROL_PLANE.md`).
            let attempt = if call.link_site.is_some() {
                attempt.with_pidf_lo()
            } else {
                attempt
            };
            let (options, delivered) = crate::emergency::attempt_invite(
                &info,
                &attempt,
                call_id,
                sdp.as_deref(),
                from,
                headers.as_ref(),
                &self.local_addr,
            );
            info!(
                call_id = %call_id,
                target = %attempt.target,
                trunk = ?attempt.trunk,
                delivery = ?delivered,
                "Placing emergency call attempt"
            );
            let trunk = attempt.trunk.clone();
            let trunk_id = attempt.trunk_id.clone();
            let _ = self.call_manager.update_call(&call_id, |call| {
                call.trunk_name = trunk;
                call.trunk_id = trunk_id;
                if let Some(em) = call.emergency.as_mut() {
                    em.delivered = delivered;
                }
            });
            uac.inner().set_display_name(display_name).await;
            let sent = tokio::time::timeout(
                Duration::from_secs(5),
                uac.invite_with_options(
                    crate::trunk_egress::take_destination(&attempt.target)
                        .0
                        .as_str(),
                    options,
                ),
            )
            .await;
            uac.inner().set_display_name(None).await;
            match sent {
                Ok(Ok(call)) => {
                    let b_leg_invite = call.invite_request().await;
                    if let Some(b_leg_cid) = sip_parse::header(b_leg_invite.headers(), "Call-ID") {
                        self.sip_call_id_map
                            .insert(b_leg_cid.as_str().to_string(), call_id.to_string());
                    }
                    return Some((attempt.target, Arc::new(call)));
                }
                Ok(Err(e)) => {
                    error!(call_id = %call_id, target = %attempt.target, error = %e, "Failed to send the emergency INVITE");
                }
                Err(_) => {
                    error!(call_id = %call_id, target = %attempt.target, "Emergency INVITE timed out (5s)");
                }
            }
        }
    }

    /// Handle INVITE in UAS mode (without B-leg forwarding).
    ///
    /// This is the fallback mode when UAC is not available or for testing.
    /// It accepts the INVITE directly and establishes only the A-leg dialog.
    async fn handle_invite_uas_mode(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: fcp_b2bua_core::CallId,
    ) -> Result<()> {
        debug!(
            call_id = %call_id,
            "Handling INVITE in UAS mode (no B-leg forwarding)"
        );

        // Send 180 Ringing to test call flow
        let mut ringing = self.uas.create_ringing(request);
        self.ensure_response_hygiene(request, &mut ringing);
        handle.send_provisional(ringing).await;

        // Generate SDP answer
        let sdp_answer = self.generate_sdp_answer(request)?;

        // Accept the INVITE and create A-leg dialog
        match self.uas.accept_invite(request, sdp_answer.as_deref()) {
            Ok((response, dialog)) => {
                debug!(
                        call_id = %call_id,
                        dialog_call_id = %dialog.id().call_id(),
                local_tag = %dialog.id().local_tag(),
                remote_tag = %dialog.id().remote_tag(),
                        "Created A-leg dialog"
                    );

                let mut response = response;
                self.ensure_response_hygiene(request, &mut response);

                // Inject RFC 4028 session timer headers into single-leg 200 OK
                inject_session_timer_headers(&self.b2bua_config, request, &mut response);

                // Update call with A-leg dialog
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.a_leg.dialog = Some(dialog.clone());
                    call.state = fcp_b2bua_core::CallState::Established;
                    call.establish();
                });
                self.start_session_timer(&dialog);

                handle.send_final(response).await;
            }
            Err(e) => {
                error!("Failed to accept INVITE: {}", e);

                // Mark call as failed
                let _ = self
                    .call_manager
                    .terminate_call(&call_id, Some(format!("Failed to accept INVITE: {}", e)));

                let response = self.create_simple_response(request, 500, "Internal Server Error");
                handle.send_final(response).await;
            }
        }

        Ok(())
    }

    /// Evaluate routing policy for an INVITE request.
    ///
    /// Extracts caller, destination, tenant ID, and source IP from the SIP request
    /// and evaluates the routing policy.
    /// The routing engine's answer for an INVITE in the tenant the
    /// handler resolved: a caller that is one of the tenant's users is
    /// evaluated as that user; one that is not — a phone whose user is
    /// gone, an edge naming nobody — is refused, never an evaluation
    /// error (`docs/PRODUCTION_STACK.md` §1.5). A carrier's call on a
    /// trunk has no user to be, and is evaluated as the trunk's tenant's
    /// with no roles. A trusted edge's call is its `edge_user`'s, not
    /// the `From`'s.
    async fn evaluate_routing_policy(
        &self,
        request: &sip_core::Request,
        routing_service: &Arc<fcp_routing::RoutingService>,
        peer: SocketAddr,
        tenant_id: &fcp_core::TenantId,
        edge_user: Option<&str>,
        on_trunk: bool,
    ) -> Result<RoutingVerdict> {
        // Extract From header (caller)
        let from = sip_parse::header(request.headers(), "From")
            .ok_or_else(|| anyhow::anyhow!("Missing From header"))?;

        // Extract caller URI from From header (strip display name and parameters)
        let caller = match edge_user {
            Some(user) => user.to_string(),
            None => from
                .split('<')
                .nth(1)
                .and_then(|s| s.split('>').next())
                .unwrap_or(from)
                .trim()
                .to_string(),
        };

        // Extract To header (destination)
        let to = sip_parse::header(request.headers(), "To")
            .ok_or_else(|| anyhow::anyhow!("Missing To header"))?;

        // Extract destination URI from To header
        let destination = to
            .split('<')
            .nth(1)
            .and_then(|s| s.split('>').next())
            .unwrap_or(to)
            .trim()
            .to_string();

        // Get source IP from peer address
        let source_ip = peer.ip();

        if !on_trunk {
            match routing_service.caller(&caller, tenant_id).await? {
                Some(user) if !user.enabled => {
                    return Ok(RoutingVerdict::Refused("Caller is suspended".to_string()));
                }
                Some(_) => {}
                None => {
                    return Ok(RoutingVerdict::Refused(
                        "Caller is not a user of this tenant".to_string(),
                    ));
                }
            }
        }

        debug!(
            caller = %caller,
            destination = %destination,
            tenant_id = %tenant_id,
            source_ip = %source_ip,
            on_trunk,
            "Evaluating routing policy"
        );

        // Evaluate the call using routing service
        routing_service
            .evaluate_call(&caller, &destination, tenant_id, source_ip)
            .await
            .map(RoutingVerdict::Decision)
    }

    /// Generate an SDP answer for an INVITE request.
    ///
    /// Returns None if no media profile is configured, or Some(SDP string) if generated.
    fn generate_sdp_answer(&self, request: &sip_core::Request) -> Result<Option<String>> {
        // Check if we have a media profile configured
        let profile = match &self.media_profile {
            Some(p) => p,
            None => {
                debug!("No media profile configured, sending 200 OK without SDP");
                return Ok(None);
            }
        };

        // Check if INVITE contains SDP offer
        if !request.body().is_empty() {
            debug!(body_len = request.body().len(), "INVITE contains SDP offer");
            // TODO: Parse and negotiate SDP offer
            // For now, just generate our own SDP regardless of offer
        }

        // Generate SDP answer using our media profile
        let user_opt = self.uas.local_uri.user();
        let username = user_opt.unwrap_or("fcp");
        let session_desc = profile.build(
            username,
            &self.local_addr,
            self.rtp_port,
            None, // No video port for now
        );

        // Serialize to SDP string
        let sdp_string = sip_sdp::serialize::serialize_sdp(&session_desc);

        debug!(sdp_len = sdp_string.len(), "Generated SDP answer");

        Ok(Some(sdp_string))
    }

    /// Handle an ACK request (confirms 2xx response to INVITE).
    /// Handle an ACK request (B2BUA mode).
    ///
    /// ACK confirms dialog establishment after 200 OK.
    /// In B2BUA mode:
    /// 1. The ACK from A-leg (caller) confirms they received our 200 OK
    /// 2. The ACK to B-leg (callee) was already sent automatically by IntegratedUAC
    ///    when it received the 200 OK from B-leg
    /// 3. No forwarding needed - both legs have received their ACKs
    fn handle_ack(&self, request: &sip_core::Request) -> Result<()> {
        debug!("Received ACK from A-leg (B2BUA mode)");

        // Extract Call-ID to find the call
        let dialog_call_id = sip_parse::header(request.headers(), "Call-ID")
            .ok_or_else(|| anyhow::anyhow!("Missing Call-ID in ACK"))?;

        // Find the call by its dialog: Call-ID and tags.
        if let Some(call) = self.call_for_request(request, dialog_call_id.as_str()) {
            info!(
                call_id = %call.id,
                dialog_call_id = %dialog_call_id,
                state = ?call.state,
                "ACK received from A-leg - call dialog fully established"
            );
            self.ack_waits.acked(&call.id);

            // The call should already be in Established state from the 200 OK
            // ACK is just confirmation, no state change needed
            // B-leg already received ACK from IntegratedUAC when 200 OK was received
        } else {
            debug!(
                dialog_call_id = %dialog_call_id,
                "ACK received for unknown call (might be OK if call just ended)"
            );
        }

        Ok(())
    }

    /// Handle a BYE request by terminating the call (B2BUA mode).
    ///
    /// In B2BUA mode, we need to:
    /// 1. Find the call by dialog Call-ID
    /// 2. Determine which leg sent the BYE (A-leg or B-leg)
    /// 3. Forward BYE to the other leg
    /// 4. Terminate the call in CallManager
    /// 5. Send 200 OK response
    fn handle_bye(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling BYE request (B2BUA mode)");

        // Extract Call-ID to find the call
        let dialog_call_id = sip_parse::header(request.headers(), "Call-ID")
            .ok_or_else(|| anyhow::anyhow!("Missing Call-ID in BYE"))?;

        // Check if conference handler owns this call
        if let Some(conf_handler) = &self.conference_handler {
            // If the call_id isn't a conference call, handle_bye returns Err
            // and we fall through to normal B2BUA handling.
            if conf_handler.handle_bye(dialog_call_id.as_str()).is_ok() {
                debug!(call_id = %dialog_call_id, "Conference handler processed BYE");
                return Ok(self.create_simple_response(request, 200, "OK"));
            }
        }

        // Check if queue handler owns this call
        if let Some(queue_handler) = &self.queue_handler {
            if queue_handler.handle_bye(dialog_call_id.as_str()).is_ok() {
                debug!(call_id = %dialog_call_id, "Queue handler processed BYE");
                return Ok(self.create_simple_response(request, 200, "OK"));
            }
        }

        // Try to find the dialog - it might be in dialog_manager (A-leg/UAS) or only in call_manager (B-leg/UAC)
        let dialog_opt = self.dialog_manager.find_by_request(request);

        // Create response immediately before any cleanup
        // For A-leg (UAS): use UAS helper with dialog
        // For B-leg (UAC): create simple 200 OK response
        let response = if let Some(dialog) = dialog_opt {
            // A-leg BYE: use UAS helper
            self.uas.handle_bye(request, &dialog)?
        } else if self
            .call_for_request(request, dialog_call_id.as_str())
            .is_some()
        {
            // B-leg BYE: create simple 200 OK response
            debug!("BYE dialog not in dialog_manager (likely B-leg UAC), creating simple 200 OK");
            self.create_simple_response(request, 200, "OK")
        } else {
            // No dialog and no call: RFC 3261 §12.2.2.
            debug!(call_id = %dialog_call_id, "BYE for no dialog; 481");
            return Ok(self.create_simple_response(
                request,
                481,
                "Call/Transaction Does Not Exist",
            ));
        };

        // Now that we've responded, proceed with forwarding and cleanup
        // Find the call by its dialog: Call-ID and tags.
        if let Some(call) = self.call_for_request(request, dialog_call_id.as_str()) {
            info!(
                call_id = %call.id,
                dialog_call_id = %dialog_call_id,
                duration = ?call.duration(),
                "Terminating call due to BYE"
            );

            stop_timers_for_call(&self.session_timer_manager, &call);

            // Check if BYE is from A-leg
            let is_a_leg = call
                .a_leg
                .dialog
                .as_ref()
                .map(|d| d.id().call_id() == dialog_call_id.as_str())
                .unwrap_or(false);

            // For the record: who hung up, and the media counters before
            // anything tears the session down. The termination itself
            // happens when the last leg's dialog is gone, which may be on
            // the forwarded BYE's completion; the reason set here wins there.
            self.call_manager.set_pending_termination(
                &call.id,
                fcp_cdr::TerminationReason::hangup_by(if is_a_leg {
                    fcp_core::storage::HangupParty::Caller
                } else {
                    fcp_core::storage::HangupParty::Callee
                }),
            );

            // Determine which leg sent the BYE and forward to the other leg
            if let Some(uac) = &self.uac {
                if is_a_leg {
                    // BYE from A-leg, send BYE to B-leg
                    if let Some(b_leg) = &call.b_leg {
                        if let Some(b_leg_dialog) = &b_leg.dialog {
                            info!(
                                call_id = %call.id,
                                has_flow_token = b_leg.flow_token.is_some(),
                                "BYE from A-leg, forwarding to B-leg"
                            );

                            self.spawn_bye_with_retry(
                                Arc::clone(uac),
                                b_leg_dialog.clone(),
                                call.id,
                                false,
                                b_leg.flow_token.clone(),
                            );
                        } else {
                            debug!(call_id = %call.id, "B-leg has no dialog, skipping BYE forward");
                        }
                    } else {
                        debug!(call_id = %call.id, "No B-leg found, skipping BYE forward");
                    }
                } else {
                    // BYE from B-leg, send BYE to A-leg
                    if let Some(a_leg_dialog) = &call.a_leg.dialog {
                        info!(
                            call_id = %call.id,
                            has_flow_token = call.a_leg.flow_token.is_some(),
                            "BYE from B-leg, forwarding to A-leg"
                        );
                        // A caller that came up a site's link is reached
                        // down that flow; any other caller by its contact.
                        self.spawn_bye_with_retry(
                            Arc::clone(uac),
                            a_leg_dialog.clone(),
                            call.id,
                            true,
                            call.a_leg.flow_token.clone(),
                        );
                    } else {
                        debug!(call_id = %call.id, "A-leg has no dialog, skipping BYE forward");
                    }
                }
            } else {
                debug!(call_id = %call.id, "No UAC available, cannot forward BYE");
            }

            // Remove the dialog for the leg that sent the BYE
            // The other leg's dialog will be removed by spawn_bye_with_retry when that BYE completes
            let removing_dialog_id = if is_a_leg {
                let dialog_id = call.a_leg.dialog.as_ref().map(|d| d.id().clone());
                let _ = self.call_manager.update_call(&call.id, |call| {
                    call.a_leg.dialog = None;
                });
                dialog_id
            } else if let Some(b_leg) = &call.b_leg {
                let dialog_id = b_leg.dialog.as_ref().map(|d| d.id().clone());
                if b_leg.dialog.is_some() {
                    let _ = self.call_manager.update_call(&call.id, |call| {
                        if let Some(b) = &mut call.b_leg {
                            b.dialog = None;
                        }
                    });
                }
                dialog_id
            } else {
                None
            };

            // Remove the dialog from dialog manager
            if let Some(dialog_id) = removing_dialog_id {
                self.dialog_manager.remove(&dialog_id);
            }

            // Check if both legs are now done (no more dialogs), and if so
            // mark the call as ending — in one step, under the call's own
            // lock: a leg a flow dialed may be answering this instant, and
            // it either lands before this (and gets the BYE here) or sees
            // the call ending and hangs itself up
            // (`docs/ENTERPRISE_CALLING.md` §3.2).
            let forwarded_to_b = is_a_leg
                && call
                    .b_leg
                    .as_ref()
                    .and_then(|b| b.dialog.as_ref())
                    .is_some();
            let mut late_b_leg = None;
            let mut both_done = false;
            let _ = self.call_manager.update_call(&call.id, |c| {
                let b_dialog = c.b_leg.as_ref().and_then(|b| b.dialog.clone());
                if is_a_leg && !forwarded_to_b && b_dialog.is_some() {
                    late_b_leg =
                        b_dialog.zip(Some(c.b_leg.as_ref().and_then(|b| b.flow_token.clone())));
                    return;
                }
                if c.a_leg.dialog.is_none() && b_dialog.is_none() {
                    both_done = true;
                    c.state = fcp_b2bua_core::CallState::Terminating;
                }
            });
            if let (Some((b_dialog, flow)), Some(uac)) = (late_b_leg, &self.uac) {
                info!(call_id = %call.id, "A leg answered as the caller hung up; forwarding the BYE to it");
                self.spawn_bye_with_retry(Arc::clone(uac), b_dialog, call.id, false, flow);
            }
            if let Some(updated_call) = self.call_manager.get_call(&call.id) {
                if both_done {
                    info!(call_id = %call.id, "Both legs terminated, cleaning up call");

                    // Cleanup per-dialog state for remaining dialogs to prevent memory leaks
                    if let Some(a_dialog) = &updated_call.a_leg.dialog {
                        self.rate_limits.remove(a_dialog.id());
                        self.mid_dialog_lock.remove(a_dialog.id());
                        let keys: Vec<_> = self
                            .subscription_index
                            .iter()
                            .filter(|entry| entry.key().0 == *a_dialog.id())
                            .map(|entry| entry.key().clone())
                            .collect();
                        for key in keys {
                            self.subscription_index.remove(&key);
                        }
                        debug!(dialog_id = ?a_dialog.id(), "Cleaned up A-leg dialog state");
                    }
                    if let Some(b_leg) = &updated_call.b_leg {
                        if let Some(b_dialog) = &b_leg.dialog {
                            self.rate_limits.remove(b_dialog.id());
                            self.mid_dialog_lock.remove(b_dialog.id());
                            let keys: Vec<_> = self
                                .subscription_index
                                .iter()
                                .filter(|entry| entry.key().0 == *b_dialog.id())
                                .map(|entry| entry.key().clone())
                                .collect();
                            for key in keys {
                                self.subscription_index.remove(&key);
                            }
                            debug!(dialog_id = ?b_dialog.id(), "Cleaned up B-leg dialog state");
                        }
                    }

                    // Finally, terminate the call in CallManager, once the
                    // media session has given up its counters.
                    self.end_call_after_media(
                        &call,
                        fcp_cdr::TerminationReason::Hangup { by: None },
                        true,
                        "BYE",
                    );
                } else {
                    // The other leg's dialog is still up: its BYE completion
                    // ends the call. Take the counters now, while the
                    // session is still there.
                    self.capture_rtp_stats(&call);
                    debug!(
                        call_id = %call.id,
                        a_done = updated_call.a_leg.dialog.is_none(),
                        "Waiting for other leg to complete BYE"
                    );
                }
            }
        } else {
            debug!(
                dialog_call_id = %dialog_call_id,
                "BYE received for unknown call"
            );
        }

        // Return the response we already created at the beginning
        Ok(response)
    }

    /// Handle in-dialog re-INVITE by forwarding to the opposite leg.
    async fn handle_reinvite(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling in-dialog re-INVITE");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let _guard = self.acquire_mid_dialog_lock(target_dialog.id()).await;
        let cseq = sip_parse::header(request.headers(), "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id().clone())
        } else {
            call.b_leg
                .as_ref()
                .and_then(|b| b.dialog.as_ref().map(|d| d.id().clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "REINVITE",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id(),
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for re-INVITE forwarding"))?;

        let prior_sdp = if is_a_leg {
            call.a_leg.sdp.as_deref()
        } else {
            call.b_leg.as_ref().and_then(|b| b.sdp.as_deref())
        };

        if self.check_rate_limit(
            "REINVITE",
            src_dialog_id.as_ref().unwrap_or(target_dialog.id()),
            self.settings.reinvite_rate_limit,
        ) {
            warn!(call_id = %call.id, "re-INVITE rate limited");
            let mut resp = self.create_simple_response(request, 429, "Too Many Requests");
            let _ = resp.headers_mut().push(
                SmolStr::new("Retry-After"),
                SmolStr::new(self.settings.rate_window.as_secs().to_string()),
            );
            return Ok(resp);
        }
        let mut body = validate_sdp_body_static(request, prior_sdp)?;
        // The sender's own offer, before any rewrite: what says whether it
        // holds the far party, and what its answer answers.
        let sender_offer = body.clone();

        // With hold music, the node answers a hold (and its resume) itself
        // and plays music into the held leg: the far phone is never told
        // (`docs/ENTERPRISE_CALLING.md` §3.3).
        if call.media_anchored && self.media_config.as_ref().is_some_and(|m| m.hold_music) {
            if let Some(offer) = sender_offer.as_deref() {
                let sender = fcp_b2bua_core::LegSide::of(is_a_leg);
                let holds = crate::hold::holds(offer);
                if holds || call.held == Some(sender.other()) {
                    return self
                        .answer_hold_locally(request, &call, sender, offer, prior_sdp, holds)
                        .await;
                }
            }
        }

        // If media is anchored, rewrite SDP before forwarding to peer
        if call.media_anchored {
            if let (Some(body_str), Some(ref media_session_id), Some(media_manager)) = (
                body.as_ref(),
                call.media_session_id.as_ref(),
                self.media_manager.as_ref(),
            ) {
                debug!(
                    call_id = %call.id,
                    media_session_id = %media_session_id,
                    "Media is anchored, rewriting incoming re-INVITE SDP"
                );

                match media_manager.get_session(media_session_id).await {
                    Ok(session) => {
                        // Forwarded to the other leg, told its own port.
                        let (rtp_port, _rtcp_port) = session
                            .ports_for(media_leg(fcp_b2bua_core::LegSide::of(is_a_leg).other()));
                        let rewritten_sdp = fcp_media::rewrite_sdp_for_anchoring(
                            body_str,
                            &self.local_addr,
                            rtp_port,
                            None,
                        );

                        info!(
                            call_id = %call.id,
                            leg = if is_a_leg { "A" } else { "B" },
                            server_ip = %self.local_addr,
                            rtp_port = rtp_port,
                            "Rewrote re-INVITE SDP for media anchoring"
                        );

                        body = Some(rewritten_sdp);
                    }
                    Err(e) => {
                        error!(
                            call_id = %call.id,
                            media_session_id = %media_session_id,
                            error = %e,
                            "Failed to get media session for re-INVITE SDP rewriting"
                        );
                    }
                }
            }
        }

        self.log_sip_message(
            "outbound",
            "INVITE",
            Some(call.id.to_string().as_str()),
            Some(target_dialog.id()),
            cseq.as_ref().map(|v| v.as_str()),
            None,
            None,
            None,
            None,
            None,
        );
        // The far leg reached down a flow — a site's link, a phone
        // registered over TLS — is re-invited down it; any other by its
        // contact.
        let target_flow = if is_a_leg {
            call.b_leg.as_ref().and_then(|b| b.flow_token.clone())
        } else {
            call.a_leg.flow_token.clone()
        }
        .and_then(|token| self.flow_registry.lookup(&token).map(|flow| (token, flow)));
        let sent = match target_flow {
            Some((token, flow)) => {
                info!(
                    call_id = %call.id,
                    flow_token = %token,
                    peer_addr = %flow.peer_addr,
                    "Forwarding re-INVITE via RFC 5626 flow"
                );
                uac.send_reinvite_via_flow(
                    &mut target_dialog,
                    body.as_deref(),
                    flow.stream,
                    flow.peer_addr,
                )
                .await
            }
            None => uac.send_reinvite(&mut target_dialog, body.as_deref()).await,
        };
        let peer_resp = match sent {
            Ok(r) => {
                info!(
                    status = r.start_line().code(),
                    "Forwarded re-INVITE to peer"
                );
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "REINVITE"), ("result", "ok")],
                );
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "REINVITE"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward re-INVITE to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "REINVITE"), ("result", "error")],
                );
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &self.subscription_created_at,
                    &call.id,
                    target_dialog.id(),
                    "re-INVITE forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start_line().code() == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                &self.rate_limits,
                &self.mid_dialog_lock,
                &self.subscription_index,
                &self.subscription_created_at,
                &call.id,
                target_dialog.id(),
                "Peer returned 481 to re-INVITE",
            );
            self.metrics.inc_counter(
                "fcp_sip_mid_dialog_forward_total",
                &[("method", "REINVITE"), ("result", "481")],
            );
        }

        // If media is anchored and peer sent SDP in response, rewrite it before relaying back
        let mut peer_resp = peer_resp;
        if call.media_anchored && peer_resp.start_line().is_success() {
            if let (Some(ref media_session_id), Some(media_manager)) =
                (call.media_session_id.as_ref(), self.media_manager.as_ref())
            {
                if !peer_resp.body().is_empty() {
                    let response_sdp = String::from_utf8_lossy(peer_resp.body().as_ref());

                    debug!(
                        call_id = %call.id,
                        media_session_id = %media_session_id,
                        "Media is anchored, rewriting peer re-INVITE response SDP"
                    );

                    match media_manager.get_session(media_session_id).await {
                        Ok(session) => {
                            // The answer goes back to the sender, on its leg's port.
                            let (rtp_port, _rtcp_port) =
                                session.ports_for(media_leg(fcp_b2bua_core::LegSide::of(is_a_leg)));
                            // An encrypted sender is answered encrypted, with its own leg's key.
                            let a_leg_srtp = match sender_offer.as_deref() {
                                Some(offer) if !fcp_media::parse_sdes_crypto(offer).is_empty() => {
                                    let side = if is_a_leg {
                                        fcp_b2bua_core::LegSide::A
                                    } else {
                                        fcp_b2bua_core::LegSide::B
                                    };
                                    self.leg_srtp(&call.id, side, &session).await
                                }
                                _ => None,
                            };

                            // The answer to the sender's own offer: its codec
                            // (the transcoder bridges the far leg's), and the
                            // direction that answers its direction — `recvonly`
                            // to a hold. It used to be built from the call's
                            // first INVITE, which answered every hold
                            // `sendrecv` (`docs/ENTERPRISE_CALLING.md` §1.2).
                            let rewritten_sdp = match sender_offer.as_deref() {
                                Some(offer)
                                    if !fcp_media::sdp::parse_sdp_codecs(offer).is_empty() =>
                                {
                                    fcp_media::construct_answer_for_a_leg(
                                        offer,
                                        &self.local_addr,
                                        rtp_port,
                                        a_leg_srtp.as_ref(),
                                    )
                                }
                                _ => fcp_media::rewrite_sdp_for_anchoring(
                                    &response_sdp,
                                    &self.local_addr,
                                    rtp_port,
                                    a_leg_srtp.as_ref(),
                                ),
                            };

                            info!(
                                call_id = %call.id,
                                leg = if is_a_leg { "B" } else { "A" },
                                server_ip = %self.local_addr,
                                rtp_port = rtp_port,
                                "Rewrote peer re-INVITE response SDP for media anchoring"
                            );

                            // Rebuild response with rewritten SDP
                            let mut new_headers = peer_resp.headers().clone();
                            let new_body = bytes::Bytes::from(rewritten_sdp.into_bytes());
                            let _ = new_headers.set_or_push(
                                SmolStr::new("Content-Length"),
                                SmolStr::new(new_body.len().to_string()),
                            );

                            peer_resp = sip_core::Response::new(
                                peer_resp.start_line().clone(),
                                new_headers,
                                new_body,
                            )?;
                        }
                        Err(e) => {
                            error!(
                                call_id = %call.id,
                                media_session_id = %media_session_id,
                                error = %e,
                                "Failed to get media session for peer response SDP rewriting"
                            );
                        }
                    }
                }
            }
        }

        // Update stored SDP for originating leg if present
        if let Some(body) = body {
            store_leg_sdp(&self.call_manager, &call.id, is_a_leg, body);
        }
        // Which leg is on hold, once the far end took the offer.
        if peer_resp.start_line().is_success() {
            if let Some(offer) = sender_offer.as_deref() {
                let sender = fcp_b2bua_core::LegSide::of(is_a_leg);
                let held = if crate::hold::holds(offer) {
                    Some(sender.other())
                } else if call.held == Some(sender.other()) {
                    None
                } else {
                    call.held
                };
                if held != call.held {
                    info!(call_id = %call.id, ?held, "Hold changed");
                    let _ = self.call_manager.update_call(&call.id, |c| c.held = held);
                }
            }
        }
        self.persist_updated_dialog(&call.id, !is_a_leg, target_dialog.clone());
        self.start_session_timer(&target_dialog);

        // Also refresh timers on the originating leg based on the response we will relay
        if let Some((mut src_dialog, is_a_leg_src)) = if is_a_leg {
            call.a_leg.dialog.clone().map(|d| (d, true))
        } else {
            call.b_leg
                .as_ref()
                .and_then(|b| b.dialog.clone())
                .map(|d| (d, false))
        } {
            if let Some((se, _refresher)) = extract_session_timer(&peer_resp) {
                let _ = src_dialog.set_session_expires(se);
                self.persist_updated_dialog(&call.id, is_a_leg_src, src_dialog.clone());
                self.start_session_timer(&src_dialog);
            } else if peer_resp.start_line().code() == 422 {
                if let Some(min_se) = extract_min_se(&peer_resp) {
                    let _ = src_dialog.set_session_expires(min_se);
                    debug!(call_id = %call.id, dialog_id = ?src_dialog.id(), min_se = ?min_se, "Applied Min-SE from 422 to originating leg dialog");
                    self.persist_updated_dialog(&call.id, is_a_leg_src, src_dialog);
                }
            }
        }

        Ok(self.build_in_dialog_response(request, &peer_resp))
    }

    /// Answer a hold, or the resume of one, from the node: the holder's
    /// media side follows its offer if it moved, the answer is the node's
    /// (`recvonly` to a hold), and the held leg is marked — which starts or
    /// stops the music (`crate::hold_music`).
    async fn answer_hold_locally(
        &self,
        request: &sip_core::Request,
        call: &fcp_b2bua_core::Call,
        sender: fcp_b2bua_core::LegSide,
        offer: &str,
        prior_sdp: Option<&str>,
        holds: bool,
    ) -> Result<sip_core::Response> {
        let (Some(session_id), Some(media_manager)) =
            (call.media_session_id, self.media_manager.as_ref())
        else {
            return Ok(self.create_simple_response(request, 500, "Media Unavailable"));
        };
        // Answered in the codec the leg uses now: the media session goes on
        // decoding it. An offer without it moves the session to the codec
        // the answer names.
        let kept = prior_sdp.and_then(|prior| crate::hold::keeping_codec(offer, prior));
        let codec_changed = kept.is_none() && prior_sdp.is_some();
        let offer = kept.as_deref().unwrap_or(offer);
        let moved = prior_sdp
            .map(|prior| crate::conference_migration::media_endpoint_changed(prior, offer))
            .unwrap_or(true);
        if moved || codec_changed {
            let updated = match sender {
                fcp_b2bua_core::LegSide::A => media_manager
                    .update_session(&session_id, offer.to_string())
                    .await
                    .map(|_| ()),
                // An encrypted callee stays keyed from this node's own offer,
                // which its answer will name again.
                fcp_b2bua_core::LegSide::B => {
                    match self.b_leg_keys.get(&call.id).map(|o| o.clone()) {
                        Some(ours) if !fcp_media::parse_sdes_crypto(offer).is_empty() => {
                            media_manager
                                .update_session_b_leg_offered(
                                    &session_id,
                                    offer.to_string(),
                                    Some(ours),
                                )
                                .await
                        }
                        _ => {
                            media_manager
                                .update_session_b_leg(&session_id, offer.to_string())
                                .await
                        }
                    }
                }
            };
            if let Err(e) = updated {
                warn!(call_id = %call.id, error = %e, "The holder's new media endpoint was not taken");
            }
        }
        let session = match media_manager.get_session(&session_id).await {
            Ok(session) => session,
            Err(e) => {
                warn!(call_id = %call.id, error = %e, "No media session to answer a hold from");
                return Ok(self.create_simple_response(request, 500, "Media Unavailable"));
            }
        };
        let port = session.ports_for(media_leg(sender)).0;
        // An encrypted holder is answered encrypted, with its leg's key.
        let srtp = if fcp_media::parse_sdes_crypto(offer).is_empty() {
            None
        } else {
            self.leg_srtp(&call.id, sender, &session).await
        };
        let answer =
            fcp_media::construct_answer_for_a_leg(offer, &self.local_addr, port, srtp.as_ref());
        store_leg_sdp(
            &self.call_manager,
            &call.id,
            sender == fcp_b2bua_core::LegSide::A,
            offer.to_string(),
        );
        let held = holds.then_some(sender.other());
        info!(call_id = %call.id, ?held, "Hold answered by the node");
        let _ = self.call_manager.update_call(&call.id, |c| c.held = held);
        self.build_final_ok_response(request, None, Some(&answer))
    }

    /// Handle in-dialog SUBSCRIBE (e.g., REFER implicit subscription) by forwarding.
    async fn handle_subscribe(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling SUBSCRIBE");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let _guard = self.acquire_mid_dialog_lock(target_dialog.id()).await;
        let cseq = sip_parse::header(request.headers(), "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id().clone())
        } else {
            call.b_leg
                .as_ref()
                .and_then(|b| b.dialog.as_ref().map(|d| d.id().clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "SUBSCRIBE",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id(),
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for SUBSCRIBE forwarding"))?;

        let event = sip_parse::header(request.headers(), "Event")
            .ok_or_else(|| anyhow::anyhow!("Missing Event header"))?;
        let expires =
            sip_parse::header(request.headers(), "Expires").and_then(|v| v.parse::<u32>().ok());

        if self.check_rate_limit(
            "SUBSCRIBE",
            src_dialog_id.as_ref().unwrap_or(target_dialog.id()),
            self.settings.update_rate_limit,
        ) {
            warn!(call_id = %call.id, "SUBSCRIBE rate limited");
            let mut resp = self.create_simple_response(request, 429, "Too Many Requests");
            let _ = resp.headers_mut().push(
                SmolStr::new("Retry-After"),
                SmolStr::new(self.settings.rate_window.as_secs().to_string()),
            );
            return Ok(resp);
        }
        self.log_sip_message(
            "outbound",
            "SUBSCRIBE",
            Some(call.id.to_string().as_str()),
            Some(target_dialog.id()),
            cseq.as_ref().map(|v| v.as_str()),
            None,
            None,
            None,
            None,
            None,
        );
        let (peer_resp, subscription) = match uac
            .send_subscribe(&mut target_dialog, event.as_str(), expires)
            .await
        {
            Ok(r) => {
                info!(
                    status = r.0.start_line().code(),
                    "Forwarded SUBSCRIBE to peer"
                );
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "SUBSCRIBE"), ("result", "ok")],
                );
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "SUBSCRIBE"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward SUBSCRIBE to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "SUBSCRIBE"), ("result", "error")],
                );
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &self.subscription_created_at,
                    &call.id,
                    target_dialog.id(),
                    "SUBSCRIBE forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start_line().code() == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                &self.rate_limits,
                &self.mid_dialog_lock,
                &self.subscription_index,
                &self.subscription_created_at,
                &call.id,
                target_dialog.id(),
                "Peer returned 481 to SUBSCRIBE",
            );
            self.metrics.inc_counter(
                "fcp_sip_mid_dialog_forward_total",
                &[("method", "SUBSCRIBE"), ("result", "481")],
            );
        }

        if let Some(sub) = subscription {
            let sub_clone = sub.clone();
            let _ = self.subscription_manager.insert(sub);
            self.index_subscription(sub_clone);
        }

        self.persist_updated_dialog(&call.id, !is_a_leg, target_dialog);

        Ok(self.build_in_dialog_response(request, &peer_resp))
    }

    /// Handle in-dialog NOTIFY by forwarding.
    async fn handle_notify(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling NOTIFY");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let target_dialog = target_dialog.clone();
        let _guard = self.acquire_mid_dialog_lock(target_dialog.id()).await;
        let cseq = sip_parse::header(request.headers(), "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id().clone())
        } else {
            call.b_leg
                .as_ref()
                .and_then(|b| b.dialog.as_ref().map(|d| d.id().clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "NOTIFY",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id(),
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for NOTIFY forwarding"))?;

        // Build a subscription placeholder from dialog + event to forward notify
        let event = sip_parse::header(request.headers(), "Event")
            .ok_or_else(|| anyhow::anyhow!("Missing Event header"))?;
        let body = if request.body().is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(request.body()).to_string())
        };

        self.check_body_size(request, self.settings.notify_max_body_bytes)?;

        // Record body size histogram
        self.metrics.record_histogram(
            "fcp_sip_notify_body_bytes",
            request.body().len() as f64,
            &[("event", event.as_str())],
        );

        // Create a synthetic subscription to use contact/route info from dialog target
        // (we don't track full subscription state yet beyond storage)
        if let Some(sub) = self.find_subscription(target_dialog.id(), event.as_str()) {
            let state = sip_dialog::SubscriptionState::Active;
            self.log_sip_message(
                "outbound",
                "NOTIFY",
                Some(call.id.to_string().as_str()),
                Some(target_dialog.id()),
                cseq.as_ref().map(|v| v.as_str()),
                None,
                None,
                None,
                body.as_ref().map(|b| b.as_bytes()),
                None,
            );
            if self.check_rate_limit(
                "NOTIFY",
                src_dialog_id.as_ref().unwrap_or(target_dialog.id()),
                self.settings.info_rate_limit,
            ) {
                warn!(call_id = %call.id, "NOTIFY rate limited");
                let mut resp = self.create_simple_response(request, 429, "Too Many Requests");
                let _ = resp.headers_mut().push(
                    SmolStr::new("Retry-After"),
                    SmolStr::new(self.settings.rate_window.as_secs().to_string()),
                );
                return Ok(resp);
            }
            let peer_resp = match uac.send_notify(&sub, state, body.as_deref()).await {
                Ok(r) => {
                    info!(status = r.start_line().code(), "Forwarded NOTIFY to peer");
                    self.metrics.inc_counter(
                        "fcp_sip_mid_dialog_forward_total",
                        &[("method", "NOTIFY"), ("result", "ok")],
                    );
                    self.metrics.observe_ms(
                        "fcp_sip_mid_dialog_forward_ms",
                        start.elapsed().as_millis(),
                        &[("method", "NOTIFY"), ("result", "ok")],
                    );
                    r
                }
                Err(e) => {
                    warn!(error = %e, "Failed to forward NOTIFY to peer");
                    self.metrics.inc_counter(
                        "fcp_sip_mid_dialog_forward_total",
                        &[("method", "NOTIFY"), ("result", "error")],
                    );
                    cleanup_after_forward_failure(
                        &self.call_manager,
                        &self.dialog_manager,
                        &self.session_timer_manager,
                        &self.rate_limits,
                        &self.mid_dialog_lock,
                        &self.subscription_index,
                        &self.subscription_created_at,
                        &call.id,
                        target_dialog.id(),
                        "NOTIFY forward failed",
                    );
                    return Ok(self.create_simple_response(
                        request,
                        481,
                        "Subscription Does Not Exist",
                    ));
                }
            };

            if peer_resp.start_line().code() == 481 {
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &self.subscription_created_at,
                    &call.id,
                    target_dialog.id(),
                    "Peer returned 481 to NOTIFY",
                );
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "NOTIFY"), ("result", "481")],
                );
            }
            self.persist_updated_dialog(&call.id, !is_a_leg, target_dialog);
            return Ok(self.build_in_dialog_response(request, &peer_resp));
        }

        // Fallback: if subscription not found, try to create a transient one from dialog/event
        let transient_sub = self.create_transient_subscription(&target_dialog, event.as_str());
        if let Some(sub) = transient_sub {
            let state = sip_dialog::SubscriptionState::Active;
            self.log_sip_message(
                "outbound",
                "NOTIFY",
                Some(call.id.to_string().as_str()),
                Some(target_dialog.id()),
                cseq.as_ref().map(|v| v.as_str()),
                None,
                None,
                None,
                body.as_ref().map(|b| b.as_bytes()),
                None,
            );
            if self.check_rate_limit(
                "NOTIFY",
                src_dialog_id.as_ref().unwrap_or(target_dialog.id()),
                self.settings.info_rate_limit,
            ) {
                warn!(call_id = %call.id, "NOTIFY rate limited");
                let mut resp = self.create_simple_response(request, 429, "Too Many Requests");
                let _ = resp.headers_mut().push(
                    SmolStr::new("Retry-After"),
                    SmolStr::new(self.settings.rate_window.as_secs().to_string()),
                );
                return Ok(resp);
            }
            let peer_resp = match uac.send_notify(&sub, state, body.as_deref()).await {
                Ok(r) => {
                    info!(
                        status = r.start_line().code(),
                        "Forwarded NOTIFY (transient sub) to peer"
                    );
                    self.metrics.inc_counter(
                        "fcp_sip_mid_dialog_forward_total",
                        &[("method", "NOTIFY"), ("result", "ok")],
                    );
                    self.metrics.observe_ms(
                        "fcp_sip_mid_dialog_forward_ms",
                        start.elapsed().as_millis(),
                        &[("method", "NOTIFY"), ("result", "ok")],
                    );
                    r
                }
                Err(e) => {
                    warn!(error = %e, "Failed to forward NOTIFY (transient) to peer");
                    self.metrics.inc_counter(
                        "fcp_sip_mid_dialog_forward_total",
                        &[("method", "NOTIFY"), ("result", "error")],
                    );
                    cleanup_after_forward_failure(
                        &self.call_manager,
                        &self.dialog_manager,
                        &self.session_timer_manager,
                        &self.rate_limits,
                        &self.mid_dialog_lock,
                        &self.subscription_index,
                        &self.subscription_created_at,
                        &call.id,
                        target_dialog.id(),
                        "NOTIFY forward failed",
                    );
                    return Ok(self.create_simple_response(
                        request,
                        481,
                        "Subscription Does Not Exist",
                    ));
                }
            };

            if peer_resp.start_line().code() == 481 {
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &self.subscription_created_at,
                    &call.id,
                    target_dialog.id(),
                    "Peer returned 481 to NOTIFY",
                );
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "NOTIFY"), ("result", "481")],
                );
            }
            return Ok(self.build_in_dialog_response(request, &peer_resp));
        }

        Ok(self.create_simple_response(request, 481, "Subscription Does Not Exist"))
    }

    /// Handle in-dialog UPDATE by forwarding to the opposite leg.
    async fn handle_update(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling UPDATE");

        // Check conference handler first — conference participants don't have B2BUA calls
        if let Some(ref conf) = self.conference_handler {
            if let Ok(resp) = conf.handle_update(request) {
                return Ok(resp);
            }
        }

        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let _guard = self.acquire_mid_dialog_lock(target_dialog.id()).await;
        let cseq = sip_parse::header(request.headers(), "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id().clone())
        } else {
            call.b_leg
                .as_ref()
                .and_then(|b| b.dialog.as_ref().map(|d| d.id().clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "UPDATE",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id(),
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for UPDATE forwarding"))?;

        let prior_sdp = if is_a_leg {
            call.a_leg.sdp.as_deref()
        } else {
            call.b_leg.as_ref().and_then(|b| b.sdp.as_deref())
        };

        if self.check_rate_limit(
            "UPDATE",
            src_dialog_id.as_ref().unwrap_or(target_dialog.id()),
            self.settings.update_rate_limit,
        ) {
            warn!(call_id = %call.id, "UPDATE rate limited");
            let mut resp = self.create_simple_response(request, 429, "Too Many Requests");
            let _ = resp.headers_mut().push(
                SmolStr::new("Retry-After"),
                SmolStr::new(self.settings.rate_window.as_secs().to_string()),
            );
            return Ok(resp);
        }
        let body = validate_sdp_body_static(request, prior_sdp)?;

        self.log_sip_message(
            "outbound",
            "UPDATE",
            Some(call.id.to_string().as_str()),
            Some(target_dialog.id()),
            cseq.as_ref().map(|v| v.as_str()),
            None,
            None,
            None,
            None,
            None,
        );
        let peer_resp = match uac.send_update(&mut target_dialog, body.as_deref()).await {
            Ok(r) => {
                info!(status = r.start_line().code(), "Forwarded UPDATE to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "UPDATE"), ("result", "ok")],
                );
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "UPDATE"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward UPDATE to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "UPDATE"), ("result", "error")],
                );
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &self.subscription_created_at,
                    &call.id,
                    target_dialog.id(),
                    "UPDATE forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start_line().code() == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                &self.rate_limits,
                &self.mid_dialog_lock,
                &self.subscription_index,
                &self.subscription_created_at,
                &call.id,
                target_dialog.id(),
                "Peer returned 481 to UPDATE",
            );
            self.metrics.inc_counter(
                "fcp_sip_mid_dialog_forward_total",
                &[("method", "UPDATE"), ("result", "481")],
            );
        }
        // Update stored SDP for originating leg if present
        if let Some(body) = body {
            store_leg_sdp(&self.call_manager, &call.id, is_a_leg, body);
        }
        self.persist_updated_dialog(&call.id, !is_a_leg, target_dialog.clone());
        self.start_session_timer(&target_dialog);

        // Refresh timers on the originating leg based on the response we will relay
        if let Some((mut src_dialog, is_a_leg_src)) = if is_a_leg {
            call.a_leg.dialog.clone().map(|d| (d, true))
        } else {
            call.b_leg
                .as_ref()
                .and_then(|b| b.dialog.clone())
                .map(|d| (d, false))
        } {
            if let Some((se, _refresher)) = extract_session_timer(&peer_resp) {
                let _ = src_dialog.set_session_expires(se);
                self.persist_updated_dialog(&call.id, is_a_leg_src, src_dialog.clone());
                self.start_session_timer(&src_dialog);
            } else if peer_resp.start_line().code() == 422 {
                if let Some(min_se) = extract_min_se(&peer_resp) {
                    let _ = src_dialog.set_session_expires(min_se);
                    debug!(call_id = %call.id, dialog_id = ?src_dialog.id(), min_se = ?min_se, "Applied Min-SE from 422 to originating leg dialog");
                    self.persist_updated_dialog(&call.id, is_a_leg_src, src_dialog);
                }
            }
        }

        Ok(self.build_in_dialog_response(request, &peer_resp))
    }

    /// An in-dialog PRACK (RFC 3262). The node sends no provisional
    /// reliably: a callee's reliable ones are PRACKed by the B-leg's own
    /// INVITE transaction (siphon-rs) and relayed to the caller as ordinary
    /// ones. So a PRACK acknowledges nothing the node sent: 481 (§3), and
    /// the call is left as it is. Relaying it would PRACK the callee twice,
    /// and its 481 to the second used to tear the dialog down.
    async fn handle_prack(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling PRACK");
        // No dialog at all is `NoSuchDialog`, answered 481 by the loop.
        self.lookup_call_for_in_dialog(request)?;
        Ok(self.create_simple_response(request, 481, "Call/Transaction Does Not Exist"))
    }

    /// Handle in-dialog INFO by forwarding to the opposite leg.
    ///
    /// If the call has an active flow and the INFO contains DTMF, routes to FlowCallHandler.
    async fn handle_info(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling INFO");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;

        // Check if this call has an active flow and INFO is DTMF
        if call.flow_id.is_some() {
            if let Some(content_type) = sip_parse::header(request.headers(), "Content-Type") {
                if content_type.as_str() == "application/dtmf-relay" {
                    // Parse DTMF digit from body (RFC 4733 format)
                    let body = String::from_utf8_lossy(request.body());
                    if let Some(digit) = self.parse_dtmf_from_info_body(&body) {
                        debug!(call_id = %call.id, digit = %digit, "Received DTMF for flow execution");

                        // Route to FlowCallHandler if available
                        if let Some(flow_handler) = &self.flow_handler {
                            match flow_handler
                                .handle_dtmf_input(call.id, digit.to_string())
                                .await
                            {
                                Ok(()) => {
                                    info!(call_id = %call.id, digit = %digit, "DTMF routed to flow execution");
                                    return Ok(self.create_simple_response(request, 200, "OK"));
                                }
                                Err(e) => {
                                    error!(call_id = %call.id, error = %e, "Failed to handle DTMF for flow");
                                    // Fall through to normal forwarding
                                }
                            }
                        }
                    }
                }
            }
        }
        // A digit for a leg that takes telephone-event instead
        // (`docs/TENANT_TRUNKS.md` §6).
        if let Some(response) = self.convert_info_dtmf(request, &call, is_a_leg).await {
            return Ok(response);
        }
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let cseq = sip_parse::header(request.headers(), "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id().clone())
        } else {
            call.b_leg
                .as_ref()
                .and_then(|b| b.dialog.as_ref().map(|d| d.id().clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "INFO",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id(),
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for INFO forwarding"))?;

        let content_type = sip_parse::header(request.headers(), "Content-Type")
            .ok_or_else(|| anyhow::anyhow!("Missing Content-Type for INFO"))?;
        let body = String::from_utf8_lossy(request.body()).to_string();

        self.check_body_size(request, self.settings.info_max_body_bytes)?;

        // Record body size histogram
        self.metrics.record_histogram(
            "fcp_sip_info_body_bytes",
            request.body().len() as f64,
            &[("content_type", content_type.as_str())],
        );

        self.check_info_content_type(content_type.as_str())?;
        if self.check_rate_limit(
            "INFO",
            src_dialog_id.as_ref().unwrap_or(target_dialog.id()),
            self.settings.info_rate_limit,
        ) {
            warn!(call_id = %call.id, "INFO rate limited");
            let mut resp = self.create_simple_response(request, 429, "Too Many Requests");
            let _ = resp.headers_mut().push(
                SmolStr::new("Retry-After"),
                SmolStr::new(self.settings.rate_window.as_secs().to_string()),
            );
            return Ok(resp);
        }
        self.log_sip_message(
            "outbound",
            "INFO",
            Some(call.id.to_string().as_str()),
            Some(target_dialog.id()),
            cseq.as_ref().map(|v| v.as_str()),
            None,
            None,
            None,
            None,
            None,
        );
        let peer_resp = match uac
            .send_info(&mut target_dialog, content_type.as_str(), &body)
            .await
        {
            Ok(r) => {
                info!(status = r.start_line().code(), "Forwarded INFO to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "INFO"), ("result", "ok")],
                );
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "INFO"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward INFO to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "INFO"), ("result", "error")],
                );
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &self.subscription_created_at,
                    &call.id,
                    target_dialog.id(),
                    "INFO forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start_line().code() == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                &self.rate_limits,
                &self.mid_dialog_lock,
                &self.subscription_index,
                &self.subscription_created_at,
                &call.id,
                target_dialog.id(),
                "Peer returned 481 to INFO",
            );
            self.metrics.inc_counter(
                "fcp_sip_mid_dialog_forward_total",
                &[("method", "INFO"), ("result", "481")],
            );
        }
        self.persist_updated_dialog(&call.id, !is_a_leg, target_dialog);

        Ok(self.build_in_dialog_response(request, &peer_resp))
    }

    /// Handle in-dialog REFER by forwarding to the opposite leg.
    async fn handle_refer(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling REFER");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let cseq = sip_parse::header(request.headers(), "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id().clone())
        } else {
            call.b_leg
                .as_ref()
                .and_then(|b| b.dialog.as_ref().map(|d| d.id().clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "REFER",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id(),
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for REFER forwarding"))?;

        let refer_to_raw = sip_parse::header(request.headers(), "Refer-To")
            .ok_or_else(|| anyhow::anyhow!("Missing Refer-To"))?;
        let refer_uri_str = refer_to_raw
            .split('<')
            .nth(1)
            .and_then(|s| s.split('>').next())
            .unwrap_or(refer_to_raw.as_str())
            .trim();

        // A conference node draining asks for the caller to be moved to
        // another node (phase 5b): executed here, never relayed.
        if !is_a_leg && call.conference_room.is_some() {
            if let Some(migration) =
                crate::conference_migration::parse_migration_refer(refer_to_raw.as_str())
            {
                return Ok(self.start_conference_migration(request, &call, migration));
            }
        }

        // The B2BUA carries the transfer out itself
        // (`docs/ENTERPRISE_CALLING.md` §3.2), after the same policy checks
        // a relayed REFER passes.
        if self.b2bua_config.transfer.execute {
            if !self
                .b2bua_config
                .transfer
                .is_allowed(fcp_b2bua_core::TransferType::Blind)
                || !self.settings.allow_refer
            {
                warn!(call_id = %call.id, "REFER denied by the transfer policy");
                return Ok(self.create_simple_response(request, 403, "Refer Not Allowed"));
            }
            if self.check_rate_limit(
                "REFER",
                src_dialog_id.as_ref().unwrap_or(target_dialog.id()),
                self.b2bua_config.transfer.rate_limit_per_dialog,
            ) {
                warn!(call_id = %call.id, "REFER rate limited");
                return Ok(self.create_simple_response(request, 429, "Too Many Requests"));
            }
            return Ok(self
                .execute_refer(request, &call, is_a_leg, refer_to_raw.as_str())
                .await);
        }

        // The transferred caller's phone copies the Refer-To's URI headers
        // into its new INVITE (RFC 3261 §19.1.5), which is how the new call
        // learns the call it came from.
        let refer_with_origin =
            with_uri_header(refer_uri_str, TRANSFERRED_FROM_HEADER, &call.id.to_string());
        let refer_uri = sip_core::SipUri::parse(&refer_with_origin)
            .or_else(|_| sip_core::SipUri::parse(refer_uri_str))
            .map_err(|_| anyhow::anyhow!("Invalid Refer-To URI"))?;

        // B2BUA transfer policy check (primary gate)
        if !self
            .b2bua_config
            .transfer
            .is_allowed(fcp_b2bua_core::TransferType::Blind)
        {
            warn!(call_id = %call.id, "REFER denied by B2BUA transfer policy");
            return Ok(self.create_simple_response(request, 403, "Refer Not Allowed"));
        }
        // Legacy settings check (secondary gate for backward compatibility)
        if !self.settings.allow_refer {
            warn!(call_id = %call.id, "REFER not allowed by legacy settings");
            return Ok(self.create_simple_response(request, 403, "Refer Not Allowed"));
        }
        if self.check_rate_limit(
            "REFER",
            src_dialog_id.as_ref().unwrap_or(target_dialog.id()),
            self.b2bua_config.transfer.rate_limit_per_dialog,
        ) {
            warn!(call_id = %call.id, "REFER rate limited");
            let mut resp = self.create_simple_response(request, 429, "Too Many Requests");
            let _ = resp.headers_mut().push(
                SmolStr::new("Retry-After"),
                SmolStr::new(self.settings.rate_window.as_secs().to_string()),
            );
            return Ok(resp);
        }
        self.log_sip_message(
            "outbound",
            "REFER",
            Some(call.id.to_string().as_str()),
            Some(target_dialog.id()),
            cseq.as_ref().map(|v| v.as_str()),
            None,
            None,
            None,
            None,
            None,
        );
        let (peer_resp, _) = match uac.send_refer(&mut target_dialog, &refer_uri, None).await {
            Ok(r) => {
                info!(status = r.0.start_line().code(), "Forwarded REFER to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "REFER"), ("result", "ok")],
                );
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "REFER"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward REFER to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "REFER"), ("result", "error")],
                );
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &self.subscription_created_at,
                    &call.id,
                    target_dialog.id(),
                    "REFER forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start_line().code() == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                &self.rate_limits,
                &self.mid_dialog_lock,
                &self.subscription_index,
                &self.subscription_created_at,
                &call.id,
                target_dialog.id(),
                "Peer returned 481 to REFER",
            );
            self.metrics.inc_counter(
                "fcp_sip_mid_dialog_forward_total",
                &[("method", "REFER"), ("result", "481")],
            );
        }
        self.persist_updated_dialog(&call.id, !is_a_leg, target_dialog);

        Ok(self.build_in_dialog_response(request, &peer_resp))
    }

    /// Handle a REGISTER request for endpoint registration.
    ///
    /// If a registrar is configured, delegates to the registrar for processing.
    /// Otherwise, returns 501 Not Implemented.
    #[allow(dead_code)]
    async fn handle_register(&self, request: &sip_core::Request) -> sip_core::Response {
        // Default peer address for UDP (no flow needed)
        let default_peer: std::net::SocketAddr = "0.0.0.0:0".parse().unwrap();
        self.handle_register_internal(
            request,
            sip_transaction::TransportKind::Udp,
            None,
            default_peer,
        )
        .await
    }

    /// Handle a REGISTER request with flow token support (RFC 5626).
    ///
    /// For TLS/TCP connections with a stream, registers a flow token for
    /// connection reuse when routing requests back to NAT'd clients.
    async fn handle_register_internal(
        &self,
        request: &sip_core::Request,
        transport: sip_transaction::TransportKind,
        stream: Option<mpsc::Sender<Bytes>>,
        peer_addr: std::net::SocketAddr,
    ) -> sip_core::Response {
        debug!(transport = ?transport, has_stream = stream.is_some(), peer = %peer_addr, "Handling REGISTER request");

        // A flood is refused before any credential work, so it never
        // reaches the store (`docs/CAPACITY_AND_SURVIVABILITY.md` §5).
        //
        // A draining node also keeps its devices only briefly: the gate
        // shortens what it grants, so each one comes back soon and is
        // placed on a node that is serving (§6). A deregistration is left
        // exactly as it arrived.
        let aor = sip_parse::header(request.headers(), "To")
            .map(|to| {
                let to = to.as_str();
                to.split('<')
                    .nth(1)
                    .and_then(|s| s.split('>').next())
                    .unwrap_or(to)
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        let capped;
        let request = match crate::admission::register_gate(
            self.admission.as_ref(),
            request,
            peer_addr.ip(),
            &aor,
        ) {
            crate::admission::RegisterGate::Pass => request,
            crate::admission::RegisterGate::Shortened(shortened) => {
                capped = *shortened;
                &capped
            }
            crate::admission::RegisterGate::Refuse {
                refusal,
                retry_after_secs,
            } => {
                warn!(
                    aor = %aor,
                    source = %peer_addr,
                    reason = %refusal.as_str(),
                    "Refusing REGISTER: {}",
                    refusal.detail()
                );
                let mut response = self.create_simple_response(request, 503, "Service Unavailable");
                let _ = response.headers_mut().push(
                    SmolStr::new("Retry-After"),
                    SmolStr::new(retry_after_secs.to_string()),
                );
                return response;
            }
        };

        match &self.registrar {
            Some(registrar) => {
                // For TLS connections with a stream, register a flow token (RFC 5626)
                let flow_token = if transport == sip_transaction::TransportKind::Tls {
                    if let Some(ref stream_sender) = stream {
                        let token = self
                            .flow_registry
                            .register(stream_sender.clone(), peer_addr);
                        info!(
                            flow_token = %token,
                            aor = %sip_parse::header(request.headers(), "To")
                                .map(|s| s.as_str())
                                .unwrap_or(""),
                            "Registered TLS flow token for RFC 5626"
                        );
                        Some(token)
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Use registrar to handle the request with flow token, and
                // keep the address it came from with the binding: a device
                // that moves is placed by where it is (docs/E911_BLOCK_2.md
                // §6.2).
                // Whose binding the flow carries: what a call another node
                // relays down it is trusted for (`docs/PUSH.md` §2.4). Noted
                // before the REGISTER is handled, since the binding is
                // announced as it is stored and another node may relay a
                // call at once; the token is known to no other node until a
                // REGISTER stores it, and a challenged one is let go below.
                if let Some(token) = &flow_token {
                    let aor = sip_parse::header(request.headers(), "To")
                        .and_then(sip_parse::parse_to_header)
                        .map(|to| {
                            sip_registrar::normalize_aor(to.inner().uri())
                                .unwrap_or_else(|_| to.inner().uri().to_string())
                        });
                    if let (Some(aor), Some(tenant)) = (aor, registrar.caller_tenant(request).await)
                    {
                        self.flow_registry.set_owner(token, tenant, aor);
                    }
                }
                let response = registrar
                    .handle_register_from(request, flow_token.as_deref(), Some(peer_addr))
                    .await;

                // Log success/failure
                if response.start_line().code() == 200 {
                    info!(
                        aor = %sip_parse::header(request.headers(), "To")
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                        flow_token = ?flow_token,
                        "REGISTER handled successfully"
                    );
                    self.note_link_peer_registration(request, peer_addr, flow_token.as_deref());
                    if flow_token.is_some() {
                        // Each refresh leaves a token; the ones whose
                        // connection closed are let go here.
                        self.flow_registry.cleanup_stale();
                    }
                } else if response.start_line().code() == 401 {
                    debug!("REGISTER requires authentication");
                    // Remove flow token if auth failed (we'll create a new one on successful auth)
                    if let Some(ref token) = flow_token {
                        self.flow_registry.remove(token);
                    }
                }

                response
            }
            None => {
                debug!("No registrar configured, responding with 501 Not Implemented");
                self.create_simple_response(request, 501, "Not Implemented")
            }
        }
    }

    /// Create a simple SIP response with standard headers.
    /// Classify the dialed destination against the tenant's emergency
    /// numbers and, when it is one, resolve the caller's dispatchable
    /// location and callback number. Never fails: without storage the US
    /// defaults apply and the call goes out without a location.
    async fn detect_emergency(
        &self,
        tenant_id: &fcp_core::TenantId,
        request: &sip_core::Request,
        peer: SocketAddr,
        edge_caller: Option<&crate::emergency::EdgeCaller>,
    ) -> Option<fcp_b2bua_core::EmergencyCallInfo> {
        let destination = sip_parse::header(request.headers(), "To")
            .map(|to| {
                let to = to.as_str();
                to.split('<')
                    .nth(1)
                    .and_then(|s| s.split('>').next())
                    .unwrap_or(to)
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        let (username, aor) = self.emergency_caller(request, edge_caller);
        // Where the caller is: what a trusted edge vouched for, else the
        // INVITE's own source (§6.2); and the location the device sent, if
        // any (§6.3, used only from a device the tenant trusts).
        let client_address = self.trusted_edges.client_address(request, peer.ip());
        let supplied = crate::emergency::supplied_location(request);
        // A site's call: named so by a trusted edge (at the call manager),
        // or up a site's link (at the cloud SBC).
        let from_site = edge_caller.is_some_and(|e| e.is_site())
            || self
                .link_peers
                .from_peer(peer, &self.flow_registry)
                .is_some();
        // Boxed: every INVITE is classified here, and the location lookups'
        // future on the stack of a deep INVITE path overflowed a debug
        // build's worker (a site's call up its link, `docs/DEEP_TESTING.md`
        // D8).
        let mut info = Box::pin(crate::emergency::detect(
            self.storage.as_ref(),
            tenant_id,
            &destination,
            username.as_deref(),
            aor.as_deref(),
            client_address,
            supplied.as_ref(),
        ))
        .await?;
        // A site's call has no user or device here: it is placed where the
        // site, which knows its caller, says (`docs/HYBRID_CONTROL_PLANE.md`
        // §3.6), and called back on the number the site sent as its caller.
        if from_site && info.location.is_none() {
            info.location = crate::emergency::site_location(supplied, request, client_address);
            info.callback_number = info
                .location
                .as_ref()
                .and_then(|l| l.callback_number.clone());
        }
        Some(info)
    }

    /// Who an emergency call is from: the user a trusted edge named, else
    /// the From URI's user; with the `user@host` that picks their device.
    fn emergency_caller(
        &self,
        request: &sip_core::Request,
        edge_caller: Option<&crate::emergency::EdgeCaller>,
    ) -> (Option<String>, Option<String>) {
        let from = extract_from_uri(request).ok();
        match edge_caller {
            Some(edge) => (edge.username.clone(), None),
            None => (
                from.as_ref().and_then(|u| u.user().map(str::to_string)),
                from.as_ref()
                    .map(|u| format!("{}@{}", u.user().unwrap_or_default(), u.host())),
            ),
        }
    }

    /// Write the audit record and publish `call.emergency` (both best-effort).
    async fn record_emergency_call(
        &self,
        call_id: fcp_b2bua_core::CallId,
        tenant_id: fcp_core::TenantId,
        request: &sip_core::Request,
        info: &fcp_b2bua_core::EmergencyCallInfo,
        edge_caller: Option<&crate::emergency::EdgeCaller>,
    ) {
        let (caller_username, _) = self.emergency_caller(request, edge_caller);
        let caller_uri = match edge_caller {
            Some(edge) => format!(
                "sip:{}@{}",
                edge.username
                    .clone()
                    .or_else(|| edge.site_id.map(|s| s.to_string()))
                    .unwrap_or_default(),
                self.local_addr
            ),
            None => extract_from_uri(request)
                .map(|u| format!("sip:{}@{}", u.user().unwrap_or_default(), u.host()))
                .unwrap_or_default(),
        };
        let record = crate::emergency::placement_record(
            call_id,
            tenant_id,
            sip_parse::header(request.headers(), "Call-ID").map(|h| h.as_str().to_string()),
            caller_uri,
            caller_username,
            info,
        );
        crate::emergency::announce(&self.call_manager, call_id, record).await;
    }

    fn create_simple_response(
        &self,
        request: &sip_core::Request,
        code: u16,
        reason: &str,
    ) -> sip_core::Response {
        use sip_core::{Headers, StatusLine};

        let mut headers = Headers::new();

        // Copy mandatory headers from request (RFC 3261 §8.2.6): every
        // Via, or a response to a request that came through a proxy stops
        // at the proxy (D8: a carrier's caller heard nothing of a refusal).
        for via in request.headers().get_all("Via") {
            let _ = headers.push(SmolStr::new("Via"), via);
        }
        if let Some(from) = request.headers().get("From") {
            let _ = headers.push(SmolStr::new("From"), from);
        }
        if let Some(to) = request.headers().get("To") {
            let to_val = if !to.contains(";tag=") {
                let tag = Uuid::new_v4().simple().to_string();
                SmolStr::new(format!("{};tag={}", to, tag))
            } else {
                SmolStr::new(to)
            };
            let _ = headers.push(SmolStr::new("To"), to_val);
        }
        if let Some(call_id) = request.headers().get("Call-ID") {
            let _ = headers.push(SmolStr::new("Call-ID"), call_id);
        }
        if let Some(cseq) = request.headers().get("CSeq") {
            let _ = headers.push(SmolStr::new("CSeq"), cseq);
        }

        // Add Contact for INVITE/dialog forming responses if absent
        if request.method() == &sip_core::Method::Invite && headers.get("Contact").is_none() {
            let _ = headers.push(
                SmolStr::new("Contact"),
                SmolStr::new(format!("<{}>", self.uas.contact_uri.as_str())),
            );
        }

        // Content-Length: 0 (no body)
        let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        let mut resp = sip_core::Response::new(
            StatusLine::new(code, reason).expect("Failed to create status line"),
            headers,
            Bytes::new(),
        )
        .expect("Failed to create response");
        self.ensure_response_hygiene(request, &mut resp);
        self.log_sip_message(
            "outbound",
            &code.to_string(),
            sip_parse::header(request.headers(), "Call-ID").map(|s| s.as_str()),
            None,
            sip_parse::header(request.headers(), "CSeq").map(|s| s.as_str()),
            None,
            None,
            Some(resp.headers()),
            Some(resp.body()),
            None,
        );
        resp
    }

    /// Send BYE with a single retry/backoff and clean up dialog state on success.
    /// If flow_token is provided (RFC 5626), uses the existing TLS connection for BYE.
    fn spawn_bye_with_retry(
        &self,
        uac: Arc<SipUserAgentClient>,
        dialog: sip_dialog::Dialog,
        call_id: fcp_b2bua_core::CallId,
        is_a_leg: bool,
        flow_token: Option<String>,
    ) {
        self.spawn_bye_with_reason(uac, dialog, call_id, is_a_leg, flow_token, None)
    }

    /// As [`spawn_bye_with_retry`](Self::spawn_bye_with_retry), with an
    /// RFC 3326 `Reason` on the BYE: what the platform tells the far end
    /// when it, rather than a party, ends the call.
    fn spawn_bye_with_reason(
        &self,
        uac: Arc<SipUserAgentClient>,
        dialog: sip_dialog::Dialog,
        call_id: fcp_b2bua_core::CallId,
        is_a_leg: bool,
        flow_token: Option<String>,
        sip_reason: Option<String>,
    ) {
        let dialog_manager = Arc::clone(&self.dialog_manager);
        let call_manager = self.call_manager.clone();
        let stm = Arc::clone(&self.session_timer_manager);
        let subscription_index = Arc::clone(&self.subscription_index);
        let flow_registry = Arc::clone(&self.flow_registry);
        let sip_trace_tx = self.sip_trace_tx.clone();
        let local_addr = self.local_addr.clone();
        let node_id = self.node_id.clone();
        let recorder = Arc::clone(&self.call_recorder);
        let media_manager = self.media_manager.clone();
        tokio::spawn(async move {
            let mut attempts = 0;
            loop {
                attempts += 1;
                // Try to use flow token for RFC 5626 TLS connection reuse
                let bye_result = if let Some(ref token) = flow_token {
                    if let Some(flow_info) = flow_registry.lookup(token) {
                        tracing::info!(
                            call_id = %call_id,
                            flow_token = %token,
                            peer_addr = %flow_info.peer_addr,
                            "Sending BYE via RFC 5626 flow (TLS connection reuse)"
                        );
                        match &sip_reason {
                            Some(reason) => {
                                uac.bye_via_flow_with_reason(
                                    &dialog,
                                    flow_info.stream,
                                    flow_info.peer_addr,
                                    reason,
                                )
                                .await
                            }
                            None => {
                                uac.bye_via_flow(&dialog, flow_info.stream, flow_info.peer_addr)
                                    .await
                            }
                        }
                    } else {
                        tracing::warn!(
                            call_id = %call_id,
                            flow_token = %token,
                            "Flow token not found or connection closed, falling back to normal BYE"
                        );
                        uac.bye(&dialog).await
                    }
                } else {
                    match &sip_reason {
                        Some(reason) => uac.bye_with_reason(&dialog, reason).await,
                        None => uac.bye(&dialog).await,
                    }
                };
                match bye_result {
                    Ok(ref bye_response) => {
                        debug!(call_id = %call_id, attempt = attempts, "Successfully sent BYE to peer leg");

                        // Log outbound BYE and inbound 200 OK for SIP trace
                        if let Some(ref trace_tx) = sip_trace_tx {
                            let dest = dialog.remote_target().as_str().to_string();
                            let sip_cid = dialog.id().call_id().to_string();
                            let from_tag = dialog.id().local_tag();
                            let to_tag = dialog.id().remote_tag();
                            let from_uri = dialog.local_uri().as_str().to_string();
                            let to_uri = dialog.remote_uri().as_str().to_string();
                            let cid_str = call_id.to_string();
                            let cseq = format!("{} BYE", dialog.local_cseq() + 1);
                            let transport_str = if flow_token.is_some() { "TCP" } else { "UDP" };
                            let branch = format!(
                                "z9hG4bK{:x}",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_nanos()
                            );

                            // Build Route headers from dialog route set
                            let route_headers: String = dialog
                                .route_set()
                                .iter()
                                .map(|r| format!("Route: <{}>\r\n", r.as_str()))
                                .collect();

                            // Construct BYE matching what the UAC actually sends
                            let bye_raw = format!(
                                "BYE {} SIP/2.0\r\n\
                                 Via: SIP/2.0/{} {};branch={}\r\n\
                                 {}\
                                 Call-ID: {}\r\n\
                                 From: <{}>;tag={}\r\n\
                                 To: <{}>;tag={}\r\n\
                                 CSeq: {}\r\n\
                                 Max-Forwards: 70\r\n\
                                 User-Agent: FCP/0.18.1\r\n\
                                 Content-Length: 0\r\n\r\n",
                                dest,
                                transport_str,
                                local_addr,
                                branch,
                                route_headers,
                                sip_cid,
                                from_uri,
                                from_tag,
                                to_uri,
                                to_tag,
                                cseq,
                            );
                            let bye_trace = fcp_core::SipTraceMessage {
                                id: 0,
                                call_id: cid_str.clone(),
                                sip_call_id: sip_cid.clone(),
                                tenant_id: "_system".to_string(),
                                timestamp: std::time::SystemTime::now(),
                                direction: "outbound".to_string(),
                                method_or_status: "BYE".to_string(),
                                cseq: cseq.clone(),
                                from_uri: format!("<{}>;tag={}", from_uri, from_tag),
                                to_uri: format!("<{}>;tag={}", to_uri, to_tag),
                                source_addr: local_addr.clone(),
                                dest_addr: dest.clone(),
                                transport: transport_str.to_lowercase(),
                                raw_message: bye_raw,
                                node_id: Some(node_id.clone()),
                            };
                            let _ = trace_tx.try_send(bye_trace);

                            // Log the 200 OK response to BYE
                            let raw_200_bytes = sip_parse::serialize_response(bye_response);
                            let raw_200 = String::from_utf8_lossy(&raw_200_bytes);
                            let resp_trace = fcp_core::SipTraceMessage {
                                id: 0,
                                call_id: cid_str,
                                sip_call_id: sip_cid,
                                tenant_id: "_system".to_string(),
                                timestamp: std::time::SystemTime::now(),
                                direction: "inbound".to_string(),
                                method_or_status: bye_response.code().to_string(),
                                cseq: sip_parse::header(bye_response.headers(), "CSeq")
                                    .map(|s| s.as_str().to_string())
                                    .unwrap_or_default(),
                                from_uri: sip_parse::header(bye_response.headers(), "From")
                                    .map(|s| s.as_str().to_string())
                                    .unwrap_or_default(),
                                to_uri: sip_parse::header(bye_response.headers(), "To")
                                    .map(|s| s.as_str().to_string())
                                    .unwrap_or_default(),
                                source_addr: dest,
                                dest_addr: local_addr.clone(),
                                transport: if flow_token.is_some() {
                                    "tcp".to_string()
                                } else {
                                    "udp".to_string()
                                },
                                raw_message: raw_200.to_string(),
                                node_id: Some(node_id.clone()),
                            };
                            let _ = trace_tx.try_send(resp_trace);
                        }

                        stm.stop_timer(dialog.id());
                        dialog_manager.remove(dialog.id());

                        // Remove dialog from CallManager leg and clean subscriptions
                        let _ = call_manager.update_call(&call_id, |call| {
                            if is_a_leg {
                                call.a_leg.dialog = None;
                            } else if let Some(b) = &mut call.b_leg {
                                b.dialog = None;
                            }
                        });

                        // Clean subscriptions tied to this dialog id in index
                        let dialog_id = dialog.id().clone();
                        let keys: Vec<_> = subscription_index
                            .iter()
                            .filter(|entry| entry.key().0 == dialog_id)
                            .map(|entry| entry.key().clone())
                            .collect();
                        for key in keys {
                            subscription_index.remove(&key);
                        }

                        // Check if both legs are now done, and if so, terminate the call
                        if let Some(updated_call) = call_manager.get_call(&call_id) {
                            let a_done = updated_call.a_leg.dialog.is_none();
                            let b_done = updated_call
                                .b_leg
                                .as_ref()
                                .map(|b| b.dialog.is_none())
                                .unwrap_or(true);

                            if a_done && b_done {
                                info!(call_id = %call_id, "Both legs terminated after forwarded BYE, terminating call");
                                recorder.call_ended(&call_id).await;
                                // The record is written by this call, so
                                // the media session's counters have to be
                                // on the call before it is: the leg that
                                // received the BYE asked for them too, but
                                // on a task of its own that this one can
                                // beat.
                                let media = anchored_media(&updated_call, media_manager.as_ref());
                                close_media(&call_manager, &call_id, media, true, "forwarded BYE")
                                    .await;
                                let _ = call_manager.terminate_call(&call_id, None);
                            }
                        }

                        break;
                    }
                    Err(e) => {
                        if attempts >= 2 {
                            error!(call_id = %call_id, error = %e, "Failed to send BYE after retry");
                            // Clean up dialog even on failure to prevent memory leak
                            dialog_manager.remove(dialog.id());
                            let _ = call_manager.update_call(&call_id, |call| {
                                if is_a_leg {
                                    call.a_leg.dialog = None;
                                } else if let Some(b) = &mut call.b_leg {
                                    b.dialog = None;
                                }
                            });
                            // Clean subscription_index entries for this dialog (error path)
                            let dialog_id = dialog.id().clone();
                            let keys: Vec<_> = subscription_index
                                .iter()
                                .filter(|entry| entry.key().0 == dialog_id)
                                .map(|entry| entry.key().clone())
                                .collect();
                            for key in keys {
                                subscription_index.remove(&key);
                            }

                            // Check if both legs are now done — terminate the call
                            // so that CallEvent::Ended fires and presence is cleared.
                            if let Some(updated_call) = call_manager.get_call(&call_id) {
                                let a_done = updated_call.a_leg.dialog.is_none();
                                let b_done = updated_call
                                    .b_leg
                                    .as_ref()
                                    .map(|b| b.dialog.is_none())
                                    .unwrap_or(true);

                                if a_done && b_done {
                                    info!(call_id = %call_id, "Both legs terminated after BYE failure, terminating call");
                                    recorder.call_ended(&call_id).await;
                                    let _ = call_manager.terminate_call(&call_id, None);
                                }
                            }

                            break;
                        } else {
                            warn!(call_id = %call_id, error = %e, "BYE send failed, retrying once");
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        }
                    }
                }
            }
        });
    }

    /// Create a 487 response for a cancelled INVITE with proper To-tag and Contact.
    ///
    /// # Arguments
    /// * `invite_req` - The original INVITE request
    /// * `saved_tag` - Optional To-tag from earlier provisional responses (RFC 3261 §12.1.1)
    fn create_cancelled_response(
        &self,
        invite_req: &sip_core::Request,
        saved_tag: Option<&str>,
    ) -> sip_core::Response {
        let mut response =
            sip_uas::UserAgentServer::create_response(invite_req, 487, "Request Terminated");

        // Ensure Contact header is present
        if response.headers().get("Contact").is_none() {
            let _ = response.headers_mut().push(
                SmolStr::new("Contact"),
                SmolStr::new(format!("<{}>", self.uas.contact_uri.as_str())),
            );
        }

        // Copy Allow/Supported if present on the INVITE so capabilities are preserved
        for name in ["Allow", "Supported"] {
            if let Some(val) = invite_req.headers().get(name) {
                let _ = response.headers_mut().push(SmolStr::new(name), val);
            }
        }

        // Ensure To header has a tag; use saved tag or generate new one
        if let Some(to) = response.headers().get("To") {
            if !to.contains(";tag=") {
                let tag = saved_tag
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
                let new_to = SmolStr::new(format!("{};tag={}", to, tag));

                // Reconstruct headers with updated To
                let mut new_headers = sip_core::Headers::new();
                for header in response.headers().iter() {
                    if header.name_smol() == "To" {
                        let _ = new_headers.push(SmolStr::new("To"), new_to.clone());
                    } else {
                        let _ = new_headers
                            .push(header.name_smol().clone(), header.value_smol().clone());
                    }
                }

                // Reconstruct response
                response = sip_core::Response::new(
                    response.start_line().clone(),
                    new_headers,
                    response.body().clone(),
                )
                .expect("Failed to reconstruct response");
            }
        }

        response
    }

    /// Build a provisional response to A-leg using the B-leg provisional as a template.
    fn create_provisional_from_b_leg(
        &self,
        a_leg_invite: &sip_core::Request,
        b_leg_resp: &sip_core::Response,
    ) -> sip_core::Response {
        use sip_core::{Headers, StatusLine};

        let mut headers = Headers::new();

        // Mandatory dialog-forming headers from A-leg INVITE: every Via,
        // or a caller behind a proxy (a carrier) never hears it ring.
        for via in a_leg_invite.headers().get_all("Via") {
            let _ = headers.push(SmolStr::new("Via"), via);
        }
        if let Some(from) = a_leg_invite.headers().get("From") {
            let _ = headers.push(SmolStr::new("From"), from);
        }
        if let Some(to) = a_leg_invite.headers().get("To") {
            // Preserve the To-tag from B-leg provisional if present
            let _to_smol = SmolStr::new(to);
            let to_with_tag = if let Some(to_header) = b_leg_resp.headers().get("To") {
                let to_header_smol = SmolStr::new(to_header);
                if let Some(tag) = extract_tag_value(Some(&to_header_smol)) {
                    format!("{};tag={}", to, tag)
                } else {
                    to.to_string()
                }
            } else {
                to.to_string()
            };
            let _ = headers.push(SmolStr::new("To"), SmolStr::new(to_with_tag));
        }
        if let Some(call_id) = a_leg_invite.headers().get("Call-ID") {
            let _ = headers.push(SmolStr::new("Call-ID"), call_id);
        }
        if let Some(cseq) = a_leg_invite.headers().get("CSeq") {
            let _ = headers.push(SmolStr::new("CSeq"), cseq);
        }

        // The A-leg's dialog is with this node, as the 200 OK says: our
        // Contact, not the callee's (which named a phone on its own network
        // to the carrier), and the A-leg INVITE's Record-Route echoed
        // (RFC 3261 §12.1.1), not the B-leg's.
        let _ = headers.push(
            SmolStr::new("Contact"),
            SmolStr::new(format!("<{}>", self.uas.contact_uri.as_str())),
        );
        for rr in a_leg_invite.headers().get_all("Record-Route") {
            let _ = headers.push(SmolStr::new("Record-Route"), rr);
        }

        // Propagate capability/signaling headers. A reliable provisional
        // was PRACKed by the B-leg's own INVITE transaction and reaches the
        // A-leg as an ordinary one: no RSeq, no `Require: 100rel`
        // (`handle_prack`).
        for name in [
            "Require",
            "Supported",
            "Allow",
            "Session-Expires",
            "Content-Type",
        ] {
            for val in b_leg_resp.headers().get_all(name) {
                let val = if name == "Require" {
                    let rest: Vec<&str> = val
                        .split(',')
                        .map(str::trim)
                        .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("100rel"))
                        .collect();
                    if rest.is_empty() {
                        continue;
                    }
                    SmolStr::new(rest.join(", "))
                } else {
                    SmolStr::new(val)
                };
                let _ = headers.push(SmolStr::new(name), val);
            }
        }

        // Body (e.g., SDP for early media)
        let body = b_leg_resp.body().clone();
        let _ = headers.push(
            SmolStr::new("Content-Length"),
            SmolStr::new(body.len().to_string()),
        );

        let mut response = sip_core::Response::new(
            StatusLine::new(
                b_leg_resp.start_line().code(),
                b_leg_resp.start_line().reason(),
            )
            .expect("Failed to create status line"),
            headers,
            body,
        )
        .expect("Failed to create response");

        self.apply_topology_hiding(&mut response);
        response
    }

    /// Build an in-dialog response for the requester based on the peer response.
    fn build_in_dialog_response(
        &self,
        request: &sip_core::Request,
        peer_response: &sip_core::Response,
    ) -> sip_core::Response {
        let mut response = if peer_response.start_line().code() == 200 {
            // Use UAS helper to ensure Contact/To-tag are present
            let body = if peer_response.body().is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(peer_response.body()).to_string())
            };
            self.uas
                .create_ok(request, body.as_deref())
                .expect("Failed to create OK response")
        } else {
            let r = self.create_simple_response(
                request,
                peer_response.start_line().code(),
                peer_response.start_line().reason(),
            );
            // If peer has a body, need to reconstruct response with it
            if !peer_response.body().is_empty() {
                let mut headers = sip_core::Headers::new();
                for header in r.headers().iter() {
                    let _ = headers.push(header.name_smol().clone(), header.value_smol().clone());
                }
                if let Some(ct) = peer_response.headers().get("Content-Type") {
                    let _ = headers.push(SmolStr::new("Content-Type"), ct);
                }
                let body = peer_response.body().clone();
                let _ = headers.push(
                    SmolStr::new("Content-Length"),
                    SmolStr::new(body.len().to_string()),
                );

                sip_core::Response::new(r.start_line().clone(), headers, body)
                    .expect("Failed to create response with body")
            } else {
                r
            }
        };

        // Propagate capability headers from peer
        for name in ["Supported", "Allow", "Session-Expires", "Require"] {
            for val in peer_response.headers().get_all(name) {
                let _ = response.headers_mut().push(SmolStr::new(name), val);
            }
        }
        // Propagate Record-Route/Contact from peer if present
        for val in peer_response.headers().get_all("Record-Route") {
            let _ = response
                .headers_mut()
                .push(SmolStr::new("Record-Route"), val);
        }
        if response.headers().get("Contact").is_none() {
            if let Some(ct) = peer_response.headers().get("Contact") {
                let _ = response.headers_mut().push(SmolStr::new("Contact"), ct);
            }
        }

        // Ensure Content-Length matches body
        let body_len = response.body().len();
        let _ = response.headers_mut().push(
            SmolStr::new("Content-Length"),
            SmolStr::new(body_len.to_string()),
        );

        self.apply_topology_hiding(&mut response);
        self.ensure_response_hygiene(request, &mut response);
        self.log_sip_message(
            "outbound",
            &response.start_line().code().to_string(),
            sip_parse::header(request.headers(), "Call-ID").map(|s| s.as_str()),
            None,
            sip_parse::header(request.headers(), "CSeq").map(|s| s.as_str()),
            None,
            None,
            Some(response.headers()),
            Some(response.body()),
            None,
        );

        response
    }

    /// Check if this INVITE is a re-INVITE on a B-leg dialog.
    ///
    /// `dialog_manager.find_by_request()` only works for UAS (A-leg) dialogs
    /// because it swaps From/To tags assuming UAS perspective. For UAC (B-leg)
    /// dialogs, we fall back to checking call_manager by Call-ID + verifying
    /// a To-tag exists (which indicates an in-dialog request per RFC 3261).
    fn is_b_leg_reinvite(&self, request: &sip_core::Request) -> bool {
        // A re-INVITE must have a To tag (in-dialog indicator per RFC 3261)
        let to_header = match sip_parse::header(request.headers(), "To") {
            Some(h) => h,
            None => return false,
        };
        if !to_header.contains("tag=") {
            return false;
        }

        // Check if call_manager has a call with this Call-ID
        let call_id = match sip_parse::header(request.headers(), "Call-ID") {
            Some(h) => h,
            None => return false,
        };

        self.call_for_request(request, call_id.as_str()).is_some()
    }

    /// The call an in-dialog request belongs to, by its Call-ID and tags
    /// (`CallManager::find_by_request_dialog`).
    fn call_for_request(
        &self,
        request: &sip_core::Request,
        dialog_call_id: &str,
    ) -> Option<fcp_b2bua_core::Call> {
        let tag = |name: &str| {
            sip_parse::header(request.headers(), name).and_then(|h| {
                h.as_str()
                    .split(';')
                    .find_map(|p| p.trim().strip_prefix("tag=").map(str::to_string))
            })
        };
        self.call_manager.find_by_request_dialog(
            dialog_call_id,
            tag("To").as_deref(),
            tag("From").as_deref(),
        )
    }

    /// Find call and which leg the request belongs to.
    fn lookup_call_for_in_dialog(
        &self,
        request: &sip_core::Request,
    ) -> Result<(fcp_b2bua_core::Call, bool)> {
        let dialog_call_id = sip_parse::header(request.headers(), "Call-ID")
            .ok_or_else(|| anyhow::anyhow!("Missing Call-ID"))?;

        let call = self
            .call_for_request(request, dialog_call_id.as_str())
            .ok_or(NoSuchDialog)?;

        let is_a_leg = call
            .a_leg
            .dialog
            .as_ref()
            .map(|d| d.id().call_id() == dialog_call_id.as_str())
            .unwrap_or(false);

        Ok((call, is_a_leg))
    }

    /// Get the opposite dialog (target leg) for forwarding.
    fn get_opposite_dialog<'a>(
        &self,
        call: &'a fcp_b2bua_core::Call,
        is_a_leg: bool,
    ) -> Result<&'a sip_dialog::Dialog> {
        if is_a_leg {
            call.b_leg
                .as_ref()
                .and_then(|b| b.dialog.as_ref())
                .ok_or_else(|| anyhow::anyhow!("No B-leg dialog available"))
        } else {
            call.a_leg
                .dialog
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No A-leg dialog available"))
        }
    }

    /// Persist updated dialog to CallManager and dialog_manager.
    fn persist_updated_dialog(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        is_a_leg: bool,
        dialog: sip_dialog::Dialog,
    ) {
        let dialog_clone = dialog.clone();
        if let Err(e) = self.call_manager.update_call(call_id, |call| {
            if is_a_leg {
                call.a_leg.dialog = Some(dialog.clone());
            } else if let Some(b) = &mut call.b_leg {
                b.dialog = Some(dialog.clone());
            }
        }) {
            warn!(call_id = %call_id, error = %e, "Failed to persist updated dialog");
        }

        let _ = self.dialog_manager.insert(dialog_clone);
    }

    #[allow(clippy::too_many_arguments)]
    fn log_sip_message(
        &self,
        direction: &str,
        method_or_status: &str,
        call_id: Option<&str>,
        dialog_id: Option<&sip_dialog::DialogId>,
        cseq: Option<&str>,
        peer: Option<SocketAddr>,
        transport: Option<&str>,
        headers: Option<&sip_core::Headers>,
        body: Option<&[u8]>,
        raw_message: Option<&str>,
    ) {
        debug!(
            direction = direction,
            method_or_status = method_or_status,
            call_id = call_id.unwrap_or("unknown"),
            dialog_id = ?dialog_id,
            cseq = cseq.unwrap_or(""),
            peer = ?peer,
            transport = transport.unwrap_or(""),
            "SIP message trace"
        );
        let mut labels = vec![("direction", direction), ("method", method_or_status)];
        if let Some(id) = dialog_id {
            labels.push(("dialog_id", id.call_id()));
        }
        self.metrics.inc_counter("fcp_sip_message_total", &labels);

        // Send to SIP trace channel for PostgreSQL storage
        if let (Some(ref trace_tx), Some(cid)) = (&self.sip_trace_tx, call_id) {
            let from_uri = headers
                .and_then(|h| h.get("From"))
                .map(|v| extract_uri_from_header(v).to_string())
                .unwrap_or_default();
            let to_uri = headers
                .and_then(|h| h.get("To"))
                .map(|v| extract_uri_from_header(v).to_string())
                .unwrap_or_default();
            let sip_call_id = headers
                .and_then(|h| h.get("Call-ID"))
                .map(|v| v.to_string())
                .unwrap_or_default();

            let (source_addr, dest_addr) = match direction {
                "inbound" => (
                    peer.map(|p| p.to_string()).unwrap_or_default(),
                    self.local_addr.clone(),
                ),
                _ => (
                    self.local_addr.clone(),
                    peer.map(|p| p.to_string()).unwrap_or_default(),
                ),
            };

            let trace = fcp_core::SipTraceMessage {
                id: 0,
                call_id: cid.to_string(),
                sip_call_id,
                tenant_id: "_system".to_string(),
                timestamp: std::time::SystemTime::now(),
                direction: direction.to_string(),
                method_or_status: method_or_status.to_string(),
                cseq: cseq.unwrap_or("").to_string(),
                from_uri,
                to_uri,
                source_addr,
                dest_addr,
                transport: transport.unwrap_or("udp").to_string(),
                raw_message: raw_message.unwrap_or("").to_string(),
                node_id: Some(self.node_id.clone()),
            };

            if let Err(e) = trace_tx.try_send(trace) {
                if matches!(e, tokio::sync::mpsc::error::TrySendError::Full(_)) {
                    tracing::warn!(
                        call_id = %cid,
                        "SIP trace channel full, dropping entry"
                    );
                }
            }
        }

        if let (Some(ref tx), Some(cid)) = (&self.transcript_tx, call_id) {
            let body_hash = if self.settings.transcript_capture_bodies {
                body.map(|b| {
                    let mut hasher = Sha256::new();
                    hasher.update(b);
                    format!("{:x}", hasher.finalize())
                })
            } else {
                None
            };

            let record_route = headers
                .map(|h| {
                    let vals: Vec<_> = h.get_all("Record-Route").collect();
                    vals.to_vec().join(",")
                })
                .unwrap_or_default();
            let route = headers
                .map(|h| {
                    let vals: Vec<_> = h.get_all("Route").collect();
                    vals.to_vec().join(",")
                })
                .unwrap_or_default();

            let entry = TranscriptEntry {
                call_id: cid.to_string(),
                timestamp: std::time::SystemTime::now(),
                direction: direction.to_string(),
                method_or_status: method_or_status.to_string(),
                dialog_id_str: dialog_id
                    .map(|d| format!("{:?}", d))
                    .unwrap_or_else(|| "none".to_string()),
                cseq: cseq.unwrap_or("").to_string(),
                peer: peer.map(|p| p.to_string()),
                transport: transport.map(|t| t.to_string()),
                record_route,
                route,
                body_hash,
            };

            // Use try_send for non-blocking send (drops if channel full)
            // Transcripts are debug/forensic data, not critical path
            if let Err(e) = tx.try_send(entry) {
                if matches!(e, tokio::sync::mpsc::error::TrySendError::Full(_)) {
                    tracing::warn!(
                        call_id = %cid,
                        "Transcript channel full, dropping entry (system under high load)"
                    );
                }
                // Channel closed: silently ignore (server shutting down)
            }
        }
    }

    fn check_body_size(&self, request: &sip_core::Request, max_bytes: usize) -> Result<()> {
        if request.body().len() > max_bytes {
            return Err(anyhow::anyhow!("Body too large"));
        }
        Ok(())
    }

    /// Parse DTMF digit from SIP INFO body (RFC 4733 format).
    ///
    /// Expected format:
    /// ```text
    /// Signal=1
    /// Duration=160
    /// ```
    fn parse_dtmf_from_info_body(&self, body: &str) -> Option<char> {
        for line in body.lines() {
            let line = line.trim();
            if let Some(signal_value) = line.strip_prefix("Signal=") {
                // Extract the digit (0-9, *, #, A-D)
                let digit_str = signal_value.trim();
                if let Some(first_char) = digit_str.chars().next() {
                    if first_char.is_ascii_digit() || matches!(first_char, '*' | '#' | 'A'..='D') {
                        return Some(first_char);
                    }
                }
            }
        }
        None
    }

    fn check_info_content_type(&self, content_type: &str) -> Result<()> {
        let ct = content_type.to_lowercase();
        if self
            .settings
            .allow_info_content_types
            .iter()
            .any(|v| v.to_lowercase() == ct)
        {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Unsupported INFO content-type"))
        }
    }

    fn check_rate_limit(
        &self,
        method: &'static str,
        dialog_id: &sip_dialog::DialogId,
        limit: u32,
    ) -> bool {
        if limit == 0 {
            return false;
        }

        let mut entry = self
            .rate_limits
            .entry(dialog_id.clone())
            .or_insert_with(|| RateWindow {
                window_start: Instant::now(),
                counts: HashMap::new(),
            });

        let now = Instant::now();
        if now.duration_since(entry.window_start) > self.settings.rate_window {
            entry.window_start = now;
            entry.counts.clear();
        }

        let count = entry.counts.entry(method).or_insert(0);
        if *count >= limit {
            return true;
        }
        *count += 1;

        // Record rate limit utilization as histogram (percentage)
        let utilization_pct = (*count as f64 / limit as f64) * 100.0;
        self.metrics.record_histogram(
            "fcp_sip_rate_limit_utilization_percent",
            utilization_pct,
            &[("method", method)],
        );

        false
    }

    /// Acquire per-dialog lock to serialize mid-dialog requests.
    async fn acquire_mid_dialog_lock(
        &self,
        dialog_id: &sip_dialog::DialogId,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        use dashmap::mapref::entry::Entry;
        let entry = self.mid_dialog_lock.entry(dialog_id.clone());
        let arc_mutex: Arc<tokio::sync::Mutex<()>> = match entry {
            Entry::Occupied(o) => Arc::clone(o.get()),
            Entry::Vacant(v) => v.insert(Arc::new(tokio::sync::Mutex::new(()))).clone(),
        };
        arc_mutex.clone().lock_owned().await
    }

    fn spawn_session_timer_listener(&self) {
        let stm = Arc::clone(&self.session_timer_manager);
        let cm = self.call_manager.clone();
        let dm = self.dialog_manager.clone();
        let uac = self.uac.clone();
        let serving = Arc::clone(&self.serving);
        tokio::spawn(async move {
            let mut rx = stm.subscribe().await;
            while let Some(evt) = rx.recv().await {
                match evt {
                    SessionTimerEvent::RefreshNeeded(dialog_id) => {
                        if let Err(e) =
                            handle_refresh_needed(&cm, &dm, &stm, uac.clone(), dialog_id.clone())
                                .await
                        {
                            warn!(error = %e, dialog_id = ?dialog_id, "Session refresh failed");
                        }
                    }
                    SessionTimerEvent::SessionExpired(dialog_id) => {
                        if let Err(e) = handle_session_expired(
                            &cm,
                            &dm,
                            &stm,
                            uac.clone(),
                            serving.get(),
                            dialog_id.clone(),
                        )
                        .await
                        {
                            warn!(error = %e, dialog_id = ?dialog_id, "Session expired handling failed");
                        }
                    }
                }
            }
        });
    }

    /// Spawns a background task to monitor session timer health.
    ///
    /// This task periodically checks the session timer manager state and logs
    /// statistics for observability. It also validates that active timers match
    /// active dialogs to detect potential leaks.
    fn spawn_session_timer_monitor(&self) {
        let stm = Arc::clone(&self.session_timer_manager);
        let dm = Arc::clone(&self.dialog_manager);
        let cm = self.call_manager.clone();
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;

                let timer_count = stm.active_count();
                let dialog_count = dm.count();
                let call_count = cm.active_call_count();

                // Log health metrics
                debug!(
                    active_session_timers = timer_count,
                    active_dialogs = dialog_count,
                    active_calls = call_count,
                    "Session timer health check"
                );

                // Update metrics
                metrics.inc_counter("fcp_sip_session_timer_health_check", &[]);

                // Record gauge metrics for monitoring
                metrics.set_gauge("fcp_sip_active_session_timers", timer_count as f64, &[]);
                metrics.set_gauge("fcp_sip_active_dialogs", dialog_count as f64, &[]);
                metrics.set_gauge("fcp_sip_active_calls", call_count as f64, &[]);

                // Warn if timer count significantly differs from dialog count
                // (some dialogs may not have session timers, so this is informational)
                if timer_count > dialog_count + 10 {
                    warn!(
                        timer_count = timer_count,
                        dialog_count = dialog_count,
                        diff = timer_count - dialog_count,
                        "Session timer count exceeds dialog count - possible leak"
                    );
                }
            }
        });
    }

    fn spawn_subscription_expiry_monitor(&self) {
        let sub_mgr = Arc::clone(&self.subscription_manager);
        let sub_index = Arc::clone(&self.subscription_index);
        let sub_created = Arc::clone(&self.subscription_created_at);
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;

                let now = Instant::now();
                let mut expired_count: usize = 0;

                // Check each tracked subscription for expiry
                let entries: Vec<_> = sub_created
                    .iter()
                    .map(|e| (e.key().clone(), *e.value()))
                    .collect();

                for (sub_id, created_at) in entries {
                    let expired = if let Some(sub) = sub_mgr.get(&sub_id) {
                        now.duration_since(created_at) > sub.expires()
                    } else {
                        // Subscription already removed from manager; clean up timestamp
                        true
                    };

                    if expired {
                        sub_mgr.remove(&sub_id);
                        sub_created.remove(&sub_id);

                        // Remove matching subscription_index entries
                        let keys: Vec<_> = sub_index
                            .iter()
                            .filter(|entry| *entry.value() == sub_id)
                            .map(|entry| entry.key().clone())
                            .collect();
                        for key in keys {
                            sub_index.remove(&key);
                        }

                        expired_count += 1;
                    }
                }

                // Also clean up any terminated subscriptions
                let terminated_count = sub_mgr.cleanup_terminated();

                if expired_count > 0 || terminated_count > 0 {
                    info!(
                        expired = expired_count,
                        terminated_cleaned = terminated_count,
                        "Subscription expiry monitor cleanup"
                    );
                }

                let active = sub_mgr.all().len();
                metrics.set_gauge("fcp_sip_active_subscriptions", active as f64, &[]);
            }
        });
    }

    fn find_subscription(
        &self,
        dialog_id: &sip_dialog::DialogId,
        event: &str,
    ) -> Option<sip_dialog::Subscription> {
        let key = (dialog_id.clone(), smol_str::SmolStr::new(event));
        if let Some(id) = self.subscription_index.get(&key) {
            return self.subscription_manager.get(&id);
        }
        // Fallback linear scan if index missed
        self.subscription_manager.all().into_iter().find(|s| {
            s.id().call_id() == dialog_id.call_id()
                && s.id().from_tag() == dialog_id.local_tag()
                && s.id().to_tag() == dialog_id.remote_tag()
                && s.id().event() == event
        })
    }

    fn index_subscription(&self, sub: sip_dialog::Subscription) {
        let key = (
            sip_dialog::DialogId::new(sub.id().call_id(), sub.id().from_tag(), sub.id().to_tag())
                .expect("Failed to create DialogId from subscription"),
            smol_str::SmolStr::new(sub.id().event()),
        );
        self.subscription_created_at
            .insert(sub.id().clone(), Instant::now());
        let _ = self.subscription_manager.insert(sub.clone());
        let _ = self.subscription_index.insert(key, sub.id().clone());
    }

    /// Remember that a call's B-leg is a conference node hosting `room`.
    fn mark_conference_call(&self, call_id: &fcp_b2bua_core::CallId, room: &str, host: &str) {
        let _ = self.call_manager.update_call(call_id, |c| {
            c.conference_room = Some(room.to_string());
            c.conference_node = Some(host.to_string());
        });
    }

    /// A REFER with a migration ticket on a conference call's B-leg
    /// (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5b): answer it at once
    /// and run the move in the background — the REFER's own transaction
    /// must not wait on an INVITE to another node.
    fn start_conference_migration(
        &self,
        request: &sip_core::Request,
        call: &fcp_b2bua_core::Call,
        migration: crate::conference_migration::MigrationRefer,
    ) -> sip_core::Response {
        if !self.b2bua_config.conference_migration {
            info!(call_id = %call.id, "Conference migration is off; refusing the REFER");
            return self.create_simple_response(request, 603, "Decline");
        }
        let Some(uac) = self.uac.clone() else {
            return self.create_simple_response(request, 500, "No UAC Configured");
        };
        let Some(old_b_dialog) = call.b_leg.as_ref().and_then(|b| b.dialog.clone()) else {
            return self.create_simple_response(request, 481, "Call/Transaction Does Not Exist");
        };
        if call.a_leg.dialog.is_none() {
            return self.create_simple_response(request, 481, "Call/Transaction Does Not Exist");
        }
        if self.conference_migrations.insert(call.id, ()).is_some() {
            return self.create_simple_response(request, 491, "Request Pending");
        }
        info!(
            call_id = %call.id,
            room = ?call.conference_room,
            to = %migration.target.as_str(),
            anchored = call.media_anchored,
            "Moving the conference leg to another node"
        );
        let ctx = ConferenceMigration {
            uac,
            local_uri: self.uas.local_uri.clone(),
            call_manager: self.call_manager.clone(),
            dialog_manager: Arc::clone(&self.dialog_manager),
            session_timer_manager: Arc::clone(&self.session_timer_manager),
            media_manager: self.media_manager.clone(),
            sip_call_id_map: Arc::clone(&self.sip_call_id_map),
            in_flight: Arc::clone(&self.conference_migrations),
            b2bua_config: self.b2bua_config.clone(),
            local_addr: self.local_addr.clone(),
            call_id: call.id,
            old_b_dialog,
            target: migration.target,
            ticket: Some(migration.ticket),
            old_node_gone: false,
        };
        tokio::spawn(ctx.run());
        self.create_simple_response(request, 202, "Accepted")
    }

    fn dialog_is_refresher(dialog: &sip_dialog::Dialog) -> bool {
        dialog
            .refresher()
            .map(|r| {
                (r == sip_core::RefresherRole::Uac && dialog.is_uac())
                    || (r == sip_core::RefresherRole::Uas && !dialog.is_uac())
            })
            .unwrap_or(false)
    }

    fn start_session_timer(&self, dialog: &sip_dialog::Dialog) {
        start_session_timer_for(&self.session_timer_manager, &self.b2bua_config, dialog);
    }

    fn create_transient_subscription(
        &self,
        _dialog: &sip_dialog::Dialog,
        _event: &str,
    ) -> Option<sip_dialog::Subscription> {
        // Transient subscriptions are intentionally not supported.
        //
        // Per RFC 3265, NOTIFY requests must correspond to an established subscription
        // created via SUBSCRIBE. Subscription objects in siphon-rs must be created
        // from actual SUBSCRIBE Request/Response pairs using Subscription::new_notifier()
        // or Subscription::new_subscriber().
        //
        // Returning None here causes NOTIFY requests without proper subscriptions to be
        // rejected with "481 Subscription Does Not Exist", which is the correct behavior.
        None
    }

    // ============================================================================
    // PBX Feature Handlers
    // ============================================================================

    /// Dispatch a PBX feature code action.
    async fn handle_pbx_feature_code(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        action: fcp_pbx::FeatureCodeAction,
    ) -> Result<()> {
        match action {
            fcp_pbx::FeatureCodeAction::GroupPickup => {
                self.handle_group_pickup(request, handle, call_id).await
            }
            fcp_pbx::FeatureCodeAction::DirectedPickup { extension } => {
                self.handle_directed_pickup(request, handle, call_id, &extension)
                    .await
            }
            fcp_pbx::FeatureCodeAction::ParkCall => {
                self.handle_park_call(request, handle, call_id).await
            }
            fcp_pbx::FeatureCodeAction::RetrieveParkedCall { slot } => {
                self.handle_retrieve_parked(request, handle, call_id, &slot)
                    .await
            }
            fcp_pbx::FeatureCodeAction::IntercomOneWay { extension } => {
                self.handle_intercom(
                    request,
                    handle,
                    call_id,
                    &extension,
                    fcp_pbx::IntercomMode::OneWay,
                )
                .await
            }
            fcp_pbx::FeatureCodeAction::IntercomTwoWay { extension } => {
                self.handle_intercom(
                    request,
                    handle,
                    call_id,
                    &extension,
                    fcp_pbx::IntercomMode::TwoWay,
                )
                .await
            }
            fcp_pbx::FeatureCodeAction::AgentLogin => {
                self.handle_agent_login(request, handle, call_id).await
            }
            fcp_pbx::FeatureCodeAction::AgentLogout => {
                self.handle_agent_logout(request, handle, call_id).await
            }
            fcp_pbx::FeatureCodeAction::VoicemailAccess { mailbox } => {
                self.handle_voicemail_access_feature(request, handle, call_id, mailbox)
                    .await
            }
            fcp_pbx::FeatureCodeAction::Calling(code) => {
                self.handle_calling_code(request, handle, call_id, code)
                    .await
            }
            fcp_pbx::FeatureCodeAction::HotelingLogin => {
                self.handle_hoteling_code(request, handle, call_id, true)
                    .await
            }
            fcp_pbx::FeatureCodeAction::HotelingLogout => {
                self.handle_hoteling_code(request, handle, call_id, false)
                    .await
            }
            fcp_pbx::FeatureCodeAction::ExecutivePush => {
                Box::pin(self.handle_executive_push(request, handle, call_id)).await
            }
            fcp_pbx::FeatureCodeAction::CallOnBehalf { target } => {
                Box::pin(self.handle_call_on_behalf(request, handle, call_id, &target)).await
            }
        }
    }

    /// `*88` and `*89` from a host phone (`docs/ENTERPRISE_CALLING.md`
    /// §3.8): a login answers the call and asks for an extension and a
    /// PIN; a logout ends the host's session and answers at once.
    async fn handle_hoteling_code(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        login: bool,
    ) -> Result<()> {
        let finish = |status: u16, reason: &'static str| async move {
            let response = self.create_simple_response(request, status, reason);
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(call_id, Some("Hoteling feature code".to_string()));
            Ok(())
        };
        let (Some(storage), Some(hoteling)) = (self.storage.as_ref(), self.hoteling.as_ref())
        else {
            return finish(501, "Hoteling Not Available").await;
        };
        let caller = extract_from_uri(request)
            .ok()
            .and_then(|u| u.user().map(str::to_string))
            .unwrap_or_default();
        let tenant_id = match self.resolve_runtime_tenant(request, call_id).await {
            Ok(tenant_id) => tenant_id,
            Err(e) => {
                warn!(call_id = %call_id, caller = %caller, error = %e, "Hoteling code: tenant resolution failed");
                return finish(403, "Tenant Context Required").await;
            }
        };
        let device = match storage.get_device_by_sip_name(&tenant_id, &caller).await {
            Ok(Some(device)) if device.enabled && device.hoteling_host => device,
            _ => {
                info!(call_id = %call_id, caller = %caller, "Hoteling code from a phone that is not a host");
                return finish(403, "Not A Hoteling Host").await;
            }
        };
        if !login {
            return match hoteling
                .logout_device(&device, fcp_core::SessionEnd::Logout)
                .await
            {
                Some(session) => {
                    info!(call_id = %call_id, device_id = %device.id, user_id = %session.user_id, "Hoteling logout by feature code");
                    self.confirm_feature(request, handle, call_id, "Logged out.")
                        .await
                }
                None => finish(404, "No One Logged In").await,
            };
        }
        let Some(flow_handler) = self.flow_handler.as_ref().map(Arc::clone) else {
            return finish(501, "Hoteling Login Not Available").await;
        };
        let flow_def = {
            use fcp_flow::{FlowDefinition, FlowNode, NodeType};
            let node = |id: &str, node_type: NodeType| {
                (
                    id.to_string(),
                    FlowNode {
                        id: id.to_string(),
                        name: None,
                        node_type,
                    },
                )
            };
            let nodes = std::collections::HashMap::from([
                node(
                    "answer",
                    NodeType::Answer {
                        next: "login".to_string(),
                        on_error: None,
                    },
                ),
                node(
                    "login",
                    NodeType::HotelingLogin {
                        device_id: device.id.to_string(),
                        max_attempts: 3,
                        next: "hangup".to_string(),
                        on_failure: None,
                    },
                ),
                node("hangup", NodeType::Hangup { reason: None }),
            ]);
            FlowDefinition {
                id: format!("hoteling-login-{call_id}"),
                name: "Hoteling login".to_string(),
                description: Some(
                    "Inline hoteling login flow from the *88 feature code".to_string(),
                ),
                version: 1,
                initial_node: "answer".to_string(),
                nodes,
                variables: std::collections::HashMap::new(),
                tenant_id: None,
                enabled: true,
                created_at: None,
                updated_at: None,
            }
        };
        self.start_inline_flow(
            request,
            handle,
            call_id,
            &flow_handler,
            flow_def,
            HashMap::new(),
            Vec::new(),
            "hoteling login",
        )
        .await
    }

    /// Handle group pickup (*8): find a ringing call in the caller's pickup group.
    async fn handle_group_pickup(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
    ) -> Result<()> {
        let pbx = match self.pbx_handler.as_ref() {
            Some(p) => p,
            None => {
                let response = self.create_simple_response(request, 500, "PBX Not Configured");
                handle.send_final(response).await;
                return Ok(());
            }
        };

        // Determine the caller's extension from From header
        let caller_ext = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();

        match pbx.find_ringing_in_group(&caller_ext, &self.call_manager) {
            Some(ringing_call_id) => {
                info!(
                    call_id = %call_id,
                    ringing_call = %ringing_call_id,
                    caller_ext = %caller_ext,
                    "Group pickup: found ringing call"
                );
                self.complete_pickup(request, handle, call_id, ringing_call_id, "group pickup")
                    .await?;
            }
            None => {
                info!(call_id = %call_id, caller_ext = %caller_ext, "Group pickup: no ringing call found");
                let response = self.create_simple_response(request, 480, "No Ringing Call");
                handle.send_final(response).await;
                let _ = self
                    .call_manager
                    .terminate_call(call_id, Some("No ringing call in pickup group".to_string()));
            }
        }
        Ok(())
    }

    /// Handle directed pickup (*9ext): find a ringing call targeting a specific extension.
    async fn handle_directed_pickup(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        target_extension: &str,
    ) -> Result<()> {
        let pbx = match self.pbx_handler.as_ref() {
            Some(p) => p,
            None => {
                let response = self.create_simple_response(request, 500, "PBX Not Configured");
                handle.send_final(response).await;
                return Ok(());
            }
        };

        match pbx.find_ringing_for_extension(target_extension, &self.call_manager) {
            Some(ringing_call_id) => {
                info!(
                    call_id = %call_id,
                    ringing_call = %ringing_call_id,
                    target = %target_extension,
                    "Directed pickup: found ringing call"
                );
                self.complete_pickup(request, handle, call_id, ringing_call_id, "directed pickup")
                    .await?;
            }
            None => {
                info!(call_id = %call_id, target = %target_extension, "Directed pickup: no ringing call found");
                let response = self.create_simple_response(request, 480, "No Ringing Call");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    call_id,
                    Some(format!(
                        "No ringing call for extension {}",
                        target_extension
                    )),
                );
            }
        }
        Ok(())
    }

    async fn complete_pickup(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        feature_call_id: &fcp_b2bua_core::CallId,
        ringing_call_id: fcp_b2bua_core::CallId,
        label: &str,
    ) -> Result<()> {
        let Some(ringing_call) = self.call_manager.get_call(&ringing_call_id) else {
            let response = self.create_simple_response(request, 404, "Ringing Call Not Found");
            handle.send_final(response).await;
            let _ = self.call_manager.terminate_call(
                feature_call_id,
                Some(format!("{} failed: ringing call disappeared", label)),
            );
            return Ok(());
        };

        if ringing_call.state != fcp_b2bua_core::CallState::Proceeding {
            let response = self.create_simple_response(request, 409, "Call No Longer Ringing");
            handle.send_final(response).await;
            let _ = self.call_manager.terminate_call(
                feature_call_id,
                Some(format!("{} failed: call is no longer ringing", label)),
            );
            return Ok(());
        }

        let Some(original_request) = ringing_call.original_invite.clone() else {
            let response = self.create_simple_response(request, 500, "Pickup Failed");
            handle.send_final(response).await;
            let _ = self.call_manager.terminate_call(
                feature_call_id,
                Some(format!("{} failed: missing original INVITE", label)),
            );
            return Ok(());
        };

        let Some(original_tx_key) = ringing_call.a_leg_invite_tx.clone() else {
            let response = self.create_simple_response(request, 500, "Pickup Failed");
            handle.send_final(response).await;
            let _ = self.call_manager.terminate_call(
                feature_call_id,
                Some(format!("{} failed: missing A-leg transaction", label)),
            );
            return Ok(());
        };

        // We defer signaling the fork arbiter until *after* the call has been
        // atomically transitioned to Established below. Waking the arbiter with
        // state still in Proceeding would race the `already_rebound` check and
        // could cause the arbiter to send 487 Request Terminated to the caller
        // while we concurrently send 200 OK from this function.
        let cancel_notify = ringing_call.cancel_notify.clone();

        let picker_offer = crate::body::request_sdp(request);
        let original_offer = ringing_call.a_leg.sdp.clone();

        let mut pickup_media_session_id = ringing_call.media_session_id;
        let mut pickup_media_anchored = ringing_call.media_anchored;

        let (original_answer, picker_answer) = if ringing_call.media_anchored {
            let Some(media_manager) = &self.media_manager else {
                let response = self.create_simple_response(request, 503, "Media Unavailable");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    feature_call_id,
                    Some(format!("{} failed: media manager unavailable", label)),
                );
                return Ok(());
            };
            let Some(media_session_id) = ringing_call.media_session_id else {
                let response = self.create_simple_response(request, 503, "Media Unavailable");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    feature_call_id,
                    Some(format!("{} failed: media session unavailable", label)),
                );
                return Ok(());
            };

            if let Some(ref picker_offer) = picker_offer {
                if let Err(e) = media_manager
                    .update_session_b_leg(&media_session_id, picker_offer.clone())
                    .await
                {
                    warn!(
                        call_id = %ringing_call_id,
                        error = %e,
                        "{} failed while updating media session for picker",
                        label
                    );
                    let response = self.create_simple_response(request, 503, "Pickup Failed");
                    handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call(feature_call_id, Some(format!("{} failed: {}", label, e)));
                    return Ok(());
                }
            }

            let session = match media_manager.get_session(&media_session_id).await {
                Ok(session) => session,
                Err(e) => {
                    warn!(
                        call_id = %ringing_call_id,
                        error = %e,
                        "{} failed while loading media session",
                        label
                    );
                    let response = self.create_simple_response(request, 503, "Pickup Failed");
                    handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call(feature_call_id, Some(format!("{} failed: {}", label, e)));
                    return Ok(());
                }
            };
            // The original caller is leg A, the picker leg B.
            let (a_port, _) = session.ports_for(fcp_media::backend::Leg::A);
            let (b_port, _) = session.ports_for(fcp_media::backend::Leg::B);

            let original_answer = original_offer
                .as_deref()
                .map(|offer| {
                    fcp_media::construct_answer_for_a_leg(offer, &self.local_addr, a_port, None)
                })
                .unwrap_or_else(|| generate_basic_sdp(&self.local_addr, a_port));
            let picker_answer = picker_offer
                .as_deref()
                .map(|offer| {
                    fcp_media::construct_answer_for_a_leg(offer, &self.local_addr, b_port, None)
                })
                .unwrap_or_else(|| generate_basic_sdp(&self.local_addr, b_port));

            (Some(original_answer), Some(picker_answer))
        } else {
            match self
                .anchor_pickup_media(
                    &ringing_call_id,
                    original_offer.as_deref(),
                    picker_offer.as_deref(),
                    label,
                )
                .await
            {
                Ok((media_session_id, original_answer, picker_answer)) => {
                    pickup_media_session_id = Some(media_session_id);
                    pickup_media_anchored = true;
                    (original_answer, picker_answer)
                }
                Err(e) => {
                    warn!(
                        call_id = %ringing_call_id,
                        error = %e,
                        "{} failed while dynamically anchoring pickup media",
                        label
                    );
                    let response = self.create_simple_response(request, 503, "Media Unavailable");
                    handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call(feature_call_id, Some(format!("{} failed: {}", label, e)));
                    return Ok(());
                }
            }
        };

        let original_response = self.build_final_ok_response(
            &original_request,
            ringing_call.a_leg_to_tag.as_deref(),
            original_answer.as_deref(),
        )?;
        let picker_response =
            self.build_final_ok_response(request, None, picker_answer.as_deref())?;

        let original_dialog = self.build_inbound_dialog(&original_request, &original_response)?;
        let picker_dialog = self.build_inbound_dialog(request, &picker_response)?;
        let picker_leg = self.build_pickup_leg(request, picker_dialog.clone());

        // Transition state to Established BEFORE signaling the arbiter so
        // that when it wakes and re-reads the call, `already_rebound` is true
        // and it takes the cancel-all-forks branch instead of the normal
        // no-winner path that would respond with 487 to the original caller.
        let _ = self.call_manager.update_call(&ringing_call_id, |call| {
            call.a_leg.dialog = Some(original_dialog.clone());
            call.b_leg = Some(picker_leg.clone());
            call.cancel_notify = None;
            call.media_session_id = pickup_media_session_id;
            call.media_anchored = pickup_media_anchored;
            call.establish();
        });

        // Now it is safe to signal the fork arbiter. Use `notify_one` so that
        // if for any reason the arbiter has not yet registered a waiter on
        // `cancel_notify.notified()`, a permit is stored and picked up on its
        // next poll.
        if let Some(cancel_notify) = cancel_notify {
            cancel_notify.notify_one();
        }

        let _ = self.dialog_manager.insert(original_dialog.clone());
        self.start_session_timer(&original_dialog);
        let _ = self.dialog_manager.insert(picker_dialog.clone());
        self.start_session_timer(&picker_dialog);

        self.transaction_manager
            .send_final(&original_tx_key, original_response)
            .await;
        handle.send_final(picker_response).await;
        self.start_automatic_recording(&ringing_call_id);

        // The feature call's record points at the call it picked up.
        let _ = self.call_manager.update_call(feature_call_id, |call| {
            call.correlation_id = Some(fcp_core::CallId::from_uuid(*ringing_call_id.as_uuid()));
        });
        let _ = self.call_manager.terminate_call_with(
            feature_call_id,
            fcp_cdr::TerminationReason::Completed {
                detail: format!("{} completed", label),
            },
        );

        info!(
            pickup_call_id = %feature_call_id,
            ringing_call_id = %ringing_call_id,
            mode = %label,
            "Bridged ringing call to picker"
        );

        Ok(())
    }

    /// Carry out a REFER (`docs/ENTERPRISE_CALLING.md` §3.2): a park code
    /// parks the transferred party; any other target is dialed on the
    /// transferred party's behalf, in the background, with the outcome
    /// NOTIFYed to the referrer.
    async fn execute_refer(
        &self,
        request: &sip_core::Request,
        call: &fcp_b2bua_core::Call,
        is_a_leg: bool,
        refer_to: &str,
    ) -> sip_core::Response {
        let referrer = fcp_b2bua_core::LegSide::of(is_a_leg);
        if call.state != fcp_b2bua_core::CallState::Established {
            return self.create_simple_response(request, 491, "Request Pending");
        }
        let Some(target) = crate::transfer::parse_refer_to(refer_to) else {
            return self.create_simple_response(request, 400, "Bad Refer-To");
        };
        let Some(uac) = self.uac.clone() else {
            return self.create_simple_response(request, 500, "No UAC Configured");
        };
        let referrer_dialog = match referrer {
            fcp_b2bua_core::LegSide::A => call.a_leg.dialog.clone(),
            fcp_b2bua_core::LegSide::B => call.b_leg.as_ref().and_then(|b| b.dialog.clone()),
        };
        let Some(referrer_dialog) = referrer_dialog else {
            return self.create_simple_response(request, 481, "Call/Transaction Does Not Exist");
        };

        // A park code or a lot's park number: the transferred party waits
        // in the next free slot; a slot itself: in that one, a directed
        // park (`docs/PARKING.md` §3).
        if target.replaces.is_none() {
            if let Some(pbx) = self.pbx_handler.as_ref().filter(|p| p.is_enabled()) {
                let park_in = if matches!(
                    pbx.parse_feature_code(&target.user),
                    Some(fcp_pbx::FeatureCodeAction::ParkCall)
                ) {
                    Some(None)
                } else {
                    let lots = self.parking_lots_of(&call.tenant_id).await;
                    let lot = lots.iter().filter(|l| l.enabled);
                    if lot
                        .clone()
                        .any(|l| l.park_number.as_deref() == Some(target.user.as_str()))
                    {
                        Some(None)
                    } else {
                        lot.clone()
                            .find(|l| l.contains(&target.user))
                            .map(|_| Some(target.user.clone()))
                    }
                };
                if let Some(only) = park_in {
                    let parker = match referrer {
                        fcp_b2bua_core::LegSide::A => call.a_leg.remote_uri.clone(),
                        fcp_b2bua_core::LegSide::B => call
                            .b_leg
                            .as_ref()
                            .map(|b| b.remote_uri.clone())
                            .unwrap_or_default(),
                    };
                    let parker = user_of_uri(&parker).unwrap_or_default();
                    return match self
                        .park_party(pbx, call, referrer, &parker, only.as_deref())
                        .await
                    {
                        Ok(slot) => {
                            info!(call_id = %call.id, slot = %slot, parker = %parker, "Parked by transfer");
                            let local_uri = self.uas.local_uri.clone();
                            let mut dialog = referrer_dialog.clone();
                            let flow = match referrer {
                                fcp_b2bua_core::LegSide::A => call.a_leg.flow_token.clone(),
                                fcp_b2bua_core::LegSide::B => {
                                    call.b_leg.as_ref().and_then(|b| b.flow_token.clone())
                                }
                            };
                            let drop = self.leg_dropper(referrer_dialog, flow);
                            let call_id = call.id;
                            tokio::spawn(async move {
                                if let Err(e) = crate::conference_migration::report_refer_outcome(
                                    &uac,
                                    &local_uri,
                                    &mut dialog,
                                    200,
                                    "OK",
                                )
                                .await
                                {
                                    warn!(call_id = %call_id, error = %e, "The park's NOTIFY was not delivered");
                                }
                                drop.await;
                            });
                            self.create_simple_response(request, 202, "Accepted")
                        }
                        Err(e) if e.downcast_ref::<SlotTaken>().is_some() => {
                            info!(call_id = %call.id, slot = ?only, "Directed park: the slot is taken");
                            self.create_simple_response(request, 486, "Parking Slot Taken")
                        }
                        Err(e) => {
                            warn!(call_id = %call.id, error = %e, "Park by transfer failed");
                            self.create_simple_response(request, 503, "No Parking Orbits Available")
                        }
                    };
                }
            }
        }

        if self.conference_migrations.insert(call.id, ()).is_some() {
            return self.create_simple_response(request, 491, "Request Pending");
        }
        info!(
            call_id = %call.id,
            referrer = referrer.as_str(),
            target = %target.user,
            attended = target.replaces.is_some(),
            "Carrying out a transfer"
        );
        let run = self.transfer_run(
            uac,
            call,
            referrer,
            Some(referrer_dialog),
            &target.user,
            target.replaces.clone(),
            true,
        );
        tokio::spawn(run.run());
        self.create_simple_response(request, 202, "Accepted")
    }

    /// A transfer of `call`'s party on the other side of `referrer` to
    /// `user`, dialed back into this node so the target is routed as any
    /// call is. `notify_referrer`: the referrer asked by REFER and is told
    /// the outcome.
    #[allow(clippy::too_many_arguments)]
    fn transfer_run(
        &self,
        uac: Arc<SipUserAgentClient>,
        call: &fcp_b2bua_core::Call,
        referrer: fcp_b2bua_core::LegSide,
        referrer_dialog: Option<sip_dialog::Dialog>,
        user: &str,
        replaces: Option<String>,
        notify_referrer: bool,
    ) -> crate::transfer::TransferRun {
        let host = match self.uas.local_uri.port() {
            Some(port) => format!("{}:{}", self.uas.local_uri.host(), port),
            None => self.uas.local_uri.host().to_string(),
        };
        let mut headers = vec![(TRANSFERRED_FROM_HEADER.to_string(), call.id.to_string())];
        if let Some(replaces) = replaces {
            headers.push(("Replaces".to_string(), replaces));
        }
        // The target sees the transferred party calling.
        let caller = call
            .original_invite
            .as_ref()
            .and_then(|invite| match referrer {
                fcp_b2bua_core::LegSide::B => extract_from_uri(invite)
                    .ok()
                    .map(|uri| (extract_from_display_name(invite), normalize_from_uri(uri))),
                fcp_b2bua_core::LegSide::A => sip_parse::header(invite.headers(), "To")
                    .and_then(|to| {
                        let to = to.as_str();
                        let uri = match (to.find('<'), to.find('>')) {
                            (Some(a), Some(b)) if a < b => &to[a + 1..b],
                            _ => to.split(';').next().unwrap_or(to),
                        };
                        sip_core::SipUri::parse(uri.trim()).ok()
                    })
                    .map(|uri| (None, uri)),
            });
        crate::transfer::TransferRun {
            uac,
            local_uri: self.uas.local_uri.clone(),
            call_manager: self.call_manager.clone(),
            dialog_manager: Arc::clone(&self.dialog_manager),
            session_timer_manager: Arc::clone(&self.session_timer_manager),
            media_manager: self.media_manager.clone(),
            sip_call_id_map: Arc::clone(&self.sip_call_id_map),
            in_flight: Arc::clone(&self.conference_migrations),
            b_leg_keys: Arc::clone(&self.b_leg_keys),
            b2bua_config: self.b2bua_config.clone(),
            local_addr: self.local_addr.clone(),
            call_id: call.id,
            referrer,
            referrer_dialog,
            target: format!("sip:{}@{}", user, host),
            headers,
            caller,
            notify_referrer,
            fallback: None,
        }
    }

    /// Park the party on the other side of `parker`: an orbit taken, the
    /// parked party moved into the A slot (the retriever takes B), the call
    /// marked parked. The parker's leg is left for the caller to drop.
    async fn park_party(
        &self,
        pbx: &fcp_pbx::PbxHandler,
        call: &fcp_b2bua_core::Call,
        parker: fcp_b2bua_core::LegSide,
        parker_ext: &str,
        only: Option<&str>,
    ) -> Result<String> {
        let lot = match only {
            Some(slot) => self
                .parking_lots_of(&call.tenant_id)
                .await
                .iter()
                .find(|l| l.enabled && l.contains(slot))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("{slot} is in none of the tenant's lots"))?,
            None => self.parking_lot_for(pbx, &call.tenant_id).await,
        };
        let orbit = self.claim_slot(pbx, &lot, call, parker_ext, only).await?;
        let lot_music = lot.music.clone();
        // The parked party goes to the A slot; its media side follows, and
        // the B side, the parker's, is left with no one on it until the
        // call is retrieved: still naming the parked party, it relayed the
        // party's own audio back between the hold music's packets
        // (`docs/DEEP_TESTING.md` D8).
        if let (true, Some(session_id), Some(media_manager)) = (
            call.media_anchored,
            call.media_session_id,
            self.media_manager.as_ref(),
        ) {
            use fcp_media::backend::Leg;
            let moved = match parker {
                fcp_b2bua_core::LegSide::A => {
                    match call.b_leg.as_ref().and_then(|b| b.sdp.clone()) {
                        Some(sdp) => match media_manager.update_session(&session_id, sdp).await {
                            Ok(_) => media_manager.move_peer(&session_id, Leg::B, Leg::A).await,
                            Err(e) => Err(e),
                        },
                        None => media_manager.release_leg(&session_id, Leg::B).await,
                    }
                }
                fcp_b2bua_core::LegSide::B => media_manager.release_leg(&session_id, Leg::B).await,
            };
            if let Err(e) = moved {
                warn!(call_id = %call.id, error = %e, "Parked party's media not moved to the A side");
            }
        }
        let parker_dialog_id = match parker {
            fcp_b2bua_core::LegSide::A => call.a_leg.dialog.as_ref().map(|d| d.id().clone()),
            fcp_b2bua_core::LegSide::B => call
                .b_leg
                .as_ref()
                .and_then(|b| b.dialog.as_ref().map(|d| d.id().clone())),
        };
        let parker_ext = parker_ext.to_string();
        self.call_manager.update_call(&call.id, |c| {
            // The parker leaves the call; the parked party, now in A, is
            // who it counts against.
            if parker == fcp_b2bua_core::LegSide::A {
                c.caller_user_id = c.callee_user_id.take();
            } else {
                c.callee_user_id = None;
            }
            if parker == fcp_b2bua_core::LegSide::A {
                if let Some(b) = c.b_leg.take() {
                    c.a_leg = b;
                }
            } else {
                c.b_leg = None;
            }
            // Held, whatever the parker did: the parked party hears the
            // hold music (`docs/ENTERPRISE_CALLING.md` §3.3).
            c.held = Some(fcp_b2bua_core::LegSide::A);
            c.parked_music = lot_music.clone();
            c.park(orbit.clone(), parker_ext.clone());
        })?;
        if let Some(id) = parker_dialog_id {
            self.sip_call_id_map.remove(id.call_id());
        }
        Ok(orbit)
    }

    /// A call to one of the tenant's lots: its park number parks as `*70`
    /// does, one of its slots takes back the call parked there. `None` for
    /// any other number.
    async fn park_by_number(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        tenant_id: fcp_core::TenantId,
        dialed: &str,
    ) -> Option<Result<()>> {
        let user = user_of_uri(dialed).filter(|u| fcp_core::parking::is_number(u))?;
        let lots = self.parking_lots_of(&tenant_id).await;
        let lots = lots.iter().filter(|l| l.enabled);
        if lots
            .clone()
            .any(|l| l.park_number.as_deref() == Some(user.as_str()))
        {
            info!(call_id = %call_id, park_number = %user, "A lot's park number");
            return Some(Box::pin(self.handle_park_call(request, handle, call_id)).await);
        }
        if lots.clone().any(|l| l.contains(&user)) {
            info!(call_id = %call_id, slot = %user, "A parking slot dialed");
            return Some(
                Box::pin(self.handle_retrieve_parked(request, handle, call_id, &user)).await,
            );
        }
        None
    }

    /// The tenant's parking lots, read at most every few seconds.
    async fn parking_lots_of(
        &self,
        tenant_id: &fcp_core::TenantId,
    ) -> Arc<Vec<fcp_core::parking::ParkingLot>> {
        const FRESH: std::time::Duration = std::time::Duration::from_secs(10);
        if let Some(entry) = self.parking_lots.get(tenant_id) {
            if entry.0.elapsed() < FRESH {
                return Arc::clone(&entry.1);
            }
        }
        let Some(store) = self.storage.as_ref().and_then(|s| s.parking_lot_store()) else {
            return Arc::new(Vec::new());
        };
        let lots = match store.list_parking_lots(tenant_id).await {
            Ok(lots) => Arc::new(lots),
            Err(e) => {
                warn!(tenant_id = %tenant_id, error = %e, "The tenant's parking lots could not be read");
                return self
                    .parking_lots
                    .get(tenant_id)
                    .map(|e| Arc::clone(&e.1))
                    .unwrap_or_default();
            }
        };
        self.parking_lots
            .insert(*tenant_id, (std::time::Instant::now(), Arc::clone(&lots)));
        lots
    }

    /// The lot a call of the tenant parks in: one of its own, else this
    /// node's range (`[pbx.parking]`).
    async fn parking_lot_for(
        &self,
        pbx: &fcp_pbx::PbxHandler,
        tenant_id: &fcp_core::TenantId,
    ) -> fcp_core::parking::ParkingLot {
        let lots = self.parking_lots_of(tenant_id).await;
        fcp_core::parking::lot_for(&lots, None)
            .cloned()
            .unwrap_or_else(|| pbx.parking_manager().node_lot(*tenant_id))
    }

    /// Take the lot's first free slot for `call`: claimed cluster-wide
    /// first, so no other node parks in it, then held here
    /// (`docs/PARKING.md` §2). A claim that cannot be made — Redis down —
    /// is not waited out: the slot is taken on this node alone.
    async fn claim_slot(
        &self,
        pbx: &fcp_pbx::PbxHandler,
        lot: &fcp_core::parking::ParkingLot,
        call: &fcp_b2bua_core::Call,
        parker_ext: &str,
        only: Option<&str>,
    ) -> Result<String> {
        let tenant = call.tenant_id;
        let parking = pbx.parking_manager();
        for slot in lot.slots().filter(|s| only.is_none_or(|o| o == s)) {
            if parking.is_held(&tenant, &slot) {
                continue;
            }
            if let Some(cp) = self.cluster_parking.as_ref() {
                let info = fcp_core::ClusterParkedCall {
                    call_id: call.id.to_string(),
                    node_id: self.node_id.clone(),
                    parker_extension: parker_ext.to_string(),
                    tenant_id: tenant.to_string(),
                    slot: slot.clone(),
                    lot_id: lot.id.clone(),
                    parked_at_unix: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };
                // Past the lot's timeout, so the claim outlives the wait
                // until this node sends the call back.
                let ttl = u64::from(lot.timeout_secs) + 60;
                match cp.cluster_park(&tenant, &slot, info, ttl).await {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(e) => {
                        warn!(slot = %slot, error = %e, "A parking slot could not be claimed in the cluster; taking it on this node");
                    }
                }
            }
            let held = parking.hold(fcp_pbx::ParkedCall {
                call_id: call.id,
                slot: slot.clone(),
                lot_id: lot.id.clone(),
                parker_extension: parker_ext.to_string(),
                tenant_id: tenant,
                parked_at: std::time::Instant::now(),
                timeout: std::time::Duration::from_secs(u64::from(lot.timeout_secs)),
            });
            match held {
                Ok(()) => return Ok(slot),
                Err(_) => {
                    if let Some(cp) = self.cluster_parking.as_ref() {
                        let _ = cp.cluster_unpark(&tenant, &slot).await;
                    }
                }
            }
        }
        if only.is_some() {
            return Err(SlotTaken.into());
        }
        Err(anyhow::anyhow!(
            "no slot free in the parking lot {:?}",
            lot.name
        ))
    }

    /// Drop a leg no call holds any more: its timer stopped, its dialog
    /// forgotten, a BYE sent (down its flow when it has one). Unlike
    /// `spawn_bye_with_retry` it touches no call: the slot it was in may
    /// already hold another leg.
    fn drop_leg(&self, dialog: sip_dialog::Dialog, flow_token: Option<String>) {
        tokio::spawn(self.leg_dropper(dialog, flow_token));
    }

    fn leg_dropper(
        &self,
        dialog: sip_dialog::Dialog,
        flow_token: Option<String>,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        let uac = self.uac.clone();
        let flow = flow_token.and_then(|t| self.flow_registry.lookup(&t));
        self.session_timer_manager.stop_timer(dialog.id());
        self.dialog_manager.remove(dialog.id());
        async move {
            let Some(uac) = uac else { return };
            let sent = match flow {
                Some(flow) => {
                    tokio::time::timeout(
                        Duration::from_secs(10),
                        uac.bye_via_flow(&dialog, flow.stream, flow.peer_addr),
                    )
                    .await
                }
                None => tokio::time::timeout(Duration::from_secs(10), uac.bye(&dialog)).await,
            };
            match sent {
                Ok(Ok(_)) => debug!(dialog = ?dialog.id(), "Dropped a leg"),
                Ok(Err(e)) => {
                    debug!(dialog = ?dialog.id(), error = %e, "BYE to a dropped leg failed")
                }
                Err(_) => debug!(dialog = ?dialog.id(), "BYE to a dropped leg timed out"),
            }
        }
    }

    /// An INVITE with `Replaces` (RFC 3891): it takes the named leg's
    /// place, and the named leg is dropped.
    async fn handle_replaces(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        replaces: &str,
    ) -> Result<()> {
        let refuse = |code: u16, reason: &'static str, why: String| async move {
            info!(call_id = %call_id, code, why = %why, "INVITE with Replaces refused");
            let response = self.create_simple_response(request, code, reason);
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(call_id, Some(format!("{code} {why}")));
            Ok(())
        };
        let (call, side, old_dialog) =
            match crate::transfer::replaced_leg(&self.call_manager, replaces) {
                Ok(found) => found,
                Err(e) => {
                    return refuse(481, "Call/Transaction Does Not Exist", e.to_string()).await
                }
            };
        let tenant = self.call_manager.get_call(call_id).map(|c| c.tenant_id);
        if tenant != Some(call.tenant_id) {
            return refuse(
                481,
                "Call/Transaction Does Not Exist",
                "another tenant's call".into(),
            )
            .await;
        }
        if call.id == *call_id {
            return refuse(482, "Loop Detected", "a call cannot replace itself".into()).await;
        }
        if call.state != fcp_b2bua_core::CallState::Established {
            // An early dialog (a ringing phone) is picked up with the
            // pickup codes; `early-only` asks for exactly that.
            return refuse(486, "Busy Here", format!("the call is {:?}", call.state)).await;
        }
        let flow = match side {
            fcp_b2bua_core::LegSide::A => call.a_leg.flow_token.clone(),
            fcp_b2bua_core::LegSide::B => call.b_leg.as_ref().and_then(|b| b.flow_token.clone()),
        };
        self.take_over_leg(
            request,
            handle,
            call_id,
            &call,
            side,
            Some((old_dialog, flow)),
            "Replaces",
        )
        .await
    }

    /// A phone on a shared line barging into an active appearance: its
    /// INVITE answered with the call's supervisor party, heard by both
    /// parties (§3.11), when the line allows barge-in and the call has no
    /// supervisor. `own` is the INVITE's own call, ended once joined.
    async fn barge_into_appearance(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        own: &fcp_b2bua_core::CallId,
        call: &fcp_b2bua_core::Call,
    ) -> Result<()> {
        let refuse = |code: u16, reason: &'static str| async move {
            let response = self.create_simple_response(request, code, reason);
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(own, Some(format!("barge-in: {reason}")));
            Ok(())
        };
        let allowed = match (&self.storage, &call.appearance) {
            (Some(storage), Some(appearance)) => storage
                .get_line_binding(&appearance.line_binding_id)
                .await
                .ok()
                .flatten()
                .is_some_and(|b| b.shared && b.barge_in),
            _ => false,
        };
        let Some(supervisions) = self.supervisions.clone().filter(|_| allowed) else {
            return refuse(403, "Appearance In Use").await;
        };
        let Some(offer) = crate::body::request_sdp(request) else {
            return refuse(488, "Not Acceptable Here").await;
        };
        let party = sip_parse::header(request.headers(), "From")
            .map(|f| f.to_string())
            .unwrap_or_default();
        let answer = match supervisions.barge_in(&call.id, &party, offer).await {
            Ok(answer) => answer,
            Err(crate::supervision::SuperviseError::AlreadySupervised) => {
                return refuse(486, "Busy Here").await;
            }
            Err(e) => {
                warn!(call_id = %call.id, error = %e, "Barge-in failed");
                return refuse(503, "Service Unavailable").await;
            }
        };
        let response = self.build_final_ok_response(request, None, Some(&answer))?;
        let dialog = self.build_inbound_dialog(request, &response)?;
        supervisions.attach_dialog(&call.id, dialog.clone());
        let _ = self.dialog_manager.insert(dialog);
        handle.send_final(response).await;
        // The INVITE's own media session, if it made one, is not needed.
        if let Some(mine) = self.call_manager.get_call(own) {
            if let (true, Some(session_id), Some(media_manager)) = (
                mine.media_anchored,
                mine.media_session_id,
                self.media_manager.as_ref(),
            ) {
                if Some(session_id) != call.media_session_id {
                    let _ = media_manager.terminate_session(&session_id).await;
                }
            }
        }
        let _ = self.call_manager.update_call(own, |c| {
            c.correlation_id = Some(fcp_core::CallId::from_uuid(*call.id.as_uuid()));
        });
        let _ = self.call_manager.terminate_call_with(
            own,
            fcp_cdr::TerminationReason::Completed {
                detail: "Barged into an appearance".to_string(),
            },
        );
        info!(call_id = %call.id, "A phone barged into an appearance");
        Ok(())
    }

    /// A call about to ring a shared line's phones takes an appearance of
    /// the line (`docs/ENTERPRISE_CALLING.md` §3.9). `false` when every
    /// appearance is in use: the caller has been told busy.
    async fn take_line_appearance(
        &self,
        request: &sip_core::Request,
        a_leg: &Arc<ServerTransactionHandle>,
        call_id: fcp_b2bua_core::CallId,
        binding: &fcp_core::LineBinding,
        number: &str,
    ) -> bool {
        let (true, Some(sca)) = (binding.shared, self.sca.as_ref()) else {
            return true;
        };
        let line = crate::sca::SharedLine {
            binding: binding.clone(),
            number: number.to_string(),
        };
        match Box::pin(sca.inbound(&line, call_id)).await {
            crate::sca::Inbound::Claimed(appearance) => {
                debug!(call_id = %call_id, line = %number, index = appearance.index, "A call to a shared line");
                let _ = self
                    .call_manager
                    .update_call(&call_id, |c| c.appearance = Some(appearance));
                true
            }
            crate::sca::Inbound::Unkept => true,
            crate::sca::Inbound::Busy => {
                info!(call_id = %call_id, line = %number, "Every appearance of the shared line is in use");
                let reason = "All Appearances In Use";
                let response = self.create_simple_response(request, 486, reason);
                a_leg.send_final(response).await;
                let _ = self.call_manager.terminate_call_with(
                    &call_id,
                    fcp_cdr::TerminationReason::rejected(486, reason),
                );
                false
            }
        }
    }

    /// A phone on a shared line picks up the held appearance `call`
    /// (`docs/ENTERPRISE_CALLING.md` §3.9): the INVITE takes the line's
    /// leg, the far party comes off hold to it, and the phone that held
    /// the call is sent a BYE. `own` is the INVITE's own call, ended once
    /// joined (one that does not exist — a retrieve handed over from
    /// another node — is fine).
    async fn retrieve_appearance(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        own: &fcp_b2bua_core::CallId,
        call: &fcp_b2bua_core::Call,
    ) -> Result<()> {
        let state = call.appearance_state();
        // An active appearance of a line that allows it is barged into
        // (`docs/ENTERPRISE_CALLING.md` §3.11): the phone joins the call,
        // heard by both parties, rather than taking it.
        if state == Some(fcp_core::sca::AppearanceState::Active) {
            return Box::pin(self.barge_into_appearance(request, handle, own, call)).await;
        }
        let held = state == Some(fcp_core::sca::AppearanceState::Held);
        let Some(appearance) = call.appearance.clone().filter(|_| held) else {
            info!(call_id = %call.id, "A retrieve of an appearance that is not held");
            let response = self.create_simple_response(request, 403, "Appearance Not Held");
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(own, Some("retrieve: not held".into()));
            return Ok(());
        };
        let side = appearance.side;
        let old = match side {
            fcp_b2bua_core::LegSide::A => call
                .a_leg
                .dialog
                .clone()
                .map(|d| (d, call.a_leg.flow_token.clone())),
            fcp_b2bua_core::LegSide::B => call
                .b_leg
                .as_ref()
                .and_then(|b| b.dialog.clone().map(|d| (d, b.flow_token.clone()))),
        };
        info!(call_id = %call.id, line = %appearance.number, index = appearance.index, "A held appearance retrieved");
        Box::pin(self.take_over_leg(request, handle, own, call, side, old, "Retrieve")).await
    }

    /// The INVITE `request` (whose own call is `feature_call_id`) takes
    /// slot `side` of `call`: the party on the other side is pointed at it
    /// — the media session's side moved when anchored, a re-INVITE when
    /// not, and off hold — the INVITE answered, the old leg (if any)
    /// dropped, and the INVITE's own call ended as completed, pointing at
    /// the call it joined (`docs/ENTERPRISE_CALLING.md` §3.2).
    #[allow(clippy::too_many_arguments)]
    async fn take_over_leg(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        feature_call_id: &fcp_b2bua_core::CallId,
        call: &fcp_b2bua_core::Call,
        side: fcp_b2bua_core::LegSide,
        old: Option<(sip_dialog::Dialog, Option<String>)>,
        label: &str,
    ) -> Result<()> {
        let fail = |code: u16, reason: &'static str, why: String| async move {
            warn!(call_id = %call.id, code, error = %why, "{label} failed");
            let response = self.create_simple_response(request, code, reason);
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(feature_call_id, Some(format!("{label} failed: {why}")));
            Ok(())
        };
        let party = side.other();
        let party_leg = match party {
            fcp_b2bua_core::LegSide::A => Some(call.a_leg.clone()),
            fcp_b2bua_core::LegSide::B => call.b_leg.clone(),
        };
        // No second party: the node answered the call itself — a flow, a
        // menu, voicemail access — and plays into the media session's A
        // side. Whoever takes the caller's place is pointed at that side,
        // and the flow carries on with them (`docs/DEEP_TESTING.md` D8: a
        // VVX's attended transfer to an IVR was refused 481).
        if party_leg.is_none() && side == fcp_b2bua_core::LegSide::A {
            let anchored = call
                .media_anchored
                .then_some(call.media_session_id)
                .flatten()
                .zip(self.media_manager.clone());
            if let Some((session_id, media_manager)) = anchored {
                let Some(offer) = crate::body::request_sdp(request) else {
                    return fail(
                        488,
                        "Not Acceptable Here",
                        "an INVITE without an offer".into(),
                    )
                    .await;
                };
                let answer = match media_manager.update_session(&session_id, offer).await {
                    Ok(answer) => answer,
                    Err(e) => {
                        return fail(503, "Service Unavailable", format!("media session: {e}"))
                            .await
                    }
                };
                let response = self.build_final_ok_response(request, None, Some(&answer))?;
                let new_dialog = self.build_inbound_dialog(request, &response)?;
                let leg = self.build_pickup_leg(request, new_dialog.clone());
                let joining_user = self
                    .call_manager
                    .get_call(feature_call_id)
                    .and_then(|c| c.caller_user_id);
                let updated = self.call_manager.update_call(&call.id, |c| {
                    c.caller_user_id = joining_user;
                    c.a_leg = leg.clone();
                    c.held = None;
                });
                if updated.is_err() {
                    return fail(
                        481,
                        "Call/Transaction Does Not Exist",
                        "the call ended".into(),
                    )
                    .await;
                }
                let _ = self.dialog_manager.insert(new_dialog.clone());
                self.start_session_timer(&new_dialog);
                self.sip_call_id_map
                    .insert(new_dialog.id().call_id().to_string(), call.id.to_string());
                handle.send_final(response).await;
                if let Some((old_dialog, flow)) = old {
                    self.sip_call_id_map.remove(old_dialog.id().call_id());
                    self.drop_leg(old_dialog, flow);
                }
                self.end_joining_call(feature_call_id, call, label).await;
                info!(call_id = %call.id, label, "A leg of a call the node answered taken over");
                return Ok(());
            }
        }
        let Some(party_leg) = party_leg else {
            return fail(
                481,
                "Call/Transaction Does Not Exist",
                "no party to join".into(),
            )
            .await;
        };
        let Some(mut party_dialog) = party_leg.dialog.clone() else {
            return fail(
                481,
                "Call/Transaction Does Not Exist",
                "the party has no dialog".into(),
            )
            .await;
        };
        let Some(offer) = crate::body::request_sdp(request) else {
            return fail(
                488,
                "Not Acceptable Here",
                "an INVITE without an offer".into(),
            )
            .await;
        };
        let Some(uac) = self.uac.clone() else {
            return fail(500, "Server Internal Error", "no UAC".into()).await;
        };
        let party_sdp = party_leg.sdp.clone().unwrap_or_default();
        let held = call.held == Some(party);

        let anchored = call
            .media_anchored
            .then_some(call.media_session_id)
            .flatten()
            .zip(self.media_manager.clone());
        let (answer, party_sdp_now) = match anchored {
            Some((session_id, media_manager)) => {
                // The phone taking the leg, if encrypted, is answered with
                // that leg's key: the backend's for the A side, a fresh one
                // of ours for the B side.
                let taken = match side {
                    fcp_b2bua_core::LegSide::A => media_manager
                        .update_session(&session_id, offer.clone())
                        .await
                        .map(|_| None),
                    fcp_b2bua_core::LegSide::B => {
                        self.key_b_leg_from_offer(&call.id, &session_id, &media_manager, &offer)
                            .await
                    }
                };
                let taker_srtp = match taken {
                    Ok(key) => key,
                    Err(e) => {
                        return fail(503, "Service Unavailable", format!("media session: {e}"))
                            .await
                    }
                };
                let session = match media_manager.get_session(&session_id).await {
                    Ok(session) => session,
                    Err(e) => {
                        return fail(503, "Service Unavailable", format!("media session: {e}"))
                            .await
                    }
                };
                // The taker is answered on its leg's port; the party is
                // taken off hold on its own.
                let port = session.ports_for(media_leg(side)).0;
                let party_port = session.ports_for(media_leg(party)).0;
                let encrypted = !fcp_media::parse_sdes_crypto(&offer).is_empty();
                let taker_srtp = match (side, encrypted) {
                    (fcp_b2bua_core::LegSide::A, true) => {
                        self.leg_srtp(&call.id, side, &session).await
                    }
                    _ => taker_srtp,
                };
                // The held party, if its leg is encrypted, is offered its
                // leg's key to come off hold.
                let party_srtp = if fcp_media::parse_sdes_crypto(&party_sdp).is_empty() {
                    None
                } else {
                    self.leg_srtp(&call.id, party, &session).await
                };
                if held {
                    let resume = crate::hold::with_direction(
                        &fcp_media::rewrite_sdp_for_anchoring(
                            &party_sdp,
                            &self.local_addr,
                            party_port,
                            party_srtp.as_ref(),
                        ),
                        "sendrecv",
                    );
                    match uac.send_reinvite(&mut party_dialog, Some(&resume)).await {
                        Ok(r) if r.start_line().is_success() => {}
                        Ok(r) => {
                            warn!(call_id = %call.id, status = r.start_line().code(), "The party refused to come off hold")
                        }
                        Err(e) => {
                            warn!(call_id = %call.id, error = %e, "The re-INVITE taking the party off hold failed")
                        }
                    }
                }
                (
                    fcp_media::construct_answer_for_a_leg(
                        &offer,
                        &self.local_addr,
                        port,
                        taker_srtp.as_ref(),
                    ),
                    party_sdp,
                )
            }
            None => {
                let to_party = crate::hold::with_direction(&offer, "sendrecv");
                match uac.send_reinvite(&mut party_dialog, Some(&to_party)).await {
                    Ok(r) if r.start_line().is_success() && !r.body().is_empty() => {
                        let answered = String::from_utf8_lossy(r.body()).to_string();
                        (answered.clone(), answered)
                    }
                    Ok(r) => {
                        return fail(
                            488,
                            "Not Acceptable Here",
                            format!("the party answered {}", r.start_line().code()),
                        )
                        .await
                    }
                    Err(e) => {
                        return fail(503, "Service Unavailable", format!("re-INVITE: {e}")).await
                    }
                }
            }
        };

        let response = self.build_final_ok_response(request, None, Some(&answer))?;
        let new_dialog = self.build_inbound_dialog(request, &response)?;
        let leg = self.build_pickup_leg(request, new_dialog.clone());
        let party_dialog_for_call = party_dialog.clone();
        let joining_user = self
            .call_manager
            .get_call(feature_call_id)
            .and_then(|c| c.caller_user_id);
        let updated = self.call_manager.update_call(&call.id, |c| {
            match side {
                fcp_b2bua_core::LegSide::A => c.caller_user_id = joining_user,
                fcp_b2bua_core::LegSide::B => c.callee_user_id = joining_user,
            }
            match side {
                fcp_b2bua_core::LegSide::A => {
                    c.a_leg = leg.clone();
                    if let Some(b) = c.b_leg.as_mut() {
                        b.dialog = Some(party_dialog_for_call.clone());
                        b.sdp = Some(party_sdp_now.clone());
                    }
                }
                fcp_b2bua_core::LegSide::B => {
                    c.b_leg = Some(leg.clone());
                    c.a_leg.dialog = Some(party_dialog_for_call.clone());
                    c.a_leg.sdp = Some(party_sdp_now.clone());
                }
            }
            c.held = None;
            if c.state == fcp_b2bua_core::CallState::Parked {
                c.unpark();
            }
        });
        if updated.is_err() {
            return fail(
                481,
                "Call/Transaction Does Not Exist",
                "the call ended".into(),
            )
            .await;
        }
        let _ = self.dialog_manager.insert(party_dialog.clone());
        let _ = self.dialog_manager.insert(new_dialog.clone());
        self.start_session_timer(&new_dialog);
        self.sip_call_id_map
            .insert(new_dialog.id().call_id().to_string(), call.id.to_string());
        handle.send_final(response).await;
        if let Some((old_dialog, flow)) = old {
            self.sip_call_id_map.remove(old_dialog.id().call_id());
            self.drop_leg(old_dialog, flow);
        }

        self.end_joining_call(feature_call_id, call, label).await;
        info!(call_id = %call.id, side = side.as_str(), label, "A leg taken over");
        Ok(())
    }

    /// The INVITE's own call, once it has joined `call`: its media session
    /// (if it made one) freed, its record pointing at the call it joined.
    async fn end_joining_call(
        &self,
        feature_call_id: &fcp_b2bua_core::CallId,
        call: &fcp_b2bua_core::Call,
        label: &str,
    ) {
        if let Some(own) = self.call_manager.get_call(feature_call_id) {
            if let (true, Some(session_id), Some(media_manager)) = (
                own.media_anchored,
                own.media_session_id,
                self.media_manager.as_ref(),
            ) {
                if Some(session_id) != call.media_session_id {
                    let _ = media_manager.terminate_session(&session_id).await;
                }
            }
        }
        let _ = self.call_manager.update_call(feature_call_id, |c| {
            c.correlation_id = Some(fcp_core::CallId::from_uuid(*call.id.as_uuid()));
        });
        let _ = self.call_manager.terminate_call_with(
            feature_call_id,
            fcp_cdr::TerminationReason::Completed {
                detail: format!("{label} completed"),
            },
        );
    }

    /// Handle park call (*70): park the current call in the next available orbit.
    ///
    /// 1. Find the caller's active (Established) call via `find_established_by_caller`
    /// 2. Park that call in the PBX parking manager
    /// 3. BYE the B-leg dialog
    /// 4. Put the A-leg on hold via re-INVITE with a=sendonly SDP
    /// 5. Respond 200 OK to the feature-code call, then terminate it
    async fn handle_park_call(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
    ) -> Result<()> {
        let pbx = match self.pbx_handler.as_ref() {
            Some(p) => p,
            None => {
                let response = self.create_simple_response(request, 500, "PBX Not Configured");
                handle.send_final(response).await;
                return Ok(());
            }
        };

        let caller_ext = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();

        // The dialer's established call, on either leg: the party at the
        // other end is parked, the dialer's own leg dropped
        // (`docs/ENTERPRISE_CALLING.md` §3.2).
        let Some((active_call, parker_side)) =
            self.call_manager.find_established_by_party(&caller_ext)
        else {
            info!(call_id = %call_id, caller = %caller_ext, "Park: no active call found for caller");
            let response = self.create_simple_response(request, 480, "No Active Call");
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(call_id, Some("Park: no active call".to_string()));
            return Ok(());
        };
        let parker_dialog = match parker_side {
            fcp_b2bua_core::LegSide::A => active_call.a_leg.dialog.clone(),
            fcp_b2bua_core::LegSide::B => active_call.b_leg.as_ref().and_then(|b| b.dialog.clone()),
        };
        let parker_flow = match parker_side {
            fcp_b2bua_core::LegSide::A => active_call.a_leg.flow_token.clone(),
            fcp_b2bua_core::LegSide::B => active_call
                .b_leg
                .as_ref()
                .and_then(|b| b.flow_token.clone()),
        };
        let orbit = match self
            .park_party(pbx, &active_call, parker_side, &caller_ext, None)
            .await
        {
            Ok(orbit) => orbit,
            Err(e) => {
                warn!(call_id = %active_call.id, error = %e, "Failed to park call");
                let response =
                    self.create_simple_response(request, 503, "No Parking Orbits Available");
                handle.send_final(response).await;
                let _ = self
                    .call_manager
                    .terminate_call(call_id, Some(format!("Park failed: {}", e)));
                return Ok(());
            }
        };
        if let Some(dialog) = parker_dialog {
            self.drop_leg(dialog, parker_flow);
        }

        // The parker is told the orbit, then hung up on. A bare 200 told
        // only a reason phrase no phone shows, answered an offer with no
        // SDP, and left the phone on a call nobody ended
        // (`docs/DEEP_TESTING.md` D8).
        let Some(flow_handler) = self.flow_handler.clone() else {
            let mut response =
                self.create_simple_response(request, 200, &format!("Parked Orbit {orbit}"));
            let _ = response.headers_mut().push(
                smol_str::SmolStr::new("X-FCP-Parked"),
                smol_str::SmolStr::new(&orbit),
            );
            handle.send_final(response).await;
            let _ = self.call_manager.terminate_call(call_id, None);
            return Ok(());
        };
        self.start_inline_flow(
            request,
            handle,
            call_id,
            &flow_handler,
            park_announcement(call_id, &orbit),
            HashMap::new(),
            vec![("X-FCP-Parked", orbit.clone())],
            "park announcement",
        )
        .await
    }

    /// Handle retrieve parked call (*71orbit): retrieve a call from a parking orbit.
    ///
    /// 1. Retrieve the ParkedCall from PBX parking manager
    /// 2. Verify the call is still in Parked state
    /// 3. Take the A-leg off hold via re-INVITE with a=sendrecv SDP
    /// 4. Look up the retriever's contact via registrar
    /// 5. Send outbound INVITE to retriever (new B-leg)
    /// 6. On success, unpark the call; on failure, re-park or terminate
    async fn handle_retrieve_parked(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        slot: &str,
    ) -> Result<()> {
        let pbx = match self.pbx_handler.as_ref() {
            Some(p) => p,
            None => {
                let response = self.create_simple_response(request, 500, "PBX Not Configured");
                handle.send_final(response).await;
                return Ok(());
            }
        };
        // The retriever's tenant's slot: another tenant's `701` is not
        // theirs to take (`docs/PARKING.md` §1).
        let Some(tenant) = self.call_manager.get_call(call_id).map(|c| c.tenant_id) else {
            let response = self.create_simple_response(request, 480, "Call Not Found");
            handle.send_final(response).await;
            return Ok(());
        };
        let slot = slot.to_string();

        // Retrieve from local parking manager (removes it from the slot)
        let parked = match pbx.retrieve_parked_call(&tenant, &slot) {
            Ok(p) => {
                // Also remove from cluster store (fire-and-forget)
                if let Some(ref cp) = self.cluster_parking {
                    let cp = Arc::clone(cp);
                    let slot = slot.clone();
                    tokio::spawn(async move {
                        if let Err(e) = cp.cluster_unpark(&tenant, &slot).await {
                            tracing::warn!(slot = %slot, error = %e, "Failed to clean up cluster parking entry");
                        }
                    });
                }
                p
            }
            Err(_local_err) => {
                // Not here: the cluster store says whether another node has it
                if let Some(ref cp) = self.cluster_parking {
                    match cp.cluster_lookup(&tenant, &slot).await {
                        Ok(Some(remote)) if remote.node_id != self.node_id => {
                            info!(
                                slot = %slot,
                                remote_node = %remote.node_id,
                                call_id = %remote.call_id,
                                "Parked call is on remote node — cross-node pickup not yet supported inline"
                            );
                            let response = self.create_simple_response(
                                request,
                                480,
                                &format!("Parked on node {}", remote.node_id),
                            );
                            handle.send_final(response).await;
                            let _ = self.call_manager.terminate_call(
                                call_id,
                                Some(format!("Parked call on remote node {}", remote.node_id)),
                            );
                            return Ok(());
                        }
                        Ok(Some(_stale)) => {
                            // Redis says this node owns it, but it is not held here:
                            // a claim a restart left behind.
                            warn!(slot = %slot, "Stale cluster parking entry for this node, cleaning up");
                            let cp = Arc::clone(cp);
                            let slot = slot.clone();
                            tokio::spawn(async move {
                                if let Err(e) = cp.cluster_unpark(&tenant, &slot).await {
                                    tracing::warn!(slot = %slot, error = %e, "Failed to clean up cluster parking entry");
                                }
                            });
                        }
                        _ => {} // Not found in cluster either, or error
                    }
                }

                info!(call_id = %call_id, slot = %slot, "Failed to retrieve parked call (not found locally or in cluster)");
                let response = self.create_simple_response(request, 480, "Orbit Not Occupied");
                handle.send_final(response).await;
                let _ = self
                    .call_manager
                    .terminate_call(call_id, Some("Orbit not occupied".to_string()));
                return Ok(());
            }
        };

        let parked_call_id = parked.call_id;

        // Verify the call is still in Parked state
        let parked_call = match self.call_manager.get_call(&parked_call_id) {
            Some(c) if c.state == fcp_b2bua_core::CallState::Parked => c,
            Some(c) => {
                warn!(
                    call_id = %parked_call_id,
                    state = ?c.state,
                    "Parked call is no longer in Parked state"
                );
                let response = self.create_simple_response(request, 480, "Call No Longer Parked");
                handle.send_final(response).await;
                let _ = self
                    .call_manager
                    .terminate_call(call_id, Some("Parked call state mismatch".to_string()));
                return Ok(());
            }
            None => {
                warn!(call_id = %parked_call_id, "Parked call not found in call manager");
                let response = self.create_simple_response(request, 480, "Parked Call Not Found");
                handle.send_final(response).await;
                let _ = self
                    .call_manager
                    .terminate_call(call_id, Some("Parked call not found".to_string()));
                return Ok(());
            }
        };

        info!(
            call_id = %parked_call_id,
            slot = %slot,
            "Retrieved parked call, bridging to retriever"
        );

        // The retriever's own INVITE becomes the call's second leg: the
        // parked party, off hold, and the retriever talk on the call that
        // was parked (`docs/ENTERPRISE_CALLING.md` §3.2).
        if parked_call.b_leg.is_some() {
            warn!(call_id = %parked_call_id, "A parked call still has a second leg");
        }
        self.take_over_leg(
            request,
            handle,
            call_id,
            &parked_call,
            fcp_b2bua_core::LegSide::B,
            None,
            "Retrieve",
        )
        .await
    }

    /// Terminate the A-leg of a parked call (BYE + cleanup).
    async fn terminate_parked_a_leg(&self, call_id: &fcp_b2bua_core::CallId) {
        if let Some(call) = self.call_manager.get_call(call_id) {
            if let Some(uac) = &self.uac {
                if let Some(a_dialog) = &call.a_leg.dialog {
                    self.spawn_bye_with_retry(
                        Arc::clone(uac),
                        a_dialog.clone(),
                        *call_id,
                        true,
                        None, // A-leg doesn't use flow tokens
                    );
                }
            }
            let _ = self
                .call_manager
                .terminate_call(call_id, Some("Parked call terminated".to_string()));
        }
    }

    /// Route an expired parked call back to the parker's extension.
    ///
    /// Called by the parking timeout monitor when a parked call expires.
    /// Looks up the parker's registered contact and routes the call back to them.
    /// If the parker is not registered, the call is terminated.
    pub async fn route_expired_parked_call(
        &self,
        parked: &fcp_pbx::ParkedCall,
        call_manager: &fcp_b2bua_core::CallManager,
    ) -> Result<()> {
        let parked_call_id = parked.call_id;
        let parker_ext = &parked.parker_extension;
        // The slot is free again for the cluster as it is here.
        if let Some(cp) = self.cluster_parking.as_ref() {
            if let Err(e) = cp.cluster_unpark(&parked.tenant_id, &parked.slot).await {
                warn!(slot = %parked.slot, error = %e, "Failed to free an expired call's cluster slot");
            }
        }
        if !call_manager
            .get_call(&parked_call_id)
            .is_some_and(|c| c.state == fcp_b2bua_core::CallState::Parked)
        {
            debug!(call_id = %parked_call_id, "Expired parked call is gone or no longer parked, skipping");
            return Ok(());
        }
        info!(
            call_id = %parked_call_id,
            slot = %parked.slot,
            parker = %parker_ext,
            "Parked call expired, routing back to parker"
        );

        // The lot's fallback, should the parker not take it back.
        let lot = self
            .parking_lots_of(&parked.tenant_id)
            .await
            .iter()
            .find(|l| l.id == parked.lot_id)
            .cloned();
        let fallback = match lot.as_ref().and_then(|l| l.fallback.as_ref()) {
            Some(target) => {
                self.parking_fallback_uri(&parked_call_id, target, parker_ext, &self.own_host())
                    .await
            }
            None => None,
        };
        self.call_manager.publish(fcp_events::PlatformEvent::Call(
            fcp_events::CallEvent::ParkTimedOut {
                tenant_id: Some(parked.tenant_id),
                call_id: fcp_core::CallId::from_uuid(*parked_call_id.as_uuid()),
                slot: parked.slot.clone(),
                lot_id: parked.lot_id.clone(),
                parker_extension: parker_ext.clone(),
                fallback: fallback.clone(),
                timestamp: std::time::SystemTime::now(),
            },
        ));
        self.send_parked_call(parked, parker_ext, fallback).await;
        Ok(())
    }

    /// This node's SIP host and port, where a number dialed is a new call
    /// into its own plan.
    fn own_host(&self) -> String {
        match self.uas.local_uri.port() {
            Some(port) => format!("{}:{}", self.uas.local_uri.host(), port),
            None => self.uas.local_uri.host().to_string(),
        }
    }

    /// Ring `target` (an extension or a user, dialed through this node so
    /// their own plan applies) with a parked call, shown as "Parked
    /// <slot>"; answering takes it. If nobody takes it, `fallback` is
    /// tried, else the call ends (`docs/ENTERPRISE_CALLING.md` §3.2,
    /// `docs/PARKING.md` §3).
    async fn send_parked_call(
        &self,
        parked: &fcp_pbx::ParkedCall,
        target: &str,
        fallback: Option<String>,
    ) {
        let parked_call_id = parked.call_id;
        let Some(parked_call) = self
            .call_manager
            .get_call(&parked_call_id)
            .filter(|c| c.state == fcp_b2bua_core::CallState::Parked)
        else {
            return;
        };
        let Some(uac) = self.uac.clone() else {
            warn!(call_id = %parked_call_id, "No UAC configured, terminating parked call");
            self.terminate_parked_a_leg(&parked_call_id).await;
            return;
        };
        if self
            .conference_migrations
            .insert(parked_call_id, ())
            .is_some()
        {
            return;
        }
        // Whoever rings sees whose call it is and the slot it waited in.
        let caller = parked_call
            .original_invite
            .as_ref()
            .and_then(|invite| extract_from_uri(invite).ok().map(|uri| (invite, uri)))
            .map(|(invite, uri)| {
                let name = match extract_from_display_name(invite) {
                    Some(name) => format!("Parked {}: {name}", parked.slot),
                    None => format!("Parked {}", parked.slot),
                };
                (Some(name), normalize_from_uri(uri))
            });
        let run = crate::transfer::TransferRun {
            uac,
            local_uri: self.uas.local_uri.clone(),
            call_manager: self.call_manager.clone(),
            dialog_manager: Arc::clone(&self.dialog_manager),
            session_timer_manager: Arc::clone(&self.session_timer_manager),
            media_manager: self.media_manager.clone(),
            sip_call_id_map: Arc::clone(&self.sip_call_id_map),
            in_flight: Arc::clone(&self.conference_migrations),
            b_leg_keys: Arc::clone(&self.b_leg_keys),
            b2bua_config: self.b2bua_config.clone(),
            local_addr: self.local_addr.clone(),
            call_id: parked_call_id,
            referrer: fcp_b2bua_core::LegSide::B,
            referrer_dialog: None,
            target: format!("sip:{}@{}", target, self.own_host()),
            headers: vec![(
                TRANSFERRED_FROM_HEADER.to_string(),
                parked_call_id.to_string(),
            )],
            caller,
            notify_referrer: true,
            fallback,
        };
        run.run().await;
    }

    /// The tenant's calls parked on this node, as `GET /parking/calls`
    /// lists them (`docs/PARKING.md` §4).
    pub fn parked_calls(&self, tenant_id: &fcp_core::TenantId) -> Vec<serde_json::Value> {
        let Some(pbx) = self.pbx_handler.as_ref() else {
            return Vec::new();
        };
        pbx.parking_manager()
            .list(tenant_id)
            .into_iter()
            .map(|parked| {
                let call = self.call_manager.get_call(&parked.call_id);
                let caller = call.as_ref().and_then(|c| {
                    c.original_invite.as_ref().and_then(|invite| {
                        extract_from_uri(invite).ok().map(|uri| {
                            serde_json::json!({
                                "uri": normalize_from_uri(uri).as_str(),
                                "name": extract_from_display_name(invite),
                            })
                        })
                    })
                });
                let waited = parked.parked_at.elapsed().as_secs();
                serde_json::json!({
                    "slot": parked.slot,
                    "lot_id": parked.lot_id,
                    "call_id": parked.call_id.to_string(),
                    "caller": caller,
                    "parker_extension": parked.parker_extension,
                    "parked_secs": waited,
                    "timeout_secs": parked.timeout.as_secs(),
                    "node_id": self.node_id,
                })
            })
            .collect()
    }

    /// Take the tenant's parked call out of its slot and ring `target`
    /// with it, in the background. `Err` with the status the API answers:
    /// 404 for an empty slot.
    pub async fn retrieve_parked_to(
        self: &Arc<Self>,
        tenant_id: &fcp_core::TenantId,
        slot: &str,
        target: &str,
    ) -> std::result::Result<String, (u16, String)> {
        let parked = self.take_parked(tenant_id, slot).await?;
        let call_id = parked.call_id.to_string();
        info!(call_id = %call_id, slot, target, "A parked call sent on");
        let handler = Arc::clone(self);
        let target = target.to_string();
        tokio::spawn(async move { handler.send_parked_call(&parked, &target, None).await });
        Ok(call_id)
    }

    /// Hang up the tenant's call parked in `slot`.
    pub async fn hang_up_parked(
        &self,
        tenant_id: &fcp_core::TenantId,
        slot: &str,
    ) -> std::result::Result<String, (u16, String)> {
        let parked = self.take_parked(tenant_id, slot).await?;
        info!(call_id = %parked.call_id, slot, "A parked call hung up");
        self.terminate_parked_a_leg(&parked.call_id).await;
        Ok(parked.call_id.to_string())
    }

    /// The parked call in the tenant's slot on this node, taken out of it
    /// here and in the cluster.
    async fn take_parked(
        &self,
        tenant_id: &fcp_core::TenantId,
        slot: &str,
    ) -> std::result::Result<fcp_pbx::ParkedCall, (u16, String)> {
        let pbx = self
            .pbx_handler
            .as_ref()
            .ok_or((503, "parking is not enabled on this node".to_string()))?;
        let parked = pbx
            .retrieve_parked_call(tenant_id, slot)
            .map_err(|_| (404, format!("No call is parked in {slot}")))?;
        if let Some(cp) = self.cluster_parking.as_ref() {
            if let Err(e) = cp.cluster_unpark(tenant_id, slot).await {
                warn!(slot, error = %e, "Failed to free a cluster parking slot");
            }
        }
        Ok(parked)
    }

    /// Where a lot's fallback is dialed for a parked call nobody took back:
    /// a number, an extension, a user or a queue at this node, so its own
    /// plan applies; an address as it is; `voicemail` the parker's mailbox.
    async fn parking_fallback_uri(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        target: &fcp_core::CallTarget,
        parker: &str,
        host: &str,
    ) -> Option<String> {
        match target {
            fcp_core::CallTarget::Voicemail => {
                let server = self
                    .voicemail_server_uri_override
                    .read()
                    .ok()
                    .and_then(|o| o.clone())
                    .or_else(|| self.b2bua_config.voicemail_server_uri.clone())?;
                let vm_host = server
                    .trim_start_matches("sips:")
                    .trim_start_matches("sip:")
                    .rsplit('@')
                    .next()
                    .unwrap_or(&server)
                    .to_string();
                Some(format!("sip:{parker}@{vm_host};reason=no-answer"))
            }
            fcp_core::CallTarget::Sip(uri) => Some(uri.clone()),
            other => {
                let destination = self.forward_destination(*call_id, other).await?;
                if destination.starts_with("sip:") || destination.starts_with("sips:") {
                    return Some(destination);
                }
                if destination.contains("~") || destination.starts_with("flow:") {
                    warn!(call_id = %call_id, target = %other.as_string(), "A parking lot's fallback cannot be dialed");
                    return None;
                }
                Some(format!("sip:{destination}@{host}"))
            }
        }
    }

    /// Handle intercom (*80ext / *81ext): send INVITE with auto-answer headers.
    async fn handle_intercom(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        target_extension: &str,
        mode: fcp_pbx::IntercomMode,
    ) -> Result<()> {
        let pbx = match self.pbx_handler.as_ref() {
            Some(p) => p,
            None => {
                let response = self.create_simple_response(request, 500, "PBX Not Configured");
                handle.send_final(response).await;
                return Ok(());
            }
        };
        let params = pbx.build_intercom_params(target_extension, mode);

        info!(
            call_id = %call_id,
            target = %params.target_extension,
            mode = ?params.mode,
            extra_headers = params.extra_headers.len(),
            "Intercom: routing to extension with auto-answer headers"
        );

        let uac = match &self.uac {
            Some(uac) => Arc::clone(uac),
            None => {
                let response = self.create_simple_response(request, 500, "UAC Not Configured");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    call_id,
                    Some("Intercom failed: UAC not configured".to_string()),
                );
                return Ok(());
            }
        };

        let routed_request = self.build_feature_route_request(
            request,
            &params.target_extension,
            &params.extra_headers,
        )?;
        let _ = self.call_manager.update_call(call_id, |call| {
            call.original_callee = Some(params.target_extension.clone());
        });

        self.forward_invite_to_b_leg(&routed_request, handle, *call_id, uac)
            .await
    }

    /// Handle agent login feature code (*50).
    async fn handle_agent_login(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
    ) -> Result<()> {
        let queue_handler = match self.queue_handler.as_ref() {
            Some(q) => q,
            None => {
                let response = self.create_simple_response(request, 500, "Queue Not Configured");
                handle.send_final(response).await;
                return Ok(());
            }
        };

        let caller_ext = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();

        let tenant_id = match self.resolve_runtime_tenant(request, call_id).await {
            Ok(tenant_id) => tenant_id,
            Err(e) => {
                warn!(call_id = %call_id, extension = %caller_ext, error = %e, "Agent login failed: tenant resolution failed");
                let status = if e.to_string().contains("unknown tenant domain")
                    || e.to_string().contains("tenant context required")
                {
                    403
                } else {
                    503
                };
                let reason = if status == 403 {
                    "Tenant context required"
                } else {
                    "Tenant lookup unavailable"
                };
                let response = self.create_simple_response(request, status, reason);
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    call_id,
                    Some("Agent login tenant resolution failed".to_string()),
                );
                return Ok(());
            }
        };

        match queue_handler.agent_login(&caller_ext, &tenant_id).await {
            Ok(()) => {
                info!(call_id = %call_id, extension = %caller_ext, "Agent logged in via feature code");
                return self
                    .confirm_feature(request, handle, call_id, "Agent logged in.")
                    .await;
            }
            Err(e) => {
                warn!(call_id = %call_id, extension = %caller_ext, error = %e, "Agent login failed");
                let response = self.create_simple_response(request, 404, "Agent Not Found");
                handle.send_final(response).await;
            }
        }

        let _ = self
            .call_manager
            .terminate_call(call_id, Some("Agent login feature code".to_string()));
        Ok(())
    }

    /// Handle agent logout feature code (*51).
    async fn handle_agent_logout(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
    ) -> Result<()> {
        let queue_handler = match self.queue_handler.as_ref() {
            Some(q) => q,
            None => {
                let response = self.create_simple_response(request, 500, "Queue Not Configured");
                handle.send_final(response).await;
                return Ok(());
            }
        };

        let caller_ext = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();

        match queue_handler.agent_logout(&caller_ext).await {
            Ok(()) => {
                info!(call_id = %call_id, extension = %caller_ext, "Agent logged out via feature code");
                return self
                    .confirm_feature(request, handle, call_id, "Agent logged out.")
                    .await;
            }
            Err(e) => {
                warn!(call_id = %call_id, extension = %caller_ext, error = %e, "Agent logout failed");
                let response = self.create_simple_response(request, 404, "Agent Not Found");
                handle.send_final(response).await;
            }
        }

        let _ = self
            .call_manager
            .terminate_call(call_id, Some("Agent logout feature code".to_string()));
        Ok(())
    }

    /// Handle voicemail access feature code (*98).
    ///
    /// Builds an inline VoicemailAccess flow and delegates to the flow handler.
    /// If `mailbox` is None, the caller's own extension is used.
    async fn handle_voicemail_access_feature(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        mailbox: Option<String>,
    ) -> Result<()> {
        let flow_handler = match &self.flow_handler {
            Some(fh) => Arc::clone(fh),
            None => {
                let response =
                    self.create_simple_response(request, 501, "Voicemail Access Not Available");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(
                    call_id,
                    Some("No flow handler for voicemail access".to_string()),
                );
                return Ok(());
            }
        };

        // The target mailbox: the one dialled, else the caller's own (found
        // below), else the caller's extension as a mailbox's.
        let mut mailbox_str = mailbox.unwrap_or_default();

        let sip_username = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();

        // Resolve the actual extension from the User record (the SIP username
        // may differ from the extension, e.g. "polycom" vs "1556").
        let caller_ext = if let Some(storage) = &self.storage {
            match self.resolve_runtime_tenant(request, call_id).await {
                Ok(tenant_id) => {
                    // A host phone's guest hears their own mailbox (§3.8).
                    let user = match crate::hoteling::caller_user(
                        storage,
                        &tenant_id,
                        &sip_username,
                    )
                    .await
                    {
                        Some(user_id) => storage.get_user_by_id(&user_id).await.ok().flatten(),
                        None => None,
                    };
                    match user {
                        Some(user) => {
                            // The user's own mailbox, whatever its extension
                            // (`docs/DEEP_TESTING.md` D8: alice's mailbox,
                            // made as `alice`, was looked for as `1001`).
                            if mailbox_str.is_empty() {
                                if let Ok(Some(own)) =
                                    fcp_core::storage::VoicemailStore::get_mailbox_by_user(
                                        storage.as_ref(),
                                        &user.id,
                                    )
                                    .await
                                {
                                    mailbox_str = own.id;
                                }
                            }
                            user.extension
                                .clone()
                                .unwrap_or_else(|| user.sip_name().to_string())
                        }
                        None => sip_username.clone(),
                    }
                }
                Err(_) => sip_username.clone(),
            }
        } else {
            sip_username.clone()
        };

        // Build an inline voicemail access flow: Answer → VoicemailAccess → Hangup
        let flow_id = format!("vm-access-{}", call_id);
        let flow_def = {
            use fcp_flow::{FlowDefinition, FlowNode, NodeType};

            let mut nodes = std::collections::HashMap::new();

            nodes.insert(
                "answer".to_string(),
                FlowNode {
                    id: "answer".to_string(),
                    name: Some("Answer call".to_string()),
                    node_type: NodeType::Answer {
                        next: "vm_access".to_string(),
                        on_error: None,
                    },
                },
            );

            nodes.insert(
                "vm_access".to_string(),
                FlowNode {
                    id: "vm_access".to_string(),
                    name: Some("Voicemail access IVR".to_string()),
                    node_type: NodeType::VoicemailAccess {
                        mailbox: mailbox_str,
                        max_pin_attempts: 3,
                        next: "hangup".to_string(),
                        on_auth_failure: Some("hangup".to_string()),
                    },
                },
            );

            nodes.insert(
                "hangup".to_string(),
                FlowNode {
                    id: "hangup".to_string(),
                    name: Some("Hang up".to_string()),
                    node_type: NodeType::Hangup { reason: None },
                },
            );

            FlowDefinition {
                id: flow_id.clone(),
                name: "Voicemail Access".to_string(),
                description: Some("Inline voicemail access flow from *98 feature code".to_string()),
                version: 1,
                initial_node: "answer".to_string(),
                nodes,
                variables: std::collections::HashMap::new(),
                tenant_id: None,
                enabled: true,
                created_at: None,
                updated_at: None,
            }
        };

        // Build initial variables
        let mut initial_variables = HashMap::new();
        initial_variables.insert("caller".to_string(), caller_ext.clone());
        initial_variables.insert("caller_extension".to_string(), caller_ext);

        self.start_inline_flow(
            request,
            handle,
            call_id,
            &flow_handler,
            flow_def,
            initial_variables,
            Vec::new(),
            "voicemail access",
        )
        .await
    }

    /// `Some` when `call_id` is an emergency test its tenant has this
    /// platform answer, with what answering it came to; `None` for every
    /// other call.
    async fn read_back_local_test(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        tenant_id: fcp_core::TenantId,
    ) -> Option<Result<()>> {
        let em = self.call_manager.get_call(call_id)?.emergency?;
        if em.kind != fcp_core::e911::EmergencyKind::Test {
            return None;
        }
        let config = crate::emergency::config_for(self.storage.as_ref(), &tenant_id).await;
        if config.test_call_mode != fcp_core::e911::TestCallMode::Local {
            return None;
        }
        Some(
            self.answer_emergency_test(request, handle, call_id, tenant_id, &em, &config)
                .await,
        )
    }

    /// Answer an emergency test call here and read back what a live call
    /// placed now would carry (`TestCallMode::Local`): the location and its
    /// source, the callback number, the emergency routes in order and how
    /// each takes the location, the contacts told. The record is completed
    /// as `read_back`, with no trunk.
    async fn answer_emergency_test(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        tenant_id: fcp_core::TenantId,
        em: &fcp_b2bua_core::EmergencyCallInfo,
        config: &fcp_core::e911::EmergencyConfig,
    ) -> Result<()> {
        let Some(flow_handler) = self.flow_handler.clone() else {
            // Nothing here can speak: refused rather than sent to a carrier
            // the tenant did not ask to test.
            warn!(call_id = %call_id, "Emergency test call refused: this node runs no flows to read it back");
            let response = self.create_simple_response(request, 503, "Service Unavailable");
            handle.send_final(response).await;
            let _ = self.call_manager.terminate_call_with(
                call_id,
                fcp_cdr::TerminationReason::rejected(503, "no flows to read a test call back"),
            );
            return Ok(());
        };
        let routes = match &self.outbound_route_provider {
            Some(provider) => {
                provider
                    .resolve_emergency_ordered(Some(tenant_id), &request.uri().to_string())
                    .await
            }
            None => Vec::new(),
        };
        let told: Vec<_> = crate::emergency::contacts_for_call(
            self.storage.as_ref(),
            config,
            em.location.as_ref(),
        )
        .await
        .into_iter()
        .filter(|c| c.problems().is_empty())
        .collect();
        let listeners = told
            .iter()
            .filter(|c| c.listen && c.kind == fcp_core::e911::notify::ContactKind::Call)
            .count();
        let parts =
            crate::emergency::test_readback(em, !routes.is_empty(), told.len(), listeners, true);
        // Synthesized while the opening plays: the caller would otherwise
        // wait out the whole readback's synthesis after the answer.
        if let Some(tts) = self.media_manager.as_ref().and_then(|mm| mm.tts_service()) {
            tts.warm(parts.clone(), Some("en-US"));
        }
        info!(
            call_id = %call_id,
            routes = routes.len(),
            located = em.location.is_some(),
            callback = em.callback_number.as_deref().unwrap_or("none"),
            "Emergency test call read back here"
        );
        let _ = self.call_manager.update_call(call_id, |call| {
            if let Some(em) = call.emergency.as_mut() {
                em.read_back = true;
                em.leg = Some(fcp_b2bua_core::EmergencyLeg {
                    outcome: "read_back".to_string(),
                    sip_status: None,
                    answered_at: Some(std::time::SystemTime::now()),
                });
            }
        });
        let mut flow = spoken_confirmation(
            &format!("emergency-test-{call_id}"),
            "Emergency test",
            "Inline flow reading back an emergency test call",
            parts,
        );
        if listeners > 0 {
            wait_before_last_part(&mut flow, crate::emergency::LISTEN_TEST_WAIT_SECS);
        }
        self.start_inline_flow(
            request,
            handle,
            call_id,
            &flow_handler,
            flow,
            HashMap::new(),
            Vec::new(),
            "emergency test",
        )
        .await
    }

    /// A feature code done: the caller is told so, then hung up on. A bare
    /// 200 answered an offer with no SDP and left the phone on a call
    /// nobody ended (`docs/DEEP_TESTING.md` D8).
    async fn confirm_feature(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        text: &str,
    ) -> Result<()> {
        let Some(flow_handler) = self.flow_handler.clone() else {
            let response = self.create_simple_response(request, 200, "OK");
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(call_id, Some("Feature code".to_string()));
            return Ok(());
        };
        self.start_inline_flow(
            request,
            handle,
            call_id,
            &flow_handler,
            feature_confirmation(call_id, text),
            HashMap::new(),
            Vec::new(),
            "feature confirmation",
        )
        .await
    }

    /// Answer the call into `flow_def`, inserted for the call alone:
    /// what a feature code that talks to the caller runs (`*98`, `*88`).
    #[allow(clippy::too_many_arguments)]
    async fn start_inline_flow(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        flow_handler: &Arc<crate::flow_handler::FlowCallHandler>,
        flow_def: fcp_flow::FlowDefinition,
        initial_variables: HashMap<String, String>,
        answer_headers: Vec<(&'static str, String)>,
        what: &str,
    ) -> Result<()> {
        // Held for the call, not written: the call manager lets it go when
        // the call ends.
        let flow_id = flow_def.id.clone();
        if let Err(e) = flow_handler.flow_store().hold_flow(flow_def).await {
            warn!(call_id = %call_id, error = %e, what, "Failed to insert an inline flow");
            let response = self.create_simple_response(request, 500, "Internal Error");
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(call_id, Some(format!("Failed to create the {what} flow")));
            return Ok(());
        }

        // Build answer callback
        let answer_call_id = *call_id;
        let answer_request = request.clone();
        let answer_handle = handle.clone();
        let answer_call_manager = Arc::clone(&self.call_manager);
        let answer_dialog_manager = Arc::clone(&self.dialog_manager);
        let answer_ack_waits = Arc::clone(&self.ack_waits);
        let answer_media_manager = self.media_manager.clone();
        let answer_local_addr = self.local_addr.clone();
        let answer_rtp_port = self.rtp_port;
        let answer_media_profile = self.media_profile.clone();
        let answer_local_uri = self.uas.local_uri.clone();
        // A 2xx to an INVITE carries a Contact (RFC 3261 §12.1.1): where the
        // caller's ACK and BYE go.
        let answer_contact = format!("<{}>", self.uas.contact_uri.as_str());
        let answer_headers = Arc::new(answer_headers);

        let answer_callback: fcp_flow::AnswerCallback = Arc::new(move || {
            let request = answer_request.clone();
            let handle = answer_handle.clone();
            let call_manager = Arc::clone(&answer_call_manager);
            let dialog_manager = Arc::clone(&answer_dialog_manager);
            let ack_waits = Arc::clone(&answer_ack_waits);
            let local_uri = answer_local_uri.clone();
            let media_manager = answer_media_manager.clone();
            let local_addr = answer_local_addr.clone();
            let rtp_port = answer_rtp_port;
            let media_profile = answer_media_profile.clone();
            let call_id = answer_call_id;
            let contact = answer_contact.clone();
            let extra_headers = Arc::clone(&answer_headers);
            Box::pin(async move {
                // Check if a media session already exists (created by evaluate_media_anchoring)
                let mut actual_rtp_port = rtp_port;
                // The session's own answer to the offer: the codec it will
                // send in, first. An answer built apart from it listed our
                // codecs in our order, so a phone offering G.722 first heard
                // the session's G.722 decoded as PCMU (`docs/DEEP_TESTING.md`
                // D8: static on every prompt of `*98`).
                let mut session_answer: Option<String> = None;
                let existing_session_id = call_manager
                    .get_call(&call_id)
                    .and_then(|c| c.media_session_id);

                if let Some(mm) = &media_manager {
                    if let Some(ref existing_msid) = existing_session_id {
                        // Reuse existing session — just feed it the remote SDP
                        if let Ok(session) = mm.get_session(existing_msid).await {
                            actual_rtp_port = session.ports_for(fcp_media::backend::Leg::A).0;
                        }
                        if let Some(remote_sdp) = crate::body::request_sdp(&request) {
                            session_answer =
                                mm.update_session(existing_msid, remote_sdp).await.ok();
                        }
                    } else {
                        // No existing session — create one
                        let core_call_id = fcp_core::CallId::from_uuid(*call_id.as_uuid());
                        let fcp_dialog_id = fcp_core::DialogId::new();
                        if let Ok(session) =
                            mm.create_session_default(core_call_id, fcp_dialog_id).await
                        {
                            actual_rtp_port = session.ports_for(fcp_media::backend::Leg::A).0;
                            if let Some(remote_sdp) = crate::body::request_sdp(&request) {
                                session_answer =
                                    mm.update_session(&core_call_id, remote_sdp).await.ok();
                            }
                            let _ = call_manager.update_call(&call_id, |c| {
                                c.media_session_id = Some(core_call_id);
                                c.media_anchored = true;
                            });
                        }
                    }
                }

                // Generate SDP with the actual RTP port from the media session
                let sdp_string = if session_answer.is_some() {
                    session_answer
                } else if let Some(profile) = media_profile {
                    let sdp = profile.build("fcp", &local_addr, actual_rtp_port, None);
                    Some(sdp.to_string())
                } else {
                    Some(generate_basic_sdp(&local_addr, actual_rtp_port))
                };

                // Build 200 OK with SDP
                let response = if let Some(ref sdp_str) = sdp_string {
                    let base = sip_uas::UserAgentServer::create_response(&request, 200, "OK");
                    let sdp_bytes = bytes::Bytes::from(sdp_str.clone());
                    let mut headers = base.headers().clone();
                    let _ = headers.push(
                        smol_str::SmolStr::new("Contact"),
                        smol_str::SmolStr::new(contact.as_str()),
                    );
                    let _ = headers.push(
                        smol_str::SmolStr::new("Content-Type"),
                        smol_str::SmolStr::new("application/sdp"),
                    );
                    let _ = headers.push(
                        smol_str::SmolStr::new("Content-Length"),
                        smol_str::SmolStr::new(sdp_bytes.len().to_string()),
                    );
                    sip_core::Response::new(base.start_line().clone(), headers, sdp_bytes)
                        .expect("Failed to create response with SDP")
                } else {
                    sip_uas::UserAgentServer::create_response(&request, 200, "OK")
                };
                // What the answer tells the phone besides (the orbit `*70`
                // parked on), for a phone to show.
                let mut response = response;
                for (name, value) in extra_headers.iter() {
                    let _ = response.headers_mut().push(
                        smol_str::SmolStr::new(*name),
                        smol_str::SmolStr::new(value.as_str()),
                    );
                }
                // The dialog before the 200 OK, as the IVR's answer does: the
                // flow's hangup needs it to send its BYE, and a caller's BYE
                // needs it to find the call.
                match sip_dialog::Dialog::new_uas(
                    &request,
                    &response,
                    local_uri,
                    extract_from_uri(&request).unwrap_or_else(|_| {
                        sip_core::SipUri::parse("sip:unknown@unknown").unwrap()
                    }),
                ) {
                    Some(dialog) => {
                        let _ = dialog_manager.insert(dialog.clone());
                        let _ = call_manager.update_call(&call_id, |call| {
                            call.a_leg.dialog = Some(dialog.clone());
                            call.establish();
                        });
                    }
                    None => {
                        warn!(call_id = %call_id, "Inline flow: no A-leg dialog from the 200 OK")
                    }
                }
                ack_waits.expect(call_id);
                handle.send_final(response).await;

                Ok(())
            })
        });

        // Build hangup callback
        let hangup_call_id = *call_id;
        let hangup_call_manager = Arc::clone(&self.call_manager);
        let hangup_uac = self.uac.clone();
        let hangup_dialog_manager = Arc::clone(&self.dialog_manager);
        let hangup_ack_waits = Arc::clone(&self.ack_waits);
        let hangup_media_manager = self.media_manager.clone();

        let hangup_callback: fcp_flow::HangupCallback = Arc::new(move |reason| {
            let call_id = hangup_call_id;
            let call_manager = Arc::clone(&hangup_call_manager);
            let uac_opt = hangup_uac.clone();
            let dialog_manager = Arc::clone(&hangup_dialog_manager);
            let ack_waits = Arc::clone(&hangup_ack_waits);
            let media_manager = hangup_media_manager.clone();
            Box::pin(async move {
                // Not before the caller has ACKed the answer (RFC 3261 §15).
                ack_waits.wait(&call_id, crate::ack_wait::ACK_WAIT).await;
                if let Some(call_entry) = call_manager.get_call(&call_id) {
                    if call_entry.media_anchored {
                        if let (Some(ref media_session_id), Some(ref mm)) =
                            (&call_entry.media_session_id, &media_manager)
                        {
                            let _ = mm.terminate_session(media_session_id).await;
                        }
                    }
                    if let Some(dialog) = call_entry.a_leg.dialog.clone() {
                        if let Some(uac) = uac_opt {
                            let _ = uac.bye(&dialog).await;
                            dialog_manager.remove(dialog.id());
                        }
                    }
                }
                let _ = call_manager.terminate_call(&call_id, reason);
                Ok(())
            })
        });

        // Start the flow
        if let Some(mut call_entry) = self.call_manager.get_call(call_id) {
            match flow_handler
                .start_flow(
                    &mut call_entry,
                    flow_id,
                    initial_variables,
                    answer_callback,
                    hangup_callback,
                )
                .await
            {
                Ok(()) => {
                    info!(call_id = %call_id, what, "Inline flow started");
                    // Send 183 while the flow spins up
                    let mut progress =
                        sip_uas::UserAgentServer::create_response(request, 183, "Session Progress");
                    self.ensure_response_hygiene(request, &mut progress);
                    handle.send_provisional(progress).await;
                }
                Err(e) => {
                    warn!(call_id = %call_id, error = %e, what, "Failed to start an inline flow");
                    let response = self.create_simple_response(request, 500, "Flow Start Failed");
                    handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call(call_id, Some(format!("The {what} flow failed: {e}")));
                }
            }
        } else {
            let response = self.create_simple_response(request, 500, "Call Not Found");
            handle.send_final(response).await;
        }

        Ok(())
    }

    /// Execute a hunt group plan (ring members in phases).
    /// Whether a hunt group member is a user in do not disturb
    /// (`docs/ENTERPRISE_CALLING.md` §3.1).
    async fn member_in_dnd(&self, call_id: &fcp_b2bua_core::CallId, member: &str) -> bool {
        let Some(storage) = self.storage.as_ref() else {
            return false;
        };
        let Some(tenant_id) = self.call_manager.get_call(call_id).map(|c| c.tenant_id) else {
            return false;
        };
        match crate::ring_plan::user_of(storage, &tenant_id, member).await {
            Some(user_id) => {
                crate::ring_plan::settings_for(storage, &tenant_id, &user_id)
                    .await
                    .dnd
            }
            None => false,
        }
    }

    async fn execute_hunt_group_plan(
        &self,
        request: &sip_core::Request,
        a_leg_handle: &ServerTransactionHandle,
        call_id: fcp_b2bua_core::CallId,
        plan: fcp_pbx::HuntGroupPlan,
        uac: Arc<SipUserAgentClient>,
    ) -> Result<()> {
        // For each phase, resolve member extensions to SIP contacts
        for (phase_idx, phase) in plan.ring_phases.iter().enumerate() {
            debug!(
                call_id = %call_id,
                phase = phase_idx,
                members = ?phase.members,
                simultaneous = phase.simultaneous,
                "Executing hunt group ring phase"
            );

            // Resolve member extensions to registered contacts
            let mut contacts = Vec::new();
            if let Some(registrar) = &self.registrar {
                for member in &phase.members {
                    if self.member_in_dnd(&call_id, member).await {
                        debug!(call_id = %call_id, member = %member, "Hunt group member in do not disturb, skipping");
                        continue;
                    }
                    let member_aor = format!("sip:{}@{}", member, self.local_addr);
                    match registrar.get_registrations(&member_aor).await {
                        Ok(bindings) if !bindings.is_empty() => {
                            for binding in bindings {
                                contacts.push((member.clone(), binding));
                            }
                        }
                        _ => {
                            debug!(
                                call_id = %call_id,
                                member = %member,
                                "Hunt group member not registered, skipping"
                            );
                        }
                    }
                }
            }

            // Filter out busy/DND members if skip_busy is enabled
            if plan.skip_busy {
                if let Some(ref presence) = self.presence_handler {
                    let before = contacts.len();
                    contacts.retain(|(member, _)| {
                        let busy = presence.is_busy(member);
                        if busy {
                            debug!(
                                call_id = %call_id,
                                member = %member,
                                "Skipping busy/DND hunt group member"
                            );
                        }
                        !busy
                    });
                    if contacts.len() < before {
                        debug!(
                            call_id = %call_id,
                            phase = phase_idx,
                            skipped = before - contacts.len(),
                            remaining = contacts.len(),
                            "Filtered busy hunt group members"
                        );
                    }
                }
            }

            if contacts.is_empty() {
                debug!(
                    call_id = %call_id,
                    phase = phase_idx,
                    "No registered members in this phase, trying next"
                );
                continue;
            }

            if phase.simultaneous && contacts.len() > 1 {
                // Use existing parallel forking infrastructure
                info!(
                    call_id = %call_id,
                    contacts = contacts.len(),
                    "Hunt group: forking to {} contacts simultaneously",
                    contacts.len()
                );

                // Build outbound calls for each contact
                // Note: Hunt group calls don't use flow tokens (RFC 5626) currently
                let mut b_leg_calls = Vec::new();
                for (member, binding) in &contacts {
                    let target_uri = binding.contact().to_string();
                    match self
                        .create_outbound_call(request, &target_uri, &uac, None)
                        .await
                    {
                        Ok(outbound_call) => {
                            b_leg_calls.push((
                                member.clone(),
                                outbound_call,
                                binding.flow_token().map(|s| s.to_string()),
                            ));
                        }
                        Err(e) => {
                            warn!(
                                call_id = %call_id,
                                member = %member,
                                error = %e,
                                "Failed to create outbound call for hunt group member"
                            );
                        }
                    }
                }

                if b_leg_calls.is_empty() {
                    continue;
                }

                // Race all B-legs using existing parallel forking
                let cancel_notify = Arc::new(tokio::sync::Notify::new());
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.cancel_notify = Some(Arc::clone(&cancel_notify));
                });

                return self
                    .handle_parallel_forking(
                        request.clone(),
                        Arc::new(a_leg_handle.clone()),
                        b_leg_calls,
                        call_id,
                        cancel_notify,
                        None,
                    )
                    .await;
            } else {
                // Sequential: try one contact at a time
                for (member, binding) in &contacts {
                    let target_uri = binding.contact().to_string();
                    info!(
                        call_id = %call_id,
                        member = %member,
                        target = %target_uri,
                        "Hunt group: trying member sequentially"
                    );

                    // Try single B-leg with timeout
                    match self
                        .create_outbound_call(request, &target_uri, &uac, None)
                        .await
                    {
                        Ok(outbound_call) => {
                            let cancel_notify = Arc::new(tokio::sync::Notify::new());
                            let _ = self.call_manager.update_call(&call_id, |call| {
                                call.cancel_notify = Some(Arc::clone(&cancel_notify));
                            });

                            // Include flow token for RFC 5626 BYE support
                            let flow_token = binding.flow_token().map(|s| s.to_string());
                            return self
                                .handle_parallel_forking(
                                    request.clone(),
                                    Arc::new(a_leg_handle.clone()),
                                    vec![(member.clone(), outbound_call, flow_token)],
                                    call_id,
                                    cancel_notify,
                                    None,
                                )
                                .await;
                        }
                        Err(e) => {
                            warn!(
                                call_id = %call_id,
                                member = %member,
                                error = %e,
                                "Failed to create outbound call, trying next member"
                            );
                        }
                    }
                }
            }
        }

        // All phases exhausted — route to no_answer_destination or 480
        if let Some(ref dest) = plan.no_answer_destination {
            info!(
                call_id = %call_id,
                destination = %dest,
                "Hunt group: no answer, routing to fallback destination"
            );
            let call = self.call_manager.get_call(&call_id);
            let callee = call.as_ref().and_then(|c| c.original_callee.as_deref());
            let a_handle = Arc::new(a_leg_handle.clone());
            match self
                .route_cfna(request, &a_handle, call_id, dest, callee, Some("no-answer"))
                .await
            {
                Ok(()) => return Ok(()),
                Err(e) => {
                    error!(call_id = %call_id, error = %e, "Hunt group CFNA routing failed");
                    // Fall through to 480
                }
            }
        }

        let response = self.create_simple_response(request, 480, "Temporarily Unavailable");
        a_leg_handle.send_final(response).await;
        let _ = self
            .call_manager
            .terminate_call_with(&call_id, fcp_cdr::TerminationReason::NoAnswer);
        Ok(())
    }

    // =========================================================================
    // Call Forward No Answer (CFNA)
    // =========================================================================

    /// What a schedule says now, when it is not open: its state, and the
    /// target for it (a holiday falls back to the closed target). `None`
    /// when open, or when the schedule is gone (a missing schedule never
    /// closes a number).
    async fn schedule_divert(
        &self,
        schedule_id: &fcp_core::ScheduleId,
        tenant: &fcp_core::TenantId,
        closed: Option<fcp_core::CallTarget>,
        holiday: Option<fcp_core::CallTarget>,
    ) -> Option<(fcp_core::ScheduleState, Option<fcp_core::CallTarget>)> {
        use fcp_core::ScheduleStore;
        let storage = self.storage.as_ref()?;
        let schedule = match storage.get_schedule(schedule_id).await {
            Ok(Some(schedule)) if schedule.tenant_id == *tenant => schedule,
            Ok(_) => {
                warn!(schedule_id = %schedule_id, "A schedule named by a route is gone; treating it as open");
                return None;
            }
            Err(e) => {
                warn!(schedule_id = %schedule_id, error = %e, "A schedule could not be read; treating it as open");
                return None;
            }
        };
        match schedule.state_now() {
            fcp_core::ScheduleState::Open => None,
            fcp_core::ScheduleState::Closed => Some((fcp_core::ScheduleState::Closed, closed)),
            fcp_core::ScheduleState::Holiday => {
                Some((fcp_core::ScheduleState::Holiday, holiday.or(closed)))
            }
        }
    }

    /// A database-configured queue's schedule for a call dialing it: the
    /// queue is the one whose extension the Request-URI names.
    async fn queue_schedule_divert(
        &self,
        request: &sip_core::Request,
        call_id: &fcp_b2bua_core::CallId,
    ) -> Option<(fcp_core::ScheduleState, Option<fcp_core::CallTarget>)> {
        let storage = self.storage.as_ref()?;
        let tenant = self.call_manager.get_call(call_id)?.tenant_id;
        let dialed = request.start_line().uri().to_string();
        let extension = user_of_uri(&dialed)?;
        let queue = storage
            .list_queue_configs(&tenant)
            .await
            .ok()?
            .into_iter()
            .find(|q| q.extension == extension)?;
        let schedule_id = queue.schedule_id?;
        self.schedule_divert(&schedule_id, &tenant, queue.closed_target.clone(), None)
            .await
    }

    /// Send a call where a closed schedule says: its target, or 480.
    async fn divert_for_schedule(
        &self,
        request: &sip_core::Request,
        a_leg_handle: &Arc<ServerTransactionHandle>,
        call_id: fcp_b2bua_core::CallId,
        state: fcp_core::ScheduleState,
        target: Option<fcp_core::CallTarget>,
        callee: &str,
    ) -> Result<()> {
        info!(call_id = %call_id, state = state.as_str(), ?target, "The schedule is not open");
        let _ = self.call_manager.update_call(&call_id, |c| {
            if c.original_callee.is_none() {
                c.original_callee = Some(callee.to_string());
            }
        });
        match target {
            Some(target) => {
                self.apply_treatment(
                    request,
                    a_leg_handle,
                    call_id,
                    fcp_core::Treatment::Forward { target },
                    Some(callee),
                    state.as_str(),
                )
                .await
            }
            None => {
                let response = self.create_simple_response(request, 480, "Closed");
                a_leg_handle.send_final(response).await;
                let _ = self.call_manager.terminate_call_with(
                    &call_id,
                    fcp_cdr::TerminationReason::Failed {
                        status: Some(480),
                        detail: format!("Schedule {}", state.as_str()),
                    },
                );
            }
        }
        Ok(())
    }

    /// Run a flow for a call that has not been answered: the IVR a
    /// number is bound to, or a schedule's or a forward's `flow:` target
    /// (`docs/ENTERPRISE_CALLING.md` §3.5). Anchors the media if it is not,
    /// answers when the flow does, and ends the call when the flow does.
    async fn start_flow_for_call(
        &self,
        request: &sip_core::Request,
        a_leg_handle: &ServerTransactionHandle,
        call_id: fcp_b2bua_core::CallId,
        flow_id: String,
        destination_aor: &str,
    ) -> Result<()> {
        let Some(flow_handler) = self.flow_handler.as_ref() else {
            let response = self.create_simple_response(request, 501, "Not Implemented");
            a_leg_handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(&call_id, Some("Flow handler not available".to_string()));
            return Ok(());
        };

        info!(
            call_id = %call_id,
            flow_id = %flow_id,
            "Starting IVR flow execution for inbound call"
        );

        // IVR flows need media anchored for prompt
        // playback and DTMF detection. If
        // evaluate_media_anchoring chose PassThrough,
        // create a media session now.
        if let Some(call_entry) = self.call_manager.get_call(&call_id) {
            if call_entry.media_session_id.is_none() {
                if let Some(ref media_manager) = self.media_manager {
                    let fcp_call_id = fcp_core::CallId::new();
                    let fcp_dialog_id = fcp_core::DialogId::new();
                    match media_manager
                        .create_session_default(fcp_call_id, fcp_dialog_id)
                        .await
                    {
                        Ok(_session) => {
                            if let Some(a_leg_sdp) = crate::body::request_sdp(request) {
                                if let Err(e) =
                                    media_manager.update_session(&fcp_call_id, a_leg_sdp).await
                                {
                                    warn!(call_id = %call_id, error = %e, "Failed to set A-leg media endpoint for IVR");
                                }
                            }
                            let _ = self.call_manager.update_call(&call_id, |call| {
                                call.media_session_id = Some(fcp_call_id);
                                call.media_anchored = true;
                            });
                            info!(call_id = %call_id, "Created media session for IVR flow (was not anchored)");
                        }
                        Err(e) => {
                            error!(call_id = %call_id, error = %e, "Failed to create media session for IVR");
                        }
                    }
                }
            }
        }

        // Build initial variables from call context
        let mut initial_variables = std::collections::HashMap::new();

        if let Some(from_header) = sip_parse::header(request.headers(), "From") {
            let from_str = from_header.as_str();
            if let Some(caller) = Self::extract_did_from_uri(from_str) {
                initial_variables.insert("caller_number".to_string(), caller.clone());
                initial_variables.insert("caller".to_string(), caller);
            }
            if let Some(display_name) = Self::extract_display_name(from_str) {
                initial_variables.insert("caller_name".to_string(), display_name);
            }
        }

        initial_variables.insert("called_number".to_string(), destination_aor.to_string());

        if let Some(sip_call_id) = sip_parse::header(request.headers(), "Call-ID") {
            initial_variables.insert("sip_call_id".to_string(), sip_call_id.to_string());
        }

        // Create answer callback
        let answer_handle = a_leg_handle.clone();
        let answer_request = request.clone();
        let answer_call_id = call_id;
        let answer_call_manager = Arc::clone(&self.call_manager);
        let answer_media_manager = self.media_manager.clone();
        let answer_uas = self.uas.clone();
        let answer_dialog_manager = Arc::clone(&self.dialog_manager);
        let answer_ack_waits = Arc::clone(&self.ack_waits);
        let answer_callback: fcp_flow::AnswerCallback = Arc::new(move || {
            let handle = answer_handle.clone();
            let request = answer_request.clone();
            let call_id = answer_call_id;
            let call_manager = Arc::clone(&answer_call_manager);
            let media_manager = answer_media_manager.clone();
            let uas = answer_uas.clone();
            let dialog_manager = Arc::clone(&answer_dialog_manager);
            let ack_waits = Arc::clone(&answer_ack_waits);
            Box::pin(async move {
                info!(call_id = %call_id, "IVR Answer callback executing - creating 200 OK");
                let mut response = sip_uas::UserAgentServer::create_response(&request, 200, "OK");
                add_response_headers(&request, &mut response, &uas);

                if let Some(media_mgr) = media_manager {
                    if let Some(call) = call_manager.get_call(&call_id) {
                        if let Some(ref media_session_id) = call.media_session_id {
                            if let Some(remote_sdp) = crate::body::request_sdp(&request) {
                                match media_mgr.update_session(media_session_id, remote_sdp).await {
                                    Ok(local_sdp) => {
                                        let sdp_bytes = bytes::Bytes::from(local_sdp);
                                        let mut headers = response.headers().clone();
                                        let _ = headers.push(
                                            smol_str::SmolStr::new("Content-Type"),
                                            smol_str::SmolStr::new("application/sdp"),
                                        );
                                        let _ = headers.push(
                                            smol_str::SmolStr::new("Content-Length"),
                                            smol_str::SmolStr::new(sdp_bytes.len().to_string()),
                                        );
                                        response = sip_core::Response::new(
                                            response.start_line().clone(),
                                            headers,
                                            sdp_bytes,
                                        )
                                        .expect("Failed to create response with SDP");
                                        info!(call_id = %call_id, "Added SDP to 200 OK response");
                                    }
                                    Err(e) => {
                                        error!(call_id = %call_id, error = %e, "Failed to update media session with SDP");
                                    }
                                }
                            }
                        }
                    }
                }

                // The dialog before the 200 OK: a caller
                // that hangs up on the answer must find the
                // call to end it (see the same order above).
                if let Some(a_leg_dialog) = sip_dialog::Dialog::new_uas(
                    &request,
                    &response,
                    uas.local_uri.clone(),
                    extract_from_uri(&request).unwrap_or_else(|_| {
                        sip_core::SipUri::parse("sip:unknown@unknown").unwrap()
                    }),
                ) {
                    let _ = dialog_manager.insert(a_leg_dialog.clone());
                    // `establish` for the answer time, as above.
                    let _ = call_manager.update_call(&call_id, |call| {
                        call.a_leg.dialog = Some(a_leg_dialog.clone());
                        call.establish();
                    });
                }

                ack_waits.expect(call_id);
                handle.send_final(response.clone()).await;

                Ok(())
            })
        });

        // Create hangup callback
        let hangup_call_id = call_id;
        let hangup_call_manager = Arc::clone(&self.call_manager);
        let hangup_uac = self.uac.clone();
        let hangup_dialog_manager = Arc::clone(&self.dialog_manager);
        let hangup_ack_waits = Arc::clone(&self.ack_waits);
        let hangup_media_manager = self.media_manager.clone();
        let hangup_callback: fcp_flow::HangupCallback = Arc::new(move |reason| {
            let call_id = hangup_call_id;
            let call_manager = Arc::clone(&hangup_call_manager);
            let uac_opt = hangup_uac.clone();
            let dialog_manager = Arc::clone(&hangup_dialog_manager);
            let ack_waits = Arc::clone(&hangup_ack_waits);
            let media_manager = hangup_media_manager.clone();
            Box::pin(async move {
                // Not before the caller has ACKed the answer (RFC 3261 §15).
                ack_waits.wait(&call_id, crate::ack_wait::ACK_WAIT).await;
                info!(call_id = %call_id, "IVR Hangup callback executing");
                // On a task of its own, and awaited so nothing
                // changes but the stack it runs on. The flow
                // executor polls this callback deep inside its
                // own chain and the BYE below is deeper still:
                // together they overflow a worker's stack and
                // abort the process. A spawned task is polled
                // from the top of one.
                let teardown = tokio::spawn(async move {
                    if let Some(call_entry) = call_manager.get_call(&call_id) {
                        if call_entry.media_anchored {
                            if let (Some(ref media_session_id), Some(ref media_mgr)) =
                                (&call_entry.media_session_id, &media_manager)
                            {
                                let _ = media_mgr.terminate_session(media_session_id).await;
                            }
                        }
                        // BYE A-leg (caller)
                        if let Some(dialog) = call_entry.a_leg.dialog.clone() {
                            if let Some(ref uac) = uac_opt {
                                match uac.bye(&dialog).await {
                                    Ok(_) => {
                                        dialog_manager.remove(dialog.id());
                                    }
                                    Err(e) => {
                                        error!(call_id = %call_id, error = %e, "Failed to send A-leg BYE");
                                    }
                                }
                            }
                        }
                        // BYE B-leg (callee from Dial node)
                        if let Some(ref b_leg) = call_entry.b_leg {
                            if let Some(dialog) = b_leg.dialog.clone() {
                                if let Some(ref uac) = uac_opt {
                                    match uac.bye(&dialog).await {
                                        Ok(_) => {
                                            dialog_manager.remove(dialog.id());
                                        }
                                        Err(e) => {
                                            error!(call_id = %call_id, error = %e, "Failed to send B-leg BYE");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let _ = call_manager.terminate_call(&call_id, reason);
                });
                if let Err(e) = teardown.await {
                    error!(call_id = %call_id, error = %e, "Hangup teardown did not finish");
                }
                Ok(())
            })
        });

        // Start the flow
        if let Some(mut call_entry) = self.call_manager.get_call(&call_id) {
            match flow_handler
                .start_flow(
                    &mut call_entry,
                    flow_id,
                    initial_variables,
                    answer_callback,
                    hangup_callback,
                )
                .await
            {
                Ok(()) => {
                    info!(call_id = %call_id, "IVR flow execution started successfully");
                    let mut progress =
                        sip_uas::UserAgentServer::create_response(request, 183, "Session Progress");
                    self.ensure_response_hygiene(request, &mut progress);
                    a_leg_handle.send_provisional(progress).await;
                    return Ok(());
                }
                Err(e) => {
                    error!(call_id = %call_id, error = %e, "Failed to start IVR flow execution");
                    let response =
                        self.create_simple_response(request, 500, "Internal Server Error");
                    a_leg_handle.send_final(response).await;
                    let _ = self
                        .call_manager
                        .terminate_call(&call_id, Some("IVR flow error".to_string()));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// The legs of the next find-me step that has anyone to ring: its
    /// targets resolved (the callee's phones, another user's, an extension
    /// or a number dialed back into this node so it is routed — out a trunk
    /// for a number — with the caller's identity), its ring time and the
    /// legs to confirm set on the call. `None` when no step is left.
    async fn next_find_me_step(
        &self,
        request: &sip_core::Request,
        call_id: fcp_b2bua_core::CallId,
    ) -> Option<Vec<(String, Arc<crate::uac::OutboundCall>, Option<String>)>> {
        let uac = self.uac.clone()?;
        loop {
            let mut step = None;
            let _ = self.call_manager.update_call(&call_id, |c| {
                if !c.find_me_steps.is_empty() {
                    step = Some(c.find_me_steps.remove(0));
                }
            });
            let step = step?;
            let call = self.call_manager.get_call(&call_id)?;
            let host = match self.uas.local_uri.port() {
                Some(port) => format!("{}:{}", self.uas.local_uri.host(), port),
                None => self.uas.local_uri.host().to_string(),
            };
            // (uri, confirm)
            let mut uris: Vec<(String, bool)> = Vec::new();
            for target in &step.targets {
                let contacts_of = |user: fcp_core::UserId| async move {
                    match (&self.storage, &self.registrar) {
                        (Some(storage), Some(registrar)) => {
                            crate::ring_plan::contacts_for_user(
                                storage,
                                registrar,
                                &call.tenant_id,
                                &user,
                                "",
                            )
                            .await
                        }
                        _ => Vec::new(),
                    }
                };
                match target {
                    fcp_core::FindMeTarget::Devices => {
                        if let Some(user) = call.callee_user_id {
                            for c in contacts_of(user).await {
                                uris.push((crate::ring_plan::contact_uri(&c), false));
                            }
                        }
                    }
                    fcp_core::FindMeTarget::Target(fcp_core::CallTarget::User(user)) => {
                        for c in contacts_of(*user).await {
                            uris.push((crate::ring_plan::contact_uri(&c), step.confirm));
                        }
                    }
                    fcp_core::FindMeTarget::Target(fcp_core::CallTarget::Extension(ext)) => {
                        let user = match &self.storage {
                            Some(storage) => {
                                crate::ring_plan::user_of(storage, &call.tenant_id, ext).await
                            }
                            None => None,
                        };
                        match user {
                            Some(user) => {
                                for c in contacts_of(user).await {
                                    uris.push((crate::ring_plan::contact_uri(&c), step.confirm));
                                }
                            }
                            None => uris.push((format!("sip:{ext}@{host}"), step.confirm)),
                        }
                    }
                    // A number or a SIP URI is rung only when the user's
                    // class of service lets them dial it (§3.7).
                    fcp_core::FindMeTarget::Target(fcp_core::CallTarget::Number(n)) => {
                        if self.forward_allowed(&call_id, n).await {
                            uris.push((format!("sip:{n}@{host}"), step.confirm))
                        }
                    }
                    fcp_core::FindMeTarget::Target(fcp_core::CallTarget::Sip(u)) => {
                        if self.forward_allowed(&call_id, u).await {
                            uris.push((u.clone(), step.confirm))
                        }
                    }
                    fcp_core::FindMeTarget::Target(_) => {}
                }
            }
            let mut seen = std::collections::HashSet::new();
            uris.retain(|(u, _)| seen.insert(u.clone()));
            if uris.is_empty() {
                info!(call_id = %call_id, "A find-me step has nobody to ring; trying the next");
                continue;
            }

            // The offer: the caller's SDP, the node's own when anchored.
            let original = crate::body::request_sdp(request);
            let sdp_override = match (
                call.media_anchored,
                call.media_session_id,
                &self.media_manager,
                &original,
            ) {
                (true, Some(session), Some(media), Some(sdp)) => {
                    match media.get_session(&session).await {
                        Ok(session) => Some(fcp_media::rewrite_sdp_for_anchoring(
                            sdp,
                            &self.local_addr,
                            session.ports_for(fcp_media::backend::Leg::B).0,
                            None,
                        )),
                        Err(_) => None,
                    }
                }
                _ => None,
            };
            let caller_display_name = extract_from_display_name(request);
            let caller_from_uri = extract_from_uri(request).ok().map(normalize_from_uri);
            uac.inner().set_display_name(caller_display_name).await;
            uac.inner().set_from_uri(caller_from_uri).await;
            let _ = self
                .call_manager
                .update_call(&call_id, |c| c.forward_reason = Some("follow-me"));
            let mut legs = Vec::new();
            for (uri, _) in &uris {
                match self
                    .create_outbound_call_for(
                        request,
                        uri,
                        &uac,
                        sdp_override.clone(),
                        Some(call_id),
                    )
                    .await
                {
                    Ok(outbound) => legs.push((uri.clone(), outbound, None)),
                    Err(e) => {
                        warn!(call_id = %call_id, target = %uri, error = %e, "A find-me leg could not be placed")
                    }
                }
            }
            uac.inner().set_display_name(None).await;
            uac.inner().set_from_uri(None).await;
            if legs.is_empty() {
                continue;
            }
            let confirm: Vec<String> = uris
                .iter()
                .filter(|(_, c)| *c)
                .map(|(u, _)| u.clone())
                .collect();
            let first = legs[0].0.clone();
            info!(call_id = %call_id, legs = legs.len(), ring_secs = step.ring_secs, confirm = confirm.len(), "Ringing the next find-me step");
            let _ = self.call_manager.update_call(&call_id, |c| {
                c.no_answer_timeout = Some(Duration::from_secs(u64::from(step.ring_secs)));
                c.confirm_targets = confirm;
                c.b_leg = Some(fcp_b2bua_core::CallLeg::new_outbound(first, None));
            });
            return Some(legs);
        }
    }

    /// Ask the leg that answered to confirm: its media pointed at the
    /// node's session, a prompt played to it alone ("call for <name>,
    /// press 1"), one digit awaited. True when it pressed 1. Without an
    /// anchored session there is nothing to ask with, and the answer
    /// stands.
    async fn confirm_answer(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        answer: &sip_core::Response,
    ) -> bool {
        let Some(call) = self.call_manager.get_call(call_id) else {
            return false;
        };
        let (Some(session), Some(media)) = (
            call.media_session_id.filter(|_| call.media_anchored),
            self.media_manager.as_ref(),
        ) else {
            warn!(call_id = %call_id, "A find-me answer cannot be confirmed without the media here; connecting it");
            return true;
        };
        let answer_sdp = String::from_utf8_lossy(answer.body()).to_string();
        if !answer_sdp.is_empty() {
            if let Err(e) = media.update_session_b_leg(&session, answer_sdp).await {
                warn!(call_id = %call_id, error = %e, "The confirming leg's media was not set");
            }
        }
        let collector = media.create_dtmf_collector(&session, 1, None);
        let name = call
            .confirm_for
            .clone()
            .unwrap_or_else(|| "you".to_string());
        let spoken = media
            .play_tts(
                &session,
                format!("Call for {name}. Press 1 to accept."),
                "en".to_string(),
                None,
                fcp_media::AudioTarget::ParticipantB,
            )
            .await;
        if spoken.is_err() {
            // No speech here: a short tone is the cue.
            let _ = media
                .play_tone(
                    &session,
                    880.0,
                    400,
                    0.5,
                    fcp_media::AudioTarget::ParticipantB,
                )
                .await;
        }
        match collector.collect(CONFIRM_WAIT).await {
            fcp_media::dtmf_collector::CollectionResult::Complete(d) => d == "1",
            fcp_media::dtmf_collector::CollectionResult::Timeout(d) => d == "1",
            fcp_media::dtmf_collector::CollectionResult::Cancelled => false,
        }
    }

    /// How the caller of a call leaving on a trunk is presented (§3.7):
    /// their line's outbound caller ID as a URI at the caller's host, and
    /// whether their class of service withholds it.
    async fn caller_presentation(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        caller_from: Option<&str>,
    ) -> (Option<String>, bool) {
        let (Some(storage), Some(call)) = (&self.storage, self.call_manager.get_call(call_id))
        else {
            return (None, false);
        };
        let Some(user) = call.caller_user_id else {
            return (None, false);
        };
        let withheld = crate::class_of_service::class_for(storage, &call.tenant_id, &user)
            .await
            .is_some_and(|c| c.caller_id == fcp_core::CallerIdPolicy::Withheld);
        // A call an assistant places for an executive leaves with the
        // executive's line (§3.10); the assistant's class still decides.
        let user = call.on_behalf_of.unwrap_or(user);
        // The line's outbound caller ID, else the line's own number: a user
        // given a number presented their SIP name to the carrier, which a
        // carrier refuses (`docs/DEEP_TESTING.md` D8).
        let lines: Vec<_> = match storage.list_line_bindings_by_user(&user).await {
            Ok(bindings) => bindings
                .into_iter()
                .filter(|b| b.tenant_id == call.tenant_id)
                .collect(),
            Err(e) => {
                warn!(call_id = %call_id, user_id = %user, error = %e, "Failed to read the caller's lines");
                Vec::new()
            }
        };
        let mut number = lines.iter().find_map(|b| {
            b.outbound_caller_id
                .clone()
                .filter(|id| !id.trim().is_empty())
        });
        if number.is_none() {
            for line in &lines {
                if let Ok(Some(n)) = storage.get_phone_number(&line.phone_number_id).await {
                    number = Some(n.e164);
                    break;
                }
            }
        }
        let host = caller_from
            .and_then(|from| {
                sip_core::SipUri::parse(from.trim_matches(|c| c == '<' || c == '>')).ok()
            })
            .map(|uri| uri.host().to_string())
            .unwrap_or_else(|| self.uas.local_uri.host().to_string());
        let line = number.map(|n| format!("sip:{}@{host}", n.trim()));
        (line, withheld)
    }

    /// The number a leg this node placed for a forward was diverted from,
    /// and why (`docs/ENTERPRISE_CALLING.md` §3.6): the number the caller
    /// dialed, else the first number of the user the call was for. `None`
    /// for a call that forwards for nobody, or a user with no number.
    async fn diversion_of(
        &self,
        call_id: &fcp_b2bua_core::CallId,
    ) -> Option<(String, &'static str)> {
        let storage = self.storage.as_ref()?;
        let forwarded_for = self.call_manager.get_call(call_id)?.forwarded_for?;
        let original = self.call_manager.get_call(&forwarded_for)?;
        let reason = original.forward_reason.unwrap_or("unknown");
        let dialed = match original.number_binding_id {
            Some(id) => storage.get_line_binding(&id).await.ok().flatten(),
            None => None,
        };
        let binding = match (dialed, original.callee_user_id) {
            (Some(binding), _) => Some(binding),
            (None, Some(user)) => match storage.list_line_bindings_by_user(&user).await {
                Ok(bindings) => bindings
                    .into_iter()
                    .find(|b| b.tenant_id == original.tenant_id),
                Err(e) => {
                    warn!(call_id = %call_id, user_id = %user, error = %e, "Failed to read the forwarding user's lines");
                    None
                }
            },
            (None, None) => None,
        }?;
        match storage.get_phone_number(&binding.phone_number_id).await {
            Ok(number) => number.map(|n| (n.e164, reason)),
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Failed to read the diverting number");
                None
            }
        }
    }

    /// Whether the class of service of the user `caller` names refuses
    /// this INVITE: what it dials, or one call more than the class allows
    /// (`docs/ENTERPRISE_CALLING.md` §3.7). `None` when the caller is no
    /// user of the tenant, or has no class.
    async fn class_of_service_refusal(
        &self,
        request: &sip_core::Request,
        tenant_id: &fcp_core::TenantId,
        caller: Option<&str>,
    ) -> Option<crate::class_of_service::Refusal> {
        let storage = self.storage.as_ref()?;
        let user = crate::hoteling::caller_user(storage, tenant_id, caller?).await?;
        let dialed = request.uri().to_string();
        let dialed = crate::class_of_service::uri_user(&dialed);
        let class = match crate::class_of_service::check_dial(
            storage,
            tenant_id,
            &user,
            dialed,
            self.b2bua_config.max_extension_digits as usize,
        )
        .await
        {
            Ok(class) => class?,
            Err(refusal) => return Some(refusal),
        };
        let (max, user_calls) = (class.max_calls?, self.user_calls.as_ref()?);
        (user_calls.count(tenant_id, &user).await >= u64::from(max)).then(|| {
            crate::class_of_service::Refusal {
                class_name: class.name.clone(),
                kind: crate::class_of_service::RefusalKind::CallLimit(max),
            }
        })
    }

    /// Whether the class of service of the user a forward or a find-me
    /// step is for lets it dial `destination` (§3.7): a number, or a SIP
    /// URI whose user is one. Logged when it does not.
    async fn forward_allowed(&self, call_id: &fcp_b2bua_core::CallId, destination: &str) -> bool {
        let (Some(storage), Some(call)) = (&self.storage, self.call_manager.get_call(call_id))
        else {
            return true;
        };
        let Some(user) = call.callee_user_id else {
            return true;
        };
        match crate::class_of_service::check_dial(
            storage,
            &call.tenant_id,
            &user,
            crate::class_of_service::uri_user(destination),
            self.b2bua_config.max_extension_digits as usize,
        )
        .await
        {
            Ok(_) => true,
            Err(refusal) => {
                info!(
                    call_id = %call_id,
                    user_id = %user,
                    destination = %destination,
                    class = %refusal.class_name,
                    "Not forwarding: {}",
                    refusal.detail()
                );
                false
            }
        }
    }

    /// Mark the call with the user its caller is, when the From names a
    /// user of the call's tenant (an extension or a username) or a device
    /// — its hoteling guest, else its owner (§3.8).
    async fn note_caller_user(
        &self,
        request: &sip_core::Request,
        call_id: &fcp_b2bua_core::CallId,
    ) {
        let (Some(storage), Some(tenant)) = (
            self.storage.as_ref(),
            self.call_manager.get_call(call_id).map(|c| c.tenant_id),
        ) else {
            return;
        };
        let Some(from) = extract_from_uri(request)
            .ok()
            .and_then(|u| u.user().map(str::to_string))
        else {
            return;
        };
        if let Some(user_id) = crate::hoteling::caller_user(storage, &tenant, &from).await {
            let _ = self
                .call_manager
                .update_call(call_id, |c| c.caller_user_id = Some(user_id));
        }
    }

    /// Whether `user` was the callee of a call `call_id` forwards for,
    /// directly or through the forwards before it, or the chain is longer
    /// than [`MAX_FORWARD_HOPS`]: the forward has come back around.
    fn forward_loop(&self, call_id: &fcp_b2bua_core::CallId, user: &fcp_core::UserId) -> bool {
        let mut next = self
            .call_manager
            .get_call(call_id)
            .and_then(|call| call.forwarded_for);
        let mut hops = 0;
        while let Some(id) = next {
            hops += 1;
            if hops > MAX_FORWARD_HOPS {
                return true;
            }
            let Some(call) = self.call_manager.get_call(&id) else {
                return false;
            };
            if call.callee_user_id.as_ref() == Some(user) {
                return true;
            }
            next = call.forwarded_for;
        }
        false
    }

    /// Ring a user by their plan (`docs/ENTERPRISE_CALLING.md` §3.1):
    /// `Some(contacts)` to fork, with the treatments and the ring time on
    /// the call; `None` when the plan applied at once and the call is
    /// handled. Without a user (a binding nobody owns) the defaults apply.
    #[allow(clippy::too_many_arguments)]
    async fn ring_by_plan(
        &self,
        request: &sip_core::Request,
        a_leg_handle: &Arc<ServerTransactionHandle>,
        call_id: fcp_b2bua_core::CallId,
        tenant_id: &fcp_core::TenantId,
        user_id: Option<fcp_core::UserId>,
        contacts: Vec<sip_registrar::Binding>,
        callee: &str,
    ) -> Result<Option<Vec<sip_registrar::Binding>>> {
        // A user this call's forwards already reached: two users forwarding
        // to each other would place calls into this node without end.
        if let Some(user) = user_id.filter(|user| self.forward_loop(&call_id, user)) {
            warn!(call_id = %call_id, user_id = %user, "The call's forwards loop back to a user they reached: refused");
            let _ = self.call_manager.update_call(&call_id, |call| {
                call.callee_user_id = Some(user);
            });
            self.apply_treatment(
                request,
                a_leg_handle,
                call_id,
                fcp_core::Treatment::Refuse { code: 482 },
                Some(callee),
                "in a forward loop",
            )
            .await;
            return Ok(None);
        }
        let (settings, has_mailbox) = match (&self.storage, user_id) {
            (Some(storage), Some(user_id)) => (
                crate::ring_plan::settings_for(storage, tenant_id, &user_id).await,
                crate::ring_plan::has_mailbox(storage, &user_id).await,
            ),
            _ => (
                fcp_core::CallingSettings::defaults(*tenant_id, user_id.unwrap_or_default()),
                false,
            ),
        };
        // Busy: on a call with call waiting off, or on as many calls as
        // the user's class of service allows (§3.7).
        let max_calls = match (&self.storage, user_id) {
            (Some(storage), Some(user_id)) => {
                crate::class_of_service::class_for(storage, tenant_id, &user_id)
                    .await
                    .and_then(|c| c.max_calls)
            }
            _ => None,
        };
        let on_a_call = match (&self.user_calls, user_id) {
            (Some(user_calls), Some(user_id)) if !settings.call_waiting || max_calls.is_some() => {
                let count = user_calls.count(tenant_id, &user_id).await;
                (!settings.call_waiting && count > 0)
                    || max_calls.is_some_and(|max| count >= u64::from(max))
            }
            _ => false,
        };
        // The user's own hours (§3.5): only worth reading with somewhere to
        // send the call.
        let after_hours = match (
            &settings.after_hours_schedule_id,
            &settings.after_hours_forward,
        ) {
            (Some(schedule_id), Some(_)) => self
                .schedule_divert(schedule_id, tenant_id, None, None)
                .await
                .is_some(),
            _ => false,
        };
        let inputs = crate::ring_plan::PlanInputs {
            has_mailbox,
            on_a_call,
            after_hours,
        };
        match crate::ring_plan::plan(&settings, contacts, inputs) {
            // No phone of theirs is up, but find-me has somewhere to ring
            // (§3.6): its steps, then the unreachable treatment.
            crate::ring_plan::RingPlan::Immediate {
                treatment,
                reason: "unreachable",
            } if !settings.find_me_steps().is_empty() => {
                let steps = settings.find_me_steps();
                let confirm_for = match (&self.storage, user_id) {
                    (Some(storage), Some(user_id)) => storage
                        .get_user_by_id(&user_id)
                        .await
                        .ok()
                        .flatten()
                        .map(|u| u.display_name.unwrap_or(u.username)),
                    _ => None,
                };
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.callee_user_id = user_id;
                    call.treatments = Some(fcp_core::Treatments {
                        busy: treatment.clone(),
                        no_answer: treatment.clone(),
                    });
                    call.find_me_steps = steps;
                    call.confirm_for = confirm_for;
                    if call.original_callee.is_none() {
                        call.original_callee = Some(callee.to_string());
                    }
                });
                match self.next_find_me_step(request, call_id).await {
                    Some(legs) => {
                        let cancel_notify = Arc::new(tokio::sync::Notify::new());
                        let _ = self.call_manager.update_call(&call_id, |call| {
                            call.cancel_notify = Some(Arc::clone(&cancel_notify));
                        });
                        self.handle_parallel_forking(
                            request.clone(),
                            Arc::clone(a_leg_handle),
                            legs,
                            call_id,
                            cancel_notify,
                            None,
                        )
                        .await?;
                    }
                    None => {
                        self.apply_treatment(
                            request,
                            a_leg_handle,
                            call_id,
                            treatment,
                            Some(callee),
                            "unreachable",
                        )
                        .await;
                    }
                }
                Ok(None)
            }
            crate::ring_plan::RingPlan::Immediate { treatment, reason } => {
                info!(call_id = %call_id, ?user_id, ?treatment, reason, "The callee's plan applies without ringing");
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.callee_user_id = user_id;
                    if call.original_callee.is_none() {
                        call.original_callee = Some(callee.to_string());
                    }
                });
                self.apply_treatment(
                    request,
                    a_leg_handle,
                    call_id,
                    treatment,
                    Some(callee),
                    reason,
                )
                .await;
                Ok(None)
            }
            crate::ring_plan::RingPlan::Ring {
                contacts,
                ring_time,
                treatments,
            } => {
                // An executive's call their assistants take first (§3.10).
                let contacts = match user_id {
                    Some(executive) => Box::pin(self.executive_screening(
                        request, call_id, tenant_id, &executive, &settings, &contacts,
                    ))
                    .await
                    .unwrap_or(contacts),
                    None => contacts,
                };
                // Find-me: the steps to ring after these phones, and the name
                // a confirming answerer hears (§3.6).
                let find_me_steps = settings.find_me_steps();
                let confirm_for = match (&self.storage, user_id, find_me_steps.is_empty()) {
                    (Some(storage), Some(user_id), false) => storage
                        .get_user_by_id(&user_id)
                        .await
                        .ok()
                        .flatten()
                        .map(|u| u.display_name.unwrap_or(u.username)),
                    _ => None,
                };
                // Apps asleep are woken by push rather than rung
                // (`docs/PUSH.md` §2.2); the fork takes them when they
                // come back.
                let (contacts, wake) = self.classify_bindings(call_id, tenant_id, contacts).await;
                if !wake.is_empty() {
                    self.push_waits.begin(
                        call_id,
                        *tenant_id,
                        wake,
                        Self::push_message(request, call_id, tenant_id),
                    );
                    // Nothing else rings: push now, and a user whose apps
                    // cannot be woken is unreachable.
                    if contacts.is_empty() && self.push_waiting(call_id).await == 0 {
                        self.push_waits.end(&call_id);
                        info!(call_id = %call_id, "The callee's apps cannot be woken: unreachable");
                        return Box::pin(self.ring_by_plan(
                            request,
                            a_leg_handle,
                            call_id,
                            tenant_id,
                            user_id,
                            Vec::new(),
                            callee,
                        ))
                        .await;
                    }
                }
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.callee_user_id = user_id;
                    call.treatments = Some(treatments);
                    call.no_answer_timeout = ring_time;
                    call.find_me_steps = find_me_steps;
                    call.confirm_for = confirm_for;
                    call.confirm_targets.clear();
                    if call.original_callee.is_none() {
                        call.original_callee = Some(callee.to_string());
                    }
                });
                Ok(Some(contacts))
            }
        }
    }

    /// Apply a treatment to the caller: forward, voicemail, or a final
    /// response. Ends the call on its record when nothing else will.
    /// The flow the callee's mailbox hands its calls to, if it has one and
    /// is enabled.
    async fn mailbox_flow_of_callee(&self, call_id: fcp_b2bua_core::CallId) -> Option<String> {
        let mailbox = self.mailbox_of_callee(call_id).await?;
        mailbox.flow_id.filter(|id| !id.trim().is_empty())
    }

    /// The enabled mailbox of the user a call is for.
    async fn mailbox_of_callee(
        &self,
        call_id: fcp_b2bua_core::CallId,
    ) -> Option<fcp_core::VoicemailMailbox> {
        let storage = self.storage.as_ref()?;
        let user = self.call_manager.get_call(&call_id)?.callee_user_id?;
        fcp_core::storage::VoicemailStore::get_mailbox_by_user(storage.as_ref(), &user)
            .await
            .ok()?
            .filter(|mailbox| mailbox.enabled)
    }

    async fn apply_treatment(
        &self,
        request: &sip_core::Request,
        a_leg_handle: &Arc<ServerTransactionHandle>,
        call_id: fcp_b2bua_core::CallId,
        treatment: fcp_core::Treatment,
        callee: Option<&str>,
        reason: &str,
    ) {
        // A flow's Dial hears that the user did not answer, and its flow
        // decides what follows (`FLOW_DIAL_HEADER`).
        let treatment = match treatment {
            fcp_core::Treatment::Voicemail if request.headers().contains(FLOW_DIAL_HEADER) => {
                info!(call_id = %call_id, reason, "A flow's Dial: the plan's voicemail answered as 480");
                fcp_core::Treatment::Refuse { code: 480 }
            }
            other => other,
        };
        // A mailbox that hands its calls to a flow (its `flow_id`): the call
        // runs that flow instead (`docs/DEEP_TESTING.md` D8).
        let treatment = match treatment {
            fcp_core::Treatment::Voicemail => match self.mailbox_flow_of_callee(call_id).await {
                Some(flow_id) => {
                    info!(call_id = %call_id, flow_id = %flow_id, "The callee's mailbox hands the call to a flow");
                    fcp_core::Treatment::Forward {
                        target: fcp_core::CallTarget::Flow(flow_id),
                    }
                }
                None => fcp_core::Treatment::Voicemail,
            },
            other => other,
        };
        let routed = match treatment {
            fcp_core::Treatment::Voicemail => {
                self.route_cfna(
                    request,
                    a_leg_handle,
                    call_id,
                    "voicemail",
                    callee,
                    Some(reason),
                )
                .await
            }
            // A flow runs for the call itself (`docs/ENTERPRISE_CALLING.md`
            // §3.5).
            fcp_core::Treatment::Forward {
                target: fcp_core::CallTarget::Flow(flow_id),
            } => {
                self.start_flow_for_call(
                    request,
                    a_leg_handle,
                    call_id,
                    flow_id,
                    callee.unwrap_or_default(),
                )
                .await
            }
            fcp_core::Treatment::Forward { target } => {
                match self.forward_destination(call_id, &target).await {
                    Some(destination) => {
                        self.route_cfna(
                            request,
                            a_leg_handle,
                            call_id,
                            &destination,
                            callee,
                            Some(reason),
                        )
                        .await
                    }
                    None => Err(anyhow::anyhow!("forward target {target} cannot be placed")),
                }
            }
            fcp_core::Treatment::Refuse { code } => {
                let response = self.create_simple_response(request, code, reason_phrase(code));
                a_leg_handle.send_final(response).await;
                let termination = if code == 486 {
                    fcp_cdr::TerminationReason::rejected(code, reason_phrase(code))
                } else {
                    fcp_cdr::TerminationReason::Failed {
                        status: Some(code),
                        detail: format!("The callee is {reason}"),
                    }
                };
                let _ = self.call_manager.terminate_call_with(&call_id, termination);
                return;
            }
        };
        if let Err(e) = routed {
            error!(call_id = %call_id, error = %e, "Applying the treatment failed, sending 480");
            let response = self.create_simple_response(request, 480, "Temporarily Unavailable");
            a_leg_handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(&call_id, Some(format!("The treatment failed: {e}")));
        }
    }

    /// What `route_cfna` dials for a forward target: a number or an
    /// extension at this node, a SIP URI as given, a user by their
    /// extension or name. A flow or a queue as a target lands with the
    /// schedules phase.
    async fn forward_destination(
        &self,
        call_id: fcp_b2bua_core::CallId,
        target: &fcp_core::CallTarget,
    ) -> Option<String> {
        match target {
            fcp_core::CallTarget::Voicemail => Some("voicemail".to_string()),
            // The forwarding user's class of service decides whether a
            // number is dialled for them (§3.7).
            fcp_core::CallTarget::Number(n) => {
                self.forward_allowed(&call_id, n).await.then(|| n.clone())
            }
            fcp_core::CallTarget::Extension(e) => Some(e.clone()),
            fcp_core::CallTarget::Sip(u) => {
                self.forward_allowed(&call_id, u).await.then(|| u.clone())
            }
            fcp_core::CallTarget::User(user_id) => {
                let storage = self.storage.as_ref()?;
                match storage.get_user_by_id(user_id).await {
                    Ok(Some(user)) => Some(
                        user.extension
                            .clone()
                            .unwrap_or_else(|| user.sip_name().to_string()),
                    ),
                    Ok(None) => {
                        warn!(user_id = %user_id, "Forward target user not found");
                        None
                    }
                    Err(e) => {
                        warn!(user_id = %user_id, error = %e, "Failed to read the forward target user");
                        None
                    }
                }
            }
            // A queue is dialed at its extension, where the queue answers.
            fcp_core::CallTarget::Queue(name) => {
                let storage = self.storage.as_ref()?;
                let tenant = self.call_manager.get_call(&call_id)?.tenant_id;
                match storage.list_queue_configs(&tenant).await {
                    Ok(queues) => {
                        let found = queues.into_iter().find(|q| q.name == *name);
                        if found.is_none() {
                            warn!(queue = %name, "Forward target queue not found");
                        }
                        found.map(|q| q.extension)
                    }
                    Err(e) => {
                        warn!(queue = %name, error = %e, "Failed to read the forward target queue");
                        None
                    }
                }
            }
            // Run for the call by `apply_treatment`, not dialed.
            fcp_core::CallTarget::Flow(_) => None,
        }
    }

    /// The phones a call to `executive` rings when their assistants take
    /// it first (`docs/ENTERPRISE_CALLING.md` §3.10): every opted-in
    /// assistant's, with the executive's own when they are alerted too.
    /// The legs carry a distinct `Alert-Info` and show the caller "for" the
    /// executive; the no-answer treatment stays the executive's. `None`
    /// rings the executive as usual: the filter does not take the call, it
    /// was pushed by an assistant or placed by one, or no assistant's
    /// phone is up.
    async fn executive_screening(
        &self,
        request: &sip_core::Request,
        call_id: fcp_b2bua_core::CallId,
        tenant_id: &fcp_core::TenantId,
        executive: &fcp_core::UserId,
        settings: &fcp_core::CallingSettings,
        own: &[sip_registrar::Binding],
    ) -> Option<Vec<sip_registrar::Binding>> {
        let handling = settings.executive.as_ref()?;
        let (storage, registrar) = (self.storage.as_ref()?, self.registrar.as_ref()?);
        let call = self.call_manager.get_call(&call_id)?;
        // A call an assistant pushed here rings the executive.
        if let Some(from) = sip_parse::header(request.headers(), TRANSFERRED_FROM_HEADER) {
            let pushed = from
                .trim()
                .parse::<fcp_core::CallId>()
                .ok()
                .map(|id| fcp_b2bua_core::CallId::from_uuid(*id.as_uuid()))
                .and_then(|id| self.call_manager.get_call(&id))
                .is_some_and(|c| c.executive_for == Some(*executive));
            if pushed {
                return None;
            }
        }
        // An assistant calling their executive reaches them.
        if call
            .caller_user_id
            .is_some_and(|u| handling.assistants.iter().any(|a| a.user_id == u))
        {
            return None;
        }
        if !handling.screens(call.caller_user_id.is_none()) {
            return None;
        }
        let host = crate::class_of_service::uri_host(&request.uri().to_string()).to_string();
        let domain = host.split(':').next().unwrap_or(&host).to_string();
        let mut contacts: Vec<sip_registrar::Binding> = Vec::new();
        for assistant in handling.opted_in() {
            for contact in crate::ring_plan::contacts_for_user(
                storage, registrar, tenant_id, &assistant, &domain,
            )
            .await
            {
                if !contacts.iter().any(|c| c.contact() == contact.contact()) {
                    contacts.push(contact);
                }
            }
        }
        if contacts.is_empty() {
            info!(call_id = %call_id, executive = %executive, "No assistant's phone is up; the executive's ring");
            return None;
        }
        if handling.alert == fcp_core::ExecutiveAlert::AssistantsAndExecutive {
            for contact in own {
                if !contacts.iter().any(|c| c.contact() == contact.contact()) {
                    contacts.push(contact.clone());
                }
            }
        }
        let executive_name = storage
            .get_user_by_id(executive)
            .await
            .ok()
            .flatten()
            .map(|u| u.display_name.unwrap_or(u.username))
            .unwrap_or_default();
        let caller_name = extract_from_display_name(request)
            .or_else(|| {
                sip_parse::header(request.headers(), "From")
                    .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            })
            .unwrap_or_else(|| "Unknown".to_string());
        let shown = crate::executive::for_executive(&caller_name, &executive_name);
        info!(call_id = %call_id, executive = %executive, legs = contacts.len(), "An executive's call rings their assistants");
        let executive = *executive;
        let _ = self.call_manager.update_call(&call_id, |c| {
            c.executive_for = Some(executive);
            c.leg_alert_info = Some(crate::executive::EXECUTIVE_ALERT_INFO.to_string());
            c.leg_display_name = Some(shown);
        });
        Some(contacts)
    }

    /// `*63` from an assistant (§3.10): the far party of the call they took
    /// for an executive is sent on to the executive's own phones — a blind
    /// transfer this node carries out, which the executive's screening
    /// lets through — and the assistant's leg is let go once it lands.
    async fn handle_executive_push(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
    ) -> Result<()> {
        let finish = |status: u16, reason: &'static str| async move {
            let response = self.create_simple_response(request, status, reason);
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(call_id, Some("Executive push".to_string()));
            Ok(())
        };
        let (Some(storage), Some(uac)) = (self.storage.as_ref(), self.uac.clone()) else {
            return finish(500, "Not Configured").await;
        };
        let dialer = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();
        // The dialer's leg of an executive's call: the phone that answered
        // it (its dialog's remote target), or the one that placed it.
        let is_dialer = |leg: &fcp_b2bua_core::CallLeg| {
            let target = leg
                .dialog
                .as_ref()
                .and_then(|d| d.remote_target().user().map(str::to_string));
            target.as_deref() == Some(dialer.as_str())
                || crate::class_of_service::uri_user(&leg.remote_uri) == dialer
        };
        let Some((call, side)) = self.call_manager.find_established_where(|c| {
            c.executive_for?;
            if c.b_leg.as_ref().is_some_and(is_dialer) {
                Some(fcp_b2bua_core::LegSide::B)
            } else if is_dialer(&c.a_leg) {
                Some(fcp_b2bua_core::LegSide::A)
            } else {
                None
            }
        }) else {
            info!(call_id = %call_id, dialer = %dialer, "Push: no executive's call to push");
            return finish(480, "No Executive Call").await;
        };
        let Some(executive) = call.executive_for else {
            return finish(480, "No Executive Call").await;
        };
        let dialer_user = crate::hoteling::caller_user(storage, &call.tenant_id, &dialer).await;
        let settings = crate::ring_plan::settings_for(storage, &call.tenant_id, &executive).await;
        if !dialer_user.is_some_and(|u| settings.assistant(&u).is_some()) {
            info!(call_id = %call_id, dialer = %dialer, "Push refused: not the executive's assistant");
            return finish(403, "Not an Assistant").await;
        }
        let Some(executive_user) = storage.get_user_by_id(&executive).await.ok().flatten() else {
            return finish(404, "No Such Executive").await;
        };
        let referrer_dialog = match side {
            fcp_b2bua_core::LegSide::A => call.a_leg.dialog.clone(),
            fcp_b2bua_core::LegSide::B => call.b_leg.as_ref().and_then(|b| b.dialog.clone()),
        };
        let Some(referrer_dialog) = referrer_dialog else {
            return finish(480, "No Executive Call").await;
        };
        if self.conference_migrations.insert(call.id, ()).is_some() {
            return finish(491, "Request Pending").await;
        }
        let host = match self.uas.local_uri.port() {
            Some(port) => format!("{}:{}", self.uas.local_uri.host(), port),
            None => self.uas.local_uri.host().to_string(),
        };
        // Dialed back into this node as the executive's extension, so it
        // is routed as any call to them; by name at the tenant's domain
        // when they have none.
        let target = match executive_user
            .extension
            .as_deref()
            .filter(|e| !e.is_empty())
        {
            Some(extension) => format!("sip:{extension}@{host}"),
            None => {
                let domain = storage
                    .get_tenant(&call.tenant_id)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|t| t.sip_domains.into_iter().next())
                    .unwrap_or_else(|| host.clone());
                format!("sip:{}@{domain}", executive_user.sip_name())
            }
        };
        // The executive sees the pushed party calling.
        let caller = call.original_invite.as_ref().and_then(|invite| match side {
            fcp_b2bua_core::LegSide::B => extract_from_uri(invite)
                .ok()
                .map(|uri| (extract_from_display_name(invite), normalize_from_uri(uri))),
            fcp_b2bua_core::LegSide::A => None,
        });
        info!(call_id = %call.id, executive = %executive_user.username, "An assistant pushes a call to the executive");
        let run = crate::transfer::TransferRun {
            uac,
            local_uri: self.uas.local_uri.clone(),
            call_manager: self.call_manager.clone(),
            dialog_manager: Arc::clone(&self.dialog_manager),
            session_timer_manager: Arc::clone(&self.session_timer_manager),
            media_manager: self.media_manager.clone(),
            sip_call_id_map: Arc::clone(&self.sip_call_id_map),
            in_flight: Arc::clone(&self.conference_migrations),
            b_leg_keys: Arc::clone(&self.b_leg_keys),
            b2bua_config: self.b2bua_config.clone(),
            local_addr: self.local_addr.clone(),
            call_id: call.id,
            referrer: side,
            referrer_dialog: Some(referrer_dialog),
            target,
            headers: vec![(TRANSFERRED_FROM_HEADER.to_string(), call.id.to_string())],
            caller,
            notify_referrer: true,
            fallback: None,
        };
        tokio::spawn(run.run());
        self.confirm_feature(request, handle, call_id, "Call pushed.")
            .await
    }

    /// `*64<number>` from an assistant (§3.10): the number called for the
    /// executive they act for — the executive's caller ID at the trunk,
    /// the assistant's class of service deciding what may be dialed, and
    /// the record naming both.
    async fn handle_call_on_behalf(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        target: &str,
    ) -> Result<()> {
        let refuse = |status: u16, reason: &'static str| async move {
            let response = self.create_simple_response(request, status, reason);
            handle.send_final(response).await;
            let _ = self.call_manager.terminate_call_with(
                call_id,
                fcp_cdr::TerminationReason::rejected(status, reason),
            );
            Ok(())
        };
        let (Some(storage), Some(uac)) = (self.storage.as_ref(), self.uac.clone()) else {
            return refuse(500, "Not Configured").await;
        };
        let Ok(tenant_id) = self.resolve_runtime_tenant(request, call_id).await else {
            return refuse(403, "Tenant Context Required").await;
        };
        let dialer = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();
        let Some(assistant) = crate::hoteling::caller_user(storage, &tenant_id, &dialer).await
        else {
            return refuse(403, "Not an Assistant").await;
        };
        let Some(executive) =
            crate::executive::first_executive(storage, &tenant_id, &assistant).await
        else {
            info!(call_id = %call_id, dialer = %dialer, "Call on behalf refused: no executive");
            return refuse(403, "Not an Assistant").await;
        };
        // The number, at the host the phone dialed.
        let host = crate::class_of_service::uri_host(&request.uri().to_string()).to_string();
        let Ok(uri) = sip_core::SipUri::parse(&format!("sip:{target}@{host}")) else {
            return refuse(484, "Address Incomplete").await;
        };
        let mut headers = request.headers().clone();
        let _ = headers.set_or_push("To", format!("<{}>", uri.as_str()));
        let Ok(placed) = sip_core::Request::new(
            sip_core::RequestLine::new(sip_core::Method::Invite, uri),
            headers,
            request.body().clone(),
        ) else {
            return refuse(484, "Address Incomplete").await;
        };
        if let Some(refusal) = self
            .class_of_service_refusal(&placed, &tenant_id, Some(&dialer))
            .await
        {
            info!(call_id = %call_id, class = %refusal.class_name, "Call on behalf refused: {}", refusal.detail());
            let mut response = self.create_simple_response(request, 403, "Forbidden");
            let _ = response.headers_mut().push(
                SmolStr::new("Reason"),
                SmolStr::new(refusal.reason_header()),
            );
            handle.send_final(response).await;
            let _ = self.call_manager.terminate_call_with(
                call_id,
                fcp_cdr::TerminationReason::Failed {
                    status: Some(403),
                    detail: refusal.detail(),
                },
            );
            return Ok(());
        }
        info!(call_id = %call_id, executive = %executive, target, "An assistant calls for their executive");
        let _ = self.call_manager.update_call(call_id, |c| {
            c.on_behalf_of = Some(executive);
            let mut data = c
                .custom_data
                .take()
                .and_then(|d| d.as_object().cloned())
                .unwrap_or_default();
            data.insert(
                "on_behalf_of".to_string(),
                serde_json::json!({
                    "executive_user_id": executive.to_string(),
                    "assistant_user_id": assistant.to_string(),
                }),
            );
            c.custom_data = Some(serde_json::Value::Object(data));
        });
        Box::pin(self.forward_invite_to_b_leg(&placed, handle, *call_id, uac)).await
    }

    /// A calling feature code (`docs/ENTERPRISE_CALLING.md` §3.1): the
    /// caller's own settings changed and the call answered then ended, as
    /// the agent login codes do.
    async fn handle_calling_code(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        call_id: &fcp_b2bua_core::CallId,
        code: fcp_pbx::CallingCode,
    ) -> Result<()> {
        let finish = |status: u16, reason: &'static str| async move {
            let response = self.create_simple_response(request, status, reason);
            handle.send_final(response).await;
            let _ = self
                .call_manager
                .terminate_call(call_id, Some("Calling feature code".to_string()));
            Ok(())
        };
        let Some(storage) = self.storage.as_ref() else {
            return finish(500, "Storage Not Configured").await;
        };
        let caller = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();
        let tenant_id = match self.resolve_runtime_tenant(request, call_id).await {
            Ok(tenant_id) => tenant_id,
            Err(e) => {
                warn!(call_id = %call_id, caller = %caller, error = %e, "Calling code: tenant resolution failed");
                return finish(403, "Tenant Context Required").await;
            }
        };
        // The phone's effective user: a host's guest, a device's owner
        // (§3.8), else the user the caller is.
        let caller_user = crate::hoteling::caller_user(storage, &tenant_id, &caller).await;
        let found = match caller_user {
            Some(user_id) => storage.get_user_by_id(&user_id).await,
            None => Ok(None),
        };
        let user = match found {
            Ok(Some(user)) => user,
            Ok(None) => {
                warn!(call_id = %call_id, caller = %caller, "Calling code from an unknown user");
                return finish(404, "Unknown Caller").await;
            }
            Err(e) => {
                error!(call_id = %call_id, caller = %caller, error = %e, "Calling code: user lookup failed");
                return finish(500, "Storage Error").await;
            }
        };
        // An assistant's opt-in is on each executive's settings (§3.10).
        if let fcp_pbx::CallingCode::AssistantOn | fcp_pbx::CallingCode::AssistantOff = code {
            let on = code == fcp_pbx::CallingCode::AssistantOn;
            return match crate::executive::set_opted_in(storage, &tenant_id, &user.id, on).await {
                Ok(executives) if executives.is_empty() => finish(404, "Not an Assistant").await,
                Ok(executives) => {
                    info!(call_id = %call_id, user_id = %user.id, on, executives = executives.len(), "An assistant opted in or out");
                    for executive in executives {
                        self.call_manager.publish(fcp_events::PlatformEvent::User(
                            fcp_events::UserEvent::CallingChanged {
                                user_id: executive.to_string(),
                                username: String::new(),
                                tenant_id,
                                timestamp: std::time::SystemTime::now(),
                            },
                        ));
                    }
                    self.confirm_feature(request, handle, call_id, calling_code_confirmation(&code))
                        .await
                }
                Err(e) => {
                    error!(call_id = %call_id, error = %e, "An assistant's opt-in not stored");
                    finish(500, "Storage Error").await
                }
            };
        }
        let confirmation = calling_code_confirmation(&code);
        let mut settings = crate::ring_plan::settings_for(storage, &tenant_id, &user.id).await;
        let target = |dialed: &str| fcp_core::CallTarget::parse(dialed);
        let executive_on = code == fcp_pbx::CallingCode::ExecutiveOn;
        use fcp_pbx::CallingCode::*;
        let changed: Result<(), fcp_core::CallTargetError> = match code {
            DndOn => {
                settings.dnd = true;
                Ok(())
            }
            DndOff => {
                settings.dnd = false;
                Ok(())
            }
            ForwardAllOn { target: t } => target(&t).map(|t| settings.forward_all = Some(t)),
            ForwardAllOff => {
                settings.forward_all = None;
                Ok(())
            }
            ForwardBusyOn { target: t } => target(&t).map(|t| settings.forward_busy = Some(t)),
            ForwardBusyOff => {
                settings.forward_busy = None;
                Ok(())
            }
            ForwardNoAnswerOn { target: t } => {
                target(&t).map(|t| settings.forward_no_answer = Some(t))
            }
            ForwardNoAnswerOff => {
                settings.forward_no_answer = None;
                Ok(())
            }
            CallWaitingOn => {
                settings.call_waiting = true;
                Ok(())
            }
            CallWaitingOff => {
                settings.call_waiting = false;
                Ok(())
            }
            FindMeOn => {
                settings
                    .find_me
                    .get_or_insert_with(Default::default)
                    .enabled = true;
                Ok(())
            }
            FindMeOff => {
                if let Some(find_me) = settings.find_me.as_mut() {
                    find_me.enabled = false;
                }
                Ok(())
            }
            ExecutiveOn | ExecutiveOff => {
                if settings
                    .executive
                    .as_ref()
                    .is_none_or(|e| e.assistants.is_empty())
                {
                    return finish(404, "No Assistants").await;
                }
                if let Some(executive) = settings.executive.as_mut() {
                    executive.enabled = executive_on;
                }
                Ok(())
            }
            AssistantOn | AssistantOff => Ok(()),
        };
        if let Err(e) = changed {
            warn!(call_id = %call_id, caller = %caller, error = %e, "Calling code with a bad target");
            return finish(404, "Invalid Forward Target").await;
        }
        settings.updated_at = std::time::SystemTime::now();
        if let Err(e) = settings.validate() {
            warn!(call_id = %call_id, caller = %caller, error = %e, "Calling code refused");
            return finish(404, "Invalid Forward Target").await;
        }
        if let Err(e) = storage.put_calling_settings(settings).await {
            error!(call_id = %call_id, caller = %caller, error = %e, "Calling code: settings not stored");
            return finish(500, "Storage Error").await;
        }
        info!(call_id = %call_id, caller = %caller, user_id = %user.id, "Calling settings changed by feature code");
        self.call_manager.publish(fcp_events::PlatformEvent::User(
            fcp_events::UserEvent::CallingChanged {
                user_id: user.id.to_string(),
                username: user.username.clone(),
                tenant_id,
                timestamp: std::time::SystemTime::now(),
            },
        ));
        self.confirm_feature(request, handle, call_id, confirmation)
            .await
    }

    ///
    /// Resolves the destination (including "voicemail" → voicemail server URI),
    /// creates a new B-leg INVITE, and runs parallel forking for the CFNA target.
    fn route_cfna<'a>(
        &'a self,
        original_request: &'a sip_core::Request,
        a_leg_handle: &'a Arc<ServerTransactionHandle>,
        call_id: fcp_b2bua_core::CallId,
        destination: &'a str,
        original_callee: Option<&'a str>,
        reason: Option<&'a str>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let uac = self
                .uac
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("UAC not configured — cannot route CFNA"))?;
            if let Some(reason) = reason {
                let reason = crate::trunk_egress::diversion_reason(reason);
                let _ = self
                    .call_manager
                    .update_call(&call_id, |c| c.forward_reason = Some(reason));
            }

            // A number or an extension is a new call into this node, which
            // rings it by its own plan and ring times.
            let into_node = !destination.eq_ignore_ascii_case("voicemail")
                && !destination.starts_with("sip:")
                && !destination.starts_with("sips:");
            let mailbox_id = if destination.eq_ignore_ascii_case("voicemail") {
                self.mailbox_of_callee(call_id).await.map(|mb| mb.id)
            } else {
                None
            };
            // Resolve the target URI
            let target_uri = if destination.eq_ignore_ascii_case("voicemail") {
                // Read the runtime-reloadable override first, fall back to static config
                let override_guard = self.voicemail_server_uri_override.read().unwrap();
                let vm_uri_owned = override_guard
                .clone()
                .or_else(|| self.b2bua_config.voicemail_server_uri.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!("CFNA destination is 'voicemail' but voicemail_server_uri is not configured")
                })?;
                drop(override_guard);
                let vm_uri = &vm_uri_owned;

                // Extract host:port from the configured voicemail server URI
                let vm_host = vm_uri
                    .strip_prefix("sip:")
                    .or_else(|| vm_uri.strip_prefix("sips:"))
                    .unwrap_or(vm_uri)
                    .split('@')
                    .next_back()
                    .unwrap_or(vm_uri);

                // The callee's mailbox by its id when the callee is a user:
                // the dialed name may be one the voicemail server does not
                // route (`docs/DEEP_TESTING.md` D8, findings 37–38). Else
                // the original callee as the user part.
                let user = mailbox_id
                    .as_deref()
                    .or(original_callee)
                    .unwrap_or("unknown");
                // Strip sip: prefix and @host from callee to get just the user part
                let user_part = user
                    .strip_prefix("sip:")
                    .or_else(|| user.strip_prefix("sips:"))
                    .unwrap_or(user)
                    .split('@')
                    .next()
                    .unwrap_or(user);

                if let Some(r) = reason {
                    format!("sip:{}@{};reason={}", user_part, vm_host, r)
                } else {
                    format!("sip:{}@{}", user_part, vm_host)
                }
            } else if destination.starts_with("sip:") || destination.starts_with("sips:") {
                destination.to_string()
            } else {
                // A number or an extension: a new call into this node, at
                // the port it listens on.
                let host = match self.uas.local_uri.port() {
                    Some(port) => format!("{}:{}", self.uas.local_uri.host(), port),
                    None => self.uas.local_uri.host().to_string(),
                };
                format!("sip:{}@{}", destination, host)
            };

            info!(
                call_id = %call_id,
                destination = %destination,
                target_uri = %target_uri,
                "CFNA: creating B-leg to forward destination"
            );

            // Rewrite SDP for media anchoring so the CFNA B-leg goes through
            // call-manager's media session (not directly to the caller).
            let sdp_override = if let Some(original_sdp) =
                crate::body::request_sdp(original_request)
            {
                let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                    if call.media_anchored {
                        call.media_session_id
                    } else {
                        None
                    }
                });
                if let (Some(media_session_id), Some(media_manager)) =
                    (media_session_id, &self.media_manager)
                {
                    match media_manager.get_session(&media_session_id).await {
                        Ok(session) => {
                            let (rtp_port, _rtcp_port) =
                                session.ports_for(fcp_media::backend::Leg::B);
                            let rewritten = fcp_media::rewrite_sdp_for_anchoring(
                                &original_sdp,
                                &self.local_addr,
                                rtp_port,
                                None,
                            );
                            info!(
                                call_id = %call_id,
                                server_ip = %self.local_addr,
                                rtp_port = rtp_port,
                                "CFNA: rewrote SDP for media anchoring"
                            );
                            Some(rewritten)
                        }
                        Err(e) => {
                            warn!(call_id = %call_id, error = %e, "CFNA: failed to get media session for SDP rewrite");
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Preserve caller identity from the original A-leg request
            let caller_display_name = extract_from_display_name(original_request);
            let caller_from_uri = extract_from_uri(original_request)
                .ok()
                .map(normalize_from_uri);
            uac.inner().set_display_name(caller_display_name).await;
            uac.inner().set_from_uri(caller_from_uri).await;

            // Create the outbound call
            let outbound_call = self
                .create_outbound_call_for(
                    original_request,
                    &target_uri,
                    uac,
                    sdp_override,
                    Some(call_id),
                )
                .await?;

            // Clear caller identity overrides after INVITE sent
            uac.inner().set_display_name(None).await;
            uac.inner().set_from_uri(None).await;

            // Create a fresh cancel_notify for the CFNA B-leg
            let cancel_notify = Arc::new(tokio::sync::Notify::new());
            let target_uri_clone = target_uri.clone();
            let _ = self.call_manager.update_call(&call_id, |call| {
                call.cancel_notify = Some(Arc::clone(&cancel_notify));
                // Replace old B-leg with a new one for the CFNA target so that
                // handle_parallel_forking can store the dialog + handle on it.
                call.b_leg = Some(fcp_b2bua_core::CallLeg::new_outbound(
                    target_uri_clone,
                    None,
                ));
            });

            // Run parallel forking with the single CFNA B-leg. The forward's
            // own legs have no treatments: a forward that fails is the
            // caller's answer, not another forward.
            // Nor the forwarding user's ring time, which has run out by
            // now: a leg into this node gives up by its own plan, and any
            // other rings for the node's time.
            let _ = self.call_manager.update_call(&call_id, |call| {
                call.line_binding_id = None;
                call.treatments = None;
                call.no_answer_timeout = into_node.then_some(FORWARD_INTO_NODE_LIMIT);
            });

            self.handle_parallel_forking(
                original_request.clone(),
                Arc::clone(a_leg_handle),
                vec![(target_uri, outbound_call, None)],
                call_id,
                cancel_notify,
                None,
            )
            .await
        }) // end Box::pin(async move { ... })
    }

    /// Create an outbound call to a target URI via the UAC.
    ///
    /// Extracts SDP from the A-leg request (if present) and sends INVITE to the
    /// target URI. Returns an Arc<OutboundCall> ready for `handle_parallel_forking(, None)`.
    ///
    /// If `sdp_override` is provided, it is used instead of the A-leg request body.
    /// This is used by CFNA routing to send a media-anchored SDP to the B-leg.
    async fn create_outbound_call(
        &self,
        request: &sip_core::Request,
        target_uri: &str,
        uac: &Arc<SipUserAgentClient>,
        sdp_override: Option<String>,
    ) -> Result<Arc<crate::uac::OutboundCall>> {
        self.create_outbound_call_for(request, target_uri, uac, sdp_override, None)
            .await
    }

    /// A user's registrations when they are dialed by name at an address
    /// that is not the one they registered under — this node's own, as a
    /// transfer dials back into it: the user of that name in the call's
    /// tenant, at each of the tenant's SIP domains.
    async fn registrations_of_named_user(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        destination_aor: &str,
    ) -> Vec<sip_registrar::Binding> {
        use fcp_core::storage::{CredentialStore, TenantStore};
        let (Some(storage), Some(registrar)) = (&self.storage, &self.registrar) else {
            return Vec::new();
        };
        let Some(tenant_id) = self.call_manager.get_call(call_id).map(|c| c.tenant_id) else {
            return Vec::new();
        };
        let bare = destination_aor
            .strip_prefix("sip:")
            .or_else(|| destination_aor.strip_prefix("sips:"))
            .unwrap_or(destination_aor);
        let Some((dialed, _)) = bare.split_once('@') else {
            return Vec::new();
        };
        let Some(user_id) = crate::ring_plan::user_of(storage, &tenant_id, dialed).await else {
            return Vec::new();
        };
        let Ok(Some(user)) = storage.get_user_by_id(&user_id).await else {
            return Vec::new();
        };
        let domains = match storage.get_tenant(&tenant_id).await {
            Ok(Some(tenant)) => tenant.sip_domains,
            _ => return Vec::new(),
        };
        let mut found = Vec::new();
        for domain in domains {
            let aor = format!("sip:{}@{}", user.sip_name(), domain);
            if let Ok(bindings) = registrar
                .get_registrations_for_tenant(&tenant_id, &aor)
                .await
            {
                found.extend(bindings);
            }
        }
        found
    }

    /// The tenant of the call an INVITE continues, when this node placed it
    /// to itself: it comes from this node's own SIP address and port and
    /// names, as its transfer origin or the call it forwards for, a call
    /// this node has (the headers survive ingress only then, `claims`).
    fn own_leg_tenant(
        &self,
        request: &sip_core::Request,
        peer: std::net::SocketAddr,
    ) -> Option<fcp_core::TenantId> {
        let own_port = self.uas.local_uri.port().unwrap_or(5060);
        let own_ip = self.local_addr.parse::<std::net::IpAddr>().ok();
        let from_here = peer.port() == own_port
            && (Some(peer.ip()) == own_ip
                || peer.ip().is_loopback()
                || self.uas.local_uri.host() == peer.ip().to_string());
        if !from_here {
            return None;
        }
        let continued = [TRANSFERRED_FROM_HEADER, FORWARDED_FOR_HEADER]
            .into_iter()
            .find_map(|name| sip_parse::header(request.headers(), name))?;
        let id = continued.as_str().trim().parse::<fcp_core::CallId>().ok()?;
        let call = self
            .call_manager
            .get_call(&fcp_b2bua_core::CallId::from_uuid(*id.as_uuid()))?;
        debug!(source = %peer, call_id = %call.id, "INVITE is a leg this node placed for one of its calls");
        Some(call.tenant_id)
    }

    /// [`Self::create_outbound_call`] for a leg forwarding `forwarded_for`:
    /// a leg to this node itself names the call it forwards for
    /// ([`FORWARDED_FOR_HEADER`]), so the new call is not checked against
    /// the original caller's class of service.
    async fn create_outbound_call_for(
        &self,
        request: &sip_core::Request,
        target_uri: &str,
        uac: &Arc<SipUserAgentClient>,
        sdp_override: Option<String>,
        forwarded_for: Option<fcp_b2bua_core::CallId>,
    ) -> Result<Arc<crate::uac::OutboundCall>> {
        // Use SDP override if provided, otherwise extract from A-leg request
        let mut a_leg_sdp = sdp_override.or_else(|| crate::body::request_sdp(request));

        if Self::is_oneway_intercom_request(request) {
            a_leg_sdp = Some(generate_hold_sdp(
                a_leg_sdp.as_deref(),
                &self.local_addr,
                self.rtp_port,
            ));
        }

        let mut extra_headers = self.collect_outbound_intercom_headers(request);
        if let Some(call_id) = forwarded_for {
            let own = match self.uas.local_uri.port() {
                Some(port) => format!("@{}:{}", self.uas.local_uri.host(), port),
                None => format!("@{}", self.uas.local_uri.host()),
            };
            if target_uri.contains(&own) {
                let _ = extra_headers.push(
                    SmolStr::new(FORWARDED_FOR_HEADER),
                    SmolStr::new(call_id.to_string()),
                );
            }
        }
        let call = if extra_headers.iter().next().is_some() {
            uac.invite_with_headers(target_uri, a_leg_sdp.as_deref(), extra_headers)
                .await?
        } else {
            uac.invite(target_uri, a_leg_sdp.as_deref()).await?
        };
        Ok(Arc::new(call))
    }

    // =========================================================================
    // Notification calls
    // =========================================================================

    /// Where a notification call to `to` goes: a SIP URI as given, else
    /// one of the tenant's users or extensions, rung at a phone of theirs
    /// that is registered (`docs/E911_BLOCK_2.md` §8.1).
    async fn notification_target(
        &self,
        tenant_id: &fcp_core::TenantId,
        to: &str,
    ) -> Result<String> {
        if to.starts_with("sip:") || to.starts_with("sips:") {
            return Ok(to.to_string());
        }
        let (Some(storage), Some(registrar)) = (&self.storage, &self.registrar) else {
            anyhow::bail!("no registrar here to find {to} in");
        };
        let user = crate::ring_plan::user_of(storage, tenant_id, to)
            .await
            .ok_or_else(|| {
                anyhow::anyhow!("{to} is not one of the tenant's users or extensions")
            })?;
        let contacts =
            crate::ring_plan::contacts_for_user(storage, registrar, tenant_id, &user, "").await;
        contacts
            .first()
            .map(crate::ring_plan::contact_uri)
            .ok_or_else(|| anyhow::anyhow!("{to} has no phone registered"))
    }

    /// Ring `to`, and when someone answers speak `text` to them: the call
    /// channel of on-site emergency notification (`docs/E911_BLOCK_2.md`
    /// §8.1). The events server asks for this over HTTP, since it has no
    /// SIP stack of its own.
    ///
    /// Returns what became of the call. Only `answered` told anybody
    /// anything; the caller decides whether to ring again.
    pub async fn announce_call(
        &self,
        tenant_id: &fcp_core::TenantId,
        to: &str,
        text: &str,
        ring_timeout: std::time::Duration,
        repeat: u32,
        listen_to: Option<fcp_b2bua_core::CallId>,
    ) -> Result<AnnouncementOutcome> {
        let uac = self.uac.as_ref().ok_or_else(|| {
            anyhow::anyhow!("UAC not configured — cannot place notification calls")
        })?;
        let media_manager = self.media_manager.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "media manager not configured — a notification call has nothing to speak with"
            )
        })?;
        if media_manager.tts_service().is_none() {
            anyhow::bail!("no TTS service configured — a notification call has no voice");
        }
        let target = self.notification_target(tenant_id, to).await?;

        // A media session of our own: this call has one leg, and what it
        // plays is played into that leg.
        let session_id = fcp_core::CallId::new();
        let dialog_id = fcp_core::DialogId::new();
        media_manager
            .create_session_default(session_id, dialog_id)
            .await
            .context("could not create the notification call's media session")?;
        let session = media_manager.get_session(&session_id).await?;
        // The callee sits in slot A (`update_session` below).
        let (rtp_port, _rtcp_port) = session.ports_for(fcp_media::backend::Leg::A);
        let offer = generate_basic_sdp(&self.local_addr, rtp_port);

        let outcome = self
            .run_announcement(
                uac,
                media_manager,
                &target,
                text,
                &offer,
                session_id,
                ring_timeout,
                repeat,
                listen_to,
            )
            .await;
        let _ = media_manager.terminate_session(&session_id).await;
        match &outcome {
            Ok(outcome) => {
                info!(target = %target, outcome = %outcome, "Notification call finished")
            }
            Err(e) => warn!(target = %target, error = %e, "Notification call failed"),
        }
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_announcement(
        &self,
        uac: &Arc<SipUserAgentClient>,
        media_manager: &Arc<fcp_media::MediaManager>,
        target: &str,
        text: &str,
        offer: &str,
        session_id: fcp_core::CallId,
        ring_timeout: std::time::Duration,
        repeat: u32,
        listen_to: Option<fcp_b2bua_core::CallId>,
    ) -> Result<AnnouncementOutcome> {
        let handle = uac
            .invite(target, Some(offer))
            .await
            .with_context(|| format!("INVITE to {target} failed"))?;

        let response = match tokio::time::timeout(ring_timeout, handle.await_final()).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => return Err(anyhow::anyhow!("notification call failed: {e}")),
            Err(_) => {
                let _ = handle.cancel().await;
                return Ok(AnnouncementOutcome::NoAnswer);
            }
        };
        let status = response.start_line().code();
        if !(200..300).contains(&status) {
            return Ok(match status {
                486 | 600 | 603 => AnnouncementOutcome::Busy,
                408 | 480 | 487 => AnnouncementOutcome::NoAnswer,
                _ => AnnouncementOutcome::Failed(status),
            });
        }

        // Answered: point the media session at whoever picked up and speak.
        if let Some(answer) = crate::body::sdp_of(response.headers(), response.body()) {
            media_manager
                .update_session(&session_id, answer)
                .await
                .context("could not set the notification call's media endpoint")?;
        }
        for _ in 0..repeat.max(1) {
            match media_manager
                .play_tts(
                    &session_id,
                    text.to_string(),
                    "en".to_string(),
                    None,
                    fcp_media::AudioTarget::Both,
                )
                .await
            {
                Ok(playback) => {
                    if let Err(e) = media_manager.wait_playback_completion(playback).await {
                        warn!(error = %e, "Notification call playback did not finish");
                        break;
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Notification call could not speak");
                    break;
                }
            }
        }

        let dialog = handle.dialog.read().await.clone();
        if let Some(call_id) = listen_to {
            if self
                .offer_listening(media_manager, session_id, &call_id, target, dialog.clone())
                .await
            {
                // The contact's call is the emergency call's supervisor now.
                return Ok(AnnouncementOutcome::Answered);
            }
        }
        if let Err(e) = uac.bye(&dialog).await {
            debug!(error = %e, "Notification call BYE failed");
        }
        Ok(AnnouncementOutcome::Answered)
    }

    /// Say `text` on a notification call's own session and wait for it.
    async fn say_on_session(
        media_manager: &Arc<fcp_media::MediaManager>,
        session_id: &fcp_core::CallId,
        text: &str,
    ) {
        if let Ok(playback) = media_manager
            .play_tts(
                session_id,
                text.to_string(),
                "en-US".to_string(),
                None,
                fcp_media::AudioTarget::Both,
            )
            .await
        {
            let _ = media_manager.wait_playback_completion(playback).await;
        }
    }

    /// Offer an emergency contact who may listen in the choice, once told
    /// (`docs/E911.md` "Listening in"): 1 to listen, 2 to listen and
    /// speak. A choice moves their call onto the emergency call as its
    /// supervisor, once that call is answered; `true` when it did, and the
    /// notification call is theirs to end.
    async fn offer_listening(
        &self,
        media_manager: &Arc<fcp_media::MediaManager>,
        session_id: fcp_core::CallId,
        call_id: &fcp_b2bua_core::CallId,
        party: &str,
        dialog: sip_dialog::Dialog,
    ) -> bool {
        const MENU: &str = "To listen to the call, press 1. To listen and speak, press 2.";
        let Some(supervisions) = self.supervisions.clone() else {
            return false;
        };
        if self.call_manager.get_call(call_id).is_none() {
            Self::say_on_session(media_manager, &session_id, "The emergency call has ended.").await;
            return false;
        }
        let mut mode = None;
        for _ in 0..2 {
            // Made before the prompt, so a key pressed over it counts.
            let collector = media_manager.create_dtmf_collector(&session_id, 1, None);
            Self::say_on_session(media_manager, &session_id, MENU).await;
            let digits = match collector.collect(std::time::Duration::from_secs(6)).await {
                fcp_media::CollectionResult::Complete(d)
                | fcp_media::CollectionResult::Timeout(d) => d,
                fcp_media::CollectionResult::Cancelled => String::new(),
            };
            mode = match digits.as_str() {
                "1" => Some(fcp_media::Supervision::Monitor),
                "2" => Some(fcp_media::Supervision::Barge),
                _ => None,
            };
            if mode.is_some() {
                break;
            }
        }
        let Some(mode) = mode else {
            return false;
        };

        // A live call may still be ringing at the emergency service.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut told_to_wait = false;
        loop {
            match self.call_manager.get_call(call_id) {
                None => {
                    Self::say_on_session(
                        media_manager,
                        &session_id,
                        "The emergency call has ended.",
                    )
                    .await;
                    return false;
                }
                Some(call) if call.state == fcp_b2bua_core::CallState::Established => break,
                Some(_) if std::time::Instant::now() >= deadline => {
                    Self::say_on_session(
                        media_manager,
                        &session_id,
                        "The emergency call was not answered in time.",
                    )
                    .await;
                    return false;
                }
                Some(_) => {
                    if !told_to_wait {
                        told_to_wait = true;
                        Self::say_on_session(
                            media_manager,
                            &session_id,
                            "The call is connecting. Please hold.",
                        )
                        .await;
                    } else {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }

        match supervisions.join_dialog(call_id, party, mode, dialog).await {
            Ok(()) => true,
            Err(e) => {
                warn!(call_id = %call_id, party, error = %e, "An emergency contact could not listen in");
                let why = match e {
                    crate::supervision::SuperviseError::AlreadySupervised => {
                        "Someone else is already listening to the call."
                    }
                    crate::supervision::SuperviseError::NoSuchCall => {
                        "The emergency call has ended."
                    }
                    _ => "The call cannot be listened to.",
                };
                Self::say_on_session(media_manager, &session_id, why).await;
                false
            }
        }
    }

    /// Put `party` of a call on hold from the node
    /// (`docs/DEVELOPER_SURFACE.md` phase 8): the party hears the hold
    /// music and the other party stops hearing them, as for a phone's own
    /// hold. Needs the call's media through the node.
    pub fn hold_call(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        party: fcp_b2bua_core::LegSide,
    ) -> std::result::Result<(), crate::call_control::CallControlError> {
        use crate::call_control::CallControlError;
        let call = self
            .call_manager
            .get_call(call_id)
            .ok_or_else(|| CallControlError::not_found(call_id))?;
        if call.state != fcp_b2bua_core::CallState::Established {
            return Err(CallControlError::conflict("the call is not answered"));
        }
        if !call.media_anchored || call.media_session_id.is_none() {
            return Err(CallControlError::conflict(
                "the call's media does not pass through the node, so it cannot be held here",
            ));
        }
        match call.held {
            Some(held) if held == party => return Ok(()),
            Some(_) => {
                return Err(CallControlError::conflict(
                    "the other party is on hold; resume the call first",
                ))
            }
            None => {}
        }
        self.call_manager
            .update_call(call_id, |c| c.held = Some(party))
            .map_err(|_| CallControlError::not_found(call_id))?;
        info!(call_id = %call_id, party = party.as_str(), "Call held by the API");
        Ok(())
    }

    /// Take a call off hold.
    pub fn resume_call(
        &self,
        call_id: &fcp_b2bua_core::CallId,
    ) -> std::result::Result<(), crate::call_control::CallControlError> {
        use crate::call_control::CallControlError;
        let call = self
            .call_manager
            .get_call(call_id)
            .ok_or_else(|| CallControlError::not_found(call_id))?;
        if call.held.is_some() {
            self.call_manager
                .update_call(call_id, |c| c.held = None)
                .map_err(|_| CallControlError::not_found(call_id))?;
            info!(call_id = %call_id, "Call resumed by the API");
        }
        Ok(())
    }

    /// Transfer a call (`docs/DEVELOPER_SURFACE.md` phase 8): `keep` is
    /// connected to `target`, dialed back into this node so it is routed as
    /// any call is (a user, a number, a trunk, a flow), and the other party
    /// is dropped once the target answers. It runs in the background; the
    /// call's events say how it went.
    pub fn transfer_call(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        keep: fcp_b2bua_core::LegSide,
        target: &str,
    ) -> std::result::Result<(), crate::call_control::CallControlError> {
        use crate::call_control::CallControlError;
        let target = target.trim();
        if target.is_empty()
            || target.starts_with(crate::call_control::REDIRECT_PREFIX)
            || target.contains(|c: char| c.is_whitespace() || "<>@;?\"".contains(c))
        {
            return Err(CallControlError::bad_request(
                "a target is a user, an extension or a number",
            ));
        }
        self.start_transfer(call_id, keep, target)
    }

    /// Redirect a call into a flow: `keep` runs `flow_id` (the call's
    /// tenant's) and the other party is dropped.
    pub async fn redirect_call(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        keep: fcp_b2bua_core::LegSide,
        flow_id: &str,
    ) -> std::result::Result<(), crate::call_control::CallControlError> {
        use crate::call_control::CallControlError;
        let call = self
            .call_manager
            .get_call(call_id)
            .ok_or_else(|| CallControlError::not_found(call_id))?;
        let Some(flows) = self.flow_handler.as_ref() else {
            return Err(CallControlError::new(
                503,
                "flows are not available on this node",
            ));
        };
        let flow = flows
            .flow_store()
            .get_flow(flow_id)
            .await
            .map_err(|e| CallControlError::new(500, format!("reading the flow: {e}")))?
            .filter(|f| f.tenant_id.as_deref() == Some(call.tenant_id.to_string().as_str()))
            .ok_or_else(|| CallControlError::new(404, format!("Flow not found: {flow_id}")))?;
        if !flow.enabled {
            return Err(CallControlError::conflict(format!(
                "the flow {flow_id} is disabled"
            )));
        }
        let user = self.redirect_tickets.issue(flow_id, call.tenant_id);
        let started = self.start_transfer(call_id, keep, &user);
        if started.is_err() {
            self.redirect_tickets.cancel(&user);
        }
        started
    }

    fn start_transfer(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        keep: fcp_b2bua_core::LegSide,
        user: &str,
    ) -> std::result::Result<(), crate::call_control::CallControlError> {
        use crate::call_control::CallControlError;
        let call = self
            .call_manager
            .get_call(call_id)
            .ok_or_else(|| CallControlError::not_found(call_id))?;
        if call.state != fcp_b2bua_core::CallState::Established {
            return Err(CallControlError::conflict("the call is not answered"));
        }
        let Some(uac) = self.uac.clone() else {
            return Err(CallControlError::new(503, "this node places no calls"));
        };
        let referrer = keep.other();
        let referrer_dialog = match referrer {
            fcp_b2bua_core::LegSide::A => call.a_leg.dialog.clone(),
            fcp_b2bua_core::LegSide::B => call.b_leg.as_ref().and_then(|b| b.dialog.clone()),
        };
        if referrer_dialog.is_none() {
            return Err(CallControlError::conflict("the call has no party to drop"));
        }
        if self.conference_migrations.insert(call.id, ()).is_some() {
            return Err(CallControlError::conflict(
                "the call is already being transferred",
            ));
        }
        info!(call_id = %call.id, keep = keep.as_str(), target = %user, "Transfer asked by the API");
        let run = self.transfer_run(uac, &call, referrer, referrer_dialog, user, None, false);
        tokio::spawn(run.run());
        Ok(())
    }

    /// Play an audio file into a call, to one party or both; the playback's
    /// id. Needs the call's media through the node.
    pub async fn play_into_call(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        file: String,
        target: fcp_media::AudioTarget,
    ) -> std::result::Result<String, crate::call_control::CallControlError> {
        let (media, session) = self.answered_media(call_id)?;
        media
            .play_file(&session, file, target)
            .await
            .map_err(|e| crate::call_control::CallControlError::new(500, e.to_string()))
    }

    /// Speak text into a call with the node's text-to-speech.
    pub async fn say_into_call(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        text: &str,
        language: Option<&str>,
        voice: Option<&str>,
        target: fcp_media::AudioTarget,
    ) -> std::result::Result<String, crate::call_control::CallControlError> {
        let (media, session) = self.answered_media(call_id)?;
        media
            .play_tts(
                &session,
                text.to_string(),
                language.unwrap_or("en-US").to_string(),
                voice.map(str::to_string),
                target,
            )
            .await
            .map_err(|e| crate::call_control::CallControlError::new(503, e.to_string()))
    }

    fn answered_media(
        &self,
        call_id: &fcp_b2bua_core::CallId,
    ) -> std::result::Result<
        (Arc<fcp_media::MediaManager>, fcp_core::CallId),
        crate::call_control::CallControlError,
    > {
        use crate::call_control::CallControlError;
        let call = self
            .call_manager
            .get_call(call_id)
            .ok_or_else(|| CallControlError::not_found(call_id))?;
        if call.state != fcp_b2bua_core::CallState::Established {
            return Err(CallControlError::conflict("the call is not answered"));
        }
        let (Some(media), Some(session)) = (self.media_manager.clone(), call.media_session_id)
        else {
            return Err(CallControlError::conflict(
                "the call's media does not pass through the node",
            ));
        };
        Ok((media, session))
    }

    /// Move every answered conference call whose node has gone to the room
    /// on a node that is still up, or end it if none will have it
    /// (`docs/DEEP_TESTING.md` D5b). A conference node killed outright
    /// drains nothing and refers nobody, and a conference call's media
    /// does not pass through this node, so without this its callers sit
    /// on dead air for as long as they will hold. The caller joins the
    /// room afresh: a PIN room asks for its PIN again. The call manager
    /// runs this on a tick.
    pub async fn rescue_conference_calls(self: &Arc<Self>) {
        let Some(selector) = self.conference_node_selector.clone() else {
            return;
        };
        for call in self.call_manager.active_calls() {
            if call.state != fcp_b2bua_core::CallState::Established {
                continue;
            }
            let (Some(room), Some(host)) =
                (call.conference_room.clone(), call.conference_node.clone())
            else {
                continue;
            };
            if self.conference_migrations.contains_key(&call.id)
                || !selector.conference_node_gone(&host).await
            {
                continue;
            }
            let Some(old_b_dialog) = call.b_leg.as_ref().and_then(|b| b.dialog.clone()) else {
                continue;
            };
            let (Some(uac), Some(_)) = (self.uac.clone(), call.a_leg.dialog.as_ref()) else {
                continue;
            };
            let new_host = selector
                .select_conference_node(&call.tenant_id, &room, None)
                .await
                .filter(|h| *h != host);
            let target = new_host
                .as_deref()
                .and_then(|h| sip_core::SipUri::parse(&format!("sip:{room}@{h}")).ok());
            let Some(target) = target else {
                warn!(call_id = %call.id, room = %room, node = %host, "A caller's conference node has gone and no other will take the room; ending the call");
                self.end_rescued_call(&call.id);
                continue;
            };
            if self.conference_migrations.insert(call.id, ()).is_some() {
                continue;
            }
            warn!(
                call_id = %call.id,
                room = %room,
                from = %host,
                to = %target.as_str(),
                "A caller's conference node has gone; moving them to another"
            );
            let ctx = ConferenceMigration {
                uac,
                local_uri: self.uas.local_uri.clone(),
                call_manager: self.call_manager.clone(),
                dialog_manager: Arc::clone(&self.dialog_manager),
                session_timer_manager: Arc::clone(&self.session_timer_manager),
                media_manager: self.media_manager.clone(),
                sip_call_id_map: Arc::clone(&self.sip_call_id_map),
                in_flight: Arc::clone(&self.conference_migrations),
                b2bua_config: self.b2bua_config.clone(),
                local_addr: self.local_addr.clone(),
                call_id: call.id,
                old_b_dialog,
                target,
                ticket: None,
                old_node_gone: true,
            };
            let this = Arc::clone(self);
            tokio::spawn(async move {
                let call_id = ctx.call_id;
                match ctx.rescue().await {
                    Ok(()) => {
                        info!(call_id = %call_id, "Conference caller moved off a node that has gone")
                    }
                    Err((code, _, why)) => {
                        warn!(call_id = %call_id, code, error = %why, "Conference caller not moved off a node that has gone; ending the call");
                        this.end_rescued_call(&call_id);
                    }
                }
            });
        }
    }

    /// End a call whose conference node has gone: a BYE to the caller
    /// only, since the node's side of the leg went with the node.
    fn end_rescued_call(&self, call_id: &fcp_b2bua_core::CallId) {
        let Some(mut call) = self.call_manager.get_call(call_id) else {
            return;
        };
        if let Some(b) = call.b_leg.as_mut() {
            if let Some(dialog) = b.dialog.take() {
                self.dialog_manager.remove(dialog.id());
                self.session_timer_manager.stop_timer(dialog.id());
            }
        }
        let _ = self.call_manager.update_call(call_id, |c| {
            if let Some(b) = c.b_leg.as_mut() {
                b.dialog = None;
            }
        });
        let _ = self.hangup_call_by_id_saying(
            call_id,
            fcp_cdr::TerminationReason::system(
                fcp_cdr::SystemCause::NodeFailed,
                "conference node gone",
            ),
            Some("SIP;cause=503;text=\"conference node gone\""),
        );
    }

    /// Hangs up an active call by sending BYE on both legs and terminating the call.
    ///
    /// This method retrieves the call from CallManager, sends BYE on each leg that
    /// has an active dialog, and then terminates the call (CDR, events, removal).
    pub fn hangup_call_by_id(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        reason: Option<String>,
    ) -> Result<()> {
        self.hangup_call_by_id_with(call_id, fcp_cdr::TerminationReason::from(reason))
    }

    /// Hang up both legs of a call for a reason the caller knows precisely
    /// (`docs/CDR_PIPELINE.md`): the record carries it, and the media
    /// counters taken before the legs go.
    pub fn hangup_call_by_id_with(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        reason: fcp_cdr::TerminationReason,
    ) -> Result<()> {
        self.hangup_call_by_id_saying(call_id, reason, None)
    }

    /// As [`hangup_call_by_id_with`](Self::hangup_call_by_id_with), with
    /// an RFC 3326 `Reason` on both legs' BYEs, so the far ends can tell
    /// the platform's hangup from a party's
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    pub fn hangup_call_by_id_saying(
        &self,
        call_id: &fcp_b2bua_core::CallId,
        reason: fcp_cdr::TerminationReason,
        sip_reason: Option<&str>,
    ) -> Result<()> {
        let call = self
            .call_manager
            .get_call(call_id)
            .ok_or_else(|| anyhow::anyhow!("Call not found: {}", call_id))?;
        self.call_manager
            .set_pending_termination(call_id, reason.clone());

        if let Some(uac) = &self.uac {
            if let Some(dialog) = &call.a_leg.dialog {
                self.spawn_bye_with_reason(
                    Arc::clone(uac),
                    dialog.clone(),
                    *call_id,
                    true,
                    None, // A-leg doesn't use flow tokens
                    sip_reason.map(str::to_string),
                );
            }
            if let Some(b_leg) = &call.b_leg {
                if let Some(dialog) = &b_leg.dialog {
                    self.spawn_bye_with_reason(
                        Arc::clone(uac),
                        dialog.clone(),
                        *call_id,
                        false,
                        b_leg.flow_token.clone(),
                        sip_reason.map(str::to_string),
                    );
                }
            }
        }

        self.end_call_after_media(&call, reason, false, "hangup");
        Ok(())
    }

    /// Start the recording a call's INVITE decided on
    /// ([`Self::automatic_recording`]), on a task of its own: the consent
    /// announcement plays while the call goes on.
    fn start_automatic_recording(&self, call_id: &fcp_b2bua_core::CallId) {
        let Some(source) = self
            .call_manager
            .get_call(call_id)
            .and_then(|call| call.auto_recording)
        else {
            return;
        };
        let recorder = Arc::clone(&self.call_recorder);
        let call_id = *call_id;
        tokio::spawn(async move {
            use crate::recording::{RecordingError, RecordingPath};
            match recorder.start(&call_id, source, RecordingPath::wav()).await {
                Ok(_) | Err(RecordingError::AlreadyRecording) => {}
                Err(RecordingError::ConsentRefused) | Err(RecordingError::CallEnded) => {}
                Err(e) => warn!(
                    call_id = %call_id,
                    source = source.as_str(),
                    error = %e,
                    "Automatic call recording not started"
                ),
            }
        });
    }

    /// Keep the media session's current counters on the call for its
    /// record, without ending anything. On a task of its own.
    fn capture_rtp_stats(&self, call: &Call) {
        let (true, Some(session_id), Some(mm)) = (
            call.media_anchored,
            call.media_session_id,
            &self.media_manager,
        ) else {
            return;
        };
        let call_manager = Arc::clone(&self.call_manager);
        let media_manager = Arc::clone(mm);
        let call_id = call.id;
        tokio::spawn(async move {
            if let Ok(stats) = media_manager.get_session_stats(&session_id).await {
                call_manager.set_rtp_stats(&call_id, rtp_stats_from(&stats));
            }
        });
    }

    /// End `call` for `reason` once its media session, if any, has given up
    /// its RTP counters for the record, and — when `teardown` — been torn
    /// down. Runs on a task of its own; the call leaves the manager when
    /// the task is done, a few milliseconds after the signaling did.
    fn end_call_after_media(
        &self,
        call: &Call,
        reason: fcp_cdr::TerminationReason,
        teardown: bool,
        what: &'static str,
    ) {
        let call_manager = Arc::clone(&self.call_manager);
        let recorder = Arc::clone(&self.call_recorder);
        let call_id = call.id;
        let media = anchored_media(call, self.media_manager.as_ref());
        tokio::spawn(async move {
            recorder.call_ended(&call_id).await;
            close_media(&call_manager, &call_id, media, teardown, what).await;
            let _ = call_manager.terminate_call_with(&call_id, reason);
        });
    }

    /// Sends an MWI (Message Waiting Indicator) NOTIFY to a SIP endpoint.
    ///
    /// Formats an RFC 3842 `application/simple-message-summary` body and sends
    /// an unsolicited NOTIFY to the given target URI.
    pub async fn send_mwi_notify(
        &self,
        target_uri: &sip_core::SipUri,
        mwi_status: &fcp_core::MwiStatus,
    ) -> Result<()> {
        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("UAC not configured — cannot send MWI NOTIFY"))?;

        let messages_waiting = if mwi_status.has_messages() {
            "yes"
        } else {
            "no"
        };

        let body = format!(
            "Messages-Waiting: {}\r\nVoice-Message: {}/{} ({}/{})\r\n",
            messages_waiting,
            mwi_status.new_messages,
            mwi_status.old_messages,
            mwi_status.urgent_new_messages,
            mwi_status.urgent_old_messages,
        );

        let response = uac
            .send_unsolicited_notify(
                target_uri,
                "message-summary",
                "application/simple-message-summary",
                &body,
            )
            .await?;

        if response.code() >= 200 && response.code() < 300 {
            info!(target = %target_uri.as_str(), "MWI NOTIFY accepted");
        } else {
            warn!(
                target = %target_uri.as_str(),
                code = response.code(),
                "MWI NOTIFY rejected"
            );
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl InboundMessageHandler for SipMessageHandler {
    async fn handle_inbound(
        &self,
        transport: sip_transaction::TransportKind,
        peer: SocketAddr,
        payload: Bytes,
        stream: Option<mpsc::Sender<Bytes>>,
        peer_identity: Option<Arc<sip_transport::PeerIdentity>>,
    ) -> Result<()> {
        // What this node accepts at its SIP port
        // (`docs/EDGE_SECURITY.md` §2.4). Before the parser, and every
        // refusal here is silent: answering an address the node denies
        // confirms the port is live and makes it a reflector, and a
        // message that is not SIP has nothing to answer to.
        if let Some(guard) = &self.edge_guard {
            if let crate::edge::EdgeVerdict::Drop(why) = guard.admit_bytes(peer.ip(), &payload) {
                self.metrics
                    .inc_counter("fcp_sip_edge_dropped_total", &[("reason", why)]);
                return Ok(());
            }
        }

        // Convert transport to string to avoid non-Send issues
        let transport_str = format!("{:?}", transport);
        debug!(
            transport = %transport_str,
            peer = %peer,
            size = payload.len(),
            "Handling inbound SIP message"
        );

        // Capture raw payload as string for SIP trace
        let raw_payload_str = String::from_utf8_lossy(&payload);

        // Try parsing as request first, then response
        if let Some(mut req) = sip_parse::parse_request(&payload) {
            // A caller behind NAT is reached inside its dialog where its
            // requests come from (`crate::nat`); a callee's dialog already
            // sends where it was rung.
            if matches!(transport, sip_transaction::TransportKind::Udp)
                && req.method() != &sip_core::Method::Register
            {
                crate::nat::fix_contact(req.headers_mut(), peer);
            }
            // Look up FCP internal CallId from the SIP Call-ID header
            // Priority: 1) Fast DashMap lookup, 2) Call manager dialog scan, 3) Raw SIP Call-ID
            let sip_cid = sip_parse::header(req.headers(), "Call-ID");
            let fcp_call_id_str = sip_cid
                .as_ref()
                .and_then(|cid| {
                    self.sip_call_id_map
                        .get(cid.as_str())
                        .map(|v| v.value().clone())
                        .or_else(|| {
                            self.call_manager
                                .find_by_dialog_call_id(cid.as_str())
                                .map(|call| call.id.to_string())
                        })
                })
                .or_else(|| sip_cid.as_ref().map(|s| s.as_str().to_string()));

            self.log_sip_message(
                "inbound",
                req.method().as_str(),
                fcp_call_id_str.as_deref(),
                self.dialog_manager
                    .find_by_request(&req)
                    .map(|d| d.id().clone())
                    .as_ref(),
                sip_parse::header(req.headers(), "CSeq").map(|s| s.as_str()),
                Some(peer),
                Some(match transport {
                    sip_transaction::TransportKind::Udp => "udp",
                    sip_transaction::TransportKind::Tcp => "tcp",
                    sip_transaction::TransportKind::Tls => "tls",
                    sip_transaction::TransportKind::Ws => "ws",
                    _ => "other",
                }),
                Some(req.headers()),
                Some(req.body()),
                Some(&raw_payload_str),
            );
            // Convert to string to avoid Send issues
            let method_str = req.method().as_str();
            debug!(
                method = %method_str,
                uri = %req.uri(),
                "Received SIP request"
            );

            // Special handling for ACK - it doesn't create a server transaction
            // ACK for 2xx responses is handled outside the transaction layer (RFC 3261 §17.1.1.3)
            if req.method() == &sip_core::Method::Ack {
                // An ACK for a non-2xx final (a 401 challenge, a 486) is the
                // INVITE server transaction's: absorbed, it stops the final's
                // retransmissions (RFC 3261 §17.2.1). Only an ACK nothing
                // absorbs — the one for a 2xx — is the B2BUA's. Without this
                // every challenged INVITE over UDP had its 401 resent until
                // Timer H.
                if let Some(branch) = sip_transaction::request_branch_id(&req) {
                    let key = sip_transaction::TransactionKey::new(
                        branch.as_str(),
                        sip_core::Method::Invite,
                        true,
                    );
                    if self.transaction_manager.ack_received(&key).await {
                        return Ok(());
                    }
                }
                if let Err(e) = self.handle_ack(&req) {
                    error!("Failed to handle ACK: {}", e);
                }
                return Ok(());
            }

            // Clone stream for REGISTER flow token support (RFC 5626)
            let stream_for_register = stream.clone();

            // Create transport context, carrying the peer's verified
            // certificate identity (mutual TLS) for transaction users.
            let ctx = sip_transaction::TransportContext::new(transport, peer, stream)
                .with_peer_identity(peer_identity.clone());

            // Create server transaction
            let handle = self
                .transaction_manager
                .receive_request(req.clone(), ctx)
                .await;

            // Handle request and send response(s)
            if let Err(e) = self
                .handle_request(
                    &req,
                    &handle,
                    peer,
                    transport,
                    stream_for_register,
                    peer_identity,
                )
                .await
            {
                // A request inside a dialog this node does not have is the
                // sender's mistake, not this node's (RFC 3261 §12.2.2).
                let response = if e.downcast_ref::<NoSuchDialog>().is_some() {
                    debug!(method = %req.method().as_str(), "In-dialog request for no dialog; 481");
                    self.create_simple_response(&req, 481, "Call/Transaction Does Not Exist")
                } else {
                    error!("Failed to handle request: {}", e);
                    self.create_simple_response(&req, 500, "Internal Server Error")
                };
                handle.send_final(response).await;
            }
        } else if let Some(resp) = sip_parse::parse_response(&payload) {
            // Look up FCP internal CallId from the SIP Call-ID header
            // Priority: 1) Fast DashMap lookup, 2) Call manager dialog scan, 3) Raw SIP Call-ID
            let sip_cid_resp = sip_parse::header(resp.headers(), "Call-ID");
            let fcp_call_id_resp = sip_cid_resp
                .as_ref()
                .and_then(|cid| {
                    self.sip_call_id_map
                        .get(cid.as_str())
                        .map(|v| v.value().clone())
                        .or_else(|| {
                            self.call_manager
                                .find_by_dialog_call_id(cid.as_str())
                                .map(|call| call.id.to_string())
                        })
                })
                .or_else(|| sip_cid_resp.as_ref().map(|s| s.as_str().to_string()));

            self.log_sip_message(
                "inbound",
                &resp.start_line().code().to_string(),
                fcp_call_id_resp.as_deref(),
                None,
                sip_parse::header(resp.headers(), "CSeq").map(|s| s.as_str()),
                Some(peer),
                Some(match transport {
                    sip_transaction::TransportKind::Udp => "udp",
                    sip_transaction::TransportKind::Tcp => "tcp",
                    sip_transaction::TransportKind::Tls => "tls",
                    sip_transaction::TransportKind::Ws => "ws",
                    _ => "other",
                }),
                Some(resp.headers()),
                Some(resp.body()),
                Some(&raw_payload_str),
            );
            debug!(
                status = resp.start_line().code(),
                reason = %resp.start_line().reason(),
                "Received SIP response"
            );

            // Route response to transaction manager
            self.transaction_manager.receive_response(resp).await;
        } else {
            error!("Failed to parse SIP message");
            return Err(anyhow::anyhow!("Invalid SIP message"));
        }

        Ok(())
    }
}

/// Helper to extract tag parameter from header value.
/// Relay a B-leg's first ringing (180/183) to the A-leg, through the one
/// relay task the fork runs.
#[allow(clippy::too_many_arguments)]
fn spawn_provisional_listener(
    i: usize,
    target: String,
    b_leg_handle: Arc<crate::uac::OutboundCall>,
    provisional_tx: tokio::sync::mpsc::Sender<sip_core::Response>,
    provisional_sent: Arc<tokio::sync::Mutex<bool>>,
    early_media_policy: fcp_b2bua_core::EarlyMediaPolicy,
    dedup_provisionals: bool,
    call_id: fcp_b2bua_core::CallId,
) {
    // Spawn provisional listener task (loops until first 180+ response)
    tokio::spawn(async move {
        // If early media is suppressed, skip provisional relay entirely
        if early_media_policy == fcp_b2bua_core::EarlyMediaPolicy::None {
            debug!(
                call_id = %call_id,
                target = %target,
                "Early media suppressed by policy — skipping provisional relay for B-leg {}", i
            );
            return;
        }

        // Loop to consume provisionals until we get a 180+ response
        while let Some(response) = b_leg_handle.await_provisional().await {
            let code = response.start_line().code();
            debug!(
                call_id = %call_id,
                target = %target,
                code = code,
                has_body = !response.body().is_empty(),
                "B-leg {} provisional response", i
            );

            // Signal if it's a ringing/progress response (skip 100 Trying etc.)
            if (180..200).contains(&code) {
                let mut sent = provisional_sent.lock().await;
                if !*sent || !dedup_provisionals {
                    info!(
                        call_id = %call_id,
                        target = %target,
                        code = code,
                        has_sdp = !response.body().is_empty(),
                        "First provisional from B-leg {} - sending full response to relay task", i
                    );
                    // Send the ENTIRE response object (preserves SDP, Require: 100rel, etc.)
                    let _ = provisional_tx.send(response).await;
                    *sent = true;
                }
                break;
            }
            // Continue looping past 100 Trying and other non-ringing provisionals
        }
    });
}

/// Where a node takes SIP, by node id (`docs/PUSH.md` §2.4).
pub type NodeSipAddress = Arc<dyn Fn(&str) -> Option<std::net::SocketAddr> + Send + Sync>;

/// Whether the node's store answers, as its probe last found
/// (`docs/DEEP_TESTING.md` D5).
pub type StoreUp = Arc<dyn Fn() -> bool + Send + Sync>;

/// How long a REGISTER may take before it is answered 503: past it, the
/// store is not answering, and a phone that hears nothing waits 32 s.
const REGISTER_DEADLINE: Duration = Duration::from_secs(5);

/// What a 503 for an unreachable store asks a phone to wait.
const STORE_RETRY_AFTER_SECS: u32 = 5;

/// A call's bindings reached through another node, by contact: that node's
/// SIP address and the flow's token.
type FlowRelays = std::collections::HashMap<String, (String, String)>;

/// A fork's leg: its target, its INVITE, the flow it went down.
type BLeg = (String, Arc<crate::uac::OutboundCall>, Option<String>);

/// How a leg finished: its index, target, final code and response, its
/// INVITE and flow; `None` when it failed outright.
type LegFinal = Option<(
    usize,
    String,
    u16,
    sip_core::Response,
    Arc<crate::uac::OutboundCall>,
    Option<String>,
)>;

/// Wait for a B-leg's final response: what the fork races on.
fn spawn_race_task(
    i: usize,
    target: String,
    b_leg_handle: Arc<crate::uac::OutboundCall>,
    flow_token: Option<String>,
    call_id: fcp_b2bua_core::CallId,
) -> tokio::task::JoinHandle<LegFinal> {
    tokio::spawn(async move {
        debug!(
            call_id = %call_id,
            target = %target,
            "B-leg {} race task waiting for final response", i
        );
        match b_leg_handle.await_final().await {
            Ok(response) => {
                let code = response.start_line().code();
                debug!(
                    call_id = %call_id,
                    target = %target,
                    code = code,
                    "B-leg {} final response", i
                );
                Some((i, target, code, response, b_leg_handle, flow_token))
            }
            Err(e) => {
                warn!(
                    call_id = %call_id,
                    target = %target,
                    error = %e,
                    "B-leg {} failed", i
                );
                None
            }
        }
    })
}

/// What a parker hears on `*70`: the orbit, digit by digit, then the end
/// of the call. Held for the call alone, like `*98`'s flow.
/// A directed park's slot holds a call already.
#[derive(Debug)]
struct SlotTaken;

impl std::fmt::Display for SlotTaken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the parking slot holds a call")
    }
}

impl std::error::Error for SlotTaken {}

fn park_announcement(call_id: &fcp_b2bua_core::CallId, slot: &str) -> fcp_flow::FlowDefinition {
    let digits: Vec<String> = slot.chars().map(String::from).collect();
    spoken_confirmation(
        &format!("park-{call_id}"),
        "Park",
        "Inline flow telling a parker the orbit (*70)",
        vec![format!("Parked on {}.", digits.join(" "))],
    )
}

/// What a caller hears after a feature code: one sentence, then the end of
/// the call.
fn feature_confirmation(call_id: &fcp_b2bua_core::CallId, text: &str) -> fcp_flow::FlowDefinition {
    spoken_confirmation(
        &format!("feature-{call_id}"),
        "Feature code",
        "Inline flow confirming a feature code",
        vec![text.to_string()],
    )
}

/// The sentence a calling code's caller hears once it is stored.
fn calling_code_confirmation(code: &fcp_pbx::CallingCode) -> &'static str {
    use fcp_pbx::CallingCode::*;
    match code {
        DndOn => "Do not disturb on.",
        DndOff => "Do not disturb off.",
        ForwardAllOn { .. } => "Call forwarding on.",
        ForwardAllOff => "Call forwarding off.",
        ForwardBusyOn { .. } => "Forward when busy on.",
        ForwardBusyOff => "Forward when busy off.",
        ForwardNoAnswerOn { .. } => "Forward on no answer on.",
        ForwardNoAnswerOff => "Forward on no answer off.",
        CallWaitingOn => "Call waiting on.",
        CallWaitingOff => "Call waiting off.",
        FindMeOn => "Find me on.",
        FindMeOff => "Find me off.",
        ExecutiveOn => "Executive screening on.",
        ExecutiveOff => "Executive screening off.",
        AssistantOn => "Assistant on.",
        AssistantOff => "Assistant off.",
    }
}

/// Make a `spoken_confirmation` wait `secs` before its last part: a test
/// call held open for a contact to try listening in.
fn wait_before_last_part(flow: &mut fcp_flow::FlowDefinition, secs: u32) {
    use fcp_flow::{FlowNode, NodeType};
    let says: Vec<String> = flow
        .nodes
        .keys()
        .filter(|id| id.starts_with("say"))
        .cloned()
        .collect();
    let Some(last) = says.iter().max_by_key(|id| {
        id.strip_prefix("say-")
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(0)
    }) else {
        return;
    };
    let Some(before) = flow
        .nodes
        .values_mut()
        .find(|node| matches!(&node.node_type, NodeType::Say { next, .. } if next == last))
    else {
        return;
    };
    if let NodeType::Say { next, .. } = &mut before.node_type {
        *next = "wait".to_string();
    }
    flow.nodes.insert(
        "wait".to_string(),
        FlowNode {
            id: "wait".to_string(),
            name: Some("Wait for a listener".to_string()),
            node_type: NodeType::Pause {
                duration: secs,
                next: last.clone(),
            },
        },
    );
}

/// An inline flow that answers, says each of `parts` in turn and hangs up.
fn spoken_confirmation(
    id: &str,
    name: &str,
    description: &str,
    parts: Vec<String>,
) -> fcp_flow::FlowDefinition {
    use fcp_flow::{FlowDefinition, FlowNode, NodeType};
    let node = |id: String, name: &str, node_type: NodeType| {
        (
            id.clone(),
            FlowNode {
                id,
                name: Some(name.to_string()),
                node_type,
            },
        )
    };
    let say_id = |i: usize| {
        if i == 0 {
            "say".to_string()
        } else {
            format!("say-{i}")
        }
    };
    let count = parts.len();
    let first = if count == 0 {
        "hangup".to_string()
    } else {
        say_id(0)
    };
    let nodes = std::iter::once(node(
        "answer".to_string(),
        "Answer call",
        NodeType::Answer {
            next: first,
            on_error: None,
        },
    ))
    .chain(parts.into_iter().enumerate().map(|(i, text)| {
        node(
            say_id(i),
            "Say the confirmation",
            NodeType::Say {
                text,
                voice: None,
                language: "en-US".to_string(),
                next: if i + 1 < count {
                    say_id(i + 1)
                } else {
                    "hangup".to_string()
                },
                on_error: Some("hangup".to_string()),
            },
        )
    }))
    .chain(std::iter::once(node(
        "hangup".to_string(),
        "Hang up",
        NodeType::Hangup { reason: None },
    )))
    .collect();
    FlowDefinition {
        id: id.to_string(),
        name: name.to_string(),
        description: Some(description.to_string()),
        version: 1,
        initial_node: "answer".to_string(),
        nodes,
        variables: HashMap::new(),
        tenant_id: None,
        enabled: true,
        created_at: None,
        updated_at: None,
    }
}

fn extract_tag_value(value: Option<&SmolStr>) -> Option<String> {
    value.and_then(|v| {
        v.split(';').find_map(|seg| {
            let s = seg.trim();
            s.strip_prefix("tag=").map(|rest| rest.to_string())
        })
    })
}

/// Extract tenant identifier from an INVITE request.
///
/// Multi-tenant isolation strategy (priority order):
/// 1. Domain from Request-URI (e.g., user@tenant1.example.com -> "tenant1.example.com")
/// 2. Domain from To header (fallback if Request-URI has no domain)
///
/// Returns the domain/tenant name as a String for lookup in TenantStore.get_tenant_by_name().
/// The caller should convert this to a TenantId via storage lookup.
///
/// # Returns
/// - Some(String) if a domain/tenant can be extracted
/// - None if no tenant context is available
fn extract_tenant_from_invite(request: &sip_core::Request) -> Option<String> {
    // Priority 1: Extract domain from Request-URI
    if let Some(domain) = extract_domain_from_uri(request.uri()) {
        return Some(domain);
    }

    // Priority 2: Extract domain from To header
    if let Some(to_header) = request.headers().get("To") {
        // Parse To header: "Display Name" <sip:user@domain>;tag=xyz
        // Extract the URI part between < and >
        let to_str = to_header.to_string();
        let uri_str = if let Some(start) = to_str.find('<') {
            if let Some(end) = to_str.find('>') {
                &to_str[start + 1..end]
            } else {
                &to_str
            }
        } else {
            &to_str
        };

        // Try parsing as SIP URI to extract domain
        if let Ok(uri) = sip_core::SipUri::parse(uri_str.trim()) {
            let domain = uri.host().to_string();
            if !domain.is_empty() && domain != "localhost" && domain != "127.0.0.1" {
                return Some(domain);
            }
        }
    }

    None
}

/// Helper to extract domain from a SIP URI.
fn extract_domain_from_uri(uri: &sip_core::Uri) -> Option<String> {
    match uri {
        sip_core::Uri::Sip(sip_uri) => {
            let domain = sip_uri.host().to_string();
            // Filter out common non-tenant domains
            if !domain.is_empty()
                && domain != "localhost"
                && domain != "127.0.0.1"
                && !domain.starts_with("192.168.")
                && !domain.starts_with("10.")
            {
                Some(domain)
            } else {
                None
            }
        }
        sip_core::Uri::Tel(_) => None, // tel URIs don't have domains
        sip_core::Uri::Absolute(_) => None, // Absolute URIs (http, etc.) not supported for tenant extraction
    }
}

/// Normalize a dialed number to E.164 format by adding "+" prefix if needed.
///
/// Handles common dialing patterns:
/// - "15185556789" (11 digits, starts with 1) -> "+15185556789"
/// - "5185556789" (10 digits, NANP) -> "+15185556789"
/// - "+15185556789" (already E.164) -> "+15185556789"
/// - "5000" (short extension) -> "5000" (unchanged)
/// - "sip:15185556789@host" -> "sip:+15185556789@host"
///
/// `max_extension_digits` controls the boundary between extensions and phone
/// numbers.  Digit strings with this many digits or fewer are left as-is
/// (treated as extensions); longer strings trigger E.164 normalization.
fn normalize_dialed_number(uri: &str, max_extension_digits: u8) -> String {
    // Check if this is a SIP URI
    let (prefix, user_and_host) = if let Some(stripped) = uri.strip_prefix("sip:") {
        ("sip:", stripped)
    } else if let Some(stripped) = uri.strip_prefix("sips:") {
        ("sips:", stripped)
    } else {
        ("", uri)
    };

    // Split into user part and host part
    let parts: Vec<&str> = user_and_host.split('@').collect();
    let user_part = parts[0];
    let host_part = if parts.len() > 1 {
        Some(parts[1])
    } else {
        None
    };

    // If already has +, it's already E.164
    if user_part.starts_with('+') {
        return uri.to_string();
    }

    // Check if it's all digits
    if !user_part.chars().all(|c| c.is_ascii_digit()) {
        return uri.to_string();
    }

    // Skip normalization for short extensions (up to max_extension_digits).
    // This preserves extensions like 5000, 1001, etc.
    if user_part.len() <= max_extension_digits as usize {
        return uri.to_string();
    }

    // Normalize based on length
    let normalized_user = if user_part.len() == 11 && user_part.starts_with('1') {
        // North American 11-digit format: 1NXXNXXXXXX -> +1NXXNXXXXXX
        format!("+{}", user_part)
    } else if user_part.len() == 10 {
        // North American 10-digit format: NXXNXXXXXX -> +1NXXNXXXXXX
        format!("+1{}", user_part)
    } else {
        // Unknown format, return as-is
        return uri.to_string();
    };

    // Reconstruct the URI
    if let Some(host) = host_part {
        format!("{}{}@{}", prefix, normalized_user, host)
    } else {
        format!("{}{}", prefix, normalized_user)
    }
}

/// Helper to check if a URI represents an E.164 phone number.
///
/// Extracts the user part from a SIP URI and checks if it looks like an E.164 number.
/// E.164 numbers start with '+' and have at least 7 digits.
///
/// # Examples
/// - `sip:+15183217034@domain` → true
/// - `sip:alice@domain` → false
/// - `sip:+1518@domain` → false (too short)
fn is_e164_number(uri: &str) -> bool {
    // Extract user part from sip:+15183217034@domain or just +15183217034
    let user_part = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .unwrap_or(uri)
        .split('@')
        .next()
        .unwrap_or("");

    // E.164 numbers start with '+' and must have at least 7 digits (country code + minimal number)
    user_part.starts_with('+')
        && user_part.len() > 7
        && user_part[1..].chars().all(|c| c.is_ascii_digit())
}

/// Deduplicate target URIs by host:port.
///
/// Softphones like Zoiper register multiple contacts with different `rinstance`
/// parameters but the same host:port. Without deduplication, the B2BUA sends
/// separate INVITEs to each registration, causing the same phone to ring
/// multiple times for a single call.
///
/// Keeps the first URI encountered for each host:port (which is typically the
/// one with the highest q-value since registrations are sorted by priority).
#[allow(dead_code)]
fn dedup_target_uris_by_hostport(uris: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for uri in uris {
        let key = extract_hostport_from_uri(&uri);
        if seen.insert(key) {
            result.push(uri);
        }
    }
    result
}

/// Deduplicate target contacts by host:port.
///
/// Same as `dedup_target_uris_by_hostport` but works with `TargetContact`
/// to preserve flow token information.
fn dedup_target_contacts_by_hostport(contacts: Vec<TargetContact>) -> Vec<TargetContact> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for contact in contacts {
        // Two addresses a trunk's name resolved to share the URI's host:
        // the destination tells them apart.
        let key = match crate::trunk_egress::take_destination(&contact.uri).1 {
            Some(destination) => format!("{}:{}", destination.host(), destination.port()),
            None => extract_hostport_from_uri(&contact.uri),
        };
        if seen.insert(key) {
            result.push(contact);
        }
    }
    result
}

/// Extract host:port from a SIP URI for deduplication purposes.
///
/// Given `sip:bob@192.168.1.80:59585;rinstance=abc;transport=tcp`,
/// returns `"192.168.1.80:59585"`.
fn extract_hostport_from_uri(uri: &str) -> String {
    // Strip scheme
    let without_scheme = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .unwrap_or(uri);

    // Take everything after '@' (or the whole thing if no '@')
    let hostpart = without_scheme.split('@').nth(1).unwrap_or(without_scheme);

    // Strip URI parameters (;rinstance=..., ;transport=..., etc.)
    hostpart.split(';').next().unwrap_or(hostpart).to_string()
}

/// SDP validation helper (exposed for tests).
fn validate_sdp_body_static(
    request: &sip_core::Request,
    prior_sdp: Option<&str>,
) -> Result<Option<String>> {
    if request.body().is_empty() {
        if prior_sdp.is_some() {
            return Ok(None);
        } else {
            return Err(anyhow::anyhow!("No SDP present and no prior offer/answer"));
        }
    }

    // The SDP alone or as the SDP part of a multipart body.
    let body_str = crate::body::request_sdp(request)
        .ok_or_else(|| anyhow::anyhow!("Unsupported Content-Type"))?;
    let sdp = parse_sdp(&body_str).map_err(|e| anyhow::anyhow!("Invalid SDP: {}", e))?;

    if sdp.media.is_empty() {
        return Err(anyhow::anyhow!("SDP missing media descriptions"));
    }

    // Any RTP section FCP can carry makes the offer usable; a video-only
    // offer (audio at port 0) is answered per section, not refused.
    let has_media = sdp.media.iter().any(|m| {
        matches!(
            m.media_type,
            sip_sdp::MediaType::Audio | sip_sdp::MediaType::Video
        )
    });
    if !has_media {
        return Err(anyhow::anyhow!("SDP missing audio or video media"));
    }

    Ok(Some(body_str))
}

/// Store SDP on the appropriate leg.
fn store_leg_sdp(
    call_manager: &fcp_b2bua_core::CallManager,
    call_id: &fcp_b2bua_core::CallId,
    is_a_leg: bool,
    sdp: String,
) {
    let _ = call_manager.update_call(call_id, |c| {
        if is_a_leg {
            c.a_leg.sdp = Some(sdp.clone());
        } else if let Some(b) = &mut c.b_leg {
            b.sdp = Some(sdp.clone());
        }
    });
}

fn extract_session_timer(
    resp: &sip_core::Response,
) -> Option<(Duration, Option<sip_core::RefresherRole>)> {
    let se_header = sip_parse::header(resp.headers(), "Session-Expires")?;
    sip_parse::parse_session_expires(se_header).map(|se| {
        (
            Duration::from_secs(se.delta_seconds() as u64),
            se.refresher(),
        )
    })
}

fn extract_min_se(resp: &sip_core::Response) -> Option<Duration> {
    sip_parse::header(resp.headers(), "Min-SE")
        .and_then(|v| v.parse::<u32>().ok())
        .map(|secs| Duration::from_secs(secs as u64))
}

async fn handle_refresh_needed(
    call_manager: &fcp_b2bua_core::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    uac: Option<Arc<SipUserAgentClient>>,
    dialog_id: sip_dialog::DialogId,
) -> Result<()> {
    let (mut dialog, call_id, is_a_leg) = find_call_and_dialog(call_manager, &dialog_id)
        .ok_or_else(|| anyhow::anyhow!("Dialog not found"))?;

    // Only refresh if we are the refresher
    let is_refresher = SipMessageHandler::dialog_is_refresher(&dialog);
    if !is_refresher {
        return Ok(());
    }

    let se_secs = dialog
        .session_expires()
        .map(|d| d.as_secs() as u32)
        .unwrap_or(1800);

    let refresher_str = if dialog.refresher() == Some(sip_core::RefresherRole::Uac) {
        "uac"
    } else {
        "uas"
    };
    let uac = uac.ok_or_else(|| anyhow::anyhow!("No UAC available for refresh"))?;
    // Attempt refresh with bounded retries to avoid tight retry loops
    let mut attempts = 0;
    let max_attempts = 3;
    let mut use_update = true;
    let mut current_se = se_secs;
    let resp = loop {
        attempts += 1;
        let result = uac
            .refresh_session(&mut dialog, current_se, refresher_str, use_update, None)
            .await
            .map_err(|e| anyhow::anyhow!("Session refresh failed: {}", e))?;

        if result.start_line().code() == 200 {
            break result;
        }

        if result.start_line().code() == 422 {
            if attempts >= max_attempts {
                break result;
            }
            if let Some(min_se) =
                sip_parse::header(result.headers(), "Min-SE").and_then(|v| v.parse::<u32>().ok())
            {
                current_se = current_se.max(min_se);
                let _ = dialog.set_session_expires(Duration::from_secs(current_se as u64));
                continue;
            }
        }

        // On non-200, fall back from UPDATE to re-INVITE once
        if use_update && attempts < max_attempts {
            use_update = false;
            continue;
        }

        break result;
    };

    if resp.start_line().code() != 200 {
        // Best effort BYE then terminate dialog on failed refresh
        let _ = uac.bye(&dialog).await;
        terminate_dialog(
            call_manager,
            dialog_manager,
            session_timer_manager,
            &call_id,
            &dialog_id,
        );
        return Err(anyhow::anyhow!(
            "Session refresh rejected: {}",
            resp.start_line().code()
        ));
    }

    dialog.update_from_response(&resp);
    // Restart timer with updated info
    if let Some(se) = dialog.session_expires() {
        let is_refresher = dialog
            .refresher()
            .map(|r| {
                (r == sip_core::RefresherRole::Uac && dialog.is_uac())
                    || (r == sip_core::RefresherRole::Uas && !dialog.is_uac())
            })
            .unwrap_or(false);
        session_timer_manager.start_timer(dialog.id().clone(), se, is_refresher);
    }
    // Persist updated dialog
    let _ = call_manager.update_call(&call_id, |c| {
        if is_a_leg {
            c.a_leg.dialog = Some(dialog.clone());
        } else if let Some(b) = &mut c.b_leg {
            b.dialog = Some(dialog.clone());
        }
    });
    let _ = dialog_manager.insert(dialog);
    Ok(())
}

/// A session nobody refreshed (RFC 4028 §10): the call ends, both legs
/// hung up and its media closed as any hangup's, through the serving
/// handler. Before a first request there is none, and nothing can have
/// expired; the fallback hangs up the expired leg alone.
async fn handle_session_expired(
    call_manager: &fcp_b2bua_core::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    uac: Option<Arc<SipUserAgentClient>>,
    serving: Option<&SipMessageHandler>,
    dialog_id: sip_dialog::DialogId,
) -> Result<()> {
    let (dialog, call_id, _is_a_leg) = find_call_and_dialog(call_manager, &dialog_id)
        .ok_or_else(|| anyhow::anyhow!("Dialog not found"))?;
    if let Some(handler) = serving {
        info!(call_id = %call_id, dialog_id = ?dialog_id, "Session expired; ending the call");
        session_timer_manager.stop_timer(&dialog_id);
        return handler.hangup_call_by_id_saying(
            &call_id,
            fcp_cdr::TerminationReason::system(
                fcp_cdr::SystemCause::SessionExpired,
                "Session expired",
            ),
            Some(SESSION_EXPIRED_REASON),
        );
    }
    // Attempt BYE to remote
    if let Some(uac) = uac {
        let _ = uac.bye(&dialog).await;
    }
    terminate_dialog(
        call_manager,
        dialog_manager,
        session_timer_manager,
        &call_id,
        &dialog_id,
    );
    Ok(())
}

/// The `Reason` on the BYEs of a call whose session expired (RFC 3326).
const SESSION_EXPIRED_REASON: &str = "SIP;cause=408;text=\"session expired\"";

fn terminate_dialog(
    call_manager: &fcp_b2bua_core::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    call_id: &fcp_b2bua_core::CallId,
    dialog_id: &sip_dialog::DialogId,
) {
    session_timer_manager.stop_timer(dialog_id);
    dialog_manager.remove(dialog_id);
    let _ = call_manager.update_call(call_id, |call| {
        if let Some(a) = call.a_leg.dialog.as_ref() {
            if a.id() == dialog_id {
                call.a_leg.dialog = None;
            }
        }
        if let Some(b) = &mut call.b_leg {
            if let Some(d) = b.dialog.as_ref() {
                if d.id() == dialog_id {
                    b.dialog = None;
                }
            }
        }
    });
    let _ = call_manager.terminate_call_with(
        call_id,
        fcp_cdr::TerminationReason::system(fcp_cdr::SystemCause::SessionExpired, "Session expired"),
    );
}

/// Each route's target and trunk, for naming the trunk that answered.
/// The trunk carrying a call's outbound leg: the one that answered, as the
/// record names it, else the primary route's.
fn carrying_trunk(call: &fcp_b2bua_core::Call) -> Option<Arc<fcp_core::Trunk>> {
    call.route_trunks
        .iter()
        .find(|rt| Some(rt.id()) == call.trunk_id)
        .or_else(|| call.route_trunks.first())
        .map(|rt| Arc::clone(&rt.trunk))
}

pub(crate) fn route_trunks(routes: &[OutboundRoute]) -> Vec<fcp_b2bua_core::RouteTrunk> {
    routes
        .iter()
        .filter_map(|r| {
            r.trunk.as_ref().map(|trunk| fcp_b2bua_core::RouteTrunk {
                target: r.target_uri.clone(),
                trunk: Arc::clone(trunk),
            })
        })
        .collect()
}

/// The telephone number in a SIP URI, as STIR/SHAKEN states it: digits
/// only, without the `+` (RFC 8224 §4). Empty when the user part is not a
/// number, which is a call that cannot be signed.
fn tn_of(uri: Option<&str>) -> String {
    let Some(uri) = uri else {
        return String::new();
    };
    let user = uri
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_start_matches("sip:")
        .trim_start_matches("sips:")
        .split('@')
        .next()
        .unwrap_or_default();
    let digits: String = user
        .trim_start_matches('+')
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() || digits.len() != user.trim_start_matches('+').len() {
        return String::new();
    }
    digits
}

/// The trunk whose route target is `winner` (a fork target, which may
/// have gained or lost angle brackets on the way).
fn answering_trunk<'a>(
    route_trunks: &'a [fcp_b2bua_core::RouteTrunk],
    winner: &str,
) -> Option<&'a fcp_b2bua_core::RouteTrunk> {
    let bare = |s: &str| {
        s.trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .to_string()
    };
    let winner = bare(winner);
    route_trunks.iter().find(|rt| bare(&rt.target) == winner)
}

/// The header a transferred call's INVITE carries naming the call it left.
/// What the routing engine said about an INVITE: a decision to apply, or
/// a refusal of the caller with the reason the 403 carries.
enum RoutingVerdict {
    Decision(fcp_routing::PolicyDecision),
    Refused(String),
}

/// The user part of a URI (`sip:alice@host` → `alice`).
fn user_of_uri(uri: &str) -> Option<String> {
    let uri = uri.trim().trim_start_matches('<');
    let rest = uri
        .strip_prefix("sips:")
        .or_else(|| uri.strip_prefix("sip:"))
        .unwrap_or(uri);
    let (user, _) = rest.split_once('@')?;
    (!user.is_empty()).then(|| user.to_string())
}

/// How long a find-me answerer has to press 1 (§3.6).
const CONFIRM_WAIT: Duration = Duration::from_secs(10);

pub const TRANSFERRED_FROM_HEADER: &str = "X-FCP-Transferred-From";

/// The call a leg this node placed to itself forwards for — a user's
/// forward, a find-me step — so the new call is not checked again against
/// the original caller's class of service: the forwarding user's was
/// checked before the leg was placed (`docs/ENTERPRISE_CALLING.md` §3.7).
/// Believed only when it names a call this node has.
pub const FORWARDED_FOR_HEADER: &str = "X-FCP-Forwarded-For";

/// On a leg a flow's `Dial` placed into this node for a user: the user's
/// plan answers its treatments as codes — busy as the busy code, voicemail
/// as 480 — so the flow takes its own branches. A forward is still
/// followed. Believed only on a leg that proves it is this node's.
pub const FLOW_DIAL_HEADER: &str = "X-FCP-Flow-Dial";

/// The most forwards one call follows, one into another, before the next
/// is taken for a loop.
const MAX_FORWARD_HOPS: usize = 10;

/// How long a forward placed into this node may ring before the forwarding
/// call gives up on it: a backstop, since the new call ends by its own
/// plan (no-answer times are at most 300 seconds).
const FORWARD_INTO_NODE_LIMIT: Duration = Duration::from_secs(600);

/// `uri` with its user part replaced (`sip:15183217034@h` →
/// `sip:+15183217034@h`); a URI with no user part as it is.
fn with_uri_user(uri: &str, user: &str) -> String {
    let (scheme, rest) = match uri.split_once(':') {
        Some(parts) => parts,
        None => return uri.to_string(),
    };
    match rest.split_once('@') {
        Some((_, host)) => format!("{scheme}:{user}@{host}"),
        None => uri.to_string(),
    }
}

/// `uri` with a URI header added (`?name=value`, or `&name=value` when it
/// has headers already).
fn with_uri_header(uri: &str, name: &str, value: &str) -> String {
    let sep = if uri.contains('?') { '&' } else { '?' };
    format!("{uri}{sep}{name}={value}")
}

/// The record's RTP counters from a media session's statistics. The
/// backend reports no jitter yet; a zero is left out rather than stored as
/// a measurement.
/// Put the media session's counters on the call before its record is
/// written, and tear the session down when this path owns it. Every
/// teardown path goes through here: a record that misses its RTP counters
/// is a record nobody can bill or debug from.
async fn close_media(
    call_manager: &Arc<fcp_b2bua_core::CallManager>,
    call_id: &fcp_b2bua_core::CallId,
    media: Option<(fcp_core::CallId, Arc<fcp_media::MediaManager>)>,
    teardown: bool,
    what: &'static str,
) {
    let Some((session_id, media_manager)) = media else {
        return;
    };
    match media_manager.get_session_stats(&session_id).await {
        Ok(stats) => call_manager.set_rtp_stats(call_id, rtp_stats_from(&stats)),
        Err(e) => debug!(call_id = %call_id, error = %e, "No media counters for the record"),
    }
    if teardown {
        match media_manager.terminate_session(&session_id).await {
            Ok(()) => info!(
                call_id = %call_id,
                media_session_id = %session_id,
                "Terminated media session ({what})"
            ),
            Err(e) => error!(
                call_id = %call_id,
                media_session_id = %session_id,
                error = %e,
                "Failed to terminate media session ({what})"
            ),
        }
    }
}

/// The media session a call's teardown owns, if it anchored one.
fn anchored_media(
    call: &fcp_b2bua_core::Call,
    media_manager: Option<&Arc<fcp_media::MediaManager>>,
) -> Option<(fcp_core::CallId, Arc<fcp_media::MediaManager>)> {
    match (call.media_anchored, call.media_session_id, media_manager) {
        (true, Some(session_id), Some(mm)) => Some((session_id, Arc::clone(mm))),
        _ => None,
    }
}

fn rtp_stats_from(stats: &fcp_media::MediaSessionStats) -> fcp_cdr::RtpStats {
    fcp_cdr::RtpStats {
        packets_sent: stats.packets_sent,
        packets_received: stats.packets_received,
        bytes_sent: stats.bytes_sent,
        bytes_received: stats.bytes_received,
        packets_lost: stats.packets_lost,
        jitter_ms: (stats.jitter_ms > 0.0).then_some(stats.jitter_ms),
    }
}

#[allow(clippy::too_many_arguments)]
fn terminate_dialog_with_cleanup(
    call_manager: &fcp_b2bua_core::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    rate_limits: &dashmap::DashMap<sip_dialog::DialogId, RateWindow>,
    mid_dialog_lock: &dashmap::DashMap<sip_dialog::DialogId, Arc<tokio::sync::Mutex<()>>>,
    subscription_index: &dashmap::DashMap<
        (sip_dialog::DialogId, smol_str::SmolStr),
        sip_dialog::SubscriptionId,
    >,
    subscription_created_at: &dashmap::DashMap<sip_dialog::SubscriptionId, Instant>,
    call_id: &fcp_b2bua_core::CallId,
    dialog_id: &sip_dialog::DialogId,
) {
    // Perform standard dialog termination
    terminate_dialog(
        call_manager,
        dialog_manager,
        session_timer_manager,
        call_id,
        dialog_id,
    );

    // Cleanup per-dialog state to prevent memory leaks
    rate_limits.remove(dialog_id);
    mid_dialog_lock.remove(dialog_id);

    // Clean subscription_index entries and their created_at timestamps for this dialog
    let keys: Vec<_> = subscription_index
        .iter()
        .filter(|entry| entry.key().0 == *dialog_id)
        .map(|entry| entry.key().clone())
        .collect();
    for key in keys {
        if let Some((_, sub_id)) = subscription_index.remove(&key) {
            subscription_created_at.remove(&sub_id);
        }
    }

    debug!(dialog_id = ?dialog_id, "Cleaned up dialog state (rate limits, locks, subscriptions)");
}

#[allow(clippy::too_many_arguments)]
fn cleanup_after_forward_failure(
    call_manager: &fcp_b2bua_core::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    rate_limits: &dashmap::DashMap<sip_dialog::DialogId, RateWindow>,
    mid_dialog_lock: &dashmap::DashMap<sip_dialog::DialogId, Arc<tokio::sync::Mutex<()>>>,
    subscription_index: &dashmap::DashMap<
        (sip_dialog::DialogId, smol_str::SmolStr),
        sip_dialog::SubscriptionId,
    >,
    subscription_created_at: &dashmap::DashMap<sip_dialog::SubscriptionId, Instant>,
    call_id: &fcp_b2bua_core::CallId,
    dialog_id: &sip_dialog::DialogId,
    reason: &str,
) {
    warn!(call_id = %call_id, dialog_id = ?dialog_id, reason = %reason, "Cleaning up after mid-dialog forward failure");
    terminate_dialog_with_cleanup(
        call_manager,
        dialog_manager,
        session_timer_manager,
        rate_limits,
        mid_dialog_lock,
        subscription_index,
        subscription_created_at,
        call_id,
        dialog_id,
    );
}

fn find_call_and_dialog(
    call_manager: &fcp_b2bua_core::CallManager,
    dialog_id: &sip_dialog::DialogId,
) -> Option<(sip_dialog::Dialog, fcp_b2bua_core::CallId, bool)> {
    for call in call_manager.all_calls() {
        if let Some(a) = &call.a_leg.dialog {
            if *a.id() == *dialog_id {
                return Some((a.clone(), call.id, true));
            }
        }
        if let Some(b_leg) = &call.b_leg {
            if let Some(d) = &b_leg.dialog {
                if *d.id() == *dialog_id {
                    return Some((d.clone(), call.id, false));
                }
            }
        }
    }
    None
}

/// A request inside a dialog this node does not have: answered 481, not
/// 500 (RFC 3261 §12.2.2; `docs/DEEP_TESTING.md` D2).
#[derive(Debug, thiserror::Error)]
#[error("no such dialog")]
struct NoSuchDialog;

fn stop_timers_for_call(session_timer_manager: &SessionTimerManager, call: &fcp_b2bua_core::Call) {
    if let Some(a) = &call.a_leg.dialog {
        session_timer_manager.stop_timer(a.id());
    }
    if let Some(b_leg) = &call.b_leg {
        if let Some(d) = &b_leg.dialog {
            session_timer_manager.stop_timer(d.id());
        }
    }
}

/// Helper to extract SIP URI from From header.
/// The caller's number for screening: the From user, `None` when the
/// caller withheld it (`Privacy: id`, an anonymous From, no user).
fn caller_number(request: &sip_core::Request) -> Option<String> {
    let private = request
        .headers()
        .get_all("Privacy")
        .any(|v| v.split(';').any(|p| p.trim().eq_ignore_ascii_case("id")));
    if private {
        return None;
    }
    let from = extract_from_uri(request).ok()?;
    let user = from.user()?.to_string();
    if user.is_empty()
        || user.eq_ignore_ascii_case("anonymous")
        || from.host().eq_ignore_ascii_case("anonymous.invalid")
    {
        return None;
    }
    Some(user)
}

fn extract_from_uri(request: &sip_core::Request) -> anyhow::Result<sip_core::SipUri> {
    let from_header = sip_parse::header(request.headers(), "From")
        .ok_or_else(|| anyhow::anyhow!("Missing From header"))?;

    // Simple extraction - look for URI between < >
    let uri_str = if let Some(start) = from_header.find('<') {
        if let Some(end) = from_header[start + 1..].find('>') {
            &from_header[start + 1..start + 1 + end]
        } else {
            return Err(anyhow::anyhow!("Malformed From header"));
        }
    } else {
        // No brackets, take until semicolon or end
        if let Some(semi) = from_header.find(';') {
            &from_header[..semi]
        } else {
            from_header.as_str()
        }
    };

    match sip_core::Uri::parse(uri_str) {
        Ok(sip_core::Uri::Sip(uri)) => Ok(uri),
        Ok(sip_core::Uri::Tel(_)) => Err(anyhow::anyhow!(
            "Tel URI in From header not supported for dialog creation"
        )),
        Ok(sip_core::Uri::Absolute(_)) => Err(anyhow::anyhow!(
            "Absolute URI in From header not supported for dialog creation"
        )),
        Err(_) => Err(anyhow::anyhow!("Failed to parse From URI")),
    }
}

/// Helper to normalize a From URI that may have a complex username.
///
/// Some SIP phones put the full AOR in the username part, like:
///   sip:bob%40192.168.1.81@192.168.1.81 (username is bob@192.168.1.81)
///
/// This creates routing problems. We normalize to: sip:bob@192.168.1.81
fn normalize_from_uri(uri: sip_core::SipUri) -> sip_core::SipUri {
    if let Some(user) = uri.user() {
        // URL decode the username
        if let Ok(decoded) = urlencoding::decode(user) {
            let decoded_str = decoded.into_owned();

            // If username contains @, take only the part before @ (the actual username)
            let simple_user = if let Some(at_pos) = decoded_str.find('@') {
                &decoded_str[..at_pos]
            } else {
                &decoded_str
            };

            // Construct normalized URI: sip:username@host
            let normalized_str = format!("sip:{}@{}", simple_user, uri.host());

            // Parse and return normalized URI, or return original if parsing fails
            if let Ok(sip_core::Uri::Sip(normalized)) = sip_core::Uri::parse(&normalized_str) {
                return normalized;
            }
        }
    }

    // Return original if normalization failed
    uri
}

/// Helper to extract display name from From header.
/// Returns Some(name) if a quoted display name is found, or falls back to username from URI.
///
/// Examples:
/// - "Bob Smith" <sip:bob@example.com> => Some("Bob Smith")
/// - <sip:bob@example.com> => Some("bob")
/// - <sip:bob%40192.168.1.81@192.168.1.81> => Some("bob@192.168.1.81")
/// - sip:bob@example.com => Some("bob")
fn extract_from_display_name(request: &sip_core::Request) -> Option<String> {
    let from_header = sip_parse::header(request.headers(), "From")?;

    // First, look for quoted display name before '<'
    if let Some(quote_start) = from_header.find('"') {
        if let Some(quote_end) = from_header[quote_start + 1..].find('"') {
            let display_name = &from_header[quote_start + 1..quote_start + 1 + quote_end];
            if !display_name.is_empty() {
                return Some(display_name.to_string());
            }
        }
    }

    // No quoted display name found - fallback to extracting username from URI
    // Extract URI part (between angle brackets or entire string before parameters)
    let uri_str = if let Some(start) = from_header.find('<') {
        {
            let end = from_header[start..].find('>')?;
            &from_header[start + 1..start + end]
        }
    } else {
        // No brackets, take until semicolon or end
        if let Some(semi) = from_header.find(';') {
            from_header[..semi].trim()
        } else {
            from_header.trim()
        }
    };

    // Parse the URI to extract username
    if let Ok(sip_core::Uri::Sip(uri)) = sip_core::Uri::parse(uri_str) {
        if let Some(user) = uri.user() {
            // URL decode the username (e.g., "bob%40192.168.1.81" -> "bob@192.168.1.81")
            let decoded = urlencoding::decode(user).ok()?.into_owned();
            return Some(decoded);
        }
    }

    None
}

/// Extracts the bare URI string from a SIP header value.
///
/// This helper handles both formats:
/// - Name-addr format: `"Display Name" <sip:user@host>;tag=xyz` → `sip:user@host`
/// - Addr-spec format: `sip:user@host;tag=xyz` → `sip:user@host`
///
/// The function strips angle brackets, display names, and parameters (like ;tag=).
///
/// # Arguments
/// * `header_value` - The raw header value (From, To, Contact, etc.)
///
/// # Returns
/// The bare URI string without brackets or parameters
fn extract_uri_from_header(header_value: &str) -> &str {
    header_value
        .split('<')
        .nth(1)
        .and_then(|s| s.split('>').next())
        .or_else(|| {
            // No angle brackets, extract URI before parameters
            header_value.split(';').next()
        })
        .unwrap_or(header_value)
}

/// Inject RFC 4028 session timer headers into a 200 OK response.
///
/// Adds `Session-Expires`, `Min-SE`, and ensures `Supported: timer` when
/// session timer enforcement is enabled and the headers are not already present.
fn inject_session_timer_headers(
    config: &B2buaConfig,
    request: &sip_core::Request,
    response: &mut sip_core::Response,
) {
    if !config.session_timer.enabled {
        return;
    }

    if response.headers().get("Session-Expires").is_none() {
        let (secs, refresher) = session_timer_answer(config, request);
        let se_value = format!("{secs};refresher={refresher}");
        let _ = response
            .headers_mut()
            .push(SmolStr::new("Session-Expires"), SmolStr::new(&se_value));
    }

    if response.headers().get("Min-SE").is_none() {
        let _ = response.headers_mut().push(
            SmolStr::new("Min-SE"),
            SmolStr::new(config.session_timer.min_se_secs.to_string()),
        );
    }

    ensure_supported_timer(response);
}

/// The session interval and refresher the node answers `request` with
/// (RFC 4028 §9): the caller's interval when it asked for one, no longer
/// than the node's own and no shorter than either side's Min-SE, and the
/// caller's refresher when it named one; otherwise the node's interval,
/// refreshed by the node.
fn session_timer_answer(config: &B2buaConfig, request: &sip_core::Request) -> (u32, &'static str) {
    let own = config.session_timer.session_expires_secs;
    let floor = sip_parse::header(request.headers(), "Min-SE")
        .and_then(|v| v.split(';').next()?.trim().parse::<u32>().ok())
        .unwrap_or(0)
        .max(config.session_timer.min_se_secs);
    let Some(asked) = sip_parse::header(request.headers(), "Session-Expires") else {
        return (own, "uas");
    };
    let mut parts = asked.split(';');
    let Some(secs) = parts.next().and_then(|d| d.trim().parse::<u32>().ok()) else {
        return (own, "uas");
    };
    let refresher = parts
        .filter_map(|p| p.split_once('='))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("refresher"))
        .map(|(_, value)| value.trim());
    let refresher = match refresher {
        Some(r) if r.eq_ignore_ascii_case("uac") => "uac",
        _ => "uas",
    };
    (secs.min(own).max(floor), refresher)
}

/// Ensure the `Supported` header includes "timer".
///
/// If a `Supported` header already exists and contains "timer", this is a no-op.
/// If it exists without "timer", appends ", timer". If absent, adds it.
fn ensure_supported_timer(response: &mut sip_core::Response) {
    if let Some(supported) = response.headers().get("Supported") {
        if supported
            .split(',')
            .any(|t| t.trim().eq_ignore_ascii_case("timer"))
        {
            return; // already present
        }
        let new_val = format!("{}, timer", supported);
        response.headers_mut().remove("Supported");
        let _ = response
            .headers_mut()
            .push(SmolStr::new("Supported"), SmolStr::new(&new_val));
    } else {
        let _ = response
            .headers_mut()
            .push(SmolStr::new("Supported"), SmolStr::new("timer"));
    }
}

// ============================================================================
// SDP Helpers
// ============================================================================

/// The media session's leg for a call's leg: each is told its own port.
pub(crate) fn media_leg(side: fcp_b2bua_core::LegSide) -> fcp_media::backend::Leg {
    match side {
        fcp_b2bua_core::LegSide::A => fcp_media::backend::Leg::A,
        fcp_b2bua_core::LegSide::B => fcp_media::backend::Leg::B,
    }
}

/// Generate a basic SDP offer with `a=sendrecv` for call origination.
///
/// Offers PCMU and PCMA (G.711) codecs which are universally supported.
fn generate_basic_sdp(local_addr: &str, rtp_port: u16) -> String {
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 {addr}\r\n\
         s=-\r\n\
         c=IN IP4 {addr}\r\n\
         t=0 0\r\n\
         m=audio {port} RTP/AVP 0 8 101\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=rtpmap:8 PCMA/8000\r\n\
         a=rtpmap:101 telephone-event/8000\r\n\
         a=fmtp:101 0-16\r\n\
         a=sendrecv\r\n",
        addr = local_addr,
        port = rtp_port,
    )
}

/// Generate an SDP body that puts the call on hold (a=sendonly).
///
/// If an existing SDP is provided, replaces `a=sendrecv` with `a=sendonly`.
/// Otherwise generates a minimal SDP with `a=sendonly`.
fn generate_hold_sdp(sdp: Option<&str>, local_addr: &str, rtp_port: u16) -> String {
    if let Some(existing) = sdp {
        // Replace sendrecv with sendonly
        let held = existing.replace("a=sendrecv", "a=sendonly");
        // If there was no explicit sendrecv attribute, append sendonly
        if !held.contains("a=sendonly") {
            format!("{}a=sendonly\r\n", held)
        } else {
            held
        }
    } else {
        // Generate minimal sendonly SDP
        format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 {addr}\r\n\
             s=-\r\n\
             c=IN IP4 {addr}\r\n\
             t=0 0\r\n\
             m=audio {port} RTP/AVP 0\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=sendonly\r\n",
            addr = local_addr,
            port = rtp_port,
        )
    }
}

/// The reason phrase a final response carries (RFC 3261 §21).
fn reason_phrase(code: u16) -> &'static str {
    match code {
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        410 => "Gone",
        480 => "Temporarily Unavailable",
        482 => "Loop Detected",
        486 => "Busy Here",
        487 => "Request Terminated",
        488 => "Not Acceptable Here",
        500 => "Server Internal Error",
        503 => "Service Unavailable",
        600 => "Busy Everywhere",
        603 => "Decline",
        604 => "Does Not Exist Anywhere",
        606 => "Not Acceptable",
        _ => "Call Failed",
    }
}

/// Arm a leg's session timer from its dialog, or from the B2BUA's own
/// setting when the timer is forced.
pub(crate) fn start_session_timer_for(
    session_timer_manager: &SessionTimerManager,
    b2bua_config: &B2buaConfig,
    dialog: &sip_dialog::Dialog,
) {
    if let Some(expires) = dialog.session_expires() {
        let is_refresher = SipMessageHandler::dialog_is_refresher(dialog);
        session_timer_manager.start_timer(dialog.id().clone(), expires, is_refresher);
    } else if b2bua_config.session_timer.enabled {
        let expires = Duration::from_secs(b2bua_config.session_timer.session_expires_secs as u64);
        // SBC is always refresher when forcing session timers
        session_timer_manager.start_timer(dialog.id().clone(), expires, true);
    }
}

/// One conference leg on its way to another node (phase 5b): everything
/// the move needs, cloned out of the handler so it can run on its own
/// task after the REFER is answered.
struct ConferenceMigration {
    uac: Arc<SipUserAgentClient>,
    /// What the NOTIFY is signed as.
    local_uri: sip_core::SipUri,
    call_manager: Arc<fcp_b2bua_core::CallManager>,
    dialog_manager: Arc<DialogManager>,
    session_timer_manager: Arc<SessionTimerManager>,
    media_manager: Option<Arc<fcp_media::MediaManager>>,
    sip_call_id_map: Arc<dashmap::DashMap<String, String>>,
    in_flight: Arc<dashmap::DashMap<fcp_b2bua_core::CallId, ()>>,
    b2bua_config: B2buaConfig,
    local_addr: String,
    call_id: fcp_b2bua_core::CallId,
    /// The leg on the draining node: where the NOTIFY goes, and what is
    /// dropped once the caller is on the new node.
    old_b_dialog: sip_dialog::Dialog,
    /// The room on the new node.
    target: sip_core::SipUri,
    /// The draining node's ticket; `None` when the old node is dead and
    /// the caller joins the room on the new node afresh.
    ticket: Option<String>,
    /// The old node is gone (`docs/DEEP_TESTING.md` D5b): nothing is sent
    /// to it, neither the NOTIFY nor the BYE.
    old_node_gone: bool,
}

/// How long the new node may take to answer the INVITE.
const MIGRATION_INVITE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a re-INVITE, a NOTIFY or a BYE in the move may take.
const MIGRATION_STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a move did not happen: what the old node is told, and why.
type MigrationFailure = (u16, &'static str, String);

impl ConferenceMigration {
    async fn run(self) {
        let call_id = self.call_id;
        let outcome = self.migrate().await;
        self.in_flight.remove(&call_id);
        match outcome {
            Ok(()) => info!(call_id = %call_id, "Conference leg moved to the new node"),
            Err((code, reason, why)) => {
                warn!(
                    call_id = %call_id,
                    code,
                    error = %why,
                    "Conference leg not moved; the caller stays on the old node"
                );
                self.notify_old_node(code, reason).await;
            }
        }
    }

    /// The move.
    async fn migrate(&self) -> std::result::Result<(), MigrationFailure> {
        let call = self
            .call_manager
            .get_call(&self.call_id)
            .ok_or_else(|| (481, "Call Gone", "call no longer exists".to_string()))?;
        let mut a_dialog = call
            .a_leg
            .dialog
            .clone()
            .ok_or_else(|| (481, "Call Gone", "no A-leg dialog".to_string()))?;
        let a_sdp = call
            .a_leg
            .sdp
            .clone()
            .or_else(|| {
                call.original_invite
                    .as_ref()
                    .and_then(crate::body::request_sdp)
            })
            .ok_or_else(|| {
                (
                    488,
                    "Not Acceptable Here",
                    "no caller SDP to offer".to_string(),
                )
            })?;
        let anchored = call
            .media_anchored
            .then_some(call.media_session_id)
            .flatten()
            .zip(self.media_manager.clone());

        // The offer: the caller's SDP, or the media session's B side when
        // media is anchored here.
        let offer = match &anchored {
            Some((session_id, media_manager)) => {
                let session = media_manager
                    .get_session(session_id)
                    .await
                    .map_err(|e| (500, "Server Internal Error", format!("media session: {e}")))?;
                let (rtp_port, _) = session.ports_for(fcp_media::backend::Leg::B);
                fcp_media::rewrite_sdp_for_anchoring(&a_sdp, &self.local_addr, rtp_port, None)
            }
            None => a_sdp.clone(),
        };

        // The new B-leg, the ticket as a request header.
        let mut headers = sip_core::Headers::new();
        if let Some(ticket) = &self.ticket {
            headers
                .push(
                    SmolStr::new(fcp_core::MIGRATION_TICKET_HEADER),
                    SmolStr::new(ticket),
                )
                .map_err(|e| (500, "Server Internal Error", format!("ticket header: {e}")))?;
        }
        let _ = headers.push(
            SmolStr::new(fcp_core::CALL_ID_HEADER),
            SmolStr::new(self.call_id.to_string()),
        );
        let target = self.target.as_str().to_string();
        // Without a ticket the new node knows the caller only by the
        // INVITE: as the old node knew them, by the old leg's From.
        let options = sip_uac::InviteOptions {
            from: self
                .ticket
                .is_none()
                .then(|| self.old_b_dialog.local_uri().clone()),
            extra_headers: headers,
            ..sip_uac::InviteOptions::with_sdp(Some(&offer))
        };
        let handle = tokio::time::timeout(
            Duration::from_secs(5),
            self.uac.invite_with_options(&target, options),
        )
        .await
        .map_err(|_| {
            (
                503,
                "Service Unavailable",
                "INVITE could not be sent".to_string(),
            )
        })?
        .map_err(|e| (503, "Service Unavailable", format!("INVITE failed: {e}")))?;
        let handle = Arc::new(handle);
        let final_response = tokio::time::timeout(MIGRATION_INVITE_TIMEOUT, handle.await_final())
            .await
            .map_err(|_| {
                (
                    408,
                    "Request Timeout",
                    "new node did not answer".to_string(),
                )
            })?
            .map_err(|e| (503, "Service Unavailable", format!("INVITE failed: {e}")))?;
        let code = final_response.start_line().code();
        if !(200..300).contains(&code) {
            let reason = match code {
                403 => "Forbidden",
                503 => "Service Unavailable",
                _ => "Not Acceptable Here",
            };
            return Err((code, reason, format!("new node answered {code}")));
        }
        let new_b_dialog = handle.dialog.read().await.clone();
        let answer = String::from_utf8_lossy(final_response.body()).to_string();
        if answer.is_empty() {
            let _ = self.uac.bye(&new_b_dialog).await;
            return Err((
                488,
                "Not Acceptable Here",
                "new node answered without SDP".to_string(),
            ));
        }

        // The caller is still here?
        if self.call_manager.get_call(&self.call_id).is_none() {
            let _ = self.uac.bye(&new_b_dialog).await;
            return Err((481, "Call Gone", "call ended during the move".to_string()));
        }

        // Point the caller at the new node.
        let mut a_sdp_now = a_sdp.clone();
        match &anchored {
            Some((session_id, media_manager)) => {
                media_manager
                    .update_session_b_leg(session_id, answer.clone())
                    .await
                    .map_err(|e| {
                        (
                            500,
                            "Server Internal Error",
                            format!("media session B side: {e}"),
                        )
                    })?;
            }
            None => {
                let resp = tokio::time::timeout(
                    MIGRATION_STEP_TIMEOUT,
                    self.uac.send_reinvite(&mut a_dialog, Some(&answer)),
                )
                .await;
                let ok = matches!(&resp, Ok(Ok(r)) if (200..300).contains(&r.start_line().code()));
                if !ok {
                    let _ = self.uac.bye(&new_b_dialog).await;
                    let why = match resp {
                        Ok(Ok(r)) => format!(
                            "caller answered the re-INVITE with {}",
                            r.start_line().code()
                        ),
                        Ok(Err(e)) => format!("re-INVITE to the caller failed: {e}"),
                        Err(_) => "re-INVITE to the caller timed out".to_string(),
                    };
                    return Err((500, "Server Internal Error", why));
                }
                if let Ok(Ok(r)) = resp {
                    if !r.body().is_empty() {
                        a_sdp_now = String::from_utf8_lossy(r.body()).to_string();
                    }
                }
                let _ = self.dialog_manager.insert(a_dialog.clone());
                // A phone that moved its media in its answer: tell the
                // new node, which was offered the old endpoint.
                if crate::conference_migration::media_endpoint_changed(&a_sdp, &a_sdp_now) {
                    let mut b = new_b_dialog.clone();
                    let outcome = tokio::time::timeout(
                        MIGRATION_STEP_TIMEOUT,
                        self.uac.send_reinvite(&mut b, Some(&a_sdp_now)),
                    )
                    .await;
                    match outcome {
                        Ok(Ok(r)) if (200..300).contains(&r.start_line().code()) => {}
                        Ok(Ok(r)) => warn!(
                            call_id = %self.call_id,
                            status = r.start_line().code(),
                            "The new node refused the caller's moved media endpoint"
                        ),
                        Ok(Err(e)) => warn!(
                            call_id = %self.call_id,
                            error = %e,
                            "The re-INVITE with the caller's moved media endpoint failed"
                        ),
                        Err(_) => warn!(
                            call_id = %self.call_id,
                            "The re-INVITE with the caller's moved media endpoint timed out"
                        ),
                    }
                }
            }
        }

        // Swap the legs.
        let new_sip_call_id = new_b_dialog.id().call_id().to_string();
        let old_sip_call_id = self.old_b_dialog.id().call_id().to_string();
        let a_dialog_for_call = a_dialog.clone();
        let new_b_dialog_for_call = new_b_dialog.clone();
        let pass_through = anchored.is_none();
        let new_host = self.target.as_str().rsplit('@').next().map(str::to_string);
        let swapped = self.call_manager.update_call(&self.call_id, |c| {
            c.conference_node = new_host.clone();
            let mut leg = fcp_b2bua_core::CallLeg::new_outbound(target.clone(), None);
            leg.dialog = Some(new_b_dialog_for_call.clone());
            leg.sdp = Some(answer.clone());
            leg.outbound_handle = Some(Arc::clone(&handle));
            c.b_leg = Some(leg);
            if pass_through {
                c.a_leg.dialog = Some(a_dialog_for_call.clone());
                c.a_leg.sdp = Some(a_sdp_now.clone());
            }
        });
        if swapped.is_err() {
            let _ = self.uac.bye(&new_b_dialog).await;
            return Err((481, "Call Gone", "call ended during the move".to_string()));
        }
        let _ = self.dialog_manager.insert(new_b_dialog.clone());
        self.sip_call_id_map
            .insert(new_sip_call_id, self.call_id.to_string());
        self.sip_call_id_map.remove(&old_sip_call_id);
        self.session_timer_manager
            .stop_timer(self.old_b_dialog.id());
        self.dialog_manager.remove(self.old_b_dialog.id());
        start_session_timer_for(
            &self.session_timer_manager,
            &self.b2bua_config,
            &new_b_dialog,
        );

        if self.old_node_gone {
            return Ok(());
        }
        // Tell the old node, then drop its leg. Its own BYE may cross
        // ours; each side answers 481 to a dialog it no longer holds.
        self.notify_old_node(200, "OK").await;
        let old = self.old_b_dialog.clone();
        let uac = Arc::clone(&self.uac);
        tokio::spawn(async move {
            let _ = tokio::time::timeout(MIGRATION_STEP_TIMEOUT, uac.bye(&old)).await;
        });
        Ok(())
    }

    /// The REFER's implicit subscription, reported once: a sipfrag NOTIFY
    /// on the old leg (RFC 3515 §2.4.5), `Subscription-State: terminated`.
    /// Move the caller off a node that has gone; what became of it.
    async fn rescue(self) -> std::result::Result<(), MigrationFailure> {
        let outcome = self.migrate().await;
        self.in_flight.remove(&self.call_id);
        outcome
    }

    async fn notify_old_node(&self, code: u16, reason: &str) {
        let mut dialog = self.old_b_dialog.clone();
        if let Err(e) = crate::conference_migration::report_refer_outcome(
            &self.uac,
            &self.local_uri,
            &mut dialog,
            code,
            reason,
        )
        .await
        {
            debug!(call_id = %self.call_id, code, error = %e, "refer NOTIFY failed");
        }
    }
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {

    #[test]
    fn a_uri_takes_a_new_user_and_keeps_its_host_and_parameters() {
        assert_eq!(
            super::with_uri_user(
                "sip:15183217034@carrier.example;fcp-dst=192.0.2.1:5060",
                "+15183217034"
            ),
            "sip:+15183217034@carrier.example;fcp-dst=192.0.2.1:5060"
        );
        assert_eq!(
            super::with_uri_user("sip:carrier.example", "+1"),
            "sip:carrier.example"
        );
    }

    /// A carrier's failure sends the call to the next route; the callee's
    /// answer does not (`docs/TENANT_TRUNKS.md` §5).
    #[test]
    fn what_counts_as_the_carrier_failing() {
        for code in [401, 403, 407, 408, 488, 500, 502, 503, 504] {
            assert!(super::carrier_failed(code), "{code} is the carrier's");
        }
        for code in [404, 410, 480, 484, 486, 487, 600, 603, 604] {
            assert!(
                !super::carrier_failed(code),
                "{code} is the callee's answer"
            );
        }
    }
    use super::*;
    use crate::registrar::FcpRegistrar;

    /// The node answers a caller's session timer with the caller's interval
    /// and refresher (RFC 4028 §9), within its own interval and Min-SE.
    #[test]
    fn a_session_timer_is_answered_as_the_caller_asked() {
        let mut config = B2buaConfig::default();
        config.session_timer.enabled = true;
        let invite = |headers: &[(&str, &str)]| {
            let mut h = sip_core::Headers::new();
            for (name, value) in headers {
                h.push(SmolStr::new(*name), SmolStr::new(*value)).unwrap();
            }
            sip_core::Request::new(
                sip_core::RequestLine::new(
                    sip_core::Method::Invite,
                    sip_core::SipUri::parse("sip:1002@pbx").unwrap(),
                ),
                h,
                Bytes::new(),
            )
            .unwrap()
        };
        let answer = |headers: &[(&str, &str)]| session_timer_answer(&config, &invite(headers));
        assert_eq!(answer(&[]), (1800, "uas"));
        assert_eq!(
            answer(&[("Session-Expires", "90;refresher=uac")]),
            (90, "uac")
        );
        assert_eq!(answer(&[("Session-Expires", "120")]), (120, "uas"));
        assert_eq!(
            answer(&[("Session-Expires", "7200;refresher=uas")]),
            (1800, "uas")
        );
        assert_eq!(
            answer(&[("Session-Expires", "100"), ("Min-SE", "150")]),
            (150, "uas")
        );
        assert_eq!(answer(&[("Session-Expires", "nonsense")]), (1800, "uas"));
    }

    #[test]
    fn a_transfer_origin_is_added_as_a_uri_header_and_survives_parsing() {
        let plain = with_uri_header("sip:1003@10.0.0.1:5060", TRANSFERRED_FROM_HEADER, "abc");
        assert_eq!(plain, "sip:1003@10.0.0.1:5060?X-FCP-Transferred-From=abc");
        let more = with_uri_header("sip:1003@pbx?Foo=bar", TRANSFERRED_FROM_HEADER, "abc");
        assert_eq!(more, "sip:1003@pbx?Foo=bar&X-FCP-Transferred-From=abc");
        let uri = sip_core::SipUri::parse(&plain).expect("parses");
        assert_eq!(
            uri.headers()
                .get("x-fcp-transferred-from")
                .map(|v| v.as_str()),
            Some("abc")
        );
    }

    #[test]
    fn a_signed_call_names_its_numbers_as_stir_shaken_states_them() {
        // Digits only, without the `+` (RFC 8224 §4).
        assert_eq!(
            tn_of(Some("sip:+15125550100@carrier.example")),
            "15125550100"
        );
        assert_eq!(tn_of(Some("<sip:15125550100@pbx>")), "15125550100");
        assert_eq!(tn_of(Some("+15125550100")), "15125550100");

        // A call that is not between numbers is not signed.
        assert_eq!(tn_of(Some("sip:alice@pbx.local")), "");
        assert_eq!(
            tn_of(Some("sip:911@psap")),
            "911",
            "an emergency number is one"
        );
        assert_eq!(tn_of(Some("sip:+1-512-555-0100@carrier")), "");
        assert_eq!(tn_of(None), "");
    }

    #[test]
    fn the_trunk_that_answered_is_found_by_its_route_target() {
        let route = |target: &str, name: &str| {
            let mut trunk = fcp_core::Trunk::new(name.to_string(), fcp_core::TenantId::new());
            trunk.id = fcp_core::TrunkId::from_string(format!("trunk-{name}"));
            fcp_b2bua_core::RouteTrunk {
                target: target.to_string(),
                trunk: Arc::new(trunk),
            }
        };
        let routes = vec![
            route("sip:+15550100@primary.example:5060", "primary"),
            route("sip:+15550100@backup.example:5060", "backup"),
        ];
        let answered = answering_trunk(&routes, "<sip:+15550100@backup.example:5060>")
            .expect("the backup route answered");
        assert_eq!(answered.name(), "backup");
        assert_eq!(
            answered.id().to_string(),
            "trunk-backup".to_string(),
            "the record names which trunk, not just what it was called"
        );
        assert!(answering_trunk(&routes, "sip:someone@else").is_none());
    }

    #[test]
    fn rewrite_sdp_direction_replaces_sendrecv_in_every_rtp_section() {
        let sdp = "v=0\r\n\
                   o=- 1 1 IN IP4 127.0.0.1\r\n\
                   s=-\r\n\
                   c=IN IP4 127.0.0.1\r\n\
                   t=0 0\r\n\
                   m=audio 10000 RTP/AVP 0\r\n\
                   a=rtpmap:0 PCMU/8000\r\n\
                   a=sendrecv\r\n\
                   m=video 10002 RTP/AVP 96\r\n\
                   a=rtpmap:96 VP8/90000\r\n\
                   a=sendrecv\r\n";
        let rewritten = SipMessageHandler::rewrite_sdp_direction(sdp, "recvonly");
        assert!(rewritten.contains("m=audio 10000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=recvonly"));
        // Hold applies to the video section too.
        assert!(
            rewritten.contains("m=video 10002 RTP/AVP 96\r\na=rtpmap:96 VP8/90000\r\na=recvonly")
        );
        assert_eq!(rewritten.matches("a=recvonly").count(), 2);
        assert!(!rewritten.contains("a=sendrecv"));
    }

    #[test]
    fn rewrite_sdp_direction_leaves_non_rtp_sections_alone() {
        let sdp = "v=0\r\n\
                   o=- 1 1 IN IP4 127.0.0.1\r\n\
                   s=-\r\n\
                   t=0 0\r\n\
                   m=audio 10000 RTP/AVP 0\r\n\
                   m=application 5000 UDP/BFCP *\r\n\
                   a=sendrecv\r\n\
                   m=video 10002 RTP/AVP 96\r\n";
        let rewritten = SipMessageHandler::rewrite_sdp_direction(sdp, "sendonly");
        // Audio and video each get an inserted direction; BFCP keeps its own.
        assert!(rewritten.contains("m=audio 10000 RTP/AVP 0\r\na=sendonly\r\n"));
        assert!(rewritten.contains("m=application 5000 UDP/BFCP *\r\na=sendrecv\r\n"));
        assert!(rewritten.contains("m=video 10002 RTP/AVP 96\r\na=sendonly"));
        assert_eq!(rewritten.matches("a=sendonly").count(), 2);
    }

    #[test]
    fn rewrite_sdp_direction_inserts_when_absent() {
        // RFC 3264 default is sendrecv; phones commonly omit the attribute.
        let sdp = "v=0\r\n\
                   o=- 1 1 IN IP4 127.0.0.1\r\n\
                   s=-\r\n\
                   c=IN IP4 127.0.0.1\r\n\
                   t=0 0\r\n\
                   m=audio 10000 RTP/AVP 0\r\n\
                   a=rtpmap:0 PCMU/8000\r\n";
        let rewritten = SipMessageHandler::rewrite_sdp_direction(sdp, "sendonly");
        // The direction line should appear right after m=audio.
        let audio_idx = rewritten.find("m=audio").unwrap();
        let sendonly_idx = rewritten.find("a=sendonly").unwrap();
        assert!(sendonly_idx > audio_idx);
        // And must not appear before m=audio (no session-level leak).
        assert!(rewritten[..audio_idx].find("a=sendonly").is_none());
    }

    #[test]
    fn rewrite_sdp_direction_touches_every_audio_section() {
        // Two audio sections (rare but legal) — hold applies to both.
        let sdp = "v=0\r\n\
                   o=- 1 1 IN IP4 127.0.0.1\r\n\
                   s=-\r\n\
                   t=0 0\r\n\
                   m=audio 10000 RTP/AVP 0\r\n\
                   a=sendrecv\r\n\
                   m=audio 10002 RTP/AVP 0\r\n\
                   a=sendrecv\r\n";
        let rewritten = SipMessageHandler::rewrite_sdp_direction(sdp, "recvonly");
        assert_eq!(rewritten.matches("a=recvonly").count(), 2);
        assert_eq!(rewritten.matches("a=sendrecv").count(), 0);
    }

    #[test]
    fn rewrite_sdp_direction_video_only_is_rewritten() {
        let sdp = "v=0\r\n\
                   o=- 1 1 IN IP4 127.0.0.1\r\n\
                   s=-\r\n\
                   t=0 0\r\n\
                   m=video 10002 RTP/AVP 96\r\n\
                   a=sendrecv\r\n";
        let rewritten = SipMessageHandler::rewrite_sdp_direction(sdp, "recvonly");
        assert!(rewritten.contains("m=video 10002 RTP/AVP 96\r\na=recvonly\r\n"));
        assert!(!rewritten.contains("a=sendrecv"));
    }

    #[test]
    fn rewrite_sdp_direction_no_rtp_section_is_noop() {
        let sdp = "v=0\r\n\
                   o=- 1 1 IN IP4 127.0.0.1\r\n\
                   s=-\r\n\
                   t=0 0\r\n\
                   m=application 5000 UDP/BFCP *\r\n\
                   a=sendrecv\r\n";
        let rewritten = SipMessageHandler::rewrite_sdp_direction(sdp, "recvonly");
        assert_eq!(rewritten, sdp);
    }

    #[test]
    fn rewrite_sdp_direction_preserves_lf_only_endings() {
        let sdp = "v=0\n\
                   o=- 1 1 IN IP4 127.0.0.1\n\
                   s=-\n\
                   t=0 0\n\
                   m=audio 10000 RTP/AVP 0\n\
                   a=sendrecv\n";
        let rewritten = SipMessageHandler::rewrite_sdp_direction(sdp, "recvonly");
        assert!(rewritten.contains("\na=recvonly\n"));
        assert!(!rewritten.contains("\r\n"));
    }

    use bytes::Bytes as StdBytes;
    use fcp_core::{FileBackedStorage, RegistrationStore};
    use sip_core::{Headers, Method, RequestLine, SipUri};
    use smol_str::SmolStr;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn test_register_without_registrar() {
        // Create dependencies
        let dispatcher = Arc::new(
            crate::transport::RealTransportDispatcher::with_udp("127.0.0.1:0".parse().unwrap())
                .await
                .unwrap(),
        );
        let transaction_manager = Arc::new(TransactionManager::new(dispatcher));
        let dialog_manager = Arc::new(DialogManager::new());
        let local_uri = SipUri::parse("sip:test@localhost").unwrap();

        // Create CallManager for tests
        let call_manager = Arc::new(fcp_b2bua_core::CallManager::new());

        // Create handler WITHOUT registrar
        let handler = SipMessageHandler::new(
            transaction_manager,
            dialog_manager,
            local_uri,
            None,
            "127.0.0.1".to_string(),
            10000,
            None, // No registrar
            None, // No routing service
            call_manager,
            None, // No UAC
            None, // No storage
            None, // No media manager
            None, // No media config
            None, // No flow handler
            B2buaConfig::default(),
            None,          // No outbound route provider
            None,          // No PBX handler
            None,          // No conference handler
            None,          // No queue handler
            None,          // presence_handler
            None,          // No cluster parking
            String::new(), // node_id
        )
        .unwrap();

        // Create a REGISTER request
        let mut headers = Headers::new();
        let _ = headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123"),
        );
        let _ = headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:alice@example.com>;tag=abc123"),
        );
        let _ = headers.push(SmolStr::new("To"), SmolStr::new("<sip:alice@example.com>"));
        let _ = headers.push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id"));
        let _ = headers.push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER"));
        let _ = headers.push(
            SmolStr::new("Contact"),
            SmolStr::new("<sip:alice@127.0.0.1:5060>"),
        );
        let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        let request = sip_core::Request::new(
            RequestLine::new(Method::Register, SipUri::parse("sip:example.com").unwrap()),
            headers,
            bytes::Bytes::new(),
        )
        .expect("Failed to create request");

        // Handle the REGISTER request
        let response = handler.handle_register(&request).await;

        // Should return 501 Not Implemented (no registrar configured)
        assert_eq!(response.start_line().code(), 501);
        assert_eq!(response.start_line().reason(), "Not Implemented");
    }

    #[tokio::test]
    async fn invite_error_response_has_core_headers() {
        let dispatcher = Arc::new(
            crate::transport::RealTransportDispatcher::with_udp("127.0.0.1:0".parse().unwrap())
                .await
                .unwrap(),
        );
        let transaction_manager = Arc::new(TransactionManager::new(dispatcher));
        let dialog_manager = Arc::new(DialogManager::new());
        let local_uri = SipUri::parse("sip:test@localhost").unwrap();
        let call_manager = Arc::new(fcp_b2bua_core::CallManager::new());

        let handler = SipMessageHandler::new(
            transaction_manager,
            dialog_manager,
            local_uri,
            None,
            "127.0.0.1".to_string(),
            10000,
            None,
            None,
            call_manager,
            None,
            None, // No storage
            None, // No media manager
            None, // No media config
            None, // No flow handler
            B2buaConfig::default(),
            None,          // No outbound route provider
            None,          // No PBX handler
            None,          // No conference handler
            None,          // No queue handler
            None,          // presence_handler
            None,          // No cluster parking
            String::new(), // node_id
        )
        .unwrap();

        let mut headers = Headers::new();
        headers
            .push(
                SmolStr::new("Via"),
                SmolStr::new("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123"),
            )
            .unwrap();
        headers
            .push(
                SmolStr::new("From"),
                SmolStr::new("<sip:alice@example.com>;tag=abc123"),
            )
            .unwrap();
        headers
            .push(SmolStr::new("To"), SmolStr::new("<sip:bob@example.com>"))
            .unwrap();
        headers
            .push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id"))
            .unwrap();
        headers
            .push(SmolStr::new("CSeq"), SmolStr::new("1 INVITE"))
            .unwrap();
        headers
            .push(SmolStr::new("Content-Length"), SmolStr::new("0"))
            .unwrap();

        let request = sip_core::Request::new(
            RequestLine::new(Method::Invite, SipUri::parse("sip:example.com").unwrap()),
            headers,
            bytes::Bytes::new(),
        )
        .expect("Failed to create request");

        let response = handler.create_simple_response(&request, 488, "Not Acceptable Here");
        assert_eq!(response.start_line().code(), 488);
        let to = response.headers().get("To").unwrap();
        assert!(to.contains("tag="));
        assert!(response.headers().get("Contact").is_some());
        assert!(response.headers().get("Allow").is_some());
        assert!(response.headers().get("Supported").is_some());
        assert_eq!(
            response.headers().get("Content-Length").unwrap(),
            &response.body().len().to_string()
        );
    }

    #[tokio::test]
    async fn in_dialog_error_response_hygiene() {
        let dispatcher = Arc::new(
            crate::transport::RealTransportDispatcher::with_udp("127.0.0.1:0".parse().unwrap())
                .await
                .unwrap(),
        );
        let transaction_manager = Arc::new(TransactionManager::new(dispatcher));
        let dialog_manager = Arc::new(DialogManager::new());
        let local_uri = SipUri::parse("sip:test@localhost").unwrap();
        let call_manager = Arc::new(fcp_b2bua_core::CallManager::new());

        let handler = SipMessageHandler::new(
            transaction_manager,
            dialog_manager,
            local_uri,
            None,
            "127.0.0.1".to_string(),
            10000,
            None,
            None,
            call_manager,
            None,
            None, // No storage
            None, // No media manager
            None, // No media config
            None, // No flow handler
            B2buaConfig::default(),
            None,          // No outbound route provider
            None,          // No PBX handler
            None,          // No conference handler
            None,          // No queue handler
            None,          // presence_handler
            None,          // No cluster parking
            String::new(), // node_id
        )
        .unwrap();

        let mut headers = Headers::new();
        headers
            .push(
                SmolStr::new("Via"),
                SmolStr::new("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bKxyz"),
            )
            .unwrap();
        headers
            .push(
                SmolStr::new("From"),
                SmolStr::new("<sip:alice@example.com>;tag=abc123"),
            )
            .unwrap();
        headers
            .push(
                SmolStr::new("To"),
                SmolStr::new("<sip:bob@example.com>;tag=bbb"),
            )
            .unwrap();
        headers
            .push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id-2"))
            .unwrap();
        headers
            .push(SmolStr::new("CSeq"), SmolStr::new("2 INVITE"))
            .unwrap();
        headers
            .push(SmolStr::new("Content-Length"), SmolStr::new("0"))
            .unwrap();

        let request = sip_core::Request::new(
            RequestLine::new(Method::Invite, SipUri::parse("sip:example.com").unwrap()),
            headers,
            bytes::Bytes::new(),
        )
        .expect("Failed to create request");

        let peer_response =
            handler.create_simple_response(&request, 481, "Call/Transaction Does Not Exist");
        // Add a dummy body to ensure Content-Length is recalculated
        let peer_response = sip_core::Response::new(
            peer_response.start_line().clone(),
            peer_response.headers().clone(),
            bytes::Bytes::from_static(b"oops"),
        )
        .expect("Failed to create response with body");
        let response = handler.build_in_dialog_response(&request, &peer_response);
        assert_eq!(response.start_line().code(), 481);
        assert!(response.headers().get("Contact").is_some());
        assert!(response.headers().get("Allow").is_some());
        assert!(response.headers().get("Supported").is_some());
        assert_eq!(
            response.headers().get("Content-Length").unwrap(),
            &response.body().len().to_string()
        );
    }

    #[tokio::test]
    async fn session_timer_emits_refresh_for_refresher() {
        let manager = SessionTimerManager::new();
        let dialog_id = sip_dialog::DialogId::new("call1", "local", "remote")
            .expect("Failed to create dialog ID");
        let mut rx = manager.subscribe().await;

        manager.start_timer(dialog_id.clone(), Duration::from_millis(40), true);

        let first = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("timeout waiting for refresh");
        assert_eq!(
            first,
            Some(SessionTimerEvent::RefreshNeeded(dialog_id.clone()))
        );

        let second = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("timeout waiting for expiration");
        assert_eq!(
            second,
            Some(SessionTimerEvent::SessionExpired(dialog_id.clone()))
        );
    }

    #[tokio::test]
    async fn session_timer_skips_refresh_when_not_refresher() {
        let manager = SessionTimerManager::new();
        let dialog_id = sip_dialog::DialogId::new("call2", "local", "remote")
            .expect("Failed to create dialog ID");
        let mut rx = manager.subscribe().await;

        manager.start_timer(dialog_id.clone(), Duration::from_millis(30), false);

        let evt = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("timeout waiting for expiration");
        assert_eq!(
            evt,
            Some(SessionTimerEvent::SessionExpired(dialog_id.clone()))
        );
    }

    // NOTE: This test is disabled because Dialog struct fields are now private
    // and there's no public API to set refresher/is_uac for testing.
    // The dialog_is_refresher logic is still tested indirectly through integration tests.
    /*
    #[test]
    fn dialog_refresher_role_matches_leg() {
        let uri = SipUri::parse("sip:alice@example.com").unwrap();
        let id = sip_dialog::DialogId::new("call3", "local", "remote").expect("Failed to create dialog ID");
        let base_dialog = sip_dialog::Dialog {
            id,
            state: sip_dialog::DialogStateType::Confirmed,
            remote_target: uri.clone(),
            route_set: Vec::new(),
            local_cseq: 1,
            remote_cseq: 1,
            last_ack_cseq: None,
            local_uri: uri.clone(),
            remote_uri: uri.clone(),
            secure: false,
            session_expires: Some(Duration::from_secs(90)),
            refresher: Some(sip_core::RefresherRole::Uac),
            is_uac: true,
        };

        assert!(SipMessageHandler::dialog_is_refresher(&base_dialog));

        let mut uas_dialog = base_dialog.clone();
        uas_dialog.refresher() = Some(sip_core::RefresherRole::Uas);
        uas_dialog.is_uac() = false;
        assert!(SipMessageHandler::dialog_is_refresher(&uas_dialog));

        let mut mismatch = base_dialog.clone();
        mismatch.refresher = Some(sip_core::RefresherRole::Uas);
        mismatch.is_uac = true;
        assert!(!SipMessageHandler::dialog_is_refresher(&mismatch));
    }
    */

    #[tokio::test]
    async fn test_register_with_registrar() {
        // Create storage
        let temp_dir = std::env::temp_dir().join("fcp-test-handler-register");
        let storage = Arc::new(FileBackedStorage::new(temp_dir.clone()).await.unwrap());

        // Create registrar (without authentication)
        let registrar = Arc::new(
            FcpRegistrar::new(storage.clone(), None, None, false)
                .with_default_tenant_id(fcp_core::TenantId::nil()),
        );

        // Create dependencies
        let dispatcher = Arc::new(
            crate::transport::RealTransportDispatcher::with_udp("127.0.0.1:0".parse().unwrap())
                .await
                .unwrap(),
        );
        let transaction_manager = Arc::new(TransactionManager::new(dispatcher));
        let dialog_manager = Arc::new(DialogManager::new());
        let local_uri = SipUri::parse("sip:test@localhost").unwrap();

        // Create CallManager for tests
        let call_manager = Arc::new(fcp_b2bua_core::CallManager::new());

        // Create handler WITH registrar
        let handler = SipMessageHandler::new(
            transaction_manager,
            dialog_manager,
            local_uri,
            None,
            "127.0.0.1".to_string(),
            10000,
            Some(registrar.clone()),
            None, // No routing service
            call_manager,
            None, // No UAC
            None, // No storage
            None, // No media manager
            None, // No media config
            None, // No flow handler
            B2buaConfig::default(),
            None,          // No outbound route provider
            None,          // No PBX handler
            None,          // No conference handler
            None,          // No queue handler
            None,          // presence_handler
            None,          // No cluster parking
            String::new(), // node_id
        )
        .unwrap();

        // Create a REGISTER request
        let mut headers = Headers::new();
        headers
            .push(
                SmolStr::new("Via"),
                SmolStr::new("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123"),
            )
            .unwrap();
        headers
            .push(
                SmolStr::new("From"),
                SmolStr::new("<sip:alice@example.com>;tag=abc123"),
            )
            .unwrap();
        headers
            .push(SmolStr::new("To"), SmolStr::new("<sip:alice@example.com>"))
            .unwrap();
        headers
            .push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id"))
            .unwrap();
        headers
            .push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER"))
            .unwrap();
        headers
            .push(
                SmolStr::new("Contact"),
                SmolStr::new("<sip:alice@127.0.0.1:5060>;expires=3600"),
            )
            .unwrap();
        headers
            .push(SmolStr::new("Expires"), SmolStr::new("3600"))
            .unwrap();
        headers
            .push(SmolStr::new("Content-Length"), SmolStr::new("0"))
            .unwrap();

        let request = sip_core::Request::new(
            RequestLine::new(Method::Register, SipUri::parse("sip:example.com").unwrap()),
            headers,
            bytes::Bytes::new(),
        )
        .expect("Failed to create request");

        // Handle the REGISTER request
        let response = handler.handle_register(&request).await;

        // Should return 200 OK (registration successful)
        assert_eq!(response.start_line().code(), 200);
        assert_eq!(response.start_line().reason(), "OK");

        // Verify the registration was stored
        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 1);
        // Contact may include angle brackets as stored in the Contact header
        assert!(
            regs[0].contact == "sip:alice@127.0.0.1:5060"
                || regs[0].contact == "<sip:alice@127.0.0.1:5060>"
        );

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir).await.ok();
    }

    fn build_invite_with_sdp(call_id: &str, has_audio: bool) -> sip_core::Request {
        let mut headers = Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123"),
        );
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:alice@example.com>;tag=abc123"),
        );
        headers.push(SmolStr::new("To"), SmolStr::new("<sip:bob@example.com>"));
        headers.push(SmolStr::new("Call-ID"), SmolStr::new(call_id));
        headers.push(SmolStr::new("CSeq"), SmolStr::new("1 INVITE"));
        headers.push(
            SmolStr::new("Contact"),
            SmolStr::new("<sip:alice@127.0.0.1:5060>"),
        );
        headers.push(
            SmolStr::new("Content-Type"),
            SmolStr::new("application/sdp"),
        );

        let sdp = if has_audio {
            "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 4000 RTP/AVP 0\r\n".to_string()
        } else {
            "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n".to_string()
        };
        headers.push(
            SmolStr::new("Content-Length"),
            SmolStr::new(format!("{}", sdp.len())),
        );

        sip_core::Request::new(
            RequestLine::new(
                Method::Invite,
                SipUri::parse("sip:bob@example.com").unwrap(),
            ),
            headers,
            StdBytes::from(sdp.into_bytes()),
        )
        .expect("Failed to create request")
    }

    #[tokio::test]
    async fn test_store_sdp_on_reinvite() {
        let call = fcp_b2bua_core::Call::from_invite(
            build_invite_with_sdp("call-a", true),
            "127.0.0.1:5060".parse().unwrap(),
            fcp_core::TenantId::nil(), // Use default tenant for test
        );
        let call_id = call.id;
        let cm = fcp_b2bua_core::CallManager::new();
        cm.create_call(call.clone());

        store_leg_sdp(&cm, &call_id, true, "test-sdp".to_string());
        let updated = cm.get_call(&call_id).unwrap();
        assert_eq!(updated.a_leg.sdp.as_deref(), Some("test-sdp"));
    }

    #[test]
    fn test_validate_sdp_body_good() {
        let req = build_invite_with_sdp("call-b", true);
        let body = validate_sdp_body_static(&req, None).unwrap();
        assert!(body.is_some());
    }

    #[test]
    fn test_validate_sdp_body_missing_audio() {
        let req = build_invite_with_sdp("call-c", false);
        let result = validate_sdp_body_static(&req, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_sdp_body_empty_without_prior_fails() {
        let req = build_invite_with_sdp("call-d", true);
        // Reconstruct request with empty body
        let mut headers = req.headers().clone();
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));
        let req = sip_core::Request::new(req.start_line().clone(), headers, StdBytes::new())
            .expect("Failed to create request");
        let result = validate_sdp_body_static(&req, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_sdp_body_empty_with_prior_passes() {
        let req = build_invite_with_sdp("call-e", true);
        // Reconstruct request with empty body
        let mut headers = req.headers().clone();
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));
        let req = sip_core::Request::new(req.start_line().clone(), headers, StdBytes::new())
            .expect("Failed to create request");
        let result = validate_sdp_body_static(&req, Some("v=0\r\ns=-\r\n"));
        assert!(result.is_ok());
    }

    // ========================================================================
    // SDP Hold/Unhold Tests
    // ========================================================================

    #[test]
    fn test_generate_hold_sdp_from_existing() {
        let sdp = "v=0\r\n\
                    o=- 0 0 IN IP4 192.168.1.1\r\n\
                    s=-\r\n\
                    c=IN IP4 192.168.1.1\r\n\
                    t=0 0\r\n\
                    m=audio 4000 RTP/AVP 0\r\n\
                    a=sendrecv\r\n";

        let held = generate_hold_sdp(Some(sdp), "10.0.0.1", 5000);
        assert!(held.contains("a=sendonly"));
        assert!(!held.contains("a=sendrecv"));
    }

    #[test]
    fn test_generate_hold_sdp_without_existing() {
        let held = generate_hold_sdp(None, "10.0.0.1", 5000);
        assert!(held.contains("a=sendonly"));
        assert!(held.contains("10.0.0.1"));
        assert!(held.contains("5000"));
    }

    #[test]
    fn test_generate_hold_sdp_no_sendrecv_attribute() {
        // SDP without any directional attribute — should get sendonly appended
        let sdp = "v=0\r\n\
                    o=- 0 0 IN IP4 192.168.1.1\r\n\
                    s=-\r\n\
                    c=IN IP4 192.168.1.1\r\n\
                    t=0 0\r\n\
                    m=audio 4000 RTP/AVP 0\r\n";

        let held = generate_hold_sdp(Some(sdp), "10.0.0.1", 5000);
        assert!(held.contains("a=sendonly"));
    }

    #[test]
    fn test_hold_unhold_roundtrip() {
        let sdp = "v=0\r\n\
                    o=- 0 0 IN IP4 192.168.1.1\r\n\
                    s=-\r\n\
                    c=IN IP4 192.168.1.1\r\n\
                    t=0 0\r\n\
                    m=audio 4000 RTP/AVP 0\r\n\
                    a=sendrecv\r\n";

        let held = generate_hold_sdp(Some(sdp), "10.0.0.1", 5000);
        assert!(held.contains("a=sendonly"));

        let unheld = crate::hold::with_direction(&held, "sendrecv");
        assert!(unheld.contains("a=sendrecv"));
        assert!(!unheld.contains("a=sendonly"));
    }

    // =========================================================================
    // CFNA / Voicemail URI construction tests
    // =========================================================================

    #[test]
    fn test_voicemail_uri_from_e164_callee() {
        // Simulate the URI construction logic from route_cfna
        let vm_uri = "sip:vm@192.168.1.50:5060";
        let original_callee = "sip:+15551234567@pbx.local";

        let vm_host = vm_uri
            .strip_prefix("sip:")
            .unwrap_or(vm_uri)
            .split('@')
            .next_back()
            .unwrap_or(vm_uri);
        let user_part = original_callee
            .strip_prefix("sip:")
            .unwrap_or(original_callee)
            .split('@')
            .next()
            .unwrap_or(original_callee);
        let result = format!("sip:{}@{}", user_part, vm_host);

        assert_eq!(result, "sip:+15551234567@192.168.1.50:5060");
    }

    #[test]
    fn test_voicemail_uri_from_extension_callee() {
        let vm_uri = "sip:vm@voicemail.local";
        let original_callee = "5000";

        let vm_host = vm_uri
            .strip_prefix("sip:")
            .unwrap_or(vm_uri)
            .split('@')
            .next_back()
            .unwrap_or(vm_uri);
        let user_part = original_callee
            .strip_prefix("sip:")
            .unwrap_or(original_callee)
            .split('@')
            .next()
            .unwrap_or(original_callee);
        let result = format!("sip:{}@{}", user_part, vm_host);

        assert_eq!(result, "sip:5000@voicemail.local");
    }

    #[test]
    // The `None` callee is the whole point of this case, so the literal-unwrap
    // lint on `original_callee.unwrap_or(..)` is expected here.
    #[allow(clippy::unnecessary_literal_unwrap)]
    fn test_voicemail_uri_unknown_callee() {
        let vm_uri = "sip:vm@192.168.1.50:5060";
        let original_callee: Option<&str> = None;

        let vm_host = vm_uri
            .strip_prefix("sip:")
            .unwrap_or(vm_uri)
            .split('@')
            .next_back()
            .unwrap_or(vm_uri);
        let user = original_callee.unwrap_or("unknown");
        let user_part = user
            .strip_prefix("sip:")
            .unwrap_or(user)
            .split('@')
            .next()
            .unwrap_or(user);
        let result = format!("sip:{}@{}", user_part, vm_host);

        assert_eq!(result, "sip:unknown@192.168.1.50:5060");
    }

    #[test]
    fn test_cfna_destination_direct_sip_uri() {
        // When destination starts with sip:, it should be used directly
        let destination = "sip:+15559876543@gateway.example.com";
        let is_direct = destination.starts_with("sip:") || destination.starts_with("sips:");
        assert!(is_direct);
    }

    #[test]
    fn test_cfna_destination_extension_resolution() {
        // When destination is a plain number, it should be resolved as extension
        let destination = "5001";
        let host = "pbx.local";
        let is_direct = destination.starts_with("sip:") || destination.starts_with("sips:");
        assert!(!is_direct);
        let is_voicemail = destination.eq_ignore_ascii_case("voicemail");
        assert!(!is_voicemail);
        let result = format!("sip:{}@{}", destination, host);
        assert_eq!(result, "sip:5001@pbx.local");
    }
}
