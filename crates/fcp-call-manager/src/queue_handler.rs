//! Concrete QueueCallHandler implementing the QueueHandler trait.
//!
//! Wires together QueueManager (distribution logic), RTP (media),
//! and SIP signaling (UAC/UAS) to handle queue-bound INVITE calls.

use anyhow::{Context, Result};
use dashmap::DashMap;
use fcp_core::TenantId;
use fcp_events::EventPublisher;
use fcp_media::{construct_answer_for_a_leg, parse_sdp_codecs};
use fcp_queue::{QueueManager, QueueSystemConfig, QueuedCall};
use fcp_sip::prelude::SipUserAgentClient;
use forge_rtp::{PortPair, PortPool, RemoteEndpoint, RtpSocketConfig, RtpSocketPair};
use sip_core::Uri;
use sip_uas::UserAgentServer;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Per-call state for a caller waiting in a queue.
#[allow(dead_code)]
struct QueueSession {
    /// SIP Call-ID.
    call_id: String,
    /// Name of the queue this call is waiting in.
    queue_name: String,
    /// Tenant context.
    tenant_id: TenantId,
    /// Caller's SIP URI (From header).
    caller_uri: String,
    /// Caller's display name.
    caller_display: Option<String>,

    // --- Caller A-leg ---
    /// Allocated RTP/RTCP port pair for the caller leg.
    port_pair: PortPair,
    /// RTP socket pair for the caller leg.
    rtp_socket: Arc<RtpSocketPair>,
    /// Negotiated codec name (e.g., "PCMU").
    codec_name: String,
    /// RTP payload type for the negotiated codec.
    payload_type: u8,
    /// Frame size in samples for the negotiated codec.
    _frame_size: usize,
    /// SIP dialog for the caller leg.
    dialog: sip_dialog::Dialog,

    // --- MOH task (aborted when agent connects or caller leaves) ---
    moh_handle: Option<JoinHandle<()>>,

    // --- Agent B-leg (populated when agent answers) ---
    agent_port_pair: Option<PortPair>,
    agent_rtp_socket: Option<Arc<RtpSocketPair>>,
    agent_dialog: Option<sip_dialog::Dialog>,
    bridge_a_to_b: Option<JoinHandle<()>>,
    bridge_b_to_a: Option<JoinHandle<()>>,

    // --- State tracking ---
    state: QueueCallState,
    /// Agent ID currently being rung (if any).
    ringing_agent: Option<String>,
    /// Handle for cancelling an in-progress agent ring task.
    ring_task_handle: Option<JoinHandle<()>>,
    /// When this call was enqueued.
    _enqueued_at: Instant,
}

/// State machine for a queue call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueCallState {
    /// Caller is waiting in the queue, hearing MOH.
    WaitingInQueue,
    /// An agent is being rung for this caller.
    RingingAgent,
    /// Agent has answered, RTP bridge is active.
    Connected,
}

/// Concrete handler for ACD/Queue calls.
///
/// Implements `fcp_sip::queue::QueueHandler` and is plugged into
/// `SipMessageHandler` to intercept INVITEs destined for queue extensions.
pub struct QueueCallHandler {
    queue_manager: QueueManager,
    sessions: Arc<DashMap<String, QueueSession>>,
    local_uri: String,
    local_addr: String,
    port_pool: Arc<PortPool>,
    event_bus: Arc<dyn EventPublisher>,
    uac: Arc<SipUserAgentClient>,
}

impl QueueCallHandler {
    /// Create a new QueueCallHandler.
    ///
    /// Initializes the QueueManager from config and spawns per-queue
    /// distribution loops that periodically try to match waiting callers
    /// with available agents.
    pub fn new(
        config: QueueSystemConfig,
        local_uri: String,
        local_addr: String,
        port_pool: Arc<PortPool>,
        event_bus: Arc<dyn EventPublisher>,
        uac: Arc<SipUserAgentClient>,
        tenant_id: TenantId,
    ) -> Self {
        let queue_manager = QueueManager::new(
            config.clone(),
            Some(Arc::clone(&event_bus)),
            tenant_id,
        );

        let handler = Self {
            queue_manager,
            sessions: Arc::new(DashMap::new()),
            local_uri,
            local_addr,
            port_pool,
            event_bus,
            uac,
        };

        // Spawn distribution loops for each configured queue
        for (queue_name, _queue_config) in &config.queues {
            handler.spawn_distribution_loop(queue_name.clone(), config.max_ring_time_secs);
        }

        handler
    }

    /// Expose the QueueManager for agent login/logout via REST API.
    pub fn queue_manager(&self) -> &QueueManager {
        &self.queue_manager
    }

    /// Parse the local SIP URI.
    fn parse_local_sip_uri(&self) -> sip_core::SipUri {
        Uri::parse(&self.local_uri)
            .ok()
            .and_then(|u| u.as_sip().cloned())
            .unwrap_or_else(|| {
                Uri::parse("sip:queue@localhost")
                    .unwrap()
                    .as_sip()
                    .cloned()
                    .unwrap()
            })
    }

    /// Extract the user part from a SIP Request-URI.
    fn extract_extension(request: &sip_core::Request) -> Option<String> {
        let uri = request.uri();
        let uri_str = uri.to_string();
        if let Some(rest) = uri_str.strip_prefix("sip:") {
            rest.split('@').next().filter(|u| !u.is_empty()).map(String::from)
        } else if let Some(rest) = uri_str.strip_prefix("tel:") {
            let number = rest.split(';').next().unwrap_or(rest).trim();
            if !number.is_empty() {
                Some(number.to_string())
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Extract the Call-ID header from a SIP request.
    fn extract_call_id(request: &sip_core::Request) -> String {
        request
            .headers()
            .get("Call-ID")
            .map(|v| v.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    }

    /// Extract tenant ID from the X-Tenant-ID header.
    fn extract_tenant(request: &sip_core::Request) -> TenantId {
        for header in request.headers() {
            if header.name().eq_ignore_ascii_case("X-Tenant-ID") {
                if let Ok(tid) = header.value().parse::<TenantId>() {
                    return tid;
                }
            }
        }
        TenantId::nil()
    }

    /// Extract caller info (display_name, caller_uri) from From header.
    fn extract_caller_info(request: &sip_core::Request) -> (Option<String>, String) {
        if let Some(from_val) = request.headers().get_smol("From") {
            if let Some(from) = sip_parse::parse_from_header(from_val) {
                let name_addr = from.inner();
                let display = name_addr.display_name().map(|s| s.to_string());
                let caller = name_addr
                    .sip_uri()
                    .map(|u| u.as_str().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                return (display, caller);
            }
        }
        (None, "unknown".to_string())
    }

    /// Parse remote RTP endpoint from an SDP offer.
    fn parse_remote_rtp_addr(sdp: &str) -> Option<SocketAddr> {
        let mut ip: Option<&str> = None;
        let mut port: Option<u16> = None;

        for line in sdp.lines() {
            let line = line.trim();
            if line.starts_with("c=IN IP4 ") || line.starts_with("c=IN ip4 ") {
                ip = line.split_whitespace().nth(2);
            } else if line.starts_with("c=IN IP6 ") || line.starts_with("c=IN ip6 ") {
                ip = line.split_whitespace().nth(2);
            } else if line.starts_with("m=audio ") {
                if let Some(port_str) = line.split_whitespace().nth(1) {
                    port = port_str.parse().ok();
                }
            }
        }

        match (ip, port) {
            (Some(ip_str), Some(p)) => format!("{}:{}", ip_str, p).parse().ok(),
            _ => None,
        }
    }

    /// Pick the first supported codec from an SDP offer.
    /// Returns (codec_name, payload_type, frame_size_in_samples).
    fn pick_codec(codecs: &[fcp_media::SdpCodecInfo]) -> Option<(String, u8, usize)> {
        for codec in codecs {
            let name_upper = codec.encoding_name.to_uppercase();
            match name_upper.as_str() {
                "PCMU" => return Some(("PCMU".to_string(), codec.payload_type, 160)),
                "PCMA" => return Some(("PCMA".to_string(), codec.payload_type, 160)),
                "G722" => return Some(("G722".to_string(), codec.payload_type, 320)),
                _ => continue,
            }
        }
        None
    }

    /// Spawn a MOH (Music On Hold) playback task that sends encoded silence
    /// RTP packets to the caller at 20ms intervals.
    fn spawn_moh(
        rtp_socket: Arc<RtpSocketPair>,
        codec_name: &str,
        payload_type: u8,
    ) -> JoinHandle<()> {
        let silence_frame: Vec<u8> = match codec_name {
            "PCMU" => vec![0xFFu8; 160],        // G.711 μ-law silence
            "PCMA" => vec![0xD5u8; 160],        // G.711 A-law silence
            "G722" => vec![0x00u8; 160],         // G.722 silence
            _ => vec![0xFFu8; 160],              // Default to μ-law
        };

        let seq = Arc::new(AtomicU16::new(0));
        let ts = Arc::new(AtomicU32::new(0));
        let pt = payload_type;
        // Generate a random SSRC from a UUID (avoids adding rand dependency)
        let ssrc = (uuid::Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(20));
            loop {
                interval.tick().await;

                let sequence = seq.fetch_add(1, Ordering::Relaxed);
                let timestamp = ts.fetch_add(160, Ordering::Relaxed);

                // Build minimal RTP header (12 bytes) + payload
                let mut packet = Vec::with_capacity(12 + silence_frame.len());
                // V=2, P=0, X=0, CC=0
                packet.push(0x80);
                // M=0, PT
                packet.push(pt);
                // Sequence number (big-endian)
                packet.extend_from_slice(&sequence.to_be_bytes());
                // Timestamp (big-endian)
                packet.extend_from_slice(&timestamp.to_be_bytes());
                // SSRC (big-endian)
                packet.extend_from_slice(&ssrc.to_be_bytes());
                // Payload
                packet.extend_from_slice(&silence_frame);

                if let Err(e) = rtp_socket.send_rtp_raw(&packet).await {
                    debug!(error = %e, "MOH send failed (caller may have hung up)");
                    break;
                }
            }
        })
    }

    /// Spawn the per-queue distribution loop.
    ///
    /// Runs every 500ms, checking for waiting calls and attempting to
    /// match them with available agents via `QueueManager::select_agent()`.
    fn spawn_distribution_loop(&self, queue_name: String, ring_timeout_secs: u32) {
        let sessions = Arc::clone(&self.sessions);
        let queue_manager = self.queue_manager.clone();
        let port_pool = Arc::clone(&self.port_pool);
        let local_addr = self.local_addr.clone();
        let local_uri = self.local_uri.clone();
        let uac = Arc::clone(&self.uac);
        let event_bus = Arc::clone(&self.event_bus);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;

                if !queue_manager.is_enabled() {
                    continue;
                }

                // Collect call_ids that are WaitingInQueue for this queue
                let waiting_call_ids: Vec<String> = sessions
                    .iter()
                    .filter(|entry| {
                        let s = entry.value();
                        s.queue_name == queue_name && s.state == QueueCallState::WaitingInQueue
                    })
                    .map(|entry| entry.key().clone())
                    .collect();

                for call_id in waiting_call_ids {
                    // Try to select an agent for this call
                    let plan = match queue_manager.select_agent(&queue_name) {
                        Some(p) => p,
                        None => break, // No available agents, stop trying for this tick
                    };

                    // Mark agent as ringing
                    queue_manager.mark_agent_ringing(&plan.agent_id, &call_id).await;

                    // Update session state to RingingAgent
                    if let Some(mut session) = sessions.get_mut(&call_id) {
                        session.state = QueueCallState::RingingAgent;
                        session.ringing_agent = Some(plan.agent_id.clone());

                        // Stop MOH while ringing agent
                        if let Some(moh) = session.moh_handle.take() {
                            moh.abort();
                        }
                    } else {
                        continue;
                    }

                    // Spawn the agent ring task
                    let ring_handle = Self::spawn_agent_ring(
                        call_id.clone(),
                        plan,
                        ring_timeout_secs,
                        Arc::clone(&sessions),
                        queue_manager.clone(),
                        Arc::clone(&port_pool),
                        local_addr.clone(),
                        local_uri.clone(),
                        Arc::clone(&uac),
                        Arc::clone(&event_bus),
                    );

                    // Store ring task handle for cancellation on BYE/CANCEL
                    if let Some(mut session) = sessions.get_mut(&call_id) {
                        session.ring_task_handle = Some(ring_handle);
                    }
                }
            }
        });
    }

    /// Spawn a task that INVITEs an agent and, on answer, bridges RTP.
    fn spawn_agent_ring(
        call_id: String,
        plan: fcp_queue::DistributionPlan,
        ring_timeout_secs: u32,
        sessions: Arc<DashMap<String, QueueSession>>,
        queue_manager: QueueManager,
        port_pool: Arc<PortPool>,
        local_addr: String,
        local_uri: String,
        uac: Arc<SipUserAgentClient>,
        _event_bus: Arc<dyn EventPublisher>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let agent_id = plan.agent_id.clone();
            let agent_extension = plan.agent_extension.clone();
            let ring_timeout = Duration::from_secs(ring_timeout_secs as u64);

            // 1. Allocate B-leg RTP port pair
            let agent_port_pair = match port_pool.allocate().await {
                Ok(pp) => pp,
                Err(e) => {
                    warn!(
                        call_id = %call_id,
                        agent = %agent_id,
                        error = %e,
                        "Failed to allocate RTP port for agent leg"
                    );
                    Self::return_to_waiting(&call_id, &agent_id, &sessions, &queue_manager).await;
                    return;
                }
            };

            // 2. Build SDP offer for agent leg
            let sdp_offer = {
                let codec_name = sessions
                    .get(&call_id)
                    .map(|s| s.codec_name.clone())
                    .unwrap_or_else(|| "PCMU".to_string());
                build_sdp_offer(&local_addr, agent_port_pair.rtp_port, &codec_name)
            };

            // 3. INVITE the agent
            let agent_uri = format!("sip:{}@{}", agent_extension, extract_domain(&local_uri));
            let call_handle = match uac.invite(&agent_uri, Some(&sdp_offer)).await {
                Ok(h) => h,
                Err(e) => {
                    warn!(
                        call_id = %call_id,
                        agent = %agent_id,
                        error = %e,
                        "Failed to INVITE agent"
                    );
                    port_pool.deallocate(agent_port_pair).await;
                    Self::return_to_waiting(&call_id, &agent_id, &sessions, &queue_manager).await;
                    return;
                }
            };

            // 4. Wait for final response with timeout
            let result = tokio::time::timeout(ring_timeout, call_handle.await_final()).await;

            match result {
                Ok(Ok(response)) if response.start_line().code() >= 200 && response.start_line().code() < 300 => {
                    // Agent answered (200 OK)
                    info!(
                        call_id = %call_id,
                        agent = %agent_id,
                        "Agent answered queue call"
                    );

                    // Parse SDP answer from agent
                    let agent_body = response.body();
                    let agent_sdp = std::str::from_utf8(&agent_body[..]).unwrap_or("");

                    // Create B-leg RTP socket
                    let agent_rtp_socket = match RtpSocketPair::new(
                        agent_port_pair,
                        RtpSocketConfig::default(),
                    ).await {
                        Ok(s) => Arc::new(s),
                        Err(e) => {
                            warn!(error = %e, "Failed to create agent RTP socket");
                            port_pool.deallocate(agent_port_pair).await;
                            Self::return_to_waiting(&call_id, &agent_id, &sessions, &queue_manager).await;
                            return;
                        }
                    };

                    // Set agent remote RTP endpoint
                    if let Some(remote_addr) = Self::parse_remote_rtp_addr(agent_sdp) {
                        agent_rtp_socket
                            .set_remote_endpoint(RemoteEndpoint {
                                rtp_addr: remote_addr,
                                rtcp_addr: None,
                            })
                            .await;
                    }

                    // Get the agent's dialog
                    let agent_dialog = call_handle.dialog.read().await.clone();

                    // Update QueueManager
                    queue_manager.agent_answered(&agent_id, &call_id).await;

                    // Start RTP bridge between caller and agent
                    if let Some(mut session) = sessions.get_mut(&call_id) {
                        let (bridge_a_to_b, bridge_b_to_a) = spawn_rtp_bridge(
                            Arc::clone(&session.rtp_socket),
                            Arc::clone(&agent_rtp_socket),
                        );

                        session.state = QueueCallState::Connected;
                        session.agent_port_pair = Some(agent_port_pair);
                        session.agent_rtp_socket = Some(agent_rtp_socket);
                        session.agent_dialog = Some(agent_dialog);
                        session.bridge_a_to_b = Some(bridge_a_to_b);
                        session.bridge_b_to_a = Some(bridge_b_to_a);
                        session.ringing_agent = None;
                        session.ring_task_handle = None;
                    }
                }
                Ok(Ok(response)) => {
                    // Agent rejected (4xx/5xx/6xx)
                    warn!(
                        call_id = %call_id,
                        agent = %agent_id,
                        status = response.start_line().code(),
                        "Agent rejected queue call"
                    );
                    port_pool.deallocate(agent_port_pair).await;
                    Self::return_to_waiting(&call_id, &agent_id, &sessions, &queue_manager).await;
                }
                Ok(Err(e)) => {
                    // Error during call setup
                    warn!(
                        call_id = %call_id,
                        agent = %agent_id,
                        error = %e,
                        "Error during agent INVITE"
                    );
                    port_pool.deallocate(agent_port_pair).await;
                    Self::return_to_waiting(&call_id, &agent_id, &sessions, &queue_manager).await;
                }
                Err(_) => {
                    // Ring timeout — cancel the agent INVITE
                    info!(
                        call_id = %call_id,
                        agent = %agent_id,
                        timeout_secs = ring_timeout_secs,
                        "Agent ring timeout, cancelling INVITE"
                    );
                    let _ = call_handle.cancel().await;
                    port_pool.deallocate(agent_port_pair).await;
                    Self::return_to_waiting(&call_id, &agent_id, &sessions, &queue_manager).await;
                }
            }
        })
    }

    /// Return a call to WaitingInQueue state after a failed agent attempt.
    async fn return_to_waiting(
        call_id: &str,
        agent_id: &str,
        sessions: &DashMap<String, QueueSession>,
        queue_manager: &QueueManager,
    ) {
        queue_manager.agent_no_answer(agent_id, call_id).await;

        if let Some(mut session) = sessions.get_mut(call_id) {
            session.state = QueueCallState::WaitingInQueue;
            session.ringing_agent = None;
            session.ring_task_handle = None;

            // Restart MOH
            let moh = Self::spawn_moh(
                Arc::clone(&session.rtp_socket),
                &session.codec_name,
                session.payload_type,
            );
            session.moh_handle = Some(moh);
        }
    }

    /// Remove and clean up a queue session.
    fn remove_session(&self, call_id: &str) {
        if let Some((_, session)) = self.sessions.remove(call_id) {
            info!(
                call_id = %call_id,
                queue = %session.queue_name,
                state = ?session.state,
                "Queue session removed"
            );

            // Abort MOH if running
            if let Some(moh) = session.moh_handle {
                moh.abort();
            }

            // Abort ring task if running
            if let Some(ring_task) = session.ring_task_handle {
                ring_task.abort();
            }

            // Abort RTP bridge tasks if running
            if let Some(bridge) = session.bridge_a_to_b {
                bridge.abort();
            }
            if let Some(bridge) = session.bridge_b_to_a {
                bridge.abort();
            }

            // Send BYE on agent dialog if connected
            if let Some(ref agent_dialog) = session.agent_dialog {
                let uac = Arc::clone(&self.uac);
                let dialog = agent_dialog.clone();
                tokio::spawn(async move {
                    if let Err(e) = uac.bye(&dialog).await {
                        warn!(error = %e, "Failed to send BYE to agent");
                    }
                });
            }

            // Deallocate A-leg and B-leg port pairs
            let port_pool = Arc::clone(&self.port_pool);
            let a_port = session.port_pair;
            let b_port = session.agent_port_pair;
            tokio::spawn(async move {
                port_pool.deallocate(a_port).await;
                if let Some(bp) = b_port {
                    port_pool.deallocate(bp).await;
                }
            });

            // Notify QueueManager if still in queue
            if session.state != QueueCallState::Connected {
                let qm = self.queue_manager.clone();
                let cid = call_id.to_string();
                tokio::spawn(async move {
                    qm.caller_abandoned(&cid).await;
                });
            } else {
                // Agent was connected — mark call completed so agent goes to WrapUp
                if let Some(ref agent_id) = session.ringing_agent {
                    self.queue_manager.agent_call_completed(agent_id);
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl fcp_sip::queue::QueueHandler for QueueCallHandler {
    async fn can_handle(&self, request: &sip_core::Request) -> bool {
        if !self.queue_manager.is_enabled() {
            return false;
        }
        let extension = match Self::extract_extension(request) {
            Some(ext) => ext,
            None => return false,
        };
        self.queue_manager.get_queue_for_extension(&extension).is_some()
    }

    async fn handle_invite(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
        _peer: SocketAddr,
    ) -> Result<()> {
        // 1. Extract extension and look up queue
        let extension = Self::extract_extension(request)
            .context("Failed to extract extension from Request-URI")?;
        let queue_name = self
            .queue_manager
            .get_queue_for_extension(&extension)
            .context("No queue found for extension")?;

        let call_id = Self::extract_call_id(request);
        let tenant_id = Self::extract_tenant(request);
        let (caller_display, caller_uri) = Self::extract_caller_info(request);

        info!(
            call_id = %call_id,
            queue = %queue_name,
            caller = %caller_uri,
            "Queue INVITE received"
        );

        // 2. Extract and parse SDP offer
        let body = request.body();
        if body.is_empty() {
            let reject = UserAgentServer::create_response(request, 400, "Missing SDP");
            handle.send_final(reject).await;
            return Ok(());
        }
        let sdp_offer = std::str::from_utf8(&body[..])
            .context("SDP body is not valid UTF-8")?;

        // 3. Parse codecs and pick first supported one
        let offered_codecs = parse_sdp_codecs(sdp_offer);
        let (codec_name, payload_type, frame_size) = match Self::pick_codec(&offered_codecs) {
            Some(c) => c,
            None => {
                let reject = UserAgentServer::create_response(request, 488, "Not Acceptable Here");
                handle.send_final(reject).await;
                return Ok(());
            }
        };

        debug!(
            call_id = %call_id,
            queue = %queue_name,
            codec = %codec_name,
            pt = payload_type,
            "Negotiated codec for queue caller"
        );

        // 4. Send 180 Ringing
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri.clone());
        let ringing = uas.create_ringing(request);
        handle.send_provisional(ringing).await;

        // 5. Allocate RTP port pair
        let port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port pair")?;

        // 6. Construct SDP answer
        let sdp_answer = construct_answer_for_a_leg(
            sdp_offer,
            &self.local_addr,
            port_pair.rtp_port,
            None, // No SRTP for queue calls (future enhancement)
        );

        // 7. Create 200 OK with SDP answer
        let (response, dialog) = uas
            .accept_invite(request, Some(&sdp_answer))
            .context("Failed to create 200 OK")?;

        // 8. Send 200 OK
        handle.send_final(response).await;

        // 9. Create RTP socket pair
        let rtp_socket = match RtpSocketPair::new(port_pair, RtpSocketConfig::default()).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                warn!(
                    call_id = %call_id,
                    rtp_port = port_pair.rtp_port,
                    error = %e,
                    "Failed to create RTP socket pair, deallocating port"
                );
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                return Err(anyhow::anyhow!("Failed to create RTP socket pair: {}", e));
            }
        };

        // 10. Set remote RTP endpoint from caller's SDP
        if let Some(remote_addr) = Self::parse_remote_rtp_addr(sdp_offer) {
            rtp_socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote_addr,
                    rtcp_addr: None,
                })
                .await;

            debug!(
                call_id = %call_id,
                remote = %remote_addr,
                local_rtp_port = port_pair.rtp_port,
                "RTP endpoints configured for queue caller"
            );
        } else {
            warn!(call_id = %call_id, "Could not parse remote RTP address from SDP offer");
        }

        // 11. Start MOH playback
        let moh_handle = Self::spawn_moh(
            Arc::clone(&rtp_socket),
            &codec_name,
            payload_type,
        );

        // 12. Enqueue call in QueueManager
        let queued_call = QueuedCall {
            id: uuid::Uuid::new_v4().to_string(),
            call_id: call_id.clone(),
            queue_name: queue_name.clone(),
            tenant_id: tenant_id.clone(),
            caller_uri: caller_uri.clone(),
            caller_display: caller_display.clone(),
            priority: 0,
            enqueued_at: Instant::now(),
            position: 0,
            attempts: 0,
        };

        match self.queue_manager.enqueue_call(queued_call).await {
            Ok(position) => {
                info!(
                    call_id = %call_id,
                    queue = %queue_name,
                    position = position,
                    "Caller enqueued"
                );
            }
            Err(e) => {
                warn!(
                    call_id = %call_id,
                    queue = %queue_name,
                    error = %e,
                    "Failed to enqueue call (queue full?), sending BYE"
                );
                moh_handle.abort();
                let uac = Arc::clone(&self.uac);
                let bye_dialog = dialog.clone();
                tokio::spawn(async move {
                    let _ = uac.bye(&bye_dialog).await;
                });
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                return Ok(());
            }
        }

        // 13. Store session
        self.sessions.insert(
            call_id.clone(),
            QueueSession {
                call_id,
                queue_name,
                tenant_id,
                caller_uri,
                caller_display,
                port_pair,
                rtp_socket,
                codec_name,
                payload_type,
                _frame_size: frame_size,
                dialog,
                moh_handle: Some(moh_handle),
                agent_port_pair: None,
                agent_rtp_socket: None,
                agent_dialog: None,
                bridge_a_to_b: None,
                bridge_b_to_a: None,
                state: QueueCallState::WaitingInQueue,
                ringing_agent: None,
                ring_task_handle: None,
                _enqueued_at: Instant::now(),
            },
        );

        Ok(())
    }

    fn handle_bye(&self, call_id: &str) -> Result<()> {
        if self.sessions.contains_key(call_id) {
            self.remove_session(call_id);
            Ok(())
        } else {
            anyhow::bail!("No queue session for call_id: {}", call_id)
        }
    }

    fn handle_cancel(&self, call_id: &str) -> Result<bool> {
        if self.sessions.contains_key(call_id) {
            info!(call_id = %call_id, "CANCEL received for queue caller");
            self.remove_session(call_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Spawn two tokio tasks that forward raw RTP packets between A-leg and B-leg sockets.
fn spawn_rtp_bridge(
    a_socket: Arc<RtpSocketPair>,
    b_socket: Arc<RtpSocketPair>,
) -> (JoinHandle<()>, JoinHandle<()>) {
    let a_to_b_a = Arc::clone(&a_socket);
    let a_to_b_b = Arc::clone(&b_socket);
    let b_to_a_a = Arc::clone(&a_socket);
    let b_to_a_b = Arc::clone(&b_socket);

    let a_to_b = tokio::spawn(async move {
        loop {
            match a_to_b_a.recv_rtp_raw().await {
                Ok((data, _addr)) => {
                    if data.is_empty() {
                        break;
                    }
                    if let Err(e) = a_to_b_b.send_rtp_raw(&data).await {
                        debug!(error = %e, "A->B bridge send failed");
                        break;
                    }
                }
                Err(e) => {
                    debug!(error = %e, "A->B bridge recv failed");
                    break;
                }
            }
        }
    });

    let b_to_a = tokio::spawn(async move {
        loop {
            match b_to_a_b.recv_rtp_raw().await {
                Ok((data, _addr)) => {
                    if data.is_empty() {
                        break;
                    }
                    if let Err(e) = b_to_a_a.send_rtp_raw(&data).await {
                        debug!(error = %e, "B->A bridge send failed");
                        break;
                    }
                }
                Err(e) => {
                    debug!(error = %e, "B->A bridge recv failed");
                    break;
                }
            }
        }
    });

    (a_to_b, b_to_a)
}

/// Build a simple SDP offer for the agent B-leg.
fn build_sdp_offer(local_addr: &str, rtp_port: u16, codec_name: &str) -> String {
    // Strip port from local_addr if present (e.g., "192.168.1.100:5060" -> "192.168.1.100")
    let ip = local_addr.split(':').next().unwrap_or(local_addr);

    let (pt, encoding, clock_rate): (u8, &str, u32) = match codec_name {
        "PCMU" => (0, "PCMU", 8000),
        "PCMA" => (8, "PCMA", 8000),
        "G722" => (9, "G722", 8000), // G.722 uses 8000 in SDP per RFC
        _ => (0, "PCMU", 8000),
    };

    format!(
        "v=0\r\n\
         o=fcp-queue 0 0 IN IP4 {ip}\r\n\
         s=Queue Call\r\n\
         c=IN IP4 {ip}\r\n\
         t=0 0\r\n\
         m=audio {rtp_port} RTP/AVP {pt} 101\r\n\
         a=rtpmap:{pt} {encoding}/{clock_rate}\r\n\
         a=rtpmap:101 telephone-event/8000\r\n\
         a=fmtp:101 0-16\r\n\
         a=sendrecv\r\n\
         a=ptime:20\r\n",
    )
}

/// Extract domain part from a SIP URI string.
fn extract_domain(uri: &str) -> String {
    // Handle "sip:user@host:port" format
    if let Some(rest) = uri.strip_prefix("sip:").or_else(|| uri.strip_prefix("sips:")) {
        if let Some(at_pos) = rest.find('@') {
            return rest[at_pos + 1..].to_string();
        }
        return rest.to_string();
    }
    // Fallback: return the whole string
    uri.to_string()
}
