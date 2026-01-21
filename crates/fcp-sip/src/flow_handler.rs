//! Flow call handler for executing flows during calls.
//!
//! This module integrates the flow engine with call management, allowing
//! calls to execute programmable flows (IVR, call routing, etc.).

use crate::call::{Call, CallId};
use crate::call_manager::CallManager;
use anyhow::{Context, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use fcp_events::{EventPublisher, FlowEvent, PlatformEvent};
use fcp_flow::{FlowContext, FlowEngine, FlowExecutionResult, FlowStore};
use fcp_media::{AudioTarget, MediaManager};
use sip_core::{Request, Response};
use sip_dialog::DialogManager;
use sip_transaction::{
    fsm::TransportKind, ClientTransactionUser, TransactionKey, TransactionManager, TransportContext,
};
use sip_uac::UserAgentClient;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, error, info, warn};

/// Flow call handler.
///
/// Manages flow execution for calls, including:
/// - Loading flows based on routing rules
/// - Executing flow nodes
/// - Tracking execution state per call
/// - Publishing flow execution events
/// - Integrating media playback for Play/Say nodes
/// - Creating outbound calls (B2BUA Dial node)
pub struct FlowCallHandler {
    /// Flow engine for executing flows
    engine: Arc<FlowEngine>,

    /// Flow store for loading flow definitions
    flow_store: Arc<dyn FlowStore>,

    /// Active flow contexts, indexed by call ID
    contexts: Arc<DashMap<CallId, Arc<RwLock<FlowContext>>>>,

    /// Event publisher for flow events
    events: Option<Arc<dyn EventPublisher>>,

    /// Flow routing rules (DID -> flow_id mapping)
    /// TODO: Phase 2 - Load from database
    routing: Arc<RwLock<HashMap<String, String>>>,

    /// Media manager for audio playback and bridging
    media_manager: Option<Arc<MediaManager>>,

    /// Call manager for tracking active calls
    call_manager: Option<Arc<CallManager>>,

    /// Transaction manager for sending SIP requests
    transaction_manager: Option<Arc<TransactionManager>>,

    /// Dialog manager for managing SIP dialogs
    dialog_manager: Option<Arc<DialogManager>>,

    /// Local SIP URI for outbound calls
    local_uri: Option<sip_core::SipUri>,

    /// Contact URI for outbound calls
    contact_uri: Option<sip_core::SipUri>,
}

impl FlowCallHandler {
    /// Create a new flow call handler.
    pub fn new(
        flow_store: Arc<dyn FlowStore>,
        events: Option<Arc<dyn EventPublisher>>,
        media_manager: Option<Arc<MediaManager>>,
    ) -> Self {
        Self {
            engine: Arc::new(FlowEngine::new()),
            flow_store,
            contexts: Arc::new(DashMap::new()),
            events,
            routing: Arc::new(RwLock::new(HashMap::new())),
            media_manager,
            call_manager: None,
            transaction_manager: None,
            dialog_manager: None,
            local_uri: None,
            contact_uri: None,
        }
    }

    /// Set the call manager (for B2BUA operations).
    pub fn with_call_manager(mut self, call_manager: Arc<CallManager>) -> Self {
        self.call_manager = Some(call_manager);
        self
    }

    /// Set the transaction manager (for sending SIP requests).
    pub fn with_transaction_manager(mut self, transaction_manager: Arc<TransactionManager>) -> Self {
        self.transaction_manager = Some(transaction_manager);
        self
    }

    /// Set the dialog manager (for managing SIP dialogs).
    pub fn with_dialog_manager(mut self, dialog_manager: Arc<DialogManager>) -> Self {
        self.dialog_manager = Some(dialog_manager);
        self
    }

    /// Set the local and contact URIs for outbound calls.
    pub fn with_uris(mut self, local_uri: sip_core::SipUri, contact_uri: sip_core::SipUri) -> Self {
        self.local_uri = Some(local_uri);
        self.contact_uri = Some(contact_uri);
        self
    }

    /// Add a flow routing rule.
    ///
    /// Maps a phone number (DID) to a flow ID.
    pub async fn add_route(&self, did: String, flow_id: String) {
        info!(did = %did, flow_id = %flow_id, "Added flow routing rule");
        let mut routing = self.routing.write().await;
        routing.insert(did, flow_id);
    }

    /// Get flow ID for a phone number (DID).
    pub async fn get_flow_for_did(&self, did: &str) -> Option<String> {
        let routing = self.routing.read().await;
        routing.get(did).cloned()
    }

    /// Start flow execution for a call.
    ///
    /// Loads the flow definition and begins execution from the initial node.
    pub async fn start_flow(
        &self,
        call: &mut Call,
        flow_id: String,
        initial_variables: HashMap<String, String>,
        answer_callback: fcp_flow::AnswerCallback,
        hangup_callback: fcp_flow::HangupCallback,
    ) -> Result<()> {
        info!(
            call_id = %call.id,
            flow_id = %flow_id,
            "Starting flow execution for call"
        );

        // Load flow definition
        let flow = self
            .flow_store
            .get_flow(&flow_id)
            .await
            .context("Failed to load flow")?
            .ok_or_else(|| anyhow::anyhow!("Flow not found: {}", flow_id))?;

        if !flow.enabled {
            return Err(anyhow::anyhow!("Flow is disabled: {}", flow_id));
        }

        let flow = Arc::new(flow);

        // Create flow context
        let call_id_core = fcp_core::CallId::from_uuid(*call.id.as_uuid());
        let mut context = self
            .engine
            .execute(flow.clone(), call_id_core, initial_variables)
            .await?;

        // Set media session ID if available (needed for Play/Say nodes)
        if let Some(media_session_id) = call.media_session_id {
            context.set_media_session_id(media_session_id);
        }

        // Set callbacks for Answer and Hangup nodes
        context.set_answer_callback(answer_callback);
        context.set_hangup_callback(hangup_callback);

        // Store context
        self.contexts
            .insert(call.id, Arc::new(RwLock::new(context)));

        // Update call
        call.flow_id = Some(flow_id.clone());
        call.flow_current_node = Some(flow.initial_node.clone());

        // Publish flow started event
        if let Some(events) = &self.events {
            let event = PlatformEvent::Flow(FlowEvent::Started {
                call_id: call_id_core,
                flow_id: flow_id.clone(),
                flow_version: flow.version,
                initial_node: flow.initial_node.clone(),
                timestamp: std::time::SystemTime::now(),
            });

            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    error!(error = %e, "Failed to publish FlowEvent::Started");
                }
            });
        }

        // Execute the initial node
        self.execute_next_node(call.id).await?;

        Ok(())
    }

    /// Execute the next node in the flow.
    ///
    /// # Media Integration (Play/Say Nodes)
    ///
    /// Play and Say nodes are handled by the FlowEngine, which returns `Continue(next_node)`.
    /// The actual media playback/TTS is currently a placeholder. For full implementation:
    ///
    /// ## Architecture Options:
    ///
    /// ### Option 1: External Media Server (Recommended)
    /// - Deploy a media server (e.g., Asterisk, FreeSWITCH, or custom RTP engine)
    /// - On Play/Say node: Send SIP re-INVITE to insert media server in call path
    /// - Media server handles RTP, plays files, generates TTS
    /// - When complete, send another re-INVITE to remove media server
    ///
    /// ### Option 2: Direct RTP Control
    /// - Implement RTP engine in fcp-media crate
    /// - Anchor media on FCP server
    /// - Send RTP packets directly for audio playback
    /// - Integrate TTS engine (e.g., Google TTS, AWS Polly) for Say nodes
    ///
    /// ### Option 3: Hybrid
    /// - Use external media server for complex operations (conferencing, recording)
    /// - Use direct RTP for simple operations (Play, Say)
    ///
    /// ## Implementation Steps (Future):
    /// 1. Add media_server field to FlowCallHandler with media server SIP URI
    /// 2. On Play/Say nodes, check node type from context before executing
    /// 3. Send re-INVITE to A-leg with media server's SDP
    /// 4. Command media server via SIP INFO or proprietary protocol
    /// 5. Wait for playback completion event
    /// 6. Send re-INVITE to restore direct media path or bridge to B-leg
    /// 7. Continue to next node
    ///
    /// ## Current State:
    /// Play/Say nodes are executed by FlowEngine with placeholder logging.
    /// No actual media playback occurs. Calls continue through flow normally.

    /// Handle media playback for Say nodes that have synthesized TTS audio.
    ///
    /// This method is called when FlowEngine returns WaitingForMedia after
    /// synthesizing TTS audio. It reads the audio path from context variables
    /// and plays it via the media manager.
    async fn handle_media_playback(&self, call_id: CallId) -> Result<()> {
        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found for call {}", call_id))?;

        let context = context_entry.value().clone();
        let context_guard = context.read().await;

        // Get TTS audio path from context variables
        let audio_path = context_guard
            .get_variable("__tts_audio_path")
            .map_err(|_| anyhow::anyhow!("No TTS audio path in context"))?;

        let next_node = context_guard
            .get_variable("__tts_next_node")
            .map_err(|_| anyhow::anyhow!("No next node in context"))?;

        let media_session_id = context_guard
            .media_session_id
            .ok_or_else(|| anyhow::anyhow!("No media session ID available for playback"))?;

        drop(context_guard);

        // Play the synthesized TTS audio
        if let Some(media_manager) = &self.media_manager {
            info!(
                call_id = %call_id,
                audio_path = %audio_path,
                "Playing synthesized TTS audio"
            );

            match media_manager
                .play_file(
                    &media_session_id,
                    audio_path.clone(),
                    fcp_media::AudioTarget::ParticipantA,
                )
                .await
            {
                Ok(playback_id) => {
                    info!(
                        call_id = %call_id,
                        playback_id = %playback_id,
                        "TTS audio playback started"
                    );

                    // Wait for playback to complete
                    match media_manager.wait_playback_completion(playback_id.clone()).await {
                        Ok(()) => {
                            info!(
                                call_id = %call_id,
                                playback_id = %playback_id,
                                "TTS audio playback completed"
                            );
                        }
                        Err(e) => {
                            warn!(
                                call_id = %call_id,
                                playback_id = %playback_id,
                                error = %e,
                                "TTS audio playback failed or was stopped"
                            );
                        }
                    }
                }
                Err(e) => {
                    error!(
                        call_id = %call_id,
                        error = %e,
                        "Failed to play TTS audio"
                    );
                }
            }
        } else {
            warn!(
                call_id = %call_id,
                "Media manager not available, skipping TTS playback"
            );
        }

        // Continue to next node
        let mut context_guard = context.write().await;
        context_guard.current_node = next_node;
        drop(context_guard);

        Box::pin(self.execute_next_node(call_id)).await
    }

    /// Handle outbound call creation for Dial node (B2BUA).
    ///
    /// Creates a B-leg call to the destination and bridges it with the A-leg.
    ///
    /// # Implementation Status
    ///
    /// Phase 1 (COMPLETE): Outbound INVITE creation ✅
    /// Phase 2 (IN PROGRESS): SIP response handling
    /// Phase 3 (PENDING): Media bridging
    /// Phase 4 (PENDING): CDR tracking
    /// Phase 5 (PENDING): Advanced features
    #[allow(unused_variables)] // TODO: Remove when all parameters are used
    async fn handle_dial(
        &self,
        a_leg_call_id: CallId,
        destination: String,
        timeout_secs: u32,
        caller_id: Option<String>,
        max_duration_secs: u32,
        on_busy: Option<String>,
        on_no_answer: Option<String>,
        on_error: Option<String>,
        next: String,
    ) -> Result<()> {
        let caller_id_str = caller_id.as_deref().unwrap_or("none");
        info!(
            a_leg_call_id = %a_leg_call_id,
            destination = %destination,
            timeout_secs = timeout_secs,
            caller_id = %caller_id_str,
            max_duration_secs = max_duration_secs,
            "Starting Dial node - creating outbound B-leg call"
        );

        // ===== PHASE 1: OUTBOUND INVITE CREATION =====

        // Check if B2BUA components are available
        if self.transaction_manager.is_none() {
            warn!(a_leg_call_id = %a_leg_call_id, "TransactionManager not available - Dial node disabled");
            return self.transition_to_error(a_leg_call_id, "TransactionManager not configured", on_error, next).await;
        }

        if self.local_uri.is_none() || self.contact_uri.is_none() {
            warn!(a_leg_call_id = %a_leg_call_id, "SIP URIs not configured - Dial node disabled");
            return self.transition_to_error(a_leg_call_id, "SIP URIs not configured", on_error, next).await;
        }

        let transaction_manager = self.transaction_manager.as_ref().unwrap();
        let local_uri = self.local_uri.as_ref().unwrap();
        let contact_uri = self.contact_uri.as_ref().unwrap();

        // Parse destination URI
        let dest_uri = match sip_core::SipUri::parse(&destination) {
            Ok(uri) => uri,
            Err(e) => {
                let error_msg = format!("Failed to parse destination URI: {:?}", e);
                error!(
                    a_leg_call_id = %a_leg_call_id,
                    destination = %destination,
                    error = %error_msg,
                    "Invalid destination URI"
                );
                return self.transition_to_error(
                    a_leg_call_id,
                    &error_msg,
                    on_error,
                    next,
                ).await;
            }
        };

        info!(
            a_leg_call_id = %a_leg_call_id,
            dest_uri = %dest_uri.as_str(),
            "Parsed destination URI for B-leg"
        );

        // Create UserAgentClient for B-leg
        let uac = UserAgentClient::new(local_uri.clone(), contact_uri.clone());

        // TODO: Override From header with caller_id if provided
        // Currently uses local_uri as From header

        // Generate minimal SDP offer for B-leg
        // TODO: Phase 3 - Use MediaManager to generate proper SDP based on A-leg capabilities
        let sdp_offer = self.generate_minimal_sdp();

        info!(
            a_leg_call_id = %a_leg_call_id,
            "Generated minimal SDP offer for B-leg INVITE"
        );

        // Create INVITE request with SDP
        let invite = uac.create_invite(&dest_uri, Some(&sdp_offer));

        info!(
            a_leg_call_id = %a_leg_call_id,
            from = %local_uri.as_str(),
            to = %dest_uri.as_str(),
            "Sending B-leg INVITE"
        );

        // Phase 2: Send INVITE and handle responses

        // Parse destination address for TransportContext
        // TODO: Phase 5 - Use proper DNS resolution (RFC 3263) instead of basic parsing
        let host = dest_uri.host();
        let port = dest_uri.port().unwrap_or(5060);

        // For MVP, resolve host to IP address using basic DNS lookup
        let peer_addr: SocketAddr = match tokio::net::lookup_host(format!("{}:{}", host, port))
            .await
            .and_then(|mut addrs| addrs.next().ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No addresses found for host"
            ))) {
            Ok(addr) => addr,
            Err(e) => {
                let error_msg = format!("Failed to resolve destination host {}: {}", host, e);
                error!(
                    a_leg_call_id = %a_leg_call_id,
                    host = %host,
                    port = %port,
                    error = %e,
                    "DNS resolution failed for B-leg destination"
                );
                return self.transition_to_error(
                    a_leg_call_id,
                    &error_msg,
                    on_error,
                    next,
                ).await;
            }
        };

        info!(
            a_leg_call_id = %a_leg_call_id,
            peer = %peer_addr,
            "Resolved B-leg destination address"
        );

        // Create transport context (MVP: use UDP)
        // TODO: Phase 5 - Support TCP/TLS based on URI scheme or transport parameter
        let ctx = TransportContext::new(
            sip_transaction::fsm::TransportKind::Udp,
            peer_addr,
            None, // No TCP/TLS stream for UDP
        );

        // Create channels for response handling
        let (prov_tx, mut prov_rx) = mpsc::channel::<Response>(16);
        let (final_tx, final_rx) = oneshot::channel::<Response>();
        let (term_tx, term_rx) = oneshot::channel::<String>();

        // Create transaction user (recreate UAC since it doesn't implement Clone)
        let tu_uac = UserAgentClient::new(local_uri.clone(), contact_uri.clone());
        let tu = Arc::new(DialTransactionUser {
            prov_tx,
            final_tx: Mutex::new(Some(final_tx)),
            term_tx: Mutex::new(Some(term_tx)),
            uac: tu_uac,
            request: invite.clone(),
            ctx: ctx.clone(),
        });

        // Start client transaction (sends INVITE)
        let _tx_key = transaction_manager
            .start_client_transaction(invite.clone(), ctx, tu)
            .await
            .map_err(|e| {
                error!(
                    a_leg_call_id = %a_leg_call_id,
                    error = %e,
                    "Failed to start B-leg client transaction"
                );
                e
            })?;

        info!(
            a_leg_call_id = %a_leg_call_id,
            "B-leg INVITE sent, waiting for responses (timeout: {}s)",
            timeout_secs
        );

        // Wait for responses with timeout
        let timeout_duration = std::time::Duration::from_secs(timeout_secs as u64);
        let mut dial_status = String::from("no_answer");
        let mut dial_sip_code: Option<u16> = None;
        let mut dial_sip_reason: Option<String> = None;
        let mut next_node = on_no_answer.as_ref().or(on_error.as_ref()).unwrap_or(&next).clone();

        // Response event loop
        tokio::select! {
            // Handle provisional responses
            _ = async {
                while let Some(response) = prov_rx.recv().await {
                    let code = response.code();
                    info!(
                        a_leg_call_id = %a_leg_call_id,
                        code = %code,
                        "B-leg provisional response"
                    );

                    // 180 Ringing, 183 Session Progress, etc. - just log for now
                    // TODO: Phase 5 - Forward ringing to A-leg, handle early media
                }
            } => {
                // All provisionals received, waiting for final
            }

            // Handle final response
            result = final_rx => {
                match result {
                    Ok(response) => {
                        let code = response.code();
                        dial_sip_code = Some(code);
                        dial_sip_reason = Some(response.reason().to_string());

                        info!(
                            a_leg_call_id = %a_leg_call_id,
                            code = %code,
                            reason = %response.reason(),
                            "B-leg final response received"
                        );

                        // Handle based on response class
                        if code >= 200 && code < 300 {
                            // 2xx Success - call answered
                            dial_status = String::from("answered");
                            next_node = next;

                            // Phase 3: Extract SDP answer, create B-leg media session, bridge calls
                            info!(
                                a_leg_call_id = %a_leg_call_id,
                                code = %code,
                                "B-leg answered, bridging media"
                            );

                            // Extract SDP from response body
                            let sdp_answer = String::from_utf8_lossy(response.body()).to_string();

                            if !sdp_answer.is_empty() && sdp_answer.starts_with("v=0") {
                                info!(
                                    a_leg_call_id = %a_leg_call_id,
                                    sdp_len = %sdp_answer.len(),
                                    "Extracted SDP answer from 200 OK"
                                );

                                // TODO: Phase 3.1 - Create B-leg dialog_id from response headers
                                // For MVP, create a temporary DialogId
                                let b_leg_dialog_id = fcp_core::DialogId::new();

                                // Create B-leg Call ID (for media session tracking)
                                // Note: MediaManager expects fcp_core::CallId, not call::CallId
                                let b_leg_call_id_media = fcp_core::CallId::new();

                                // Bridge media if MediaManager is available
                                if let Some(ref media_mgr) = self.media_manager {
                                    // Create B-leg media session
                                    match media_mgr.create_session_default(b_leg_call_id_media, b_leg_dialog_id).await {
                                        Ok(_b_leg_session) => {
                                            info!(
                                                a_leg_call_id = %a_leg_call_id,
                                                b_leg_call_id = %b_leg_call_id_media,
                                                "Created B-leg media session"
                                            );

                                            // Update B-leg session with SDP answer
                                            match media_mgr.update_session(&b_leg_call_id_media, sdp_answer).await {
                                                Ok(_local_sdp) => {
                                                    info!(
                                                        a_leg_call_id = %a_leg_call_id,
                                                        b_leg_call_id = %b_leg_call_id_media,
                                                        "Updated B-leg session with SDP answer"
                                                    );

                                                    // Bridge the two call legs
                                                    // Convert a_leg_call_id from call::CallId to fcp_core::CallId
                                                    let a_leg_call_id_core = fcp_core::CallId::from_uuid(*a_leg_call_id.as_uuid());
                                                    match media_mgr.bridge_calls(&a_leg_call_id_core, &b_leg_call_id_media).await {
                                                        Ok(()) => {
                                                            info!(
                                                                a_leg_call_id = %a_leg_call_id,
                                                                b_leg_call_id = %b_leg_call_id_media,
                                                                "Media bridge established between A-leg and B-leg"
                                                            );

                                                            // Store B-leg call ID for later cleanup
                                                            let context_entry = self.contexts.get(&a_leg_call_id);
                                                            if let Some(entry) = context_entry {
                                                                let context = Arc::clone(entry.value());
                                                                drop(entry);
                                                                let mut context_guard = context.write().await;
                                                                context_guard.set_variable("b_leg_call_id".to_string(), b_leg_call_id_media.to_string());
                                                                drop(context_guard);
                                                            }
                                                        }
                                                        Err(e) => {
                                                            error!(
                                                                a_leg_call_id = %a_leg_call_id,
                                                                b_leg_call_id = %b_leg_call_id_media,
                                                                error = %e,
                                                                "Failed to bridge media sessions"
                                                            );
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    error!(
                                                        a_leg_call_id = %a_leg_call_id,
                                                        b_leg_call_id = %b_leg_call_id_media,
                                                        error = %e,
                                                        "Failed to update B-leg session with SDP"
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!(
                                                a_leg_call_id = %a_leg_call_id,
                                                b_leg_call_id = %b_leg_call_id_media,
                                                error = %e,
                                                "Failed to create B-leg media session"
                                            );
                                        }
                                    }
                                } else {
                                    warn!(
                                        a_leg_call_id = %a_leg_call_id,
                                        "MediaManager not available, skipping media bridging"
                                    );
                                }
                            } else {
                                warn!(
                                    a_leg_call_id = %a_leg_call_id,
                                    "200 OK response has no SDP body, cannot bridge media"
                                );
                            }
                        } else if code == 486 || code == 600 {
                            // 486 Busy Here, 600 Busy Everywhere
                            dial_status = String::from("busy");
                            next_node = on_busy.unwrap_or_else(|| next.clone());
                        } else if code == 408 || code == 480 || code == 487 {
                            // 408 Request Timeout, 480 Temporarily Unavailable, 487 Request Terminated
                            dial_status = String::from("no_answer");
                            next_node = on_no_answer.unwrap_or_else(|| next.clone());
                        } else {
                            // Other 4xx/5xx/6xx - generic failure
                            dial_status = String::from("failed");
                            next_node = on_error.unwrap_or_else(|| next.clone());
                        }
                    }
                    Err(_) => {
                        warn!(
                            a_leg_call_id = %a_leg_call_id,
                            "Final response channel closed unexpectedly"
                        );
                        dial_status = String::from("error");
                        next_node = on_error.unwrap_or_else(|| next.clone());
                    }
                }
            }

            // Handle transaction termination
            result = term_rx => {
                match result {
                    Ok(reason) => {
                        warn!(
                            a_leg_call_id = %a_leg_call_id,
                            reason = %reason,
                            "B-leg transaction terminated without final response"
                        );
                        dial_status = String::from("failed");
                        dial_sip_reason = Some(reason);
                        next_node = on_error.unwrap_or_else(|| next.clone());
                    }
                    Err(_) => {
                        warn!(
                            a_leg_call_id = %a_leg_call_id,
                            "Termination channel closed unexpectedly"
                        );
                        dial_status = String::from("error");
                        next_node = on_error.unwrap_or_else(|| next.clone());
                    }
                }
            }

            // Handle timeout
            _ = tokio::time::sleep(timeout_duration) => {
                warn!(
                    a_leg_call_id = %a_leg_call_id,
                    timeout_secs = %timeout_secs,
                    "B-leg INVITE timeout"
                );
                dial_status = String::from("no_answer");
                next_node = on_no_answer.unwrap_or_else(|| next.clone());

                // TODO: Phase 2.1 - Send CANCEL to terminate the INVITE transaction
            }
        }

        // Set flow variables with dial results
        let context_entry = self.contexts.get(&a_leg_call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found after dial"))?;
        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let mut context_guard = context.write().await;
        context_guard.set_variable("dial_status".to_string(), dial_status.clone());
        context_guard.set_variable("dial_destination".to_string(), destination);

        if let Some(code) = dial_sip_code {
            context_guard.set_variable("dial_sip_code".to_string(), code.to_string());
        }

        if let Some(reason) = dial_sip_reason {
            context_guard.set_variable("dial_sip_reason".to_string(), reason);
        }

        context_guard.current_node = next_node;
        drop(context_guard);

        info!(
            a_leg_call_id = %a_leg_call_id,
            dial_status = %dial_status,
            "Dial node complete, transitioning to next node"
        );

        Box::pin(self.execute_next_node(a_leg_call_id)).await
    }

    /// Helper: Generate minimal SDP offer for B-leg INVITE.
    ///
    /// TODO: Phase 3 - Replace with proper SDP generation from MediaManager
    /// based on A-leg SDP capabilities (codecs, media types, etc.)
    fn generate_minimal_sdp(&self) -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        format!(
            "v=0\r\n\
             o=- {} 1 IN IP4 127.0.0.1\r\n\
             s=FCP B2BUA\r\n\
             c=IN IP4 127.0.0.1\r\n\
             t=0 0\r\n\
             m=audio 8000 RTP/AVP 0 8\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n",
            timestamp
        )
    }

    /// Helper: Transition to error node or next node on failure.
    async fn transition_to_error(
        &self,
        call_id: CallId,
        error_msg: &str,
        on_error: Option<String>,
        next: String,
    ) -> Result<()> {
        let context_entry = self.contexts.get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let mut context_guard = context.write().await;
        context_guard.set_variable("dial_status".to_string(), "error".to_string());
        context_guard.set_variable("dial_error".to_string(), error_msg.to_string());

        if let Some(error_node) = on_error {
            context_guard.current_node = error_node;
        } else {
            context_guard.current_node = next;
        }
        drop(context_guard);

        Box::pin(self.execute_next_node(call_id)).await
    }

    pub async fn execute_next_node(&self, call_id: CallId) -> Result<()> {
        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("No flow context for call: {}", call_id))?;

        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let mut context_guard = context.write().await;
        let current_node = context_guard.current_node.clone();

        debug!(
            call_id = %call_id,
            node_id = %current_node,
            "Executing flow node"
        );

        // Check if this is an Answer or Hangup node - execute callbacks
        let node_type = context_guard
            .flow
            .nodes
            .get(&current_node)
            .map(|n| n.node_type.clone());

        if let Some(node_type) = node_type {
            use fcp_flow::NodeType;

            match node_type {
                NodeType::Answer { next } => {
                    info!(
                        call_id = %call_id,
                        node_id = %current_node,
                        "Answer node: calling answer callback"
                    );

                    // Clone callback before dropping guard
                    let callback = context_guard.answer_callback.clone();
                    drop(context_guard);

                    // Execute answer callback if available
                    if let Some(callback) = callback {
                        match callback().await {
                            Ok(()) => {
                                info!(call_id = %call_id, "Call answered successfully");
                            }
                            Err(e) => {
                                error!(call_id = %call_id, error = %e, "Failed to answer call");
                            }
                        }

                        // Re-acquire lock and continue to next node
                        let mut context_guard = context.write().await;
                        context_guard.current_node = next.clone();
                        drop(context_guard);

                        return Box::pin(self.execute_next_node(call_id)).await;
                    } else {
                        warn!(call_id = %call_id, "Answer callback not set");
                        let mut context_guard = context.write().await;
                        context_guard.current_node = next.clone();
                        drop(context_guard);
                        return Box::pin(self.execute_next_node(call_id)).await;
                    }
                }

                NodeType::Hangup { reason } => {
                    info!(
                        call_id = %call_id,
                        node_id = %current_node,
                        reason = ?reason,
                        "Hangup node: calling hangup callback"
                    );

                    // Clone callback before dropping guard
                    let callback = context_guard.hangup_callback.clone();
                    drop(context_guard);

                    // Execute hangup callback if available
                    if let Some(callback) = callback {
                        match callback(reason).await {
                            Ok(()) => {
                                info!(call_id = %call_id, "Call hung up successfully");
                            }
                            Err(e) => {
                                error!(call_id = %call_id, error = %e, "Failed to hang up call");
                            }
                        }

                        // Clean up context
                        self.contexts.remove(&call_id);
                        return Ok(());
                    } else {
                        warn!(call_id = %call_id, "Hangup callback not set");
                        self.contexts.remove(&call_id);
                        return Ok(());
                    }
                }

                _ => {
                    // Not Answer or Hangup, continue with normal processing
                }
            }
        }

        // Check if this is a Play or Say node - handle media playback before execution
        if let Some(media_manager) = &self.media_manager {
            if let Some(node) = context_guard.flow.nodes.get(&current_node) {
                use fcp_flow::NodeType;

                match &node.node_type {
                    NodeType::Play { url, next, .. } => {
                        info!(
                            call_id = %call_id,
                            node_id = %current_node,
                            url = %url,
                            "Playing audio file"
                        );

                        // TODO: Interpolate variables in URL (requires implementing interpolation)
                        // For now, use raw URL value
                        let url_value = url.clone();

                        // Use media session ID from context (not SIP call ID)
                        let session_id = context_guard.media_session_id.ok_or_else(|| {
                            anyhow::anyhow!("No media session ID available for playback")
                        })?;

                        // Start playback to ParticipantA (the caller in IVR mode)
                        // Note: Using ParticipantA instead of Both to avoid echo in IVR scenarios
                        match media_manager
                            .play_file(&session_id, url_value.clone(), AudioTarget::ParticipantA)
                            .await
                        {
                            Ok(playback_id) => {
                                info!(
                                    call_id = %call_id,
                                    playback_id = %playback_id,
                                    "Audio playback started"
                                );

                                // Clone next node ID before dropping guard
                                let next_node = next.clone();

                                // Drop context guard before waiting for completion
                                drop(context_guard);

                                // Wait for playback to complete before continuing
                                match media_manager.wait_playback_completion(playback_id.clone()).await
                                {
                                    Ok(()) => {
                                        info!(
                                            call_id = %call_id,
                                            playback_id = %playback_id,
                                            "Audio playback completed"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            call_id = %call_id,
                                            playback_id = %playback_id,
                                            error = %e,
                                            "Audio playback failed or was stopped"
                                        );
                                    }
                                }

                                // Re-acquire lock and continue to next node
                                let mut context_guard = context.write().await;
                                context_guard.current_node = next_node;
                                drop(context_guard);

                                // Continue to next node
                                return Box::pin(self.execute_next_node(call_id)).await;
                            }
                            Err(e) => {
                                error!(
                                    call_id = %call_id,
                                    error = %e,
                                    "Failed to play audio file"
                                );

                                // Continue to next node anyway
                                context_guard.current_node = next.clone();
                                drop(context_guard);

                                return Box::pin(self.execute_next_node(call_id)).await;
                            }
                        }
                    }

                    NodeType::Say {
                        text,
                        language,
                        voice,
                        next,
                        ..
                    } => {
                        info!(
                            call_id = %call_id,
                            node_id = %current_node,
                            text = %text,
                            "Playing text-to-speech"
                        );

                        // TODO: Interpolate variables in text (requires implementing interpolation)
                        // For now, use raw values
                        let text_value = text.clone();
                        let language_value = language.clone();
                        let voice_value = voice.clone();

                        // Use media session ID from context (not SIP call ID)
                        let session_id = context_guard.media_session_id.ok_or_else(|| {
                            anyhow::anyhow!("No media session ID available for TTS playback")
                        })?;

                        // Start TTS playback
                        match media_manager
                            .play_tts(
                                &session_id,
                                text_value,
                                language_value,
                                voice_value,
                                AudioTarget::Both,
                            )
                            .await
                        {
                            Ok(playback_id) => {
                                info!(
                                    call_id = %call_id,
                                    playback_id = %playback_id,
                                    "TTS playback started"
                                );

                                // Clone next node ID before dropping guard
                                let next_node = next.clone();

                                // Drop context guard before waiting for completion
                                drop(context_guard);

                                // Wait for playback to complete before continuing
                                match media_manager.wait_playback_completion(playback_id.clone()).await
                                {
                                    Ok(()) => {
                                        info!(
                                            call_id = %call_id,
                                            playback_id = %playback_id,
                                            "TTS playback completed"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            call_id = %call_id,
                                            playback_id = %playback_id,
                                            error = %e,
                                            "TTS playback failed or was stopped"
                                        );
                                    }
                                }

                                // Re-acquire lock and continue to next node
                                let mut context_guard = context.write().await;
                                context_guard.current_node = next_node;
                                drop(context_guard);

                                return Box::pin(self.execute_next_node(call_id)).await;
                            }
                            Err(e) => {
                                // TTS not configured yet - log and continue
                                warn!(
                                    call_id = %call_id,
                                    error = %e,
                                    "TTS not available, continuing flow"
                                );

                                // Continue to next node
                                context_guard.current_node = next.clone();
                                drop(context_guard);

                                return Box::pin(self.execute_next_node(call_id)).await;
                            }
                        }
                    }

                    _ => {
                        // Not a media node, continue with normal execution
                    }
                }
            }
        }

        // Normal node execution (non-media nodes or when media_manager not available)
        let result = self
            .engine
            .execute_node(&mut context_guard, &current_node)
            .await?;

        // Publish node executed event
        if let Some(events) = &self.events {
            let event = PlatformEvent::Flow(FlowEvent::NodeExecuted {
                call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                flow_id: context_guard.flow.id.clone(),
                node_id: current_node.clone(),
                timestamp: std::time::SystemTime::now(),
            });

            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    error!(error = %e, "Failed to publish FlowEvent::NodeExecuted");
                }
            });
        }

        match result {
            FlowExecutionResult::Continue(next_node) => {
                info!(
                    call_id = %call_id,
                    current = %current_node,
                    next = %next_node,
                    "Flow continuing to next node"
                );

                context_guard.current_node = next_node;
                drop(context_guard);

                // Recursively execute next node
                Box::pin(self.execute_next_node(call_id)).await?;
            }

            FlowExecutionResult::Hangup(reason) => {
                info!(
                    call_id = %call_id,
                    reason = ?reason,
                    "Flow requested hangup"
                );

                // Publish flow completed event
                if let Some(events) = &self.events {
                    let event = PlatformEvent::Flow(FlowEvent::Completed {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        flow_id: context_guard.flow.id.clone(),
                        outcome: "hangup".to_string(),
                        timestamp: std::time::SystemTime::now(),
                    });

                    let events = Arc::clone(events);
                    tokio::spawn(async move {
                        if let Err(e) = events.publish(event).await {
                            error!(error = %e, "Failed to publish FlowEvent::Completed");
                        }
                    });
                }

                // Clean up context
                drop(context_guard);
                self.contexts.remove(&call_id);

                // TODO: Send BYE to hangup the call
                warn!(call_id = %call_id, "Flow requested hangup, but SIP hangup not yet implemented");
            }

            FlowExecutionResult::WaitingForInput {
                variable,
                transitions,
                default,
            } => {
                info!(
                    call_id = %call_id,
                    variable = %variable,
                    "Flow waiting for user input (DTMF)"
                );

                // Get gather configuration from context
                let num_digits = context_guard
                    .get_variable("__gather_num_digits")
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1);

                let timeout_secs = context_guard
                    .get_variable("__gather_timeout")
                    .ok()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(5);

                let finish_on_key = context_guard
                    .get_variable("__gather_finish_on_key")
                    .ok()
                    .and_then(|s| s.chars().next());

                let on_timeout_node = context_guard
                    .get_variable("__gather_on_timeout")
                    .ok();

                // Check for prompt
                let prompt_url = context_guard
                    .get_variable("__gather_prompt_url")
                    .ok();

                let media_session_id = context_guard.media_session_id;
                let flow_id = context_guard.flow.id.clone();

                drop(context_guard);

                // Publish waiting for input event
                if let Some(events) = &self.events {
                    let event = PlatformEvent::Flow(FlowEvent::WaitingForInput {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        flow_id,
                        variable: variable.clone(),
                        timeout_secs,
                        timestamp: std::time::SystemTime::now(),
                    });

                    let events = Arc::clone(events);
                    tokio::spawn(async move {
                        if let Err(e) = events.publish(event).await {
                            error!(error = %e, "Failed to publish FlowEvent::WaitingForInput");
                        }
                    });
                }

                // Play prompt if available
                if let (Some(prompt_url), Some(media_manager), Some(session_id)) = (
                    prompt_url,
                    &self.media_manager,
                    media_session_id,
                ) {
                    info!(
                        call_id = %call_id,
                        prompt_url = %prompt_url,
                        "Playing Gather prompt"
                    );

                    match media_manager
                        .play_file(&session_id, prompt_url.clone(), fcp_media::AudioTarget::ParticipantA)
                        .await
                    {
                        Ok(playback_id) => {
                            // Wait for prompt playback to complete
                            if let Err(e) = media_manager.wait_playback_completion(playback_id.clone()).await {
                                warn!(
                                    call_id = %call_id,
                                    error = %e,
                                    "Gather prompt playback failed"
                                );
                            } else {
                                info!(call_id = %call_id, "Gather prompt playback complete");
                            }
                        }
                        Err(e) => {
                            warn!(
                                call_id = %call_id,
                                error = %e,
                                "Failed to play Gather prompt"
                            );
                        }
                    }
                }

                // Create DTMF collector
                if let Some(media_manager) = &self.media_manager {
                    let collector = media_manager.create_dtmf_collector(num_digits, finish_on_key);

                    info!(
                        call_id = %call_id,
                        num_digits = num_digits,
                        timeout_secs = timeout_secs,
                        finish_on_key = ?finish_on_key,
                        "Starting DTMF collection"
                    );

                    // TODO: Wire up DTMF events from media backend to collector
                    // For now, we simulate a timeout scenario
                    let result = collector
                        .collect(std::time::Duration::from_secs(timeout_secs as u64))
                        .await;

                    info!(
                        call_id = %call_id,
                        result = ?result,
                        "DTMF collection completed"
                    );

                    // Process collection result
                    match result {
                        fcp_media::CollectionResult::Complete(digits) => {
                            // Store collected digits in variable
                            let mut context_guard = context.write().await;
                            context_guard.set_variable(variable.clone(), digits.clone());

                            // Determine next node based on transitions
                            let next_node = if let Some(next) = transitions.get(&digits) {
                                next.clone()
                            } else if let Some(default_node) = &default {
                                default_node.clone()
                            } else {
                                // No transition found and no default - stay on current node
                                warn!(
                                    call_id = %call_id,
                                    digits = %digits,
                                    "No transition found for collected digits"
                                );
                                context_guard.current_node.clone()
                            };

                            info!(
                                call_id = %call_id,
                                digits = %digits,
                                next_node = %next_node,
                                "DTMF collection complete, continuing to next node"
                            );

                            context_guard.current_node = next_node;
                            drop(context_guard);

                            // Continue to next node
                            Box::pin(self.execute_next_node(call_id)).await?;
                        }
                        fcp_media::CollectionResult::Timeout(partial_digits) => {
                            // Store partial digits if any
                            let mut context_guard = context.write().await;
                            if !partial_digits.is_empty() {
                                context_guard.set_variable(variable.clone(), partial_digits.clone());
                            }

                            // Go to timeout node if specified, otherwise use default
                            let next_node = if let Some(timeout_node) = on_timeout_node {
                                timeout_node
                            } else if let Some(default_node) = default {
                                default_node
                            } else {
                                warn!(
                                    call_id = %call_id,
                                    "DTMF collection timeout with no timeout handler"
                                );
                                context_guard.current_node.clone()
                            };

                            warn!(
                                call_id = %call_id,
                                partial_digits = %partial_digits,
                                next_node = %next_node,
                                "DTMF collection timeout, continuing to timeout node"
                            );

                            context_guard.current_node = next_node;
                            drop(context_guard);

                            // Continue to next node
                            Box::pin(self.execute_next_node(call_id)).await?;
                        }
                        fcp_media::CollectionResult::Cancelled => {
                            warn!(call_id = %call_id, "DTMF collection cancelled");
                            // Don't continue execution - call likely terminated
                        }
                    }
                } else {
                    warn!(
                        call_id = %call_id,
                        "Media manager not available for DTMF collection"
                    );
                }
            }

            FlowExecutionResult::WaitingForMedia => {
                // Handle Say and Play nodes that need media playback
                self.handle_media_playback(call_id).await?;
            }

            FlowExecutionResult::WaitingForDial {
                destination,
                timeout_secs,
                caller_id,
                max_duration_secs,
                on_busy,
                on_no_answer,
                on_error,
                next,
            } => {
                info!(
                    call_id = %call_id,
                    destination = %destination,
                    timeout_secs = timeout_secs,
                    max_duration_secs = max_duration_secs,
                    "Starting outbound call (Dial node)"
                );

                // Clone error/next node for fallback error handling
                let error_fallback = on_error.clone();
                let next_fallback = next.clone();

                // Handle the dial operation
                match self.handle_dial(
                    call_id,
                    destination,
                    timeout_secs,
                    caller_id,
                    max_duration_secs,
                    on_busy,
                    on_no_answer,
                    on_error,
                    next,
                ).await {
                    Ok(()) => {
                        // Dial completed successfully, flow continues
                    }
                    Err(e) => {
                        error!(
                            call_id = %call_id,
                            error = %e,
                            "Dial node failed"
                        );

                        // Store error and transition to error node or next
                        let mut context_guard = context.write().await;
                        context_guard.set_variable("dial_status".to_string(), "error".to_string());
                        context_guard.set_variable("dial_error".to_string(), e.to_string());

                        if let Some(error_node) = error_fallback {
                            context_guard.current_node = error_node;
                        } else {
                            context_guard.current_node = next_fallback;
                        }
                        drop(context_guard);

                        Box::pin(self.execute_next_node(call_id)).await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle DTMF input from the caller.
    ///
    /// This is called when DTMF digits are received during a Gather node.
    pub async fn handle_dtmf_input(&self, call_id: CallId, digits: String) -> Result<()> {
        info!(
            call_id = %call_id,
            digits = %digits,
            "Received DTMF input"
        );

        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("No flow context for call: {}", call_id))?;

        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let mut context_guard = context.write().await;

        // TODO: Process DTMF input based on current Gather node
        // For now, just store the digits as a variable
        context_guard.set_variable("dtmf_input".to_string(), digits.clone());

        drop(context_guard);

        // Continue execution
        self.execute_next_node(call_id).await?;

        Ok(())
    }

    /// Stop flow execution for a call.
    pub async fn stop_flow(&self, call_id: CallId) -> Result<()> {
        info!(call_id = %call_id, "Stopping flow execution");

        if let Some((_, context)) = self.contexts.remove(&call_id) {
            let context_guard = context.read().await;

            // Publish flow stopped event
            if let Some(events) = &self.events {
                let event = PlatformEvent::Flow(FlowEvent::Completed {
                    call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                    flow_id: context_guard.flow.id.clone(),
                    outcome: "stopped".to_string(),
                    timestamp: std::time::SystemTime::now(),
                });

                let events = Arc::clone(events);
                tokio::spawn(async move {
                    if let Err(e) = events.publish(event).await {
                        error!(error = %e, "Failed to publish FlowEvent::Completed");
                    }
                });
            }
        }

        Ok(())
    }

    /// Get the current flow context for a call.
    pub async fn get_context(&self, call_id: &CallId) -> Option<Arc<RwLock<FlowContext>>> {
        self.contexts.get(call_id).map(|entry| Arc::clone(entry.value()))
    }
}

/// Transaction user for handling Dial node INVITE responses.
///
/// This struct implements ClientTransactionUser to receive callbacks
/// when responses arrive for the outbound B-leg INVITE.
struct DialTransactionUser {
    /// Send provisional responses (1xx) to the handler
    prov_tx: mpsc::Sender<Response>,
    /// Send final response (2xx-6xx) to the handler
    final_tx: Mutex<Option<oneshot::Sender<Response>>>,
    /// Send termination reason to the handler
    term_tx: Mutex<Option<oneshot::Sender<String>>>,
    /// UserAgentClient helper for creating ACK
    uac: UserAgentClient,
    /// Original INVITE request (for creating ACK)
    request: Request,
    /// Transport context for sending ACK
    ctx: TransportContext,
}

#[async_trait]
impl ClientTransactionUser for DialTransactionUser {
    async fn on_provisional(&self, _key: &TransactionKey, response: &Response) {
        info!("B-leg provisional response: {}", response.code());

        // Forward provisional response to handler (non-blocking)
        let _ = self.prov_tx.send(response.clone()).await;
    }

    async fn on_final(&self, _key: &TransactionKey, response: &Response) {
        info!("B-leg final response: {}", response.code());

        // Send final response to handler via oneshot
        let mut tx = self.final_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send(response.clone());
        }
    }

    async fn on_terminated(&self, _key: &TransactionKey, reason: &str) {
        warn!("B-leg transaction terminated: {}", reason);

        // Send termination reason to handler
        let mut tx = self.term_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send(reason.to_string());
        }
    }

    async fn send_ack(
        &self,
        _key: &TransactionKey,
        response: Response,
        _ctx: &TransportContext,
        is_2xx: bool,
    ) {
        info!(
            "Sending ACK for {} response (is_2xx={})",
            response.code(),
            is_2xx
        );

        // Create ACK using UAC helper
        let _ack = self.uac.create_ack(&self.request, &response, None);

        // Send ACK via transport (for now, log that we would send it)
        // TODO: Phase 3 - Actually send ACK via dispatcher when we have full transport integration
        debug!(
            "ACK prepared for B-leg response {} (actual sending pending transport integration)",
            response.code()
        );
    }

    async fn send_prack(
        &self,
        _key: &TransactionKey,
        response: Response,
        _ctx: &TransportContext,
    ) {
        debug!("PRACK for reliable provisional {} (not implemented for Dial node MVP)", response.code());
        // TODO: Phase 5 - Implement PRACK for reliable provisionals if needed
    }

    async fn on_transport_error(&self, _key: &TransactionKey) {
        error!("B-leg transport error occurred");

        // Treat transport error as transaction termination
        let mut tx = self.term_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send("Transport error".to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_flow::InMemoryFlowStore;

    #[tokio::test]
    async fn test_flow_routing() {
        let store = Arc::new(InMemoryFlowStore::new());
        let handler = FlowCallHandler::new(store, None, None);

        handler.add_route("+15551234567".to_string(), "test-flow".to_string()).await;

        let flow_id = handler.get_flow_for_did("+15551234567").await;
        assert_eq!(flow_id, Some("test-flow".to_string()));

        let no_route = handler.get_flow_for_did("+15559999999").await;
        assert_eq!(no_route, None);
    }
}
