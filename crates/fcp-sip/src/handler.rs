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

use anyhow::Result;
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
use fcp_core::storage::{
    DeviceStore, ExtensionStore, LineBindingStore, PhoneNumberStore, TenantStore,
};

/// Result of outbound route resolution.
#[derive(Debug, Clone)]
pub struct OutboundRoute {
    /// Target SIP URI to INVITE.
    pub target_uri: String,
    /// Name of the trunk selected (if trunk-based routing was used).
    pub trunk_name: Option<String>,
    /// Allowed codecs for this trunk in preference order (empty = no restriction).
    pub allowed_codecs: Option<Vec<String>>,
}

/// Provides fallback outbound routing when registrar lookup fails.
///
/// Used by the SBC to route calls to trunks when the destination
/// is not locally registered.
#[async_trait::async_trait]
pub trait OutboundRouteProvider: Send + Sync {
    /// Given a destination URI (e.g. "sip:+15551234@example.com"),
    /// return an outbound route with target URI and optional trunk name,
    /// or None if no route available.
    async fn resolve_outbound(&self, destination_uri: &str) -> Option<OutboundRoute>;

    /// Return an ordered list of routes for failover. Default wraps single route.
    async fn resolve_outbound_ordered(&self, destination_uri: &str) -> Vec<OutboundRoute> {
        self.resolve_outbound(destination_uri)
            .await
            .into_iter()
            .collect()
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

    /// User Agent Client for making outbound calls (B-leg)
    uac: Option<Arc<SipUserAgentClient>>,

    /// Optional storage for line binding lookups
    storage: Option<Arc<fcp_core::DynStorage>>,

    /// Media manager for RTP handling
    media_manager: Option<Arc<fcp_media::MediaManager>>,

    /// Media configuration
    media_config: Option<fcp_core::config::MediaConfig>,

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
        settings.validate().map_err(|e| anyhow::anyhow!("Invalid ServerSettings: {}", e))?;

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

        let this = Self {
            transaction_manager,
            dialog_manager,
            uas: Arc::new(uas),
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
            node_id,
            flow_registry: Arc::new(crate::flow::FlowRegistry::new()),
            sip_trace_tx: None,
            sip_call_id_map: Arc::new(dashmap::DashMap::new()),
        };

        // Start session timer event loop
        this.spawn_session_timer_listener();

        // Start session timer health monitor
        this.spawn_session_timer_monitor();

        // Start subscription expiry monitor
        this.spawn_subscription_expiry_monitor();

        Ok(this)
    }

    /// Get a reference to the B2BUA policy configuration.
    pub fn b2bua_config(&self) -> &B2buaConfig {
        &self.b2bua_config
    }

    /// Get a reference to the runtime-reloadable voicemail server URI override.
    pub fn voicemail_server_uri_override(&self) -> &Arc<std::sync::RwLock<Option<String>>> {
        &self.voicemail_server_uri_override
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
    ) -> Result<()> {
        use sip_core::Method;

        match request.method() {
            Method::Invite => {
                // If this is an in-dialog re-INVITE, handle separately
                if self.dialog_manager.find_by_request(request).is_some()
                    || self.is_b_leg_reinvite(request)
                {
                    // Check conference handler first — conference participants
                    // don't have B2BUA call objects
                    if let Some(ref conf) = self.conference_handler {
                        if let Ok(resp) = conf.handle_reinvite(request) {
                            handle.send_final(resp).await;
                            return Ok(());
                        }
                    }
                    // Fall through to B2BUA re-INVITE
                    let response = self.handle_reinvite(request).await?;
                    handle.send_final(response).await;
                } else {
                    // Spawn INVITE handling in separate task to avoid blocking server transaction
                    // This prevents B2BUA from blocking for 32 seconds waiting for B-leg responses
                    let handler = self.clone();
                    let request_clone = request.clone();
                    let handle_clone = handle.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handler
                            .handle_invite(&request_clone, &handle_clone, peer)
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
                let response = self.handle_subscribe(request).await?;
                handle.send_final(response).await;
            }
            Method::Notify => {
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
                let response = self
                    .handle_register_internal(request, transport, stream, peer)
                    .await;
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

                // Signal the parallel forking task to cancel all B-legs immediately.
                // This wakes up the select! in handle_parallel_forking which then
                // CANCELs every B-leg it owns.
                if let Some(notify) = &call.cancel_notify {
                    info!(call_id = %call.id, "Signaling forking task to CANCEL all B-legs via cancel_notify");
                    notify.notify_one();
                    info!(call_id = %call.id, "cancel_notify.notify_one() called successfully");
                } else {
                    warn!(
                        call_id = %call.id,
                        call_state = ?call.state,
                        "CANCEL received but cancel_notify is None - B-legs may not be cancelled immediately"
                    );
                }

                // Also try the direct outbound handle path (for established calls
                // where the forking task has already completed and stored the handle)
                let dialog_manager = Arc::clone(&self.dialog_manager);
                if let Some(b_leg) = &call.b_leg {
                    let outbound_handle_clone = b_leg.outbound_handle.clone();

                    // Clean any stored B-leg dialog in CallManager
                    let _ = self.call_manager.update_call(&call.id, |c| {
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
                            if b.outbound_handle.is_some() {
                                info!(call_id = %c.id, "Clearing B-leg outbound handle due to CANCEL");
                                b.outbound_handle = None;
                            }
                        }
                    });

                    // Send CANCEL via direct outbound handle (works for established calls)
                    if let Some(outbound) = outbound_handle_clone {
                        info!(call_id = %call.id, "Sending CANCEL to B-leg via outbound handle");
                        let call_id_log = call.id;

                        tokio::spawn(async move {
                            match outbound.cancel().await {
                                Ok(_response) => {
                                    info!(call_id = %call_id_log, "Successfully sent CANCEL to B-leg");
                                }
                                Err(e) => {
                                    warn!(call_id = %call_id_log, error = %e, "Failed to send CANCEL to B-leg");
                                }
                            }
                        });
                    }
                }

                // Send 487 to the original INVITE transaction on A-leg if we still have it
                if let (Some(tx_key), Some(invite_req)) =
                    (&call.a_leg_invite_tx, &call.original_invite)
                {
                    let response =
                        self.create_cancelled_response(invite_req, call.a_leg_to_tag.as_deref());
                    self.transaction_manager.send_final(tx_key, response).await;
                }

                // Terminate media session if anchored
                if call.media_anchored {
                    if let (Some(media_session_id), Some(media_manager)) =
                        (&call.media_session_id, &self.media_manager)
                    {
                        let media_session_id = media_session_id.clone();
                        let media_manager = Arc::clone(media_manager);
                        let call_id = call.id;

                        tokio::spawn(async move {
                            match media_manager.terminate_session(&media_session_id).await {
                                Ok(()) => {
                                    info!(
                                        call_id = %call_id,
                                        media_session_id = %media_session_id,
                                        "Terminated media session (CANCEL)"
                                    );
                                }
                                Err(e) => {
                                    error!(
                                        call_id = %call_id,
                                        media_session_id = %media_session_id,
                                        error = %e,
                                        "Failed to terminate media session (CANCEL)"
                                    );
                                }
                            }
                        });
                    }
                }

                stop_timers_for_call(&self.session_timer_manager, &call);
                let _ = self
                    .call_manager
                    .terminate_call(&call.id, Some("Cancelled by caller".to_string()));
            } else {
                debug!("CANCEL received but no matching call found");
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

        // Check routing policy if routing service is configured
        if let Some(routing_service) = &self.routing_service {
            match self
                .evaluate_routing_policy(request, routing_service, peer)
                .await
            {
                Ok(decision) => {
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

        // Extract tenant for multi-tenant isolation
        // Try to extract domain from INVITE and look up tenant in storage
        let tenant_id = if let Some(storage) = &self.storage {
            if let Some(domain) = extract_tenant_from_invite(request) {
                // Look up tenant by domain name
                match storage.get_tenant_by_name(&domain).await {
                    Ok(Some(tenant)) => {
                        debug!(
                            domain = %domain,
                            tenant_id = %tenant.id,
                            tenant_name = %tenant.name,
                            "Resolved tenant from INVITE domain"
                        );
                        tenant.id
                    }
                    Ok(None) => {
                        debug!(
                            domain = %domain,
                            "Domain in INVITE not found in tenant store, using default tenant"
                        );
                        fcp_core::TenantId::nil()
                    }
                    Err(e) => {
                        warn!(
                            domain = %domain,
                            error = %e,
                            "Failed to lookup tenant by domain, using default tenant"
                        );
                        fcp_core::TenantId::nil()
                    }
                }
            } else {
                // No domain found in INVITE, use default tenant
                debug!("No domain found in INVITE for tenant resolution, using default tenant");
                fcp_core::TenantId::nil()
            }
        } else {
            // No storage available, use default tenant
            fcp_core::TenantId::nil()
        };

        // Create Call object from the incoming INVITE with tenant context
        let mut call = fcp_b2bua_core::Call::from_invite(request.clone(), peer, tenant_id);
        call.state = fcp_b2bua_core::CallState::RoutingPending;
        call.a_leg_invite_tx = Some(handle.key().clone());

        // Store call in CallManager
        let call_id = self.call_manager.create_call(call);

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
            request.headers().iter()
                .filter(|h| {
                    let n = h.name();
                    n.eq_ignore_ascii_case("Via") || n.eq_ignore_ascii_case("From")
                        || n.eq_ignore_ascii_case("To") || n.eq_ignore_ascii_case("Call-ID")
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
                    return self.handle_pbx_feature_code(request, &handle, &call_id, action).await;
                }
            }
        }

        // Check if a conference handler wants this call (before flow routing)
        if let Some(conf_handler) = &self.conference_handler {
            if conf_handler.can_handle(request).await {
                debug!(call_id = %call_id, "Routing to conference handler");
                return conf_handler.handle_invite(request, handle, peer).await;
            }
        }

        // Check if a queue handler wants this call (before flow routing)
        if let Some(queue_handler) = &self.queue_handler {
            if queue_handler.can_handle(request).await {
                debug!(call_id = %call_id, "Routing to queue handler");
                return queue_handler.handle_invite(request, handle, peer).await;
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

                        // Build initial variables from call context
                        let mut initial_variables = std::collections::HashMap::new();

                        // Extract From URI (caller number and display name)
                        if let Some(from_header) = sip_parse::header(request.headers(), "From") {
                            let from_str = from_header.as_str();
                            if let Some(caller) = Self::extract_did_from_uri(from_str) {
                                initial_variables.insert("caller_number".to_string(), caller.clone());
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
                            initial_variables.insert(
                                "is_busy".to_string(),
                                (val == "busy").to_string(),
                            );
                        }

                        // Create callbacks for Answer and Hangup nodes
                        let answer_handle = handle.clone();
                        let answer_request = request.clone();
                        let answer_call_id = call_id;
                        let answer_call_manager = Arc::clone(&self.call_manager);
                        let answer_media_manager = self.media_manager.clone();
                        let answer_uas = self.uas.clone();
                        let answer_dialog_manager = Arc::clone(&self.dialog_manager);
                        let answer_callback: fcp_flow::AnswerCallback = Arc::new(move || {
                            let handle = answer_handle.clone();
                            let request = answer_request.clone();
                            let call_id = answer_call_id;
                            let call_manager = Arc::clone(&answer_call_manager);
                            let media_manager = answer_media_manager.clone();
                            let uas = answer_uas.clone();
                            let dialog_manager = Arc::clone(&answer_dialog_manager);
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
                                            if !request.body().is_empty() {
                                                let remote_sdp =
                                                    String::from_utf8_lossy(request.body())
                                                        .to_string();

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
                                                                &sdp_bytes.len().to_string(),
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

                                // Send 200 OK
                                handle.send_final(response.clone()).await;

                                // Create A-leg dialog from the INVITE request and 200 OK response
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

                                    // Update call with dialog
                                    let _ = call_manager.update_call(&call_id, |call| {
                                        call.a_leg.dialog = Some(a_leg_dialog.clone());
                                        call.state = fcp_b2bua_core::CallState::Established;
                                    });

                                    info!(call_id = %call_id, "A-leg dialog stored in call");
                                } else {
                                    error!(call_id = %call_id, "Failed to create A-leg dialog from 200 OK");
                                }

                                Ok(())
                            })
                        });

                        let hangup_call_id = call_id;
                        let hangup_call_manager = Arc::clone(&self.call_manager);
                        let hangup_uac = self.uac.clone();
                        let hangup_dialog_manager = Arc::clone(&self.dialog_manager);
                        let hangup_media_manager = self.media_manager.clone();
                        let hangup_callback: fcp_flow::HangupCallback = Arc::new(move |reason| {
                            let call_id = hangup_call_id;
                            let call_manager = Arc::clone(&hangup_call_manager);
                            let uac_opt = hangup_uac.clone();
                            let dialog_manager = Arc::clone(&hangup_dialog_manager);
                            let media_manager = hangup_media_manager.clone();
                            Box::pin(async move {
                                info!(call_id = %call_id, "Hangup callback executing");
                                // Get the dialog for this call
                                if let Some(call_entry) = call_manager.get_call(&call_id) {
                                    info!(call_id = %call_id, "Found call entry, checking for dialog");

                                    // Terminate media session if anchored
                                    if call_entry.media_anchored {
                                        if let (Some(ref media_session_id), Some(ref media_mgr)) =
                                            (&call_entry.media_session_id, &media_manager)
                                        {
                                            match media_mgr.terminate_session(media_session_id).await {
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

                                // Terminate the call
                                let _ = call_manager.terminate_call(&call_id, reason);
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
        // Policy short-circuit: B2buaConfig media anchoring overrides
        if self.b2bua_config.media_anchoring.force_anchor() {
            info!(call_id = %call_id, "Media anchoring forced by B2BUA policy (Always mode)");
            // Mark call as media-anchored and create a media session
            if let (Some(media_manager), Some(_)) = (&self.media_manager, &self.media_config) {
                let fcp_call_id = fcp_core::CallId::new();
                let fcp_dialog_id = fcp_core::DialogId::new();
                match media_manager
                    .create_session_default(fcp_call_id.clone(), fcp_dialog_id)
                    .await
                {
                    Ok(_session) => {
                        // Set A-leg remote endpoint from INVITE SDP
                        if !request.body().is_empty() {
                            let a_leg_sdp = String::from_utf8_lossy(request.body()).to_string();
                            if let Err(e) = media_manager.update_session(&fcp_call_id, a_leg_sdp).await {
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

        // Extract codecs from A-leg SDP
        let a_leg_codecs = if !request.body().is_empty() {
            let sdp = String::from_utf8_lossy(request.body());
            fcp_media::extract_codecs_from_sdp(&sdp)
        } else {
            Vec::new()
        };

        debug!(
            call_id = %call_id,
            codecs = ?a_leg_codecs,
            "Extracted codecs from A-leg SDP"
        );

        // Look up user media preferences
        let user_prefs = if let Some(ref registrar) = self.registrar {
            // Extract username from From header (simple parsing)
            // From: "Alice" <sip:alice@example.com> or sip:alice@example.com
            let username = sip_parse::header(request.headers(), "From").and_then(|from| {
                // Extract URI between < > if present, otherwise use whole string
                let uri_part = if let Some(start) = from.as_str().find('<') {
                    if let Some(end) = from.as_str().find('>') {
                        &from.as_str()[start + 1..end]
                    } else {
                        from.as_str()
                    }
                } else {
                    from.as_str()
                };

                // Extract username from sip:username@domain
                if let Some(user_start) = uri_part.find("sip:") {
                    let after_sip = &uri_part[user_start + 4..];
                    if let Some(at_pos) = after_sip.find('@') {
                        return Some(after_sip[..at_pos].to_string());
                    }
                }
                None
            });

            if let Some(user) = username {
                registrar.get_user_media_preferences(&user).await
            } else {
                fcp_core::MediaPreferences::default()
            }
        } else {
            fcp_core::MediaPreferences::default()
        };

        // Build decision context using user preferences
        let ctx = fcp_media::MediaDecisionContext {
            media_enabled: media_config.enabled,
            transcoding_enabled: media_config.enable_transcoding,
            recording_enabled: media_config.enable_recording,
            dtmf_enabled: media_config.enable_dtmf,
            a_leg_codecs,
            b_leg_codecs: None, // B-leg not known yet
            recording_requested: user_prefs.record_all_calls,
            dtmf_requested: user_prefs.enable_dtmf_detection || user_prefs.force_media_anchoring,
            conference_call: false, // TODO: Detect conference scenarios
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
                    .create_session_default(fcp_call_id.clone(), fcp_dialog_id)
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
                        if !request.body().is_empty() {
                            let a_leg_sdp = String::from_utf8_lossy(request.body()).to_string();
                            if let Err(e) = media_manager.update_session(&fcp_call_id, a_leg_sdp).await {
                                warn!(call_id = %call_id, error = %e, "Failed to set A-leg media endpoint");
                            }
                        }

                        // Update call object to track media session
                        let _ = self.call_manager.update_call(call_id, |call| {
                            call.media_session_id = Some(fcp_call_id);
                            call.media_anchored = true;
                        });
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
        if config.hide_contact {
            if headers.get("Contact").is_some() {
                headers.remove("Contact");
                let contact_value = format!("<sip:{}>", external_domain);
                let _ = headers.push("Contact", &contact_value);
            }
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
        // Extract To header to get destination AOR
        let to = sip_parse::header(request.headers(), "To")
            .ok_or_else(|| anyhow::anyhow!("Missing To header"))?;

        // Extract destination URI
        let destination_uri = to
            .split('<')
            .nth(1)
            .and_then(|s| s.split('>').next())
            .unwrap_or(&to)
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
        let destination_contacts = if is_e164_number(&normalized_aor) {
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

                                // Store routing context for CFNA lookup
                                {
                                    let lb_id = line_binding.id;
                                    let callee = destination_aor.to_string();
                                    let rna_timeout = line_binding.no_answer_timeout;
                                    let _ = self.call_manager.update_call(&call_id, |call| {
                                        call.line_binding_id = Some(lb_id);
                                        call.original_callee = Some(callee);
                                        call.no_answer_timeout = Some(rna_timeout);
                                    });
                                }

                                // Handle different binding types
                                match line_binding.binding_type {
                                    fcp_core::BindingType::User => {
                                        // Get devices from line binding and look up their registrations
                                        let enabled_devices = line_binding.enabled_devices();

                                        if enabled_devices.is_empty() {
                                            warn!(call_id = %call_id, "Line binding has no enabled devices");
                                            let response = self.create_simple_response(
                                                request,
                                                480,
                                                "Temporarily Unavailable",
                                            );
                                            a_leg_handle.send_final(response).await;
                                            let _ = self.call_manager.terminate_call(
                                                &call_id,
                                                Some("No devices enabled for line".to_string()),
                                            );
                                            return Ok(());
                                        }

                                        // Collect all contact URIs from all device registrations
                                        let mut all_contacts = Vec::new();
                                        for device_binding in enabled_devices {
                                            // Get the device record from storage
                                            match storage
                                                .get_device(&device_binding.device_id)
                                                .await
                                            {
                                                Ok(Some(device)) => {
                                                    if let Some(sip_uri) = &device.sip_uri {
                                                        // Look up registrations for this device's SIP URI
                                                        if let Some(registrar) = &self.registrar {
                                                            match registrar
                                                                .get_registrations(sip_uri)
                                                                .await
                                                            {
                                                                Ok(contacts)
                                                                    if !contacts.is_empty() =>
                                                                {
                                                                    debug!(
                                                                        call_id = %call_id,
                                                                        device = %device.name,
                                                                        sip_uri = %sip_uri,
                                                                        contacts = contacts.len(),
                                                                        "Found registrations for device"
                                                                    );
                                                                    all_contacts.extend(contacts);
                                                                }
                                                                _ => {
                                                                    debug!(
                                                                        call_id = %call_id,
                                                                        device = %device.name,
                                                                        sip_uri = %sip_uri,
                                                                        "Device not currently registered"
                                                                    );
                                                                }
                                                            }
                                                        }
                                                    } else {
                                                        debug!(
                                                            call_id = %call_id,
                                                            device = %device.name,
                                                            "Device has no SIP URI configured"
                                                        );
                                                    }
                                                }
                                                Ok(None) => {
                                                    warn!(call_id = %call_id, device_id = %device_binding.device_id, "Device not found in storage");
                                                }
                                                Err(e) => {
                                                    error!(call_id = %call_id, device_id = %device_binding.device_id, error = %e, "Failed to get device");
                                                }
                                            }
                                        }

                                        // Check if we found any registered contacts
                                        if all_contacts.is_empty() {
                                            info!(call_id = %call_id, phone_number = %destination_aor, "No devices registered for phone number");
                                            let response = self.create_simple_response(
                                                request,
                                                480,
                                                "Temporarily Unavailable",
                                            );
                                            a_leg_handle.send_final(response).await;
                                            let _ = self.call_manager.terminate_call(
                                                &call_id,
                                                Some("No devices registered".to_string()),
                                            );
                                            return Ok(());
                                        }

                                        all_contacts
                                    }
                                    fcp_core::BindingType::Ivr => {
                                        // IVR not yet implemented (Phase 2)
                                        info!(call_id = %call_id, "IVR routing not yet implemented");
                                        let response = self.create_simple_response(
                                            request,
                                            501,
                                            "Not Implemented",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("IVR not implemented".to_string()),
                                        );
                                        return Ok(());
                                    }
                                    _ => {
                                        // Other binding types not yet implemented
                                        info!(call_id = %call_id, binding_type = %line_binding.binding_type, "Binding type not yet supported");
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

                                if let Some(registrar) = &self.registrar {
                                    match registrar.get_registrations(destination_aor).await {
                                        Ok(bindings) if !bindings.is_empty() => bindings,
                                        _ => {
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
                                _ => {
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
                            // Try outbound route provider with failover before giving up
                            if let Some(provider) = &self.outbound_route_provider {
                                let routes = provider.resolve_outbound_ordered(destination_aor).await;
                                if let Some(route) = routes.first() {
                                    info!(call_id = %call_id, target = %route.target_uri, trunk = ?route.trunk_name, allowed_codecs = ?route.allowed_codecs, routes = routes.len(), "Routing via outbound trunk (failover available)");
                                    // Store trunk name and allowed codecs on call for lifecycle tracking
                                    {
                                        let trunk_name = route.trunk_name.clone();
                                        let allowed_codecs = route.allowed_codecs.clone();
                                        let _ = self.call_manager.update_call(&call_id, |call| {
                                            call.trunk_name = trunk_name;
                                            call.outbound_allowed_codecs = allowed_codecs;
                                        });
                                    }
                                    // Build bindings for primary + failover routes
                                    let mut failover_bindings = Vec::with_capacity(routes.len());
                                    for r in &routes {
                                        match sip_registrar::Binding::new(
                                            SmolStr::new(destination_aor),
                                            SmolStr::new(&r.target_uri),
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
                                            request, 500, "Internal Server Error",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id, Some("Failed to create trunk binding".to_string()),
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
                        return self.execute_hunt_group_plan(
                            request,
                            a_leg_handle,
                            call_id,
                            plan,
                            uac,
                        ).await;
                    }
                }
            }

            // Try extension lookup if storage is available
            let mut extension_contacts = Vec::new();
            if let Some(storage) = &self.storage {
                // Get tenant ID from the call (already stored when call was created)
                // TODO: Phase 2 - Extract tenant from INVITE domain/headers/auth
                let tenant_id = self
                    .call_manager
                    .get_call(&call_id)
                    .map(|call| call.tenant_id)
                    .unwrap_or_else(|| fcp_core::TenantId::nil());

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
                                                // Store routing context for CFNA lookup
                                                {
                                                    let lb_id = line_binding.id;
                                                    let callee = destination_aor.to_string();
                                                    let rna_timeout = line_binding.no_answer_timeout;
                                                    let _ = self.call_manager.update_call(&call_id, |call| {
                                                        call.line_binding_id = Some(lb_id);
                                                        call.original_callee = Some(callee);
                                                        call.no_answer_timeout = Some(rna_timeout);
                                                    });
                                                }

                                                // Get devices from line binding
                                                let enabled_devices =
                                                    line_binding.enabled_devices();
                                                if enabled_devices.is_empty() {
                                                    warn!(call_id = %call_id, "Extension target has no enabled devices");
                                                    let response = self.create_simple_response(
                                                        request,
                                                        480,
                                                        "Temporarily Unavailable",
                                                    );
                                                    a_leg_handle.send_final(response).await;
                                                    let _ = self.call_manager.terminate_call(
                                                        &call_id,
                                                        Some("No devices enabled".to_string()),
                                                    );
                                                    return Ok(());
                                                }

                                                // Collect all contact URIs from device registrations
                                                for device_binding in enabled_devices {
                                                    match storage
                                                        .get_device(&device_binding.device_id)
                                                        .await
                                                    {
                                                        Ok(Some(device)) => {
                                                            if let Some(sip_uri) = &device.sip_uri {
                                                                if let Some(registrar) =
                                                                    &self.registrar
                                                                {
                                                                    match registrar
                                                                        .get_registrations(sip_uri)
                                                                        .await
                                                                    {
                                                                        Ok(contacts)
                                                                            if !contacts
                                                                                .is_empty() =>
                                                                        {
                                                                            debug!(
                                                                                call_id = %call_id,
                                                                                extension = %extension_number,
                                                                                device = %device.name,
                                                                                contacts = contacts.len(),
                                                                                "Found registrations for extension target device"
                                                                            );
                                                                            extension_contacts
                                                                                .extend(contacts);
                                                                        }
                                                                        _ => {}
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        _ => {}
                                                    }
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
                                    "Extension routes to user - ring all user's devices"
                                );

                                // Get all devices owned by this user
                                match storage.list_devices_by_user(user_id).await {
                                    Ok(devices) if !devices.is_empty() => {
                                        debug!(
                                            call_id = %call_id,
                                            user_id = %user_id,
                                            device_count = devices.len(),
                                            "Found devices for user"
                                        );

                                        // Get registrations for each device
                                        if let Some(registrar) = &self.registrar {
                                            for device in devices {
                                                if !device.enabled {
                                                    debug!(
                                                        call_id = %call_id,
                                                        device_id = %device.id,
                                                        "Skipping disabled device"
                                                    );
                                                    continue;
                                                }

                                                if let Some(sip_uri) = &device.sip_uri {
                                                    match registrar.get_registrations(sip_uri).await
                                                    {
                                                        Ok(registrations)
                                                            if !registrations.is_empty() =>
                                                        {
                                                            debug!(
                                                                call_id = %call_id,
                                                                device_id = %device.id,
                                                                sip_uri = %sip_uri,
                                                                reg_count = registrations.len(),
                                                                "Found registrations for user device"
                                                            );
                                                            extension_contacts
                                                                .extend(registrations);
                                                        }
                                                        Ok(_) => {
                                                            debug!(
                                                                call_id = %call_id,
                                                                device_id = %device.id,
                                                                sip_uri = %sip_uri,
                                                                "Device not registered"
                                                            );
                                                        }
                                                        Err(e) => {
                                                            warn!(
                                                                call_id = %call_id,
                                                                device_id = %device.id,
                                                                error = %e,
                                                                "Failed to get registrations for device"
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        } else {
                                            warn!(call_id = %call_id, "No registrar available for user device lookup");
                                        }

                                        if extension_contacts.is_empty() {
                                            info!(
                                                call_id = %call_id,
                                                user_id = %user_id,
                                                "User has no registered devices"
                                            );
                                            let response = self.create_simple_response(
                                                request,
                                                480,
                                                "Temporarily Unavailable",
                                            );
                                            a_leg_handle.send_final(response).await;
                                            let _ = self.call_manager.terminate_call(
                                                &call_id,
                                                Some("User has no registered devices".to_string()),
                                            );
                                            return Ok(());
                                        }
                                    }
                                    Ok(_) => {
                                        warn!(call_id = %call_id, user_id = %user_id, "User has no devices");
                                        let response = self.create_simple_response(
                                            request,
                                            480,
                                            "Temporarily Unavailable",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some("User has no devices".to_string()),
                                        );
                                        return Ok(());
                                    }
                                    Err(e) => {
                                        error!(call_id = %call_id, user_id = %user_id, error = %e, "Failed to list user devices");
                                        let response = self.create_simple_response(
                                            request,
                                            500,
                                            "Internal Server Error",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id,
                                            Some(format!("Failed to list user devices: {}", e)),
                                        );
                                        return Ok(());
                                    }
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
                                                match registrar.get_registrations(sip_uri).await {
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
                                                        extension_contacts.extend(registrations);
                                                    }
                                                    Ok(_) => {
                                                        info!(
                                                            call_id = %call_id,
                                                            device_id = %device_id,
                                                            sip_uri = %sip_uri,
                                                            "Device not registered"
                                                        );
                                                        let response = self.create_simple_response(
                                                            request,
                                                            480,
                                                            "Temporarily Unavailable",
                                                        );
                                                        a_leg_handle.send_final(response).await;
                                                        let _ = self.call_manager.terminate_call(
                                                            &call_id,
                                                            Some(
                                                                "Device not registered".to_string(),
                                                            ),
                                                        );
                                                        return Ok(());
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
                                info!(
                                    call_id = %call_id,
                                    service_id = %service_id,
                                    "Extension routes to service (IVR/Queue/Voicemail) - not yet implemented"
                                );
                                // TODO: Phase 2 - Implement service routing
                                // Services include: IVR, call queues, voicemail, conferencing, etc.
                                // This will require integration with fcp-flow for IVR flows,
                                // fcp-queue for ACD queues, and fcp-voicemail for voicemail.
                                let response = self.create_simple_response(
                                    request,
                                    501,
                                    "Service Routing Not Implemented",
                                );
                                a_leg_handle.send_final(response).await;
                                let _ = self.call_manager.terminate_call(
                                    &call_id,
                                    Some(format!(
                                        "Service routing not implemented (service_id: {})",
                                        service_id
                                    )),
                                );
                                return Ok(());
                            }
                        }
                    }
                    Ok(None) => {
                        debug!(call_id = %call_id, extension = %extension_number, "Extension not found, falling back to direct registrar lookup");
                    }
                    Err(e) => {
                        warn!(call_id = %call_id, extension = %extension_number, error = %e, "Failed to lookup extension, falling back to direct registrar lookup");
                    }
                }
            }

            // If we found contacts via extension routing, use them
            if !extension_contacts.is_empty() {
                extension_contacts
            } else {
                // Fall back to direct registrar lookup (existing behavior)
                debug!(call_id = %call_id, destination = %destination_aor, "No extension found, using direct registrar lookup");

                if let Some(registrar) = &self.registrar {
                    match registrar.get_registrations(destination_aor).await {
                        Ok(bindings) if !bindings.is_empty() => {
                            debug!(
                                call_id = %call_id,
                                destination = %destination_aor,
                                contacts = bindings.len(),
                                "Found registered contacts for destination"
                            );
                            bindings
                        }
                        Ok(_) => {
                            // Try outbound route provider with failover before giving up
                            if let Some(provider) = &self.outbound_route_provider {
                                let routes = provider.resolve_outbound_ordered(destination_aor).await;
                                if let Some(route) = routes.first() {
                                    info!(call_id = %call_id, target = %route.target_uri, trunk = ?route.trunk_name, allowed_codecs = ?route.allowed_codecs, routes = routes.len(), "Routing via outbound trunk (extension path, failover available)");
                                    // Store trunk name and allowed codecs on call for lifecycle tracking
                                    {
                                        let trunk_name = route.trunk_name.clone();
                                        let allowed_codecs = route.allowed_codecs.clone();
                                        let _ = self.call_manager.update_call(&call_id, |call| {
                                            call.trunk_name = trunk_name;
                                            call.outbound_allowed_codecs = allowed_codecs;
                                        });
                                    }
                                    // Build bindings for primary + failover routes
                                    let mut failover_bindings = Vec::with_capacity(routes.len());
                                    for r in &routes {
                                        match sip_registrar::Binding::new(
                                            SmolStr::new(destination_aor),
                                            SmolStr::new(&r.target_uri),
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
                                            request, 500, "Internal Server Error",
                                        );
                                        a_leg_handle.send_final(response).await;
                                        let _ = self.call_manager.terminate_call(
                                            &call_id, Some("Failed to create trunk binding".to_string()),
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
            .map(|contact| {
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
                TargetContact {
                    uri,
                    flow_token: contact.flow_token().map(|s| s.to_string()),
                }
            })
            .collect();

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
        let mut a_leg_sdp = if !request.body().is_empty() {
            Some(String::from_utf8_lossy(request.body()).to_string())
        } else {
            None
        };

        // Rewrite SDP if media is anchored on server
        if let Some(ref original_sdp) = a_leg_sdp {
            // Check if this call has anchored media
            let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                if call.media_anchored {
                    call.media_session_id.clone()
                } else {
                    None
                }
            });

            if let (Some(media_session_id), Some(media_manager)) =
                (media_session_id, &self.media_manager)
            {
                // Get media session to extract RTP port
                match media_manager.get_session(&media_session_id).await {
                    Ok(session) => {
                        let (rtp_port, _rtcp_port) = session.ports();

                        // Rewrite SDP to point to server's RTP endpoint
                        let rewritten_sdp = fcp_media::rewrite_sdp_for_anchoring(
                            original_sdp,
                            &self.local_addr,
                            rtp_port,
                            None,
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
                let filtered = fcp_media::sdp::filter_sdp_codecs(sdp, &allowed_refs);
                a_leg_sdp = Some(filtered);
            }
        }

        // Extract caller's identity from A-leg for B2BUA caller ID preservation
        let caller_display_name = extract_from_display_name(request);
        let caller_from_uri = extract_from_uri(request).ok().map(normalize_from_uri);

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
        let cancel_notify = Arc::new(tokio::sync::Notify::new());
        let cancel_notify_clone = Arc::clone(&cancel_notify);
        match self.call_manager.update_call(&call_id, |call| {
            call.cancel_notify = Some(cancel_notify_clone);
        }) {
            Ok(_) => {
                debug!(
                    call_id = %call_id,
                    "Stored cancel_notify in call before B-leg INVITEs"
                );
            }
            Err(e) => {
                error!(
                    call_id = %call_id,
                    error = %e,
                    "Failed to store cancel_notify - CANCELs may not be delivered"
                );
            }
        }

        // Create outbound INVITEs to all target contacts in PARALLEL
        // This prevents TLS connection timeouts from blocking other B-legs
        // For TLS contacts with flow tokens (RFC 5626), use the existing connection
        let mut invite_futures = Vec::new();
        for target_contact in &target_contacts {
            let uac_clone = Arc::clone(&uac);
            let target_uri_clone = target_contact.uri.clone();
            let a_leg_sdp_clone = a_leg_sdp.clone();
            let call_id_clone = call_id.clone();
            let flow_token_clone = target_contact.flow_token.clone();
            let flow_registry_clone = Arc::clone(&self.flow_registry);
            let sip_call_id_map_clone = Arc::clone(&self.sip_call_id_map);
            let sip_trace_tx_clone = self.sip_trace_tx.clone();
            let local_addr_clone = self.local_addr.clone();
            let node_id_clone = self.node_id.clone();

            let future = async move {
                debug!(
                    call_id = %call_id_clone,
                    target = %target_uri_clone,
                    has_sdp = a_leg_sdp_clone.is_some(),
                    has_flow_token = flow_token_clone.is_some(),
                    "Creating B-leg INVITE"
                );

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
                        tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            uac_clone.invite_via_flow(
                                &target_uri_clone,
                                a_leg_sdp_clone.as_deref(),
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
                            uac_clone.invite(&target_uri_clone, a_leg_sdp_clone.as_deref()),
                        )
                        .await
                    }
                } else {
                    // Normal INVITE (UDP/TCP without flow)
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        uac_clone.invite(&target_uri_clone, a_leg_sdp_clone.as_deref()),
                    )
                    .await
                };

                match invite_result {
                    Ok(Ok(call)) => {
                        // Register B-leg SIP Call-ID → FCP CallId mapping for trace correlation
                        if let Some(b_leg_cid) = sip_parse::header(
                            call.invite_request().headers(),
                            "Call-ID",
                        ) {
                            sip_call_id_map_clone.insert(
                                b_leg_cid.as_str().to_string(),
                                call_id_clone.to_string(),
                            );
                        }

                        // Log outbound B-leg INVITE for SIP trace
                        if let Some(ref trace_tx) = sip_trace_tx_clone {
                            let b_req = call.invite_request();
                            let raw_bytes = sip_parse::serialize_request(b_req);
                            let raw_str = String::from_utf8_lossy(&raw_bytes);
                            let from_uri = sip_parse::header(b_req.headers(), "From")
                                .map(|s| s.as_str().to_string()).unwrap_or_default();
                            let to_uri = sip_parse::header(b_req.headers(), "To")
                                .map(|s| s.as_str().to_string()).unwrap_or_default();
                            let sip_cid = sip_parse::header(b_req.headers(), "Call-ID")
                                .map(|s| s.as_str().to_string()).unwrap_or_default();
                            let cseq = sip_parse::header(b_req.headers(), "CSeq")
                                .map(|s| s.as_str().to_string()).unwrap_or_default();
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
                                transport: if flow_token_clone.is_some() { "tcp".to_string() } else { "udp".to_string() },
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
            };
            invite_futures.push(future);
        }

        // Wait for all INVITEs to complete (concurrently)
        // Each result includes (target_uri, call_handle, flow_token) for RFC 5626 BYE support
        let results = futures::future::join_all(invite_futures).await;
        let b_leg_calls: Vec<_> = results.into_iter().flatten().collect();

        // Clear caller identity overrides after INVITEs sent
        uac.inner().set_display_name(None).await;
        uac.inner().set_from_uri(None).await;

        // Check if we successfully created any B-legs
        if b_leg_calls.is_empty() {
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
        let call_id_clone = call_id.clone();

        tokio::spawn(async move {
            if let Err(e) = handler
                .handle_parallel_forking(
                    request_clone,
                    a_leg_handle_arc,
                    b_leg_calls,
                    call_id_clone,
                    cancel_notify,
                )
                .await
            {
                error!(call_id = %call_id_clone, error = %e, "Error handling parallel forking");
            }
        });

        // Return immediately - responses will be handled in background task
        info!(call_id = %call_id, "Parallel forking in progress, handling responses in background");
        return Ok(());
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
            if status_code >= 180 && status_code < 200 {
                info!(
                    call_id = %call_id,
                    code = status_code,
                    has_rseq = provisional.headers().get("RSeq").is_some(),
                    "Relaying provisional response to A-leg"
                );

                if provisional.headers().get("RSeq").is_some() {
                    let _ = self.call_manager.update_call(&call_id, |call| {
                        if let Some(b) = &mut call.b_leg {
                            b.reliable_provisionals.push(provisional.clone());
                        }
                        call.a_leg.reliable_provisionals.push(provisional.clone());
                    });
                }

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
                            SmolStr::new(&final_body.len().to_string()),
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
                        .terminate_call(&call_id, Some("Cancelled".to_string()));
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
                        .terminate_call(&call_id, Some(format!("B-leg timeout: {}", e)));
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
                .terminate_call(&call_id, Some("Cancelled".to_string()));
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

            // Set B-leg remote endpoint on the media session (before SDP rewriting)
            if let Some(ref original_sdp) = b_leg_sdp {
                let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                    if call.media_anchored {
                        call.media_session_id.clone()
                    } else {
                        None
                    }
                });

                if let (Some(ref media_session_id), Some(media_manager)) =
                    (&media_session_id, &self.media_manager)
                {
                    if let Err(e) = media_manager
                        .update_session_b_leg(media_session_id, original_sdp.clone())
                        .await
                    {
                        warn!(
                            call_id = %call_id,
                            error = %e,
                            "Failed to set B-leg media endpoint"
                        );
                    }
                }
            }

            // Rewrite B-leg SDP if media is anchored on server
            if let Some(ref original_sdp) = b_leg_sdp {
                // Check if this call has anchored media
                let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                    if call.media_anchored {
                        call.media_session_id.clone()
                    } else {
                        None
                    }
                });

                if let (Some(media_session_id), Some(media_manager)) =
                    (media_session_id, &self.media_manager)
                {
                    // Get media session to extract RTP port
                    match media_manager.get_session(&media_session_id).await {
                        Ok(session) => {
                            let (rtp_port, _rtcp_port) = session.ports();

                            // Construct answer for A-leg using A-leg's preferred codec
                            // This is critical for transcoding: A-leg sees its preferred codec,
                            // B-leg uses whatever it answered with, and the transcoder handles
                            // conversion in the middle transparently
                            let rewritten_sdp = if let Some(original_invite) = self
                                .call_manager
                                .get_call(&call_id)
                                .and_then(|c| c.original_invite.clone())
                            {
                                if !original_invite.body().is_empty() {
                                    let a_leg_offer =
                                        String::from_utf8_lossy(original_invite.body());
                                    let a_leg_codecs =
                                        fcp_media::sdp::parse_sdp_codecs(&a_leg_offer);
                                    if let Some(preferred) = a_leg_codecs.first() {
                                        // Construct answer using A-leg's preferred codec
                                        let constructed_sdp =
                                            fcp_media::construct_answer_for_a_leg(
                                                &a_leg_offer,
                                                &self.local_addr,
                                                rtp_port,
                                                None,
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
                                            None,
                                        )
                                    }
                                } else {
                                    // Fallback: just rewrite the SDP
                                    fcp_media::rewrite_sdp_for_anchoring(
                                        original_sdp,
                                        &self.local_addr,
                                        rtp_port,
                                        None,
                                    )
                                }
                            } else {
                                // Fallback: just rewrite the SDP
                                fcp_media::rewrite_sdp_for_anchoring(
                                    original_sdp,
                                    &self.local_addr,
                                    rtp_port,
                                    None,
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
            inject_session_timer_headers(
                &self.b2bua_config,
                &mut a_leg_response,
            );

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

            let _ = self.call_manager.terminate_call(
                &call_id,
                Some(format!("B-leg rejected: {} {}", b_leg_status, reason)),
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
            let b_leg_handle_clone = Arc::clone(b_leg_handle);
            let target_clone = target.clone();
            let call_id_clone = call_id.clone();
            let provisional_tx_clone = provisional_tx.clone();
            let provisional_sent_clone = Arc::clone(&provisional_sent);
            let task_early_media_policy = early_media_policy;
            let task_dedup = dedup_provisionals;

            // Spawn provisional listener task (loops until first 180+ response)
            tokio::spawn(async move {
                // If early media is suppressed, skip provisional relay entirely
                if task_early_media_policy == fcp_b2bua_core::EarlyMediaPolicy::None {
                    debug!(
                        call_id = %call_id_clone,
                        target = %target_clone,
                        "Early media suppressed by policy — skipping provisional relay for B-leg {}", i
                    );
                    return;
                }

                // Loop to consume provisionals until we get a 180+ response
                while let Some(response) = b_leg_handle_clone.await_provisional().await {
                    let code = response.start_line().code();
                    debug!(
                        call_id = %call_id_clone,
                        target = %target_clone,
                        code = code,
                        has_body = !response.body().is_empty(),
                        "B-leg {} provisional response", i
                    );

                    // Signal if it's a ringing/progress response (skip 100 Trying etc.)
                    if code >= 180 && code < 200 {
                        let mut sent = provisional_sent_clone.lock().await;
                        if !*sent || !task_dedup {
                            info!(
                                call_id = %call_id_clone,
                                target = %target_clone,
                                code = code,
                                has_sdp = !response.body().is_empty(),
                                "First provisional from B-leg {} - sending full response to relay task", i
                            );
                            // Send the ENTIRE response object (preserves SDP, Require: 100rel, etc.)
                            let _ = provisional_tx_clone.send(response).await;
                            *sent = true;
                        }
                        break;
                    }
                    // Continue looping past 100 Trying and other non-ringing provisionals
                }
            });
        }

        // Drop original sender so channel closes when all provisional tasks complete
        drop(provisional_tx);

        // Spawn relay task to forward the actual B-leg provisional to A-leg
        // CRITICAL: This fixes two issues:
        // 1. Early media/183 with SDP is now forwarded (not dropped)
        // 2. To-tag is extracted and saved for reuse in 200 OK
        let call_id_clone = call_id.clone();
        let a_leg_handle_clone = Arc::clone(&a_leg_handle);
        let request_clone = request.clone();
        let call_manager_clone = self.call_manager.clone();
        let handler_clone = self.clone();
        tokio::spawn(async move {
            if let Some(b_leg_provisional) = provisional_rx.recv().await {
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
        let mut tasks = Vec::new();
        for (i, (target, b_leg_handle, flow_token)) in b_leg_calls.iter().enumerate() {
            let b_leg_handle_clone = Arc::clone(b_leg_handle);
            let target_clone = target.clone();
            let call_id_clone = call_id.clone();
            let flow_token_clone = flow_token.clone();

            let task = tokio::spawn(async move {
                debug!(
                    call_id = %call_id_clone,
                    target = %target_clone,
                    "B-leg {} race task waiting for final response", i
                );

                // Wait for final response (this is what we race on)
                match b_leg_handle_clone.await_final().await {
                    Ok(response) => {
                        let code = response.start_line().code();
                        debug!(
                            call_id = %call_id_clone,
                            target = %target_clone,
                            code = code,
                            "B-leg {} final response", i
                        );
                        Some((
                            i,
                            target_clone,
                            code,
                            response,
                            Arc::clone(&b_leg_handle_clone),
                            flow_token_clone,
                        ))
                    }
                    Err(e) => {
                        warn!(
                            call_id = %call_id_clone,
                            target = %target_clone,
                            error = %e,
                            "B-leg {} failed", i
                        );
                        None
                    }
                }
            });
            tasks.push(task);
        }

        // Race all tasks and take the first successful 2xx response.
        // Also listen for A-leg cancellation so we can abort immediately.
        let mut winning_index = None;
        let mut winning_target = None;
        let mut winning_response = None;
        let mut winning_handle = None;
        let mut winning_flow_token: Option<String> = None;
        let mut cancelled_by_caller = false;
        let mut last_failure_code: Option<u16> = None;

        debug!(call_id = %call_id, task_count = tasks.len(), "Starting race for first 2xx response");

        // Wrap the B-leg race into a single future so we can select against cancel
        let race_future = async {
            let mut race_failure_code: Option<u16> = None;
            while !tasks.is_empty() {
                let (result, _task_index, remaining) = futures::future::select_all(tasks).await;
                tasks = remaining;

                match result {
                    Ok(Some((i, target, code, response, handle, flow_token))) => {
                        debug!(
                            call_id = %call_id,
                            b_leg_index = i,
                            target = %target,
                            code = code,
                            "B-leg {} completed with code {}", i, code
                        );

                        if code >= 200 && code < 300 {
                            info!(
                                call_id = %call_id,
                                winner = %target,
                                code = code,
                                has_flow_token = flow_token.is_some(),
                                "B-leg {} won the race with {}",
                                i, code
                            );
                            return (Some((i, target, response, handle, flow_token)), race_failure_code);
                        } else {
                            debug!(
                                call_id = %call_id,
                                code = code,
                                "B-leg {} returned non-2xx {}, continuing race", i, code
                            );
                            race_failure_code = Some(code);
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

        let per_call_timeout = self.call_manager
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
            }
        }

        debug!(
            call_id = %call_id,
            has_winner = winning_index.is_some(),
            cancelled = cancelled_by_caller,
            "Race complete"
        );

        // CANCEL all non-winning B-legs
        for (i, (target, b_leg_handle, _flow_token)) in b_leg_calls.iter().enumerate() {
            if Some(i) != winning_index {
                info!(
                    call_id = %call_id,
                    target = %target,
                    "CANCELing non-winning B-leg {}", i
                );
                if let Err(e) = b_leg_handle.cancel().await {
                    warn!(
                        call_id = %call_id,
                        target = %target,
                        error = %e,
                        "Failed to CANCEL B-leg {}", i
                    );
                }
            }
        }

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
            let _ = self.call_manager.update_call(&call_id, |call| {
                if let Some(b_leg) = &mut call.b_leg {
                    b_leg.outbound_handle = Some(Arc::clone(&handle));
                    b_leg.flow_token = winning_flow_token_clone;
                }
                call.state = fcp_b2bua_core::CallState::Established;
            });

            // Extract SDP from winning B-leg response
            let mut b_leg_sdp = if !response.body().is_empty() {
                Some(String::from_utf8_lossy(response.body()).to_string())
            } else {
                None
            };

            // Set B-leg remote endpoint on the media session (before SDP rewriting)
            if let Some(ref original_sdp) = b_leg_sdp {
                let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                    if call.media_anchored {
                        call.media_session_id.clone()
                    } else {
                        None
                    }
                });

                if let (Some(ref media_session_id), Some(media_manager)) =
                    (&media_session_id, &self.media_manager)
                {
                    if let Err(e) = media_manager
                        .update_session_b_leg(media_session_id, original_sdp.clone())
                        .await
                    {
                        warn!(
                            call_id = %call_id,
                            error = %e,
                            "Failed to set B-leg media endpoint in parallel fork"
                        );
                    }
                }
            }

            // Rewrite B-leg SDP if media is anchored on server (for A-leg response)
            if let Some(ref original_sdp) = b_leg_sdp {
                // Check if this call has anchored media
                let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                    if call.media_anchored {
                        call.media_session_id.clone()
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
                            let (rtp_port, _rtcp_port) = session.ports();

                            // Construct answer for A-leg using A-leg's preferred codec
                            // This is critical for transcoding: A-leg sees its preferred codec,
                            // B-leg uses whatever it answered with, and the transcoder handles
                            // conversion in the middle transparently
                            let rewritten_sdp = if let Some(original_invite) = self
                                .call_manager
                                .get_call(&call_id)
                                .and_then(|c| c.original_invite.clone())
                            {
                                if !original_invite.body().is_empty() {
                                    let a_leg_offer =
                                        String::from_utf8_lossy(original_invite.body());
                                    let a_leg_codecs =
                                        fcp_media::sdp::parse_sdp_codecs(&a_leg_offer);
                                    if let Some(preferred) = a_leg_codecs.first() {
                                        // Construct answer using A-leg's preferred codec
                                        let constructed_sdp =
                                            fcp_media::construct_answer_for_a_leg(
                                                &a_leg_offer,
                                                &self.local_addr,
                                                rtp_port,
                                                None,
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
                                            None,
                                        )
                                    }
                                } else {
                                    // Fallback: just rewrite the SDP
                                    fcp_media::rewrite_sdp_for_anchoring(
                                        original_sdp,
                                        &self.local_addr,
                                        rtp_port,
                                        None,
                                    )
                                }
                            } else {
                                // Fallback: just rewrite the SDP
                                fcp_media::rewrite_sdp_for_anchoring(
                                    original_sdp,
                                    &self.local_addr,
                                    rtp_port,
                                    None,
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
                // Fallback: No provisional was sent (shouldn't happen in forking)
                warn!(
                    call_id = %call_id,
                    "No A-leg To-tag saved in forking path, generating new one (dialog mismatch possible)"
                );
                self.uas
                    .create_ok(&request, b_leg_sdp.as_deref())
                    .expect("Failed to create 200 OK")
            };

            self.ensure_response_hygiene(&request, &mut ok_response);

            // Inject RFC 4028 session timer headers into forked 200 OK
            inject_session_timer_headers(
                &self.b2bua_config,
                &mut ok_response,
            );

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
        } else if cancelled_by_caller {
            // Caller already cancelled — handle_cancel sent 487 and terminated the call.
            // Just clean up; don't send another response to A-leg.
            info!(call_id = %call_id, "Forking task exiting after caller cancellation");
        } else {
            // All B-legs failed (not due to cancellation) — check CFNA
            let call = self.call_manager.get_call(&call_id);
            let cfna_dest = if let Some(ref call) = call {
                if let Some(ref lb_id) = call.line_binding_id {
                    self.check_cfna_destination(lb_id).await
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(dest) = cfna_dest {
                let reason = match last_failure_code {
                    Some(486) | Some(600) | Some(603) => Some("busy"),
                    _ => Some("no-answer"),
                };
                info!(call_id = %call_id, destination = %dest, reason = ?reason, "CFNA: routing to forward destination");
                let callee = call.as_ref().and_then(|c| c.original_callee.as_deref());
                match self
                    .route_cfna(&request, &a_leg_handle, call_id, &dest, callee, reason)
                    .await
                {
                    Ok(()) => {} // route_cfna handles forking + response
                    Err(e) => {
                        error!(call_id = %call_id, error = %e, "CFNA routing failed, sending 480");
                        let resp = self.create_simple_response(
                            &request,
                            480,
                            "Temporarily Unavailable",
                        );
                        a_leg_handle.send_final(resp).await;
                        let _ = self.call_manager.terminate_call(
                            &call_id,
                            Some(format!("CFNA failed: {}", e)),
                        );
                    }
                }
            } else {
                // No CFNA configured — original behavior
                warn!(call_id = %call_id, "All B-legs failed, returning 480");
                let response =
                    self.create_simple_response(&request, 480, "Temporarily Unavailable");
                a_leg_handle.send_final(response).await;
                let _ = self
                    .call_manager
                    .terminate_call(&call_id, Some("All B-legs failed".to_string()));
            }
        }

        Ok(())
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
                inject_session_timer_headers(
                    &self.b2bua_config,
                    &mut response,
                );

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
    async fn evaluate_routing_policy(
        &self,
        request: &sip_core::Request,
        routing_service: &Arc<fcp_routing::RoutingService>,
        peer: SocketAddr,
    ) -> Result<fcp_routing::PolicyDecision> {
        use anyhow::Context as _;

        // Extract From header (caller)
        let from = sip_parse::header(request.headers(), "From")
            .ok_or_else(|| anyhow::anyhow!("Missing From header"))?;

        // Extract caller URI from From header (strip display name and parameters)
        let caller = from
            .split('<')
            .nth(1)
            .and_then(|s| s.split('>').next())
            .unwrap_or(&from)
            .trim()
            .to_string();

        // Extract To header (destination)
        let to = sip_parse::header(request.headers(), "To")
            .ok_or_else(|| anyhow::anyhow!("Missing To header"))?;

        // Extract destination URI from To header
        let destination = to
            .split('<')
            .nth(1)
            .and_then(|s| s.split('>').next())
            .unwrap_or(&to)
            .trim()
            .to_string();

        // Get source IP from peer address
        let source_ip = peer.ip();

        // Look up the caller to get their tenant ID
        // This ensures we use the correct tenant's policies
        let tenant_id = routing_service
            .get_user_tenant(&caller)
            .await
            .with_context(|| format!("Failed to determine tenant for caller: {}", caller))?;

        debug!(
            caller = %caller,
            destination = %destination,
            tenant_id = %tenant_id,
            source_ip = %source_ip,
            "Evaluating routing policy"
        );

        // Evaluate the call using routing service
        routing_service
            .evaluate_call(&caller, &destination, &tenant_id, source_ip)
            .await
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
        let username = user_opt.as_deref().unwrap_or("fcp");
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

        // Find the call by dialog Call-ID
        if let Some(call) = self
            .call_manager
            .find_by_dialog_call_id(dialog_call_id.as_str())
        {
            info!(
                call_id = %call.id,
                dialog_call_id = %dialog_call_id,
                state = ?call.state,
                "ACK received from A-leg - call dialog fully established"
            );

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
        } else {
            // B-leg BYE: create simple 200 OK response
            debug!("BYE dialog not in dialog_manager (likely B-leg UAC), creating simple 200 OK");
            self.create_simple_response(request, 200, "OK")
        };

        // Now that we've responded, proceed with forwarding and cleanup
        // Find the call by dialog Call-ID
        if let Some(call) = self
            .call_manager
            .find_by_dialog_call_id(dialog_call_id.as_str())
        {
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
                            "BYE from B-leg, forwarding to A-leg"
                        );

                        self.spawn_bye_with_retry(
                            Arc::clone(uac),
                            a_leg_dialog.clone(),
                            call.id,
                            true,
                            None, // A-leg doesn't use flow tokens
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

            // Check if both legs are now done (no more dialogs)
            // If so, terminate the call and clean up
            if let Some(updated_call) = self.call_manager.get_call(&call.id) {
                let a_done = updated_call.a_leg.dialog.is_none();
                let b_done = updated_call
                    .b_leg
                    .as_ref()
                    .map(|b| b.dialog.is_none())
                    .unwrap_or(true);

                if a_done && b_done {
                    info!(call_id = %call.id, "Both legs terminated, cleaning up call");

                    // Terminate media session if anchored
                    if call.media_anchored {
                        if let (Some(media_session_id), Some(media_manager)) =
                            (&call.media_session_id, &self.media_manager)
                        {
                            let media_session_id = media_session_id.clone();
                            let media_manager = Arc::clone(media_manager);
                            let call_id = call.id;

                            // Spawn async task to terminate media session
                            tokio::spawn(async move {
                                match media_manager.terminate_session(&media_session_id).await {
                                    Ok(()) => {
                                        info!(
                                            call_id = %call_id,
                                            media_session_id = %media_session_id,
                                            "Terminated media session"
                                        );
                                    }
                                    Err(e) => {
                                        error!(
                                            call_id = %call_id,
                                            media_session_id = %media_session_id,
                                            error = %e,
                                            "Failed to terminate media session"
                                        );
                                    }
                                }
                            });
                        }
                    }

                    // Cleanup per-dialog state for remaining dialogs to prevent memory leaks
                    if let Some(a_dialog) = &updated_call.a_leg.dialog {
                        self.rate_limits.remove(&a_dialog.id());
                        self.mid_dialog_lock.remove(&a_dialog.id());
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
                            self.rate_limits.remove(&b_dialog.id());
                            self.mid_dialog_lock.remove(&b_dialog.id());
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

                    // Finally, terminate the call in CallManager
                    let _ = self.call_manager.terminate_call(&call.id, None);
                } else {
                    debug!(
                        call_id = %call.id,
                        a_done = a_done,
                        b_done = b_done,
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
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id()).await;
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
            src_dialog_id.as_ref().unwrap_or(&target_dialog.id()),
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

        // If media is anchored, rewrite SDP before forwarding to peer
        if call.media_anchored {
            if let (Some(ref body_str), Some(ref media_session_id), Some(ref media_manager)) = (
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
                        let (rtp_port, _rtcp_port) = session.ports();
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
            Some(&target_dialog.id()),
            cseq.as_ref().map(|v| v.as_str()),
            None,
            None,
            None,
            None,
            None,
        );
        let peer_resp = match uac.send_reinvite(&mut target_dialog, body.as_deref()).await {
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
                    &target_dialog.id(),
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
                &target_dialog.id(),
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
            if let (Some(ref media_session_id), Some(ref media_manager)) =
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
                            let (rtp_port, _rtcp_port) = session.ports();

                            // Construct answer for A-leg using A-leg's preferred codec
                            // This is critical for transcoding: A-leg sees its preferred codec,
                            // B-leg uses whatever it answered with, and the transcoder handles
                            // conversion in the middle transparently
                            let rewritten_sdp = if let Some(original_invite) =
                                call.original_invite.as_ref()
                            {
                                if !original_invite.body().is_empty() {
                                    let a_leg_offer =
                                        String::from_utf8_lossy(original_invite.body());
                                    let a_leg_codecs =
                                        fcp_media::sdp::parse_sdp_codecs(&a_leg_offer);
                                    if let Some(preferred) = a_leg_codecs.first() {
                                        // Construct answer using A-leg's preferred codec
                                        let constructed_sdp =
                                            fcp_media::construct_answer_for_a_leg(
                                                &a_leg_offer,
                                                &self.local_addr,
                                                rtp_port,
                                                None,
                                            );
                                        info!(
                                            call_id = %call.id,
                                            a_leg_codec = %preferred.encoding_name,
                                            "Constructed A-leg re-INVITE answer with preferred codec"
                                        );
                                        constructed_sdp
                                    } else {
                                        // Fallback: just rewrite the SDP
                                        fcp_media::rewrite_sdp_for_anchoring(
                                            &response_sdp,
                                            &self.local_addr,
                                            rtp_port,
                                            None,
                                        )
                                    }
                                } else {
                                    // Fallback: just rewrite the SDP
                                    fcp_media::rewrite_sdp_for_anchoring(
                                        &response_sdp,
                                        &self.local_addr,
                                        rtp_port,
                                        None,
                                    )
                                }
                            } else {
                                // Fallback: just rewrite the SDP
                                fcp_media::rewrite_sdp_for_anchoring(
                                    &response_sdp,
                                    &self.local_addr,
                                    rtp_port,
                                    None,
                                )
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

    /// Handle in-dialog SUBSCRIBE (e.g., REFER implicit subscription) by forwarding.
    async fn handle_subscribe(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling SUBSCRIBE");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id()).await;
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
            src_dialog_id.as_ref().unwrap_or(&target_dialog.id()),
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
            Some(&target_dialog.id()),
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
                    &target_dialog.id(),
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
                &target_dialog.id(),
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
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id()).await;
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
        if let Some(sub) = self.find_subscription(&target_dialog.id(), event.as_str()) {
            let state = sip_dialog::SubscriptionState::Active;
            self.log_sip_message(
                "outbound",
                "NOTIFY",
                Some(call.id.to_string().as_str()),
                Some(&target_dialog.id()),
                cseq.as_ref().map(|v| v.as_str()),
                None,
                None,
                None,
                body.as_ref().map(|b| b.as_bytes()),
                None,
            );
            if self.check_rate_limit(
                "NOTIFY",
                src_dialog_id.as_ref().unwrap_or(&target_dialog.id()),
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
                        &target_dialog.id(),
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
                    &target_dialog.id(),
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
                Some(&target_dialog.id()),
                cseq.as_ref().map(|v| v.as_str()),
                None,
                None,
                None,
                body.as_ref().map(|b| b.as_bytes()),
                None,
            );
            if self.check_rate_limit(
                "NOTIFY",
                src_dialog_id.as_ref().unwrap_or(&target_dialog.id()),
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
                        &target_dialog.id(),
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
                    &target_dialog.id(),
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
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id()).await;
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
            src_dialog_id.as_ref().unwrap_or(&target_dialog.id()),
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
            Some(&target_dialog.id()),
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
                    &target_dialog.id(),
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
                &target_dialog.id(),
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

    /// Handle in-dialog PRACK by forwarding to the opposite leg.
    async fn handle_prack(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling PRACK");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id()).await;
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
            method = "PRACK",
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
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for PRACK forwarding"))?;

        // Need the reliable provisional we forwarded earlier to build PRACK
        let reliable = self
            .get_opposite_reliable_provisional(&call, is_a_leg)
            .ok_or_else(|| anyhow::anyhow!("No reliable provisional to PRACK"))?;

        if self.check_rate_limit(
            "PRACK",
            src_dialog_id.as_ref().unwrap_or(&target_dialog.id()),
            self.settings.update_rate_limit,
        ) {
            warn!(call_id = %call.id, "PRACK rate limited");
            let mut resp = self.create_simple_response(request, 429, "Too Many Requests");
            let _ = resp.headers_mut().push(
                SmolStr::new("Retry-After"),
                SmolStr::new(self.settings.rate_window.as_secs().to_string()),
            );
            return Ok(resp);
        }
        self.log_sip_message(
            "outbound",
            "PRACK",
            Some(call.id.to_string().as_str()),
            Some(&target_dialog.id()),
            cseq.as_ref().map(|v| v.as_str()),
            None,
            None,
            None,
            None,
            None,
        );
        let peer_resp = match uac.send_prack(&mut target_dialog, &reliable).await {
            Ok(r) => {
                info!(status = r.start_line().code(), "Forwarded PRACK to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "PRACK"), ("result", "ok")],
                );
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "PRACK"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward PRACK to peer");
                self.metrics.inc_counter(
                    "fcp_sip_mid_dialog_forward_total",
                    &[("method", "PRACK"), ("result", "error")],
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
                    &target_dialog.id(),
                    "PRACK forward failed",
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
                &target_dialog.id(),
                "Peer returned 481 to PRACK",
            );
            self.metrics.inc_counter(
                "fcp_sip_mid_dialog_forward_total",
                &[("method", "PRACK"), ("result", "481")],
            );
        }
        self.persist_updated_dialog(&call.id, !is_a_leg, target_dialog);

        Ok(self.build_in_dialog_response(request, &peer_resp))
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
            src_dialog_id.as_ref().unwrap_or(&target_dialog.id()),
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
            Some(&target_dialog.id()),
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
                    &target_dialog.id(),
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
                &target_dialog.id(),
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

        let refer_uri = sip_core::SipUri::parse(refer_uri_str)
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
            src_dialog_id.as_ref().unwrap_or(&target_dialog.id()),
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
            Some(&target_dialog.id()),
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
                    &target_dialog.id(),
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
                &target_dialog.id(),
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
        self.handle_register_internal(request, sip_transaction::TransportKind::Udp, None, default_peer)
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

        match &self.registrar {
            Some(registrar) => {
                // For TLS connections with a stream, register a flow token (RFC 5626)
                let flow_token = if transport == sip_transaction::TransportKind::Tls {
                    if let Some(ref stream_sender) = stream {
                        let token = self.flow_registry.register(stream_sender.clone(), peer_addr);
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

                // Use registrar to handle the request with flow token
                let response = registrar
                    .handle_register_with_flow(request, flow_token.as_deref())
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
    fn create_simple_response(
        &self,
        request: &sip_core::Request,
        code: u16,
        reason: &str,
    ) -> sip_core::Response {
        use sip_core::{Headers, StatusLine};

        let mut headers = Headers::new();

        // Copy mandatory headers from request (RFC 3261 §8.2.6)
        if let Some(via) = request.headers().get("Via") {
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
            Some(&resp.headers()),
            Some(&resp.body()),
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
        let dialog_manager = Arc::clone(&self.dialog_manager);
        let call_manager = self.call_manager.clone();
        let stm = Arc::clone(&self.session_timer_manager);
        let subscription_index = Arc::clone(&self.subscription_index);
        let flow_registry = Arc::clone(&self.flow_registry);
        let sip_trace_tx = self.sip_trace_tx.clone();
        let local_addr = self.local_addr.clone();
        let node_id = self.node_id.clone();
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
                        uac.bye_via_flow(&dialog, flow_info.stream, flow_info.peer_addr).await
                    } else {
                        tracing::warn!(
                            call_id = %call_id,
                            flow_token = %token,
                            "Flow token not found or connection closed, falling back to normal BYE"
                        );
                        uac.bye(&dialog).await
                    }
                } else {
                    uac.bye(&dialog).await
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
                            let branch = format!("z9hG4bK{:x}", std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());

                            // Build Route headers from dialog route set
                            let route_headers: String = dialog.route_set().iter()
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
                                dest, transport_str, local_addr, branch,
                                route_headers,
                                sip_cid,
                                from_uri, from_tag,
                                to_uri, to_tag,
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
                                    .map(|s| s.as_str().to_string()).unwrap_or_default(),
                                from_uri: sip_parse::header(bye_response.headers(), "From")
                                    .map(|s| s.as_str().to_string()).unwrap_or_default(),
                                to_uri: sip_parse::header(bye_response.headers(), "To")
                                    .map(|s| s.as_str().to_string()).unwrap_or_default(),
                                source_addr: dest,
                                dest_addr: local_addr.clone(),
                                transport: if flow_token.is_some() { "tcp".to_string() } else { "udp".to_string() },
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
                                // Clean subscriptions for B-leg dialog
                                b.reliable_provisionals.clear();
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

        // Mandatory dialog-forming headers from A-leg INVITE
        if let Some(via) = a_leg_invite.headers().get("Via") {
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

        // Copy Contact from B-leg provisional (remote target)
        if let Some(contact) = b_leg_resp.headers().get("Contact") {
            let _ = headers.push(SmolStr::new("Contact"), contact);
        }

        // Propagate Record-Route for correct route set
        for rr in b_leg_resp.headers().get_all("Record-Route") {
            let _ = headers.push(SmolStr::new("Record-Route"), rr);
        }

        // Propagate capability/signaling headers
        for name in [
            "Require",
            "Supported",
            "Allow",
            "RSeq",
            "Session-Expires",
            "Content-Type",
        ] {
            for val in b_leg_resp.headers().get_all(name) {
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
            Some(&response.body()),
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

        self.call_manager
            .find_by_dialog_call_id(call_id.as_str())
            .is_some()
    }

    /// Find call and which leg the request belongs to.
    fn lookup_call_for_in_dialog(
        &self,
        request: &sip_core::Request,
    ) -> Result<(fcp_b2bua_core::Call, bool)> {
        let dialog_call_id = sip_parse::header(request.headers(), "Call-ID")
            .ok_or_else(|| anyhow::anyhow!("Missing Call-ID"))?;

        let call = self
            .call_manager
            .find_by_dialog_call_id(dialog_call_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("Dialog not found"))?;

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

    /// Get the opposite leg's last reliable provisional.
    fn get_opposite_reliable_provisional(
        &self,
        call: &fcp_b2bua_core::Call,
        is_a_leg: bool,
    ) -> Option<sip_core::Response> {
        let list = if is_a_leg {
            call.b_leg.as_ref().map(|b| &b.reliable_provisionals)
        } else {
            Some(&call.a_leg.reliable_provisionals)
        }?;

        // Use the latest reliable provisional (highest RSeq)
        list.iter()
            .max_by_key(|resp| {
                sip_parse::header(&resp.headers(), "RSeq")
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(0)
            })
            .cloned()
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
                    vals.iter()
                        .map(|v| v)
                        .map(|v| *v)
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let route = headers
                .map(|h| {
                    let vals: Vec<_> = h.get_all("Route").collect();
                    vals.iter()
                        .map(|v| v)
                        .map(|v| *v)
                        .collect::<Vec<_>>()
                        .join(",")
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
                        if let Err(e) =
                            handle_session_expired(&cm, &dm, &stm, uac.clone(), dialog_id.clone())
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
        let key = (dialog_id.clone(), smol_str::SmolStr::new(event.to_string()));
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
            smol_str::SmolStr::new(sub.id().event().to_string()),
        );
        self.subscription_created_at
            .insert(sub.id().clone(), Instant::now());
        let _ = self.subscription_manager.insert(sub.clone());
        let _ = self.subscription_index.insert(key, sub.id().clone());
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
        if let Some(expires) = dialog.session_expires() {
            let is_refresher = Self::dialog_is_refresher(dialog);
            self.session_timer_manager
                .start_timer(dialog.id().clone(), expires, is_refresher);
        } else if self.b2bua_config.session_timer.enabled {
            let expires = Duration::from_secs(
                self.b2bua_config.session_timer.session_expires_secs as u64,
            );
            // SBC is always refresher when forcing session timers
            self.session_timer_manager
                .start_timer(dialog.id().clone(), expires, true);
        }
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
                self.handle_directed_pickup(request, handle, call_id, &extension).await
            }
            fcp_pbx::FeatureCodeAction::ParkCall => {
                self.handle_park_call(request, handle, call_id).await
            }
            fcp_pbx::FeatureCodeAction::RetrieveParkedCall { orbit } => {
                self.handle_retrieve_parked(request, handle, call_id, orbit).await
            }
            fcp_pbx::FeatureCodeAction::IntercomOneWay { extension } => {
                self.handle_intercom(request, handle, call_id, &extension, fcp_pbx::IntercomMode::OneWay).await
            }
            fcp_pbx::FeatureCodeAction::IntercomTwoWay { extension } => {
                self.handle_intercom(request, handle, call_id, &extension, fcp_pbx::IntercomMode::TwoWay).await
            }
            fcp_pbx::FeatureCodeAction::AgentLogin => {
                self.handle_agent_login(request, handle, call_id).await
            }
            fcp_pbx::FeatureCodeAction::AgentLogout => {
                self.handle_agent_logout(request, handle, call_id).await
            }
            fcp_pbx::FeatureCodeAction::VoicemailAccess { mailbox } => {
                self.handle_voicemail_access_feature(request, handle, call_id, mailbox).await
            }
        }
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
                // TODO: Phase 2 — CANCEL the ringing B-leg and bridge the picker to the A-leg
                let response = self.create_simple_response(request, 501, "Pickup Not Yet Implemented");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some("Group pickup not yet fully implemented".to_string()));
            }
            None => {
                info!(call_id = %call_id, caller_ext = %caller_ext, "Group pickup: no ringing call found");
                let response = self.create_simple_response(request, 480, "No Ringing Call");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some("No ringing call in pickup group".to_string()));
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
                // TODO: Phase 2 — CANCEL the ringing B-leg and bridge the picker to the A-leg
                let response = self.create_simple_response(request, 501, "Pickup Not Yet Implemented");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some("Directed pickup not yet fully implemented".to_string()));
            }
            None => {
                info!(call_id = %call_id, target = %target_extension, "Directed pickup: no ringing call found");
                let response = self.create_simple_response(request, 480, "No Ringing Call");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some(format!("No ringing call for extension {}", target_extension)));
            }
        }
        Ok(())
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

        // Find the caller's active (Established) call
        let active_call = match self.call_manager.find_established_by_caller(&caller_ext) {
            Some(c) => c,
            None => {
                info!(call_id = %call_id, caller = %caller_ext, "Park: no active call found for caller");
                let response = self.create_simple_response(request, 480, "No Active Call");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some("Park: no active call".to_string()));
                return Ok(());
            }
        };

        let active_call_id = active_call.id;
        let tenant_id = active_call.tenant_id;

        // Park the active call in the PBX parking manager
        let orbit = match pbx.park_call(active_call_id, caller_ext.clone(), tenant_id) {
            Ok(o) => o,
            Err(e) => {
                warn!(call_id = %active_call_id, error = %e, "Failed to park call");
                let response = self.create_simple_response(request, 503, "No Parking Orbits Available");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some(format!("Park failed: {}", e)));
                return Ok(());
            }
        };

        info!(
            call_id = %active_call_id,
            orbit = orbit,
            parker = %caller_ext,
            "Call parked in orbit"
        );

        // Dual-write to cluster parking store (Redis) for cross-node pickup
        if let Some(ref cluster_parking) = self.cluster_parking {
            let info = fcp_core::ClusterParkedCall {
                call_id: active_call_id.to_string(),
                node_id: self.node_id.clone(),
                parker_extension: caller_ext.clone(),
                tenant_id: tenant_id.to_string(),
                parked_at_unix: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            };
            let timeout_secs = pbx.parking_manager().config().timeout_seconds;
            let cp = Arc::clone(cluster_parking);
            tokio::spawn(async move {
                if let Err(e) = cp.cluster_park(orbit, info, timeout_secs).await {
                    warn!(orbit = orbit, error = %e, "Failed to write parked call to cluster store");
                }
            });
        }

        // BYE the B-leg (if present)
        if let Some(uac) = &self.uac {
            if let Some(b_leg) = &active_call.b_leg {
                if let Some(b_dialog) = &b_leg.dialog {
                    self.spawn_bye_with_retry(
                        Arc::clone(uac),
                        b_dialog.clone(),
                        active_call_id,
                        false,
                        b_leg.flow_token.clone(),
                    );
                }
            }
        }

        // Clear B-leg and mark call as Parked
        let _ = self.call_manager.update_call(&active_call_id, |c| {
            c.b_leg = None;
            c.park(orbit, caller_ext.clone());
        });

        // Put A-leg on hold via re-INVITE with a=sendonly SDP
        if let Some(uac) = &self.uac {
            if let Some(a_dialog) = &active_call.a_leg.dialog {
                let hold_sdp = generate_hold_sdp(
                    active_call.a_leg.sdp.as_deref(),
                    &self.local_addr,
                    self.rtp_port,
                );
                let mut a_dialog_clone = a_dialog.clone();
                match uac.send_reinvite(&mut a_dialog_clone, Some(&hold_sdp)).await {
                    Ok(resp) => {
                        debug!(
                            call_id = %active_call_id,
                            status = resp.start_line().code(),
                            "Hold re-INVITE sent to A-leg"
                        );
                        // Persist updated dialog
                        let _ = self.dialog_manager.insert(a_dialog_clone.clone());
                        let _ = self.call_manager.update_call(&active_call_id, |c| {
                            c.a_leg.dialog = Some(a_dialog_clone);
                        });
                    }
                    Err(e) => {
                        warn!(call_id = %active_call_id, error = %e, "Failed to send hold re-INVITE to A-leg");
                    }
                }
            }
        }

        // Respond 200 OK to the feature-code INVITE with orbit info
        let response = self.create_simple_response(request, 200, &format!("Parked Orbit {}", orbit));
        handle.send_final(response).await;

        // Terminate the feature-code call
        let _ = self.call_manager.terminate_call(call_id, None);

        Ok(())
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
        orbit: u16,
    ) -> Result<()> {
        let pbx = match self.pbx_handler.as_ref() {
            Some(p) => p,
            None => {
                let response = self.create_simple_response(request, 500, "PBX Not Configured");
                handle.send_final(response).await;
                return Ok(());
            }
        };

        // Retrieve from local parking manager (removes from orbit)
        let parked = match pbx.retrieve_parked_call(orbit) {
            Ok(p) => {
                // Also remove from cluster store (fire-and-forget)
                if let Some(ref cp) = self.cluster_parking {
                    let cp = Arc::clone(cp);
                    tokio::spawn(async move {
                        if let Err(e) = cp.cluster_unpark(orbit).await {
                            tracing::warn!(orbit = orbit, error = %e, "Failed to clean up cluster parking entry");
                        }
                    });
                }
                p
            }
            Err(_local_err) => {
                // Local orbit is empty — check cluster store for cross-node parked calls
                if let Some(ref cp) = self.cluster_parking {
                    match cp.cluster_lookup(orbit).await {
                        Ok(Some(remote)) if remote.node_id != self.node_id => {
                            // Call is parked on a different node.
                            // Respond 302 with the owning node's info so the caller
                            // or a higher-level proxy can redirect, or log it.
                            info!(
                                orbit = orbit,
                                remote_node = %remote.node_id,
                                call_id = %remote.call_id,
                                "Parked call is on remote node — cross-node pickup not yet supported inline"
                            );
                            let response = self.create_simple_response(
                                request, 480,
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
                            // Redis says this node owns it, but it's not in local DashMap.
                            // Stale entry — clean it up.
                            warn!(orbit = orbit, "Stale cluster parking entry for this node, cleaning up");
                            let cp = Arc::clone(cp);
                            tokio::spawn(async move {
                                if let Err(e) = cp.cluster_unpark(orbit).await {
                                    tracing::warn!(orbit = orbit, error = %e, "Failed to clean up cluster parking entry");
                                }
                            });
                        }
                        _ => {} // Not found in cluster either, or error
                    }
                }

                info!(call_id = %call_id, orbit = orbit, "Failed to retrieve parked call (not found locally or in cluster)");
                let response = self.create_simple_response(request, 480, "Orbit Not Occupied");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some("Orbit not occupied".to_string()));
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
                let _ = self.call_manager.terminate_call(call_id, Some("Parked call state mismatch".to_string()));
                return Ok(());
            }
            None => {
                warn!(call_id = %parked_call_id, "Parked call not found in call manager");
                let response = self.create_simple_response(request, 480, "Parked Call Not Found");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some("Parked call not found".to_string()));
                return Ok(());
            }
        };

        info!(
            call_id = %parked_call_id,
            orbit = orbit,
            "Retrieved parked call, bridging to retriever"
        );

        let retriever_ext = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();

        // Take A-leg off hold via re-INVITE with a=sendrecv SDP
        if let Some(uac) = &self.uac {
            if let Some(a_dialog) = &parked_call.a_leg.dialog {
                let unhold_sdp = generate_unhold_sdp(
                    parked_call.a_leg.sdp.as_deref(),
                    &self.local_addr,
                    self.rtp_port,
                );
                let mut a_dialog_clone = a_dialog.clone();
                match uac.send_reinvite(&mut a_dialog_clone, Some(&unhold_sdp)).await {
                    Ok(resp) => {
                        debug!(
                            call_id = %parked_call_id,
                            status = resp.start_line().code(),
                            "Unhold re-INVITE sent to A-leg"
                        );
                        let _ = self.dialog_manager.insert(a_dialog_clone.clone());
                        let _ = self.call_manager.update_call(&parked_call_id, |c| {
                            c.a_leg.dialog = Some(a_dialog_clone);
                        });
                    }
                    Err(e) => {
                        warn!(call_id = %parked_call_id, error = %e, "Failed to send unhold re-INVITE");
                    }
                }
            }
        }

        // Resolve retriever's contact via registrar and create new B-leg
        let uac = match &self.uac {
            Some(u) => Arc::clone(u),
            None => {
                warn!(call_id = %parked_call_id, "No UAC configured, cannot bridge to retriever");
                let response = self.create_simple_response(request, 500, "No UAC Configured");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some("No UAC".to_string()));
                return Ok(());
            }
        };

        // Build the retriever's AOR from their extension and our domain
        let host = self.uas.local_uri.host().to_string();
        let retriever_aor = format!("sip:{}@{}", retriever_ext, host);

        // Look up retriever's contact in registrar
        let target_uri = if let Some(registrar) = &self.registrar {
            match registrar.get_registrations(&retriever_aor).await {
                Ok(bindings) if !bindings.is_empty() => {
                    bindings[0].contact().to_string()
                }
                _ => {
                    // Fallback: use the retriever's AOR directly
                    retriever_aor.clone()
                }
            }
        } else {
            retriever_aor.clone()
        };

        // Get the A-leg SDP to use as the offer for the new B-leg
        let a_sdp = self.call_manager.get_call(&parked_call_id)
            .and_then(|c| c.a_leg.sdp.clone());

        // Send outbound INVITE to retriever (new B-leg)
        match uac.invite(&target_uri, a_sdp.as_deref()).await {
            Ok(b_leg_handle) => {
                let b_leg_handle = Arc::new(b_leg_handle);
                // Wait for the final response
                match b_leg_handle.await_final().await {
                    Ok(resp) if resp.start_line().code() >= 200 && resp.start_line().code() < 300 => {
                        // Success - bridge the call
                        let b_dialog = b_leg_handle.dialog.read().await.clone();
                        let b_sdp = if !resp.body().is_empty() {
                            Some(String::from_utf8_lossy(resp.body()).to_string())
                        } else {
                            None
                        };

                        let _ = self.call_manager.update_call(&parked_call_id, |c| {
                            let mut new_b_leg = fcp_b2bua_core::CallLeg::new_outbound(
                                target_uri.clone(),
                                None,
                            );
                            new_b_leg.dialog = Some(b_dialog);
                            new_b_leg.sdp = b_sdp;
                            new_b_leg.outbound_handle = Some(Arc::clone(&b_leg_handle));
                            c.b_leg = Some(new_b_leg);
                            c.unpark();
                        });

                        info!(
                            call_id = %parked_call_id,
                            orbit = orbit,
                            retriever = %retriever_ext,
                            "Parked call retrieved and bridged to retriever"
                        );

                        // Respond 200 OK to the feature-code INVITE
                        let response = self.create_simple_response(request, 200, "OK");
                        handle.send_final(response).await;
                    }
                    Ok(resp) => {
                        // Non-2xx response — retriever unreachable
                        let code = resp.start_line().code();
                        warn!(
                            call_id = %parked_call_id,
                            orbit = orbit,
                            retriever = %retriever_ext,
                            status = code,
                            "Retriever returned error, terminating parked call"
                        );
                        // BYE the A-leg since we already removed from parking
                        self.terminate_parked_a_leg(&parked_call_id).await;
                        let response = self.create_simple_response(request, 480, "Retriever Unavailable");
                        handle.send_final(response).await;
                    }
                    Err(e) => {
                        warn!(
                            call_id = %parked_call_id,
                            orbit = orbit,
                            error = %e,
                            "Failed to reach retriever"
                        );
                        self.terminate_parked_a_leg(&parked_call_id).await;
                        let response = self.create_simple_response(request, 480, "Retriever Unavailable");
                        handle.send_final(response).await;
                    }
                }
            }
            Err(e) => {
                warn!(
                    call_id = %parked_call_id,
                    error = %e,
                    "Failed to create B-leg INVITE to retriever"
                );
                self.terminate_parked_a_leg(&parked_call_id).await;
                let response = self.create_simple_response(request, 500, "Internal Error");
                handle.send_final(response).await;
            }
        }

        // Terminate the feature-code call
        let _ = self.call_manager.terminate_call(call_id, None);

        Ok(())
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
            let _ = self.call_manager.terminate_call(call_id, Some("Parked call terminated".to_string()));
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

        // Verify the call is still in Parked state
        let parked_call = match call_manager.get_call(&parked_call_id) {
            Some(c) if c.state == fcp_b2bua_core::CallState::Parked => c,
            Some(c) => {
                debug!(
                    call_id = %parked_call_id,
                    state = ?c.state,
                    "Expired parked call is no longer in Parked state, skipping"
                );
                return Ok(());
            }
            None => {
                debug!(call_id = %parked_call_id, "Expired parked call not found, skipping");
                return Ok(());
            }
        };

        info!(
            call_id = %parked_call_id,
            orbit = parked.orbit,
            parker = %parker_ext,
            "Parked call expired, routing back to parker"
        );

        // Build parker's AOR
        let host = self.uas.local_uri.host().to_string();
        let parker_aor = format!("sip:{}@{}", parker_ext, host);

        // Look up parker's contact in registrar
        let target_uri = if let Some(registrar) = &self.registrar {
            match registrar.get_registrations(&parker_aor).await {
                Ok(bindings) if !bindings.is_empty() => {
                    bindings[0].contact().to_string()
                }
                _ => {
                    warn!(
                        call_id = %parked_call_id,
                        parker = %parker_ext,
                        "Parker not registered, terminating parked call"
                    );
                    self.terminate_parked_a_leg(&parked_call_id).await;
                    return Ok(());
                }
            }
        } else {
            warn!(call_id = %parked_call_id, "No registrar configured, terminating parked call");
            self.terminate_parked_a_leg(&parked_call_id).await;
            return Ok(());
        };

        // Get UAC
        let uac = match &self.uac {
            Some(u) => Arc::clone(u),
            None => {
                warn!(call_id = %parked_call_id, "No UAC configured, terminating parked call");
                self.terminate_parked_a_leg(&parked_call_id).await;
                return Ok(());
            }
        };

        // Take A-leg off hold
        if let Some(a_dialog) = &parked_call.a_leg.dialog {
            let unhold_sdp = generate_unhold_sdp(
                parked_call.a_leg.sdp.as_deref(),
                &self.local_addr,
                self.rtp_port,
            );
            let mut a_dialog_clone = a_dialog.clone();
            match uac.send_reinvite(&mut a_dialog_clone, Some(&unhold_sdp)).await {
                Ok(resp) => {
                    debug!(
                        call_id = %parked_call_id,
                        status = resp.start_line().code(),
                        "Unhold re-INVITE sent to A-leg for expired park"
                    );
                    let _ = self.dialog_manager.insert(a_dialog_clone.clone());
                    let _ = call_manager.update_call(&parked_call_id, |c| {
                        c.a_leg.dialog = Some(a_dialog_clone);
                    });
                }
                Err(e) => {
                    warn!(call_id = %parked_call_id, error = %e, "Failed to unhold A-leg, terminating");
                    self.terminate_parked_a_leg(&parked_call_id).await;
                    return Ok(());
                }
            }
        }

        // Get updated A-leg SDP for the outbound INVITE
        let a_sdp = call_manager.get_call(&parked_call_id)
            .and_then(|c| c.a_leg.sdp.clone());

        // Send INVITE to parker's contact (new B-leg)
        match uac.invite(&target_uri, a_sdp.as_deref()).await {
            Ok(b_leg_handle) => {
                let b_leg_handle = Arc::new(b_leg_handle);
                match b_leg_handle.await_final().await {
                    Ok(resp) if resp.start_line().code() >= 200 && resp.start_line().code() < 300 => {
                        let b_dialog = b_leg_handle.dialog.read().await.clone();
                        let b_sdp = if !resp.body().is_empty() {
                            Some(String::from_utf8_lossy(resp.body()).to_string())
                        } else {
                            None
                        };

                        let _ = call_manager.update_call(&parked_call_id, |c| {
                            let mut new_b_leg = fcp_b2bua_core::CallLeg::new_outbound(
                                target_uri.clone(),
                                None,
                            );
                            new_b_leg.dialog = Some(b_dialog);
                            new_b_leg.sdp = b_sdp;
                            new_b_leg.outbound_handle = Some(Arc::clone(&b_leg_handle));
                            c.b_leg = Some(new_b_leg);
                            c.unpark();
                        });

                        info!(
                            call_id = %parked_call_id,
                            parker = %parker_ext,
                            "Expired parked call routed back to parker"
                        );
                    }
                    Ok(resp) => {
                        warn!(
                            call_id = %parked_call_id,
                            status = resp.start_line().code(),
                            "Parker rejected expired parked call, terminating"
                        );
                        self.terminate_parked_a_leg(&parked_call_id).await;
                    }
                    Err(e) => {
                        warn!(call_id = %parked_call_id, error = %e, "Failed to reach parker, terminating");
                        self.terminate_parked_a_leg(&parked_call_id).await;
                    }
                }
            }
            Err(e) => {
                warn!(call_id = %parked_call_id, error = %e, "Failed to create INVITE to parker, terminating");
                self.terminate_parked_a_leg(&parked_call_id).await;
            }
        }

        Ok(())
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

        // TODO: Phase 2 — resolve extension to contact, send INVITE with extra_headers,
        // and for one-way mode, modify SDP direction to sendonly on the intercom leg.
        let response = self.create_simple_response(request, 501, "Intercom Not Yet Implemented");
        handle.send_final(response).await;
        let _ = self.call_manager.terminate_call(call_id, Some("Intercom not yet fully implemented".to_string()));
        Ok(())
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

        let tenant_id = fcp_core::TenantId::nil();

        match queue_handler.agent_login(&caller_ext, &tenant_id).await {
            Ok(()) => {
                info!(call_id = %call_id, extension = %caller_ext, "Agent logged in via feature code");
                let response = self.create_simple_response(request, 200, "OK");
                handle.send_final(response).await;
            }
            Err(e) => {
                warn!(call_id = %call_id, extension = %caller_ext, error = %e, "Agent login failed");
                let response = self.create_simple_response(request, 404, "Agent Not Found");
                handle.send_final(response).await;
            }
        }

        let _ = self.call_manager.terminate_call(call_id, Some("Agent login feature code".to_string()));
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
                let response = self.create_simple_response(request, 200, "OK");
                handle.send_final(response).await;
            }
            Err(e) => {
                warn!(call_id = %call_id, extension = %caller_ext, error = %e, "Agent logout failed");
                let response = self.create_simple_response(request, 404, "Agent Not Found");
                handle.send_final(response).await;
            }
        }

        let _ = self.call_manager.terminate_call(call_id, Some("Agent logout feature code".to_string()));
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
                let response = self.create_simple_response(request, 501, "Voicemail Access Not Available");
                handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(call_id, Some("No flow handler for voicemail access".to_string()));
                return Ok(());
            }
        };

        // Determine the target mailbox: explicit or derive from caller extension
        let mailbox_str = mailbox.unwrap_or_default();

        let caller_ext = sip_parse::header(request.headers(), "From")
            .and_then(|from| Self::extract_did_from_uri(from.as_str()))
            .unwrap_or_default();

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
                webhook_url: None,
                created_at: None,
                updated_at: None,
            }
        };

        // Insert flow into the store
        if let Err(e) = flow_handler.flow_store().insert_flow(flow_def).await {
            warn!(call_id = %call_id, error = %e, "Failed to insert voicemail access flow");
            let response = self.create_simple_response(request, 500, "Internal Error");
            handle.send_final(response).await;
            let _ = self.call_manager.terminate_call(call_id, Some("Failed to create voicemail access flow".to_string()));
            return Ok(());
        }

        // Build initial variables
        let mut initial_variables = HashMap::new();
        initial_variables.insert("caller".to_string(), caller_ext.clone());
        initial_variables.insert("caller_extension".to_string(), caller_ext);

        // Build answer callback
        let answer_call_id = *call_id;
        let answer_request = request.clone();
        let answer_handle = handle.clone();
        let answer_call_manager = Arc::clone(&self.call_manager);
        let answer_dialog_manager = Arc::clone(&self.dialog_manager);
        let answer_media_manager = self.media_manager.clone();
        let answer_local_addr = self.local_addr.clone();
        let answer_rtp_port = self.rtp_port;
        let answer_media_profile = self.media_profile.clone();

        let answer_callback: fcp_flow::AnswerCallback = Arc::new(move || {
            let request = answer_request.clone();
            let handle = answer_handle.clone();
            let call_manager = Arc::clone(&answer_call_manager);
            let _dialog_manager = Arc::clone(&answer_dialog_manager);
            let media_manager = answer_media_manager.clone();
            let local_addr = answer_local_addr.clone();
            let rtp_port = answer_rtp_port;
            let media_profile = answer_media_profile.clone();
            let call_id = answer_call_id;
            Box::pin(async move {
                // Check if a media session already exists (created by evaluate_media_anchoring)
                let mut actual_rtp_port = rtp_port;
                let existing_session_id = call_manager.get_call(&call_id)
                    .and_then(|c| c.media_session_id.clone());

                if let Some(mm) = &media_manager {
                    if let Some(ref existing_msid) = existing_session_id {
                        // Reuse existing session — just feed it the remote SDP
                        if let Ok(session) = mm.get_session(existing_msid).await {
                            actual_rtp_port = session.ports().0;
                        }
                        if !request.body().is_empty() {
                            let remote_sdp = String::from_utf8_lossy(request.body()).to_string();
                            let _ = mm.update_session(existing_msid, remote_sdp).await;
                        }
                    } else {
                        // No existing session — create one
                        let core_call_id = fcp_core::CallId::from_uuid(*call_id.as_uuid());
                        let fcp_dialog_id = fcp_core::DialogId::new();
                        if let Ok(session) = mm.create_session_default(core_call_id.clone(), fcp_dialog_id).await {
                            actual_rtp_port = session.ports().0;
                            if !request.body().is_empty() {
                                let remote_sdp = String::from_utf8_lossy(request.body()).to_string();
                                let _ = mm.update_session(&core_call_id, remote_sdp).await;
                            }
                            let _ = call_manager.update_call(&call_id, |c| {
                                c.media_session_id = Some(core_call_id);
                                c.media_anchored = true;
                            });
                        }
                    }
                }

                // Generate SDP with the actual RTP port from the media session
                let sdp_string = if let Some(profile) = media_profile {
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
                        smol_str::SmolStr::new("Content-Type"),
                        smol_str::SmolStr::new("application/sdp"),
                    );
                    let _ = headers.push(
                        smol_str::SmolStr::new("Content-Length"),
                        smol_str::SmolStr::new(&sdp_bytes.len().to_string()),
                    );
                    sip_core::Response::new(base.start_line().clone(), headers, sdp_bytes)
                        .expect("Failed to create response with SDP")
                } else {
                    sip_uas::UserAgentServer::create_response(&request, 200, "OK")
                };
                handle.send_final(response).await;

                Ok(())
            })
        });

        // Build hangup callback
        let hangup_call_id = *call_id;
        let hangup_call_manager = Arc::clone(&self.call_manager);
        let hangup_uac = self.uac.clone();
        let hangup_dialog_manager = Arc::clone(&self.dialog_manager);
        let hangup_media_manager = self.media_manager.clone();

        let hangup_callback: fcp_flow::HangupCallback = Arc::new(move |reason| {
            let call_id = hangup_call_id;
            let call_manager = Arc::clone(&hangup_call_manager);
            let uac_opt = hangup_uac.clone();
            let dialog_manager = Arc::clone(&hangup_dialog_manager);
            let media_manager = hangup_media_manager.clone();
            Box::pin(async move {
                if let Some(call_entry) = call_manager.get_call(&call_id) {
                    if call_entry.media_anchored {
                        if let (Some(ref media_session_id), Some(ref mm)) = (&call_entry.media_session_id, &media_manager) {
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
            match flow_handler.start_flow(&mut call_entry, flow_id, initial_variables, answer_callback, hangup_callback).await {
                Ok(()) => {
                    info!(call_id = %call_id, "Voicemail access flow started");
                    // Send 183 while the flow spins up
                    let mut progress = sip_uas::UserAgentServer::create_response(request, 183, "Session Progress");
                    self.ensure_response_hygiene(request, &mut progress);
                    handle.send_provisional(progress).await;
                }
                Err(e) => {
                    warn!(call_id = %call_id, error = %e, "Failed to start voicemail access flow");
                    let response = self.create_simple_response(request, 500, "Flow Start Failed");
                    handle.send_final(response).await;
                    let _ = self.call_manager.terminate_call(call_id, Some(format!("Voicemail access flow failed: {}", e)));
                }
            }
        } else {
            let response = self.create_simple_response(request, 500, "Call Not Found");
            handle.send_final(response).await;
        }

        Ok(())
    }

    /// Execute a hunt group plan (ring members in phases).
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
                    match self.create_outbound_call(request, &target_uri, &uac, None).await {
                        Ok(outbound_call) => {
                            b_leg_calls.push((member.clone(), outbound_call, binding.flow_token().map(|s| s.to_string())));
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
                    match self.create_outbound_call(request, &target_uri, &uac, None).await {
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
        let _ = self.call_manager.terminate_call(
            &call_id,
            Some("Hunt group: no members answered".to_string()),
        );
        Ok(())
    }

    // =========================================================================
    // Call Forward No Answer (CFNA)
    // =========================================================================

    /// Look up the CFNA destination for a line binding.
    ///
    /// Returns the `forward_on_no_answer` value from the line binding if set.
    async fn check_cfna_destination(
        &self,
        line_binding_id: &fcp_core::LineBindingId,
    ) -> Option<String> {
        let storage = self.storage.as_ref()?;
        match storage.get_line_binding(line_binding_id).await {
            Ok(Some(lb)) => lb.forward_on_no_answer.clone(),
            Ok(None) => {
                debug!(line_binding_id = %line_binding_id, "Line binding not found for CFNA check");
                None
            }
            Err(e) => {
                warn!(line_binding_id = %line_binding_id, error = %e, "Failed to get line binding for CFNA check");
                None
            }
        }
    }

    /// Route a call via CFNA to a forward destination.
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
                .last()
                .unwrap_or(vm_uri);

            // Use the original callee as the user part
            let user = original_callee.unwrap_or("unknown");
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
            // Assume it's a number/extension — resolve via registrar or outbound route
            let host = self.uas.local_uri.host().to_string();
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
        let sdp_override = if !original_request.body().is_empty() {
            let original_sdp = String::from_utf8_lossy(original_request.body()).to_string();
            let media_session_id = self.call_manager.get_call(&call_id).and_then(|call| {
                if call.media_anchored {
                    call.media_session_id.clone()
                } else {
                    None
                }
            });
            if let (Some(media_session_id), Some(media_manager)) =
                (media_session_id, &self.media_manager)
            {
                match media_manager.get_session(&media_session_id).await {
                    Ok(session) => {
                        let (rtp_port, _rtcp_port) = session.ports();
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

        // Create the outbound call
        let outbound_call = self
            .create_outbound_call(original_request, &target_uri, uac, sdp_override)
            .await?;

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

        // Run parallel forking with the single CFNA B-leg
        // Clear line_binding_id so CFNA doesn't recurse infinitely
        let _ = self.call_manager.update_call(&call_id, |call| {
            call.line_binding_id = None;
        });

        self.handle_parallel_forking(
            original_request.clone(),
            Arc::clone(a_leg_handle),
            vec![(target_uri, outbound_call, None)],
            call_id,
            cancel_notify,
        )
        .await
        }) // end Box::pin(async move { ... })
    }

    /// Create an outbound call to a target URI via the UAC.
    ///
    /// Extracts SDP from the A-leg request (if present) and sends INVITE to the
    /// target URI. Returns an Arc<OutboundCall> ready for `handle_parallel_forking()`.
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
        // Use SDP override if provided, otherwise extract from A-leg request
        let a_leg_sdp = sdp_override.or_else(|| {
            if !request.body().is_empty() {
                Some(String::from_utf8_lossy(request.body()).to_string())
            } else {
                None
            }
        });

        let call = uac.invite(target_uri, a_leg_sdp.as_deref()).await?;
        Ok(Arc::new(call))
    }

    // =========================================================================
    // Call Origination (Click-to-Call)
    // =========================================================================

    /// Resolve an extension or SIP URI to a registered contact URI.
    ///
    /// If the input is already a SIP URI (starts with `sip:` / `sips:`), it is
    /// looked up in the registrar directly. Plain extensions are converted to
    /// `sip:{ext}@{local_host}` first.
    async fn resolve_contact(&self, uri_or_ext: &str) -> Result<String> {
        let aor = if uri_or_ext.starts_with("sip:") || uri_or_ext.starts_with("sips:") {
            uri_or_ext.to_string()
        } else {
            let host = self.uas.local_uri.host().to_string();
            format!("sip:{}@{}", uri_or_ext, host)
        };

        if let Some(registrar) = &self.registrar {
            match registrar.get_registrations(&aor).await {
                Ok(bindings) if !bindings.is_empty() => {
                    Ok(bindings[0].contact().to_string())
                }
                Ok(_) => Err(anyhow::anyhow!("No registration found for {}", aor)),
                Err(e) => Err(anyhow::anyhow!("Registrar lookup failed for {}: {}", aor, e)),
            }
        } else {
            // No registrar — use the AOR directly as the target
            Ok(aor)
        }
    }

    /// Originate a call via REST API (click-to-call).
    ///
    /// The A-leg INVITE is sent to the `from` party first. When the `from`
    /// party answers, the B-leg INVITE is sent to the `to` party. When both
    /// legs are established the call is bridged.
    ///
    /// Returns the call ID immediately; the call progresses asynchronously.
    pub async fn originate_call(
        &self,
        from: &str,
        to: &str,
        tenant_id: fcp_core::TenantId,
        from_display_name: Option<String>,
    ) -> Result<fcp_b2bua_core::CallId> {
        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("UAC not configured — cannot originate calls"))?;

        // Resolve contacts
        let from_contact = self.resolve_contact(from).await?;
        let to_contact = self.resolve_contact(to).await?;

        info!(
            from = %from,
            from_contact = %from_contact,
            to = %to,
            to_contact = %to_contact,
            "Originating click-to-call"
        );

        // Create the call in CallManager
        let call = fcp_b2bua_core::Call::new_originated(
            from_contact.clone(),
            from_display_name,
            to_contact.clone(),
            tenant_id,
        );
        let call_id = call.id;
        self.call_manager.create_call(call);

        // Prepare dependencies for the background task
        let uac = Arc::clone(uac);
        let call_manager = self.call_manager.clone();
        let dialog_manager = Arc::clone(&self.dialog_manager);
        let local_addr = self.local_addr.clone();
        let rtp_port = self.rtp_port;

        tokio::spawn(async move {
            if let Err(e) = run_origination(
                uac,
                call_manager,
                dialog_manager,
                call_id,
                from_contact,
                to_contact,
                local_addr,
                rtp_port,
            )
            .await
            {
                tracing::error!(
                    call_id = %call_id,
                    error = %e,
                    "Call origination failed"
                );
            }
        });

        Ok(call_id)
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
        let call = self
            .call_manager
            .get_call(call_id)
            .ok_or_else(|| anyhow::anyhow!("Call not found: {}", call_id))?;

        if let Some(uac) = &self.uac {
            if let Some(dialog) = &call.a_leg.dialog {
                self.spawn_bye_with_retry(
                    Arc::clone(uac),
                    dialog.clone(),
                    *call_id,
                    true,
                    None, // A-leg doesn't use flow tokens
                );
            }
            if let Some(b_leg) = &call.b_leg {
                if let Some(dialog) = &b_leg.dialog {
                    self.spawn_bye_with_retry(
                        Arc::clone(uac),
                        dialog.clone(),
                        *call_id,
                        false,
                        b_leg.flow_token.clone(),
                    );
                }
            }
        }

        self.call_manager.terminate_call(call_id, reason)?;
        Ok(())
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

/// Background task that drives a click-to-call origination to completion.
///
/// 1. INVITE the `from` party (A-leg).
/// 2. When A-leg answers, INVITE the `to` party (B-leg).
/// 3. When B-leg answers, bridge both legs.
/// 4. On failure at any step, clean up (BYE answered legs, mark call failed).
async fn run_origination(
    uac: Arc<SipUserAgentClient>,
    call_manager: Arc<fcp_b2bua_core::CallManager>,
    dialog_manager: Arc<DialogManager>,
    call_id: fcp_b2bua_core::CallId,
    from_contact: String,
    to_contact: String,
    local_addr: String,
    rtp_port: u16,
) -> Result<()> {
    use std::time::Duration;

    let leg_timeout = Duration::from_secs(30);

    // Generate SDP offer for the A-leg
    let a_sdp = generate_basic_sdp(&local_addr, rtp_port);

    // --- A-leg: INVITE the "from" party ---
    let _ = call_manager.update_call(&call_id, |c| {
        c.state = fcp_b2bua_core::CallState::Inviting;
    });

    let a_handle = match uac.invite(&from_contact, Some(&a_sdp)).await {
        Ok(h) => Arc::new(h),
        Err(e) => {
            let _ = call_manager.update_call(&call_id, |c| {
                c.fail(format!("A-leg INVITE failed: {}", e));
            });
            return Err(anyhow::anyhow!("A-leg INVITE to {} failed: {}", from_contact, e));
        }
    };

    info!(call_id = %call_id, target = %from_contact, "A-leg INVITE sent");

    // Wait for A-leg final response
    let a_response = match tokio::time::timeout(leg_timeout, a_handle.await_final()).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => {
            let _ = call_manager.update_call(&call_id, |c| {
                c.fail(format!("A-leg failed: {}", e));
            });
            return Err(anyhow::anyhow!("A-leg failed: {}", e));
        }
        Err(_) => {
            // Timeout — cancel the A-leg
            let _ = a_handle.cancel().await;
            let _ = call_manager.update_call(&call_id, |c| {
                c.fail("A-leg timeout".to_string());
            });
            return Err(anyhow::anyhow!("A-leg timeout"));
        }
    };

    let a_status = a_response.start_line().code();
    if a_status < 200 || a_status >= 300 {
        let _ = call_manager.update_call(&call_id, |c| {
            c.fail(format!(
                "A-leg rejected: {} {}",
                a_status,
                a_response.start_line().reason()
            ));
        });
        return Err(anyhow::anyhow!(
            "A-leg rejected: {} {}",
            a_status,
            a_response.start_line().reason()
        ));
    }

    info!(call_id = %call_id, "A-leg answered");

    // Store A-leg dialog
    let a_dialog = a_handle.dialog.read().await.clone();
    let _ = dialog_manager.insert(a_dialog.clone());
    let _ = call_manager.update_call(&call_id, |c| {
        c.a_leg.dialog = Some(a_dialog.clone());
        c.a_leg.outbound_handle = Some(Arc::clone(&a_handle));
        c.state = fcp_b2bua_core::CallState::Proceeding;
    });

    // --- B-leg: INVITE the "to" party ---
    let b_sdp = generate_basic_sdp(&local_addr, rtp_port);

    let b_handle = match uac.invite(&to_contact, Some(&b_sdp)).await {
        Ok(h) => Arc::new(h),
        Err(e) => {
            // BYE the A-leg
            let _ = uac.bye(&a_dialog).await;
            dialog_manager.remove(a_dialog.id());
            let _ = call_manager.update_call(&call_id, |c| {
                c.fail(format!("B-leg INVITE failed: {}", e));
            });
            return Err(anyhow::anyhow!("B-leg INVITE to {} failed: {}", to_contact, e));
        }
    };

    info!(call_id = %call_id, target = %to_contact, "B-leg INVITE sent");

    // Wait for B-leg final response
    let b_response = match tokio::time::timeout(leg_timeout, b_handle.await_final()).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => {
            // BYE the A-leg
            let _ = uac.bye(&a_dialog).await;
            dialog_manager.remove(a_dialog.id());
            let _ = call_manager.update_call(&call_id, |c| {
                c.fail(format!("B-leg failed: {}", e));
            });
            return Err(anyhow::anyhow!("B-leg failed: {}", e));
        }
        Err(_) => {
            // Timeout — cancel B-leg, BYE A-leg
            let _ = b_handle.cancel().await;
            let _ = uac.bye(&a_dialog).await;
            dialog_manager.remove(a_dialog.id());
            let _ = call_manager.update_call(&call_id, |c| {
                c.fail("B-leg timeout".to_string());
            });
            return Err(anyhow::anyhow!("B-leg timeout"));
        }
    };

    let b_status = b_response.start_line().code();
    if b_status < 200 || b_status >= 300 {
        // BYE the A-leg
        let _ = uac.bye(&a_dialog).await;
        dialog_manager.remove(a_dialog.id());
        let _ = call_manager.update_call(&call_id, |c| {
            c.fail(format!(
                "B-leg rejected: {} {}",
                b_status,
                b_response.start_line().reason()
            ));
        });
        return Err(anyhow::anyhow!(
            "B-leg rejected: {} {}",
            b_status,
            b_response.start_line().reason()
        ));
    }

    info!(call_id = %call_id, "B-leg answered — call established");

    // Store B-leg dialog and establish the call
    let b_dialog = b_handle.dialog.read().await.clone();
    let _ = dialog_manager.insert(b_dialog.clone());
    let _ = call_manager.update_call(&call_id, |c| {
        if let Some(b_leg) = &mut c.b_leg {
            b_leg.dialog = Some(b_dialog.clone());
            b_leg.outbound_handle = Some(Arc::clone(&b_handle));
        }
        c.establish();
    });

    Ok(())
}

#[async_trait::async_trait]
impl InboundMessageHandler for SipMessageHandler {
    async fn handle_inbound(
        &self,
        transport: sip_transaction::TransportKind,
        peer: SocketAddr,
        payload: Bytes,
        stream: Option<mpsc::Sender<Bytes>>,
    ) -> Result<()> {
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
        if let Some(req) = sip_parse::parse_request(&payload) {
            // Look up FCP internal CallId from the SIP Call-ID header
            // Priority: 1) Fast DashMap lookup, 2) Call manager dialog scan, 3) Raw SIP Call-ID
            let sip_cid = sip_parse::header(&req.headers(), "Call-ID");
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
                sip_parse::header(&req.headers(), "CSeq").map(|s| s.as_str()),
                Some(peer),
                Some(match transport {
                    sip_transaction::TransportKind::Udp => "udp",
                    sip_transaction::TransportKind::Tcp => "tcp",
                    sip_transaction::TransportKind::Tls => "tls",
                    sip_transaction::TransportKind::Ws => "ws",
                    _ => "other",
                }),
                Some(&req.headers()),
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
                if let Err(e) = self.handle_ack(&req) {
                    error!("Failed to handle ACK: {}", e);
                }
                return Ok(());
            }

            // Clone stream for REGISTER flow token support (RFC 5626)
            let stream_for_register = stream.clone();

            // Create transport context
            let ctx = sip_transaction::TransportContext::new(transport, peer, stream);

            // Create server transaction
            let handle = self
                .transaction_manager
                .receive_request(req.clone(), ctx)
                .await;

            // Handle request and send response(s)
            if let Err(e) = self
                .handle_request(&req, &handle, peer, transport, stream_for_register)
                .await
            {
                error!("Failed to handle request: {}", e);
                // Send error response
                let response = self.create_simple_response(&req, 500, "Internal Server Error");
                handle.send_final(response).await;
            }
        } else if let Some(resp) = sip_parse::parse_response(&payload) {
            // Look up FCP internal CallId from the SIP Call-ID header
            // Priority: 1) Fast DashMap lookup, 2) Call manager dialog scan, 3) Raw SIP Call-ID
            let sip_cid_resp = sip_parse::header(&resp.headers(), "Call-ID");
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
                sip_parse::header(&resp.headers(), "CSeq").map(|s| s.as_str()),
                Some(peer),
                Some(match transport {
                    sip_transaction::TransportKind::Udp => "udp",
                    sip_transaction::TransportKind::Tcp => "tcp",
                    sip_transaction::TransportKind::Tls => "tls",
                    sip_transaction::TransportKind::Ws => "ws",
                    _ => "other",
                }),
                Some(&resp.headers()),
                Some(&resp.body()),
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
fn extract_tag_value(value: Option<&SmolStr>) -> Option<String> {
    value.and_then(|v| {
        v.split(';').find_map(|seg| {
            let s = seg.trim();
            if let Some(rest) = s.strip_prefix("tag=") {
                Some(rest.to_string())
            } else {
                None
            }
        })
    })
}

/// Extract tenant identifier from an INVITE request.
///
/// Multi-tenant isolation strategy (priority order):
/// 1. X-Tenant-ID header (explicit tenant override for authenticated requests)
/// 2. Domain from Request-URI (e.g., user@tenant1.example.com -> "tenant1.example.com")
/// 3. Domain from To header (fallback if Request-URI has no domain)
///
/// Returns the domain/tenant name as a String for lookup in TenantStore.get_tenant_by_name().
/// The caller should convert this to a TenantId via storage lookup.
///
/// # Returns
/// - Some(String) if a domain/tenant can be extracted
/// - None if no tenant context is available (use TenantId::nil() as fallback)
fn extract_tenant_from_invite(request: &sip_core::Request) -> Option<String> {
    // Priority 1: Check for explicit X-Tenant-ID header (for authenticated requests)
    if let Some(tenant_header) = request.headers().get("X-Tenant-ID") {
        let tenant_id = tenant_header.trim();
        if !tenant_id.is_empty() {
            return Some(tenant_id.to_string());
        }
    }

    // Priority 2: Extract domain from Request-URI
    if let Some(domain) = extract_domain_from_uri(request.uri()) {
        return Some(domain);
    }

    // Priority 3: Extract domain from To header
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

/// Helper to check if a URI represents an E.164 phone number.
///
/// Extracts the user part from a SIP URI and checks if it looks like an E.164 number.
/// E.164 numbers start with '+' and have at least 7 digits.
///
/// # Examples
/// - `sip:+15183217034@domain` → true
/// - `sip:alice@domain` → false
/// - `sip:+1518@domain` → false (too short)
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
        let key = extract_hostport_from_uri(&contact.uri);
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

    let ctype = sip_parse::header(request.headers(), "Content-Type")
        .ok_or_else(|| anyhow::anyhow!("Missing Content-Type"))?;
    if ctype.to_lowercase() != "application/sdp" {
        return Err(anyhow::anyhow!("Unsupported Content-Type"));
    }

    let body_str = String::from_utf8_lossy(request.body()).to_string();
    let sdp = parse_sdp(&body_str).map_err(|e| anyhow::anyhow!("Invalid SDP: {}", e))?;

    if sdp.media.is_empty() {
        return Err(anyhow::anyhow!("SDP missing media descriptions"));
    }

    let has_audio = sdp
        .media
        .iter()
        .any(|m| matches!(m.media_type, sip_sdp::MediaType::Audio));
    if !has_audio {
        return Err(anyhow::anyhow!("SDP missing audio media"));
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
    let se_header = sip_parse::header(&resp.headers(), "Session-Expires")?;
    sip_parse::parse_session_expires(se_header).map(|se| {
        (
            Duration::from_secs(se.delta_seconds() as u64),
            se.refresher(),
        )
    })
}

fn extract_min_se(resp: &sip_core::Response) -> Option<Duration> {
    sip_parse::header(&resp.headers(), "Min-SE")
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

async fn handle_session_expired(
    call_manager: &fcp_b2bua_core::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    uac: Option<Arc<SipUserAgentClient>>,
    dialog_id: sip_dialog::DialogId,
) -> Result<()> {
    let (dialog, call_id, _is_a_leg) = find_call_and_dialog(call_manager, &dialog_id)
        .ok_or_else(|| anyhow::anyhow!("Dialog not found"))?;
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
    let _ = call_manager.terminate_call(call_id, Some("Session expired".to_string()));
}

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
        if let Some(end) = from_header[start..].find('>') {
            &from_header[start + 1..start + end]
        } else {
            return None;
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
    response: &mut sip_core::Response,
) {
    if !config.session_timer.enabled {
        return;
    }

    if response.headers().get("Session-Expires").is_none() {
        let se_value = format!(
            "{};refresher=uas",
            config.session_timer.session_expires_secs
        );
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
        let held = existing
            .replace("a=sendrecv", "a=sendonly");
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

/// Generate an SDP body that takes the call off hold (a=sendrecv).
///
/// If an existing SDP is provided, replaces `a=sendonly` with `a=sendrecv`.
/// Otherwise generates a minimal SDP with `a=sendrecv`.
fn generate_unhold_sdp(sdp: Option<&str>, local_addr: &str, rtp_port: u16) -> String {
    if let Some(existing) = sdp {
        // Replace sendonly with sendrecv
        let unheld = existing
            .replace("a=sendonly", "a=sendrecv");
        // If there was no explicit sendonly attribute, append sendrecv
        if !unheld.contains("a=sendrecv") {
            format!("{}a=sendrecv\r\n", unheld)
        } else {
            unheld
        }
    } else {
        // Generate minimal sendrecv SDP
        format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 {addr}\r\n\
             s=-\r\n\
             c=IN IP4 {addr}\r\n\
             t=0 0\r\n\
             m=audio {port} RTP/AVP 0\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=sendrecv\r\n",
            addr = local_addr,
            port = rtp_port,
        )
    }
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    use crate::registrar::FcpRegistrar;
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
            None, // No outbound route provider
            None, // No PBX handler
            None, // No conference handler
            None, // No queue handler
            None, // presence_handler
            None, // No cluster parking
            String::new(), // node_id
        ).unwrap();

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
            None, // No outbound route provider
            None, // No PBX handler
            None, // No conference handler
            None, // No queue handler
            None, // presence_handler
            None, // No cluster parking
            String::new(), // node_id
        ).unwrap();

        let mut headers = Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123"),
        ).unwrap();
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:alice@example.com>;tag=abc123"),
        ).unwrap();
        headers.push(SmolStr::new("To"), SmolStr::new("<sip:bob@example.com>")).unwrap();
        headers.push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id")).unwrap();
        headers.push(SmolStr::new("CSeq"), SmolStr::new("1 INVITE")).unwrap();
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0")).unwrap();

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
            None, // No outbound route provider
            None, // No PBX handler
            None, // No conference handler
            None, // No queue handler
            None, // presence_handler
            None, // No cluster parking
            String::new(), // node_id
        ).unwrap();

        let mut headers = Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bKxyz"),
        ).unwrap();
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:alice@example.com>;tag=abc123"),
        ).unwrap();
        headers.push(
            SmolStr::new("To"),
            SmolStr::new("<sip:bob@example.com>;tag=bbb"),
        ).unwrap();
        headers.push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id-2")).unwrap();
        headers.push(SmolStr::new("CSeq"), SmolStr::new("2 INVITE")).unwrap();
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0")).unwrap();

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
        let registrar = Arc::new(FcpRegistrar::new(storage.clone(), None, None, false));

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
            None, // No outbound route provider
            None, // No PBX handler
            None, // No conference handler
            None, // No queue handler
            None, // presence_handler
            None, // No cluster parking
            String::new(), // node_id
        ).unwrap();

        // Create a REGISTER request
        let mut headers = Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123"),
        ).unwrap();
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:alice@example.com>;tag=abc123"),
        ).unwrap();
        headers.push(SmolStr::new("To"), SmolStr::new("<sip:alice@example.com>")).unwrap();
        headers.push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id")).unwrap();
        headers.push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER")).unwrap();
        headers.push(
            SmolStr::new("Contact"),
            SmolStr::new("<sip:alice@127.0.0.1:5060>;expires=3600"),
        ).unwrap();
        headers.push(SmolStr::new("Expires"), SmolStr::new("3600")).unwrap();
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0")).unwrap();

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
            SmolStr::new(&format!("{}", sdp.len())),
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
    fn test_generate_unhold_sdp_from_existing() {
        let sdp = "v=0\r\n\
                    o=- 0 0 IN IP4 192.168.1.1\r\n\
                    s=-\r\n\
                    c=IN IP4 192.168.1.1\r\n\
                    t=0 0\r\n\
                    m=audio 4000 RTP/AVP 0\r\n\
                    a=sendonly\r\n";

        let unheld = generate_unhold_sdp(Some(sdp), "10.0.0.1", 5000);
        assert!(unheld.contains("a=sendrecv"));
        assert!(!unheld.contains("a=sendonly"));
    }

    #[test]
    fn test_generate_unhold_sdp_without_existing() {
        let unheld = generate_unhold_sdp(None, "10.0.0.1", 5000);
        assert!(unheld.contains("a=sendrecv"));
        assert!(unheld.contains("10.0.0.1"));
        assert!(unheld.contains("5000"));
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

        let unheld = generate_unhold_sdp(Some(&held), "10.0.0.1", 5000);
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
            .last()
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
            .last()
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
    fn test_voicemail_uri_unknown_callee() {
        let vm_uri = "sip:vm@192.168.1.50:5060";
        let original_callee: Option<&str> = None;

        let vm_host = vm_uri
            .strip_prefix("sip:")
            .unwrap_or(vm_uri)
            .split('@')
            .last()
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
