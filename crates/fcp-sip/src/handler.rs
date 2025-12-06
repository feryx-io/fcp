//! Message handler for routing inbound SIP messages.

use anyhow::Result;
use bytes::Bytes;
use smol_str::SmolStr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::path::PathBuf;
use std::fs::OpenOptions;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;
use sip_sdp::parse::parse_sdp;
use sip_dialog::session_timer_manager::{SessionTimerEvent, SessionTimerManager};
use tracing::{debug, error, info, warn};

use crate::prelude::*;
use crate::registrar::FcpRegistrar;
use crate::metrics::MetricsHandle;
use crate::settings::ServerSettings;
use sip_transaction::ServerTransactionHandle;

/// Routes inbound SIP messages to the transaction manager.
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
    call_manager: Arc<crate::call_manager::CallManager>,

    /// User Agent Client for making outbound calls (B-leg)
    uac: Option<Arc<SipUserAgentClient>>,

    /// Per-dialog lock to serialize mid-dialog requests (re-INVITE/UPDATE) to avoid glare
    mid_dialog_lock: Arc<dashmap::DashMap<sip_dialog::DialogId, Arc<tokio::sync::Mutex<()>>>>,

    /// Subscription manager for REFER/NOTIFY tracking
    subscription_manager: Arc<sip_dialog::SubscriptionManager>,
    /// Index of subscriptions by dialog/event for quick lookup
    subscription_index: Arc<
        dashmap::DashMap<(sip_dialog::DialogId, smol_str::SmolStr), sip_dialog::SubscriptionId>,
    >,

    /// Session timer manager for RFC 4028 enforcement
    session_timer_manager: Arc<SessionTimerManager>,

    /// Metrics sink
    metrics: MetricsHandle,

    /// Optional transcript directory for SIP message capture
    transcript_dir: Option<PathBuf>,

    /// Channel for async transcript writes (background task)
    transcript_tx: Option<mpsc::UnboundedSender<TranscriptEntry>>,

    /// Settings
    settings: ServerSettings,

    /// Per-dialog rate limits
    rate_limits: Arc<dashmap::DashMap<sip_dialog::DialogId, RateWindow>>,
}

#[derive(Clone)]
struct RateWindow {
    window_start: Instant,
    counts: HashMap<&'static str, u32>,
}

/// Entry for async transcript logging
struct TranscriptEntry {
    call_id: String,
    timestamp: std::time::SystemTime,
    direction: String,
    method_or_status: String,
    dialog_id_str: String,
    cseq: String,
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
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        dialog_manager: Arc<DialogManager>,
        local_uri: sip_core::SipUri,
        media_profile: Option<sip_sdp::profiles::MediaProfileBuilder>,
        local_addr: String,
        rtp_port: u16,
        registrar: Option<Arc<FcpRegistrar>>,
        routing_service: Option<Arc<fcp_routing::RoutingService>>,
        call_manager: Arc<crate::call_manager::CallManager>,
        uac: Option<Arc<SipUserAgentClient>>,
    ) -> Self {
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
            settings,
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
        call_manager: Arc<crate::call_manager::CallManager>,
        uac: Option<Arc<SipUserAgentClient>>,
        settings: ServerSettings,
    ) -> Self {
        // Validate settings early to catch configuration errors at startup
        if let Err(e) = settings.validate() {
            panic!("Invalid ServerSettings: {}", e);
        }

        // Create User Agent Server helper with shared dialog manager
        let mut uas = sip_uas::UserAgentServer::new(local_uri.clone(), local_uri);
        uas.dialog_manager = Arc::clone(&dialog_manager);

        let subscription_manager = Arc::new(sip_dialog::SubscriptionManager::new());
        let session_timer_manager = Arc::new(SessionTimerManager::new());

        // Initialize transcript background writer if directory configured
        let transcript_tx = if let Some(ref dir) = settings.transcript_dir {
            let (tx, mut rx) = mpsc::unbounded_channel::<TranscriptEntry>();
            let transcript_dir = dir.clone();

            tokio::spawn(async move {
                use std::collections::HashMap;
                use std::io::Write;

                // Keep file handles open per call_id for efficient writing
                let mut file_handles: HashMap<String, std::fs::File> = HashMap::new();

                while let Some(entry) = rx.recv().await {
                    let filename = format!("{}.log", entry.call_id.replace(':', "_"));
                    let path = transcript_dir.join(&filename);

                    // Get or create file handle for this call_id
                    let file = file_handles.entry(entry.call_id.clone()).or_insert_with(|| {
                        OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                            .expect("Failed to open transcript file")
                    });

                    // Write entry
                    let _ = writeln!(
                        file,
                        "{:?} {} dialog={} cseq={}",
                        entry.timestamp,
                        entry.direction,
                        entry.dialog_id_str,
                        entry.cseq
                    );

                    // Flush to ensure data is written
                    let _ = file.flush();

                    // Limit cache size - close files for old calls (simple LRU)
                    if file_handles.len() > 100 {
                        // Keep most recent 50, close others
                        let mut keys: Vec<_> = file_handles.keys().cloned().collect();
                        keys.sort();
                        for key in keys.iter().take(50) {
                            file_handles.remove(key);
                        }
                    }
                }
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
            mid_dialog_lock: Arc::new(dashmap::DashMap::new()),
            subscription_manager,
            subscription_index: Arc::new(dashmap::DashMap::new()),
            session_timer_manager,
            metrics: settings.metrics.clone(),
            transcript_dir: settings.transcript_dir.clone(),
            transcript_tx,
            settings,
            rate_limits: Arc::new(dashmap::DashMap::new()),
        };

        // Start session timer event loop
        this.spawn_session_timer_listener();

        // Start session timer health monitor
        this.spawn_session_timer_monitor();

        this
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
    ) -> Result<()> {
        use sip_core::Method;

        match request.start.method {
            Method::Invite => {
                // If this is an in-dialog re-INVITE, handle separately
                if self.dialog_manager.find_by_request(request).is_some() {
                    let response = self.handle_reinvite(request).await?;
                    handle.send_final(response).await;
                } else {
                    self.handle_invite(request, handle, peer).await?;
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
                let response = self.handle_info(request).await?;
                handle.send_final(response).await;
            }
            Method::Refer => {
                let response = self.handle_refer(request).await?;
                handle.send_final(response).await;
            }
            Method::Subscribe => {
                let response = self.handle_subscribe(request).await?;
                handle.send_final(response).await;
            }
            Method::Notify => {
                let response = self.handle_notify(request).await?;
                handle.send_final(response).await;
            }
            Method::Options => {
                debug!("Responding to OPTIONS with 200 OK");
                let response = self.create_simple_response(request, 200, "OK");
                handle.send_final(response).await;
            }
            Method::Register => {
                let response = self.handle_register(request).await;
                handle.send_final(response).await;
            }
            _ => {
                debug!(
                    method = ?request.start.method,
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

        // Try to locate the related call via INVITE Call-ID and terminate it
        if let Some(call_id_hdr) = sip_parse::header(&request.headers, "Call-ID") {
            if let Some(call) = self.call_manager.find_by_invite_call_id(call_id_hdr.as_str()) {
                info!(call_id = %call.id, "CANCEL matched call; marking as failed");

                // Attempt to CANCEL the outbound B-leg transaction if it is still pending
                let dialog_manager = Arc::clone(&self.dialog_manager);
                if let Some(b_leg) = &call.b_leg {
                    // Clone the outbound handle BEFORE clearing it to avoid race condition
                    let outbound_handle_clone = b_leg.outbound_handle.clone();

                    // Clean any stored B-leg dialog in CallManager before removal
                    let _ = self.call_manager.update_call(&call.id, |c| {
                        if let Some(b) = &mut c.b_leg {
                            if let Some(dialog) = b.dialog.take() {
                                info!(
                                    call_id = %c.id,
                                    dialog_id = ?dialog.id,
                                    "Removing stored B-leg dialog due to CANCEL"
                                );
                                dialog_manager.remove(&dialog.id);
                                self.session_timer_manager.stop_timer(&dialog.id);
                            }
                            if b.outbound_handle.is_some() {
                                info!(call_id = %c.id, "Clearing B-leg outbound handle due to CANCEL");
                                b.outbound_handle = None;
                            }
                        }
                    });

                    // Use the cloned handle (not the one from the old call snapshot)
                    if let Some(outbound) = outbound_handle_clone {
                        let outbound = Arc::clone(&outbound);
                        tokio::spawn(async move {
                            let (result, dialog_id) = {
                                let guard = outbound.lock().await;
                                let dialog_id = guard.as_ref().map(|h| h.dialog.id.clone());
                                let res = if let Some(handle) = guard.as_ref() {
                                    Some(handle.cancel().await)
                                } else {
                                    None
                                };
                                (res, dialog_id)
                            };
                            if let Some(Err(e)) = result {
                                warn!(error = %e, "Failed to send CANCEL to B-leg");
                            }
                            if let Some(id) = dialog_id {
                                dialog_manager.remove(&id);
                            }
                        });
                    }
                }

                // Send 487 to the original INVITE transaction on A-leg if we still have it
                if let (Some(tx_key), Some(invite_req)) = (&call.a_leg_invite_tx, &call.original_invite) {
                    let response = self.create_cancelled_response(invite_req);
                    self.transaction_manager.send_final(tx_key, response).await;
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

        // Check routing policy if routing service is configured
        if let Some(routing_service) = &self.routing_service {
            match self.evaluate_routing_policy(request, routing_service, peer).await {
                Ok(decision) => {
                    if !decision.action.is_allow() {
                        info!(
                            reason = %decision.reason,
                            matched_rule = ?decision.matched_rule,
                            "Call denied by routing policy"
                        );
                        let response = self.create_simple_response(request, 403, &decision.reason);
                        handle.send_final(response).await;
                        return Ok(());
                    }
                    debug!(
                        reason = %decision.reason,
                        matched_rule = ?decision.matched_rule,
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

        // Create Call object from the incoming INVITE
        let mut call = crate::call::Call::from_invite(request.clone(), peer);
        call.state = crate::call::CallState::RoutingPending;
        call.a_leg_invite_tx = Some(handle.key().clone());

        // Store call in CallManager
        let call_id = self.call_manager.create_call(call);

        info!(
            call_id = %call_id,
            caller = ?sip_parse::header(&request.headers, "From"),
            callee = ?sip_parse::header(&request.headers, "To"),
            "Created B2BUA call"
        );

        // If UAC is available, forward to B-leg; otherwise use UAS mode
        if let Some(uac) = &self.uac {
            // Full B2BUA mode: Forward INVITE to B-leg
            self.forward_invite_to_b_leg(request, handle, call_id, uac.clone()).await
        } else {
            // Fallback UAS mode: Accept call directly without B-leg forwarding
            self.handle_invite_uas_mode(request, handle, call_id).await
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

        // Ensure To-tag present
        if let Some(to) = response.headers.get("To") {
            if !to.contains(";tag=") {
                let tag = Uuid::new_v4().simple().to_string();
                let new_to = SmolStr::new(format!("{};tag={}", to.as_str(), tag));
                response.headers.remove("To");
                response.headers.push(SmolStr::new("To"), new_to);
            }
        }

        // INVITE-related responses should carry Contact/Record-Route
        if request.start.method == sip_core::Method::Invite {
            if response.headers.get("Contact").is_none() {
                response.headers.push(
                    SmolStr::new("Contact"),
                    SmolStr::new(format!("<{}>", self.uas.contact_uri.as_str())),
                );
            }
            if response.headers.get("Record-Route").is_none() {
                response.headers.push(
                    SmolStr::new("Record-Route"),
                    SmolStr::new(format!("<{};lr>", self.uas.contact_uri.as_str())),
                );
            }
        }

        // Capability headers
        if response.headers.get("Allow").is_none() {
            response
                .headers
                .push(SmolStr::new("Allow"), SmolStr::new(ALLOW));
        }
        if response.headers.get("Supported").is_none() {
            response
                .headers
                .push(SmolStr::new("Supported"), SmolStr::new(SUPPORTED));
        }

        // Ensure Content-Length matches body
        response.headers.remove("Content-Length");
        response.headers.push(
            SmolStr::new("Content-Length"),
            SmolStr::new(response.body.len().to_string()),
        );
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
        call_id: crate::call::CallId,
        uac: Arc<SipUserAgentClient>,
    ) -> Result<()> {
        // Extract To header to get destination AOR
        let to = sip_parse::header(&request.headers, "To")
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

        debug!(
            call_id = %call_id,
            destination_uri = %destination_uri,
            destination_aor = %destination_aor,
            "Looking up destination in registrar"
        );

        // Update call state to LookingUpDestination
        let _ = self.call_manager.update_call(&call_id, |call| {
            call.state = crate::call::CallState::LookingUpDestination;
        });

        // Look up destination in registrar
        let destination_contacts = if let Some(registrar) = &self.registrar {
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
                    info!(
                        call_id = %call_id,
                        destination = %destination_aor,
                        "Destination not registered (no bindings)"
                    );
                    let response = self.create_simple_response(request, 480, "Temporarily Unavailable");
                    a_leg_handle.send_final(response).await;
                    let _ = self.call_manager.terminate_call(&call_id, Some("Destination not registered".to_string()));
                    return Ok(());
                }
                Err(e) => {
                    error!(
                        call_id = %call_id,
                        destination = %destination_aor,
                        error = %e,
                        "Failed to lookup destination"
                    );
                    let response = self.create_simple_response(request, 500, "Internal Server Error");
                    a_leg_handle.send_final(response).await;
                    let _ = self.call_manager.terminate_call(&call_id, Some(format!("Failed to lookup destination: {}", e)));
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
            let _ = self.call_manager.terminate_call(&call_id, Some("No registrar configured".to_string()));
            return Ok(());
        };

        // Use the contact with the highest q-value (RFC 3261 contact prioritization)
        // If q-values are equal, use the last one (likely most recent)
        let destination_contact = destination_contacts
            .iter()
            .max_by(|a, b| {
                a.q_value.partial_cmp(&b.q_value).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(&destination_contacts[0]);
        let target_uri = destination_contact.contact.to_string();

        info!(
            call_id = %call_id,
            target = %target_uri,
            "Forwarding INVITE to B-leg"
        );

        // Update call state to Inviting and create B-leg
        let _ = self.call_manager.update_call(&call_id, |call| {
            call.state = crate::call::CallState::Inviting;

            // Create B-leg CallLeg if it doesn't exist
            if call.b_leg.is_none() {
                // Extract To header for callee info
                let to_header = request.headers.get("To")
                    .map(|h| h.as_str())
                    .unwrap_or("");
                let (callee_uri, callee_display_name) = {
                    let parts: Vec<&str> = to_header.split(';').collect();
                    let name_addr = parts[0].trim();
                    if let Some(start) = name_addr.find('<') {
                        if let Some(end) = name_addr.find('>') {
                            let uri = name_addr[start+1..end].to_string();
                            let display_name = name_addr[..start].trim()
                                .trim_matches('"')
                                .to_string();
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

                call.b_leg = Some(crate::call::CallLeg::new_outbound(
                    callee_uri.clone(),
                    callee_display_name.clone()
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
        let a_leg_sdp = if !request.body.is_empty() {
            Some(String::from_utf8_lossy(&request.body).to_string())
        } else {
            None
        };

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
        uac.inner().set_display_name(caller_display_name.clone()).await;
        uac.inner().set_from_uri(caller_from_uri.clone()).await;

        // Create outbound INVITE to B-leg using UAC
        debug!(
            call_id = %call_id,
            target = %target_uri,
            has_sdp = a_leg_sdp.is_some(),
            "Creating B-leg INVITE"
        );

        let b_leg_call = match uac.invite(&target_uri, a_leg_sdp.as_deref()).await {
            Ok(call) => call,
            Err(e) => {
                error!(
                    call_id = %call_id,
                    target = %target_uri,
                    error = %e,
                    "Failed to create B-leg INVITE"
                );
                // Clear caller identity overrides to prevent leaking to subsequent calls
                uac.inner().set_display_name(None).await;
                uac.inner().set_from_uri(None).await;

                let response = self.create_simple_response(request, 500, "Failed to forward call");
                a_leg_handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(&call_id, Some(format!("Failed to create B-leg INVITE: {}", e)));
                return Ok(());
            }
        };

        // Clear caller identity overrides after INVITE sent to prevent leaking to subsequent calls
        uac.inner().set_display_name(None).await;
        uac.inner().set_from_uri(None).await;

        // Shareable handle for cancel/final response tracking
        let b_leg_handle = Arc::new(tokio::sync::Mutex::new(Some(b_leg_call)));

        // Persist the outbound handle for later control (CANCEL/BYE)
        let _ = self.call_manager.update_call(&call_id, |call| {
            if let Some(b_leg) = &mut call.b_leg {
                b_leg.outbound_handle = Some(Arc::clone(&b_leg_handle));
            }
        });

        info!(
            call_id = %call_id,
            target = %target_uri,
            "B-leg INVITE sent, awaiting responses"
        );

        // Update call state to Proceeding
        let _ = self.call_manager.update_call(&call_id, |call| {
            call.state = crate::call::CallState::Proceeding;
        });

        // Track A-leg To-tag for dialog consistency (RFC 3261 §12.1.1)
        // Once we send a provisional with a To-tag, we must use the SAME tag for 200 OK
        let mut a_leg_to_tag: Option<String> = None;

        // Relay provisional responses from B-leg to A-leg until we get a 180 Ringing
        info!(call_id = %call_id, "Waiting for provisional responses from B-leg");
        loop {
            // Take the handle out without holding the lock during async operations
            let mut call_handle = {
                let mut guard = b_leg_handle.lock().await;
                guard.take()
            };

            let provisional = if let Some(ref mut call) = call_handle {
                // Await WITHOUT holding the lock
                let res = call.await_provisional().await;
                if let Some(ref r) = res {
                    call.dialog.update_from_response(r);
                }
                res
            } else {
                None
            };

            // Put the handle back
            if let Some(call) = call_handle {
                let mut guard = b_leg_handle.lock().await;
                *guard = Some(call);
            }

            // Check if we got a provisional response
            let provisional = match provisional {
                Some(p) => p,
                None => break,
            };
            let status_code = provisional.start.code;
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
                    has_rseq = provisional.headers.get("RSeq").is_some(),
                    "Relaying provisional response to A-leg"
                );

                if provisional.headers.get("RSeq").is_some() {
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
                    let mut resp = self.uas.create_ringing(request);

                    // Extract and save the To-tag for later use with 200 OK
                    if let Some(to_header) = resp.headers.get("To") {
                        if let Some(tag) = extract_tag_value(Some(to_header)) {
                            a_leg_to_tag = Some(tag.to_string());
                            debug!(call_id = %call_id, a_leg_to_tag = %tag, "Generated A-leg To-tag from first provisional");
                        }
                    }

                    // Update status code and reason if not 180
                    if status_code != 180 {
                        resp.start.code = status_code;
                        resp.start.reason = provisional.start.reason.clone();
                    }

                    // Copy body from B-leg provisional (if any) for early media
                    if !provisional.body.is_empty() {
                        resp.body = provisional.body.clone();
                        if let Some(ct) = provisional.headers.get("Content-Type") {
                            resp.headers.push(SmolStr::new("Content-Type"), ct.clone());
                        }
                        // Update Content-Length
                        for header in resp.headers.iter_mut() {
                            if header.name.as_str() == "Content-Length" {
                                header.value = SmolStr::new(resp.body.len().to_string());
                                break;
                            }
                        }
                    }

                    resp
                } else {
                    // Subsequent provisionals - reuse the established To-tag
                    self.create_provisional_from_b_leg(request, &provisional)
                };

                info!(call_id = %call_id, code = status_code, "Sending provisional to A-leg");
                a_leg_handle.send_provisional(response).await;

                // Break after first 18x response to proceed to final response handling
                // This prevents hanging while waiting for more provisionals
                break;
            }
        }

        // Clone the early B-leg dialog before consuming CallHandle
        // This will be updated with the confirmed tag from the 200 OK
        let mut b_leg_dialog = b_leg_handle
            .lock()
            .await
            .as_ref()
            .map(|h| h.dialog.clone())
            .ok_or_else(|| anyhow::anyhow!("B-leg handle missing dialog"))?;
        debug!(
            call_id = %call_id,
            early_dialog_id = ?b_leg_dialog.id,
            "Cloned early B-leg dialog before await_final"
        );

        // Wait for final response from B-leg
        info!(call_id = %call_id, "Waiting for final response from B-leg");
        let b_leg_final = match {
            let mut guard = b_leg_handle.lock().await;
            let handle = guard
                .take()
                .ok_or_else(|| anyhow::anyhow!("B-leg handle missing for await_final"))?;
            drop(guard);
            handle.await_final().await
        } {
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
                    let _ = self.call_manager.terminate_call(&call_id, Some("Cancelled".to_string()));
                } else {
                    error!(
                        call_id = %call_id,
                        error = %e,
                        "Failed to receive final response from B-leg"
                    );
                    let response = self.create_simple_response(request, 408, "Request Timeout");
                    a_leg_handle.send_final(response).await;
                    let _ = self.call_manager.terminate_call(&call_id, Some(format!("B-leg timeout: {}", e)));
                }
                return Ok(());
            }
        };

        let b_leg_status = b_leg_final.start.code;
        info!(
            call_id = %call_id,
            code = b_leg_status,
            "Received final response from B-leg"
        );

        // Update B-leg dialog with routing/target/session info from final response
        b_leg_dialog.update_from_response(&b_leg_final);

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
            let b_leg_sdp = if !b_leg_final.body.is_empty() {
                Some(String::from_utf8_lossy(&b_leg_final.body).to_string())
            } else {
                None
            };

            // Update B-leg dialog with confirmed To-tag from 200 OK
            if let Some(to_header) = sip_parse::header(&b_leg_final.headers, "To") {
                // Extract tag parameter from To header
                // To header format: "Display Name" <sip:user@host>;tag=xyz123;other=params
                if let Some(tag_start) = to_header.find("tag=") {
                    // Find the tag value (everything after "tag=" until semicolon or end)
                    let tag_value_start = tag_start + 4; // length of "tag="
                    let remaining = &to_header[tag_value_start..];

                    // Extract tag until semicolon or end of string
                    let confirmed_tag = if let Some(semicolon_pos) = remaining.find(';') {
                        &remaining[..semicolon_pos]
                    } else {
                        remaining
                    }.trim();

                    if !confirmed_tag.is_empty() {
                        // Update dialog with confirmed tag
                        b_leg_dialog.id.remote_tag = confirmed_tag.into();
                        b_leg_dialog.state = sip_dialog::DialogStateType::Confirmed;

                        debug!(
                            call_id = %call_id,
                            confirmed_tag = confirmed_tag,
                            confirmed_dialog_id = ?b_leg_dialog.id,
                            "Updated B-leg dialog with confirmed tag from 200 OK"
                        );

                        // Register confirmed B-leg dialog in dialog_manager for in-dialog request routing
                        self.dialog_manager.insert(b_leg_dialog.clone());
                        self.start_session_timer(&b_leg_dialog);
                        debug!(
                            call_id = %call_id,
                            "Registered confirmed B-leg dialog in dialog_manager"
                        );
                    } else {
                        warn!(call_id = %call_id, "Empty tag value in To header of B-leg 200 OK");
                    }
                } else {
                    warn!(call_id = %call_id, "No tag parameter in To header of B-leg 200 OK");
                }
            } else {
                warn!(call_id = %call_id, "No To header in B-leg 200 OK");
            }

            // Create 200 OK response manually to ensure To-tag consistency
            // CRITICAL: Must reuse the To-tag from the provisional response (RFC 3261 §12.1.1)
            let a_leg_response = if let Some(ref saved_tag) = a_leg_to_tag {
                // Build 200 OK with the SAME To-tag as the provisional
                let mut resp = self.uas.create_ok(request, b_leg_sdp.as_deref());

                // Replace To header with one containing our saved tag
                if let Some(to) = request.headers.get("To") {
                    let to_with_tag = format!("{};tag={}", to.as_str(), saved_tag);
                    // Remove old To header and add new one with correct tag
                    resp.headers.retain(|h| h.name.as_str() != "To");
                    resp.headers.push(SmolStr::new("To"), SmolStr::new(to_with_tag));
                    debug!(call_id = %call_id, a_leg_to_tag = %saved_tag, "Reused To-tag in 200 OK for dialog consistency");
                }

                resp
            } else {
                // Fallback: No provisional was sent (shouldn't happen in B2BUA)
                warn!(call_id = %call_id, "No A-leg To-tag saved, generating new one (dialog mismatch possible)");
                self.uas.create_ok(request, b_leg_sdp.as_deref())
            };

            // Create A-leg dialog from the 200 OK response
            let a_leg_dialog = match sip_dialog::Dialog::new_uas(
                request,
                &a_leg_response,
                self.uas.local_uri.clone(),
                extract_from_uri(request).unwrap_or_else(|_| sip_core::SipUri::parse("sip:unknown@unknown").unwrap()),
            ) {
                Some(dialog) => dialog,
                None => {
                    error!(call_id = %call_id, "Failed to create A-leg dialog from 200 OK");
                    let response = self.create_simple_response(request, 500, "Internal Server Error");
                    a_leg_handle.send_final(response).await;
                    let _ = self.call_manager.terminate_call(&call_id, Some("Failed to create A-leg dialog".to_string()));
                    return Ok(());
                }
            };

            info!(
                call_id = %call_id,
                "Call established - both legs connected"
            );

            // Register A-leg dialog in dialog_manager for in-dialog request routing
                    self.dialog_manager.insert(a_leg_dialog.clone());
                    self.start_session_timer(&a_leg_dialog);
            debug!(
                call_id = %call_id,
                a_leg_dialog_id = ?a_leg_dialog.id,
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
                        b_leg_dialog_call_id = ?b_leg_dialog.id.call_id,
                        "Stored confirmed B-leg dialog in CallManager"
                    );
                }
                call.state = crate::call::CallState::Established;
                call.establish();
            }) {
                Ok(_) => debug!(call_id = %call_id, "Successfully updated call in CallManager"),
                Err(e) => error!(call_id = %call_id, error = %e, "Failed to update call in CallManager"),
            }

            // Send 200 OK to A-leg
            a_leg_handle.send_final(a_leg_response).await;

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
            let reason = b_leg_final.start.reason.as_str();
            let response = self.create_simple_response(request, b_leg_status, reason);
            a_leg_handle.send_final(response).await;

            let _ = self.call_manager.terminate_call(
                &call_id,
                Some(format!("B-leg rejected: {} {}", b_leg_status, reason))
            );
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
        call_id: crate::call::CallId,
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
                            dialog_call_id = %dialog.id.call_id,
                    local_tag = %dialog.id.local_tag,
                    remote_tag = %dialog.id.remote_tag,
                            "Created A-leg dialog"
                        );

                        let mut response = response;
                        self.ensure_response_hygiene(request, &mut response);

                        // Update call with A-leg dialog
                        let _ = self.call_manager.update_call(&call_id, |call| {
                            call.a_leg.dialog = Some(dialog.clone());
                            call.state = crate::call::CallState::Established;
                            call.establish();
                        });
                        self.start_session_timer(&dialog);

                        handle.send_final(response).await;
            }
            Err(e) => {
                error!("Failed to accept INVITE: {}", e);

                // Mark call as failed
                let _ = self.call_manager.terminate_call(&call_id, Some(format!("Failed to accept INVITE: {}", e)));

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
        let from = sip_parse::header(&request.headers, "From")
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
        let to = sip_parse::header(&request.headers, "To")
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
        if !request.body.is_empty() {
            debug!(
                body_len = request.body.len(),
                "INVITE contains SDP offer"
            );
            // TODO: Parse and negotiate SDP offer
            // For now, just generate our own SDP regardless of offer
        }

        // Generate SDP answer using our media profile
        let username = self.uas.local_uri.user.as_deref().unwrap_or("fcp");
        let session_desc = profile.build(
            username,
            &self.local_addr,
            self.rtp_port,
            None, // No video port for now
        );

        // Serialize to SDP string
        let sdp_string = sip_sdp::serialize::serialize_sdp(&session_desc);

        debug!(
            sdp_len = sdp_string.len(),
            "Generated SDP answer"
        );

        Ok(Some(sdp_string))
    }

    /// Handle an ACK request (confirms 2xx response to INVITE).
    /// Handle an ACK request (B2BUA mode).
    ///
    /// ACK confirms dialog establishment after 200 OK.
    /// In B2BUA mode, we need to:
    /// 1. Find the call by dialog Call-ID
    /// 2. Confirm the call is established
    /// 3. TODO: Forward ACK to B-leg
    fn handle_ack(&self, request: &sip_core::Request) -> Result<()> {
        debug!("Received ACK (B2BUA mode)");

        // Extract Call-ID to find the call
        let dialog_call_id = sip_parse::header(&request.headers, "Call-ID")
            .ok_or_else(|| anyhow::anyhow!("Missing Call-ID in ACK"))?;

        // Find the call by dialog Call-ID
        if let Some(call) = self.call_manager.find_by_dialog_call_id(dialog_call_id.as_str()) {
            debug!(
                call_id = %call.id,
                dialog_call_id = %dialog_call_id,
                "ACK received, call confirmed established"
            );

            // The call should already be in Established state from the 200 OK
            // ACK is just confirmation, no state change needed

            // TODO: Forward ACK to B-leg when we implement full B2BUA forwarding
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
        let dialog_call_id = sip_parse::header(&request.headers, "Call-ID")
            .ok_or_else(|| anyhow::anyhow!("Missing Call-ID in BYE"))?;

        // Find the call by dialog Call-ID
        if let Some(call) = self.call_manager.find_by_dialog_call_id(dialog_call_id.as_str()) {
            info!(
                call_id = %call.id,
                dialog_call_id = %dialog_call_id,
                duration = ?call.duration(),
                "Terminating call due to BYE"
            );

            stop_timers_for_call(&self.session_timer_manager, &call);

            // Determine which leg sent the BYE and forward to the other leg
            if let Some(uac) = &self.uac {
                // Check if BYE is from A-leg
                let is_a_leg = call.a_leg.dialog.as_ref()
                    .map(|d| d.id.call_id.as_str() == dialog_call_id.as_str())
                    .unwrap_or(false);

                if is_a_leg {
                    // BYE from A-leg, send BYE to B-leg
                    if let Some(b_leg) = &call.b_leg {
                        if let Some(b_leg_dialog) = &b_leg.dialog {
                            info!(
                                call_id = %call.id,
                                "BYE from A-leg, forwarding to B-leg"
                            );

                            self.spawn_bye_with_retry(
                                Arc::clone(uac),
                                b_leg_dialog.clone(),
                                call.id,
                                false,
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
                        );
                    } else {
                        debug!(call_id = %call.id, "A-leg has no dialog, skipping BYE forward");
                    }
                }
            } else {
                debug!(call_id = %call.id, "No UAC available, cannot forward BYE");
            }

            // Terminate the call in CallManager
            let _ = self.call_manager.terminate_call(&call.id, None);

            // Cleanup per-dialog state to prevent memory leaks
            if let Some(a_dialog) = &call.a_leg.dialog {
                self.rate_limits.remove(&a_dialog.id);
                self.mid_dialog_lock.remove(&a_dialog.id);
                // Clean subscription_index entries for A-leg dialog
                let keys: Vec<_> = self.subscription_index
                    .iter()
                    .filter(|entry| entry.key().0 == a_dialog.id)
                    .map(|entry| entry.key().clone())
                    .collect();
                for key in keys {
                    self.subscription_index.remove(&key);
                }
                debug!(dialog_id = ?a_dialog.id, "Cleaned up A-leg dialog state");
            }
            if let Some(b_leg) = &call.b_leg {
                if let Some(b_dialog) = &b_leg.dialog {
                    self.rate_limits.remove(&b_dialog.id);
                    self.mid_dialog_lock.remove(&b_dialog.id);
                    // Clean subscription_index entries for B-leg dialog
                    let keys: Vec<_> = self.subscription_index
                        .iter()
                        .filter(|entry| entry.key().0 == b_dialog.id)
                        .map(|entry| entry.key().clone())
                        .collect();
                    for key in keys {
                        self.subscription_index.remove(&key);
                    }
                    debug!(dialog_id = ?b_dialog.id, "Cleaned up B-leg dialog state");
                }
            }
        } else {
            debug!(
                dialog_call_id = %dialog_call_id,
                "BYE received for unknown call"
            );
        }

        // Find the dialog to send proper response
        let dialog = self
            .dialog_manager
            .find_by_request(request)
            .ok_or_else(|| anyhow::anyhow!("Dialog not found for BYE"))?;

        // Handle BYE using UAS helper (sends 200 OK)
        self.uas.handle_bye(request, &dialog)
    }

    /// Handle in-dialog re-INVITE by forwarding to the opposite leg.
    async fn handle_reinvite(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling in-dialog re-INVITE");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id).await;
        let cseq = sip_parse::header(&request.headers, "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id.clone())
        } else {
            call.b_leg.as_ref().and_then(|b| b.dialog.as_ref().map(|d| d.id.clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "REINVITE",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id,
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
            src_dialog_id
                .as_ref()
                .unwrap_or(&target_dialog.id),
            self.settings.reinvite_rate_limit,
        ) {
            warn!(call_id = %call.id, "re-INVITE rate limited");
            return Ok(self.create_simple_response(request, 429, "Too Many Requests"));
        }
        let body = validate_sdp_body_static(request, prior_sdp)?;

        self.log_sip_message(
            "outbound",
            "INVITE",
            Some(call.id.to_string().as_str()),
            Some(&target_dialog.id),
            cseq.as_ref().map(|v| v.as_str()),
        );
        let peer_resp = match uac.send_reinvite(&mut target_dialog, body.as_deref()).await {
            Ok(r) => {
                info!(status = r.start.code, "Forwarded re-INVITE to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "REINVITE"), ("result", "ok")]);
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "REINVITE"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward re-INVITE to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "REINVITE"), ("result", "error")]);
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &call.id,
                    &target_dialog.id,
                    "re-INVITE forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start.code == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                &call.id,
                &target_dialog.id,
                "Peer returned 481 to re-INVITE",
            );
            self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "REINVITE"), ("result", "481")]);
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
            if let Some((se, refresher)) = extract_session_timer(&peer_resp) {
                src_dialog.session_expires = Some(se);
                src_dialog.refresher = refresher;
                self.persist_updated_dialog(&call.id, is_a_leg_src, src_dialog.clone());
                self.start_session_timer(&src_dialog);
            } else if peer_resp.start.code == 422 {
                if let Some(min_se) = extract_min_se(&peer_resp) {
                    src_dialog.session_expires = Some(min_se);
                    debug!(call_id = %call.id, dialog_id = ?src_dialog.id, min_se = ?min_se, "Applied Min-SE from 422 to originating leg dialog");
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
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id).await;
        let cseq = sip_parse::header(&request.headers, "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id.clone())
        } else {
            call.b_leg.as_ref().and_then(|b| b.dialog.as_ref().map(|d| d.id.clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "SUBSCRIBE",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id,
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for SUBSCRIBE forwarding"))?;

        let event = sip_parse::header(&request.headers, "Event")
            .ok_or_else(|| anyhow::anyhow!("Missing Event header"))?;
        let expires = sip_parse::header(&request.headers, "Expires")
            .and_then(|v| v.parse::<u32>().ok());

        self.log_sip_message(
            "outbound",
            "SUBSCRIBE",
            Some(call.id.to_string().as_str()),
            Some(&target_dialog.id),
            cseq.as_ref().map(|v| v.as_str()),
        );
        let (peer_resp, subscription) = match uac
            .send_subscribe(&mut target_dialog, event.as_str(), expires)
            .await
        {
            Ok(r) => {
                info!(status = r.0.start.code, "Forwarded SUBSCRIBE to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "SUBSCRIBE"), ("result", "ok")]);
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "SUBSCRIBE"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward SUBSCRIBE to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "SUBSCRIBE"), ("result", "error")]);
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &call.id,
                    &target_dialog.id,
                    "SUBSCRIBE forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start.code == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                &call.id,
                &target_dialog.id,
                "Peer returned 481 to SUBSCRIBE",
            );
            self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "SUBSCRIBE"), ("result", "481")]);
        }

        if let Some(sub) = subscription {
            let sub_clone = sub.clone();
            self.subscription_manager.insert(sub);
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
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id).await;
        let cseq = sip_parse::header(&request.headers, "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id.clone())
        } else {
            call.b_leg.as_ref().and_then(|b| b.dialog.as_ref().map(|d| d.id.clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "NOTIFY",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id,
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for NOTIFY forwarding"))?;

        // Build a subscription placeholder from dialog + event to forward notify
        let event = sip_parse::header(&request.headers, "Event")
            .ok_or_else(|| anyhow::anyhow!("Missing Event header"))?;
        let body = if request.body.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&request.body).to_string())
        };

        self.check_body_size(request, self.settings.notify_max_body_bytes)?;

        // Record body size histogram
        self.metrics.record_histogram(
            "fcp_sip_notify_body_bytes",
            request.body.len() as f64,
            &[("event", event.as_str())],
        );

        // Create a synthetic subscription to use contact/route info from dialog target
        // (we don't track full subscription state yet beyond storage)
        if let Some(sub) = self.find_subscription(&target_dialog.id, event.as_str()) {
            let state = sip_dialog::SubscriptionState::Active;
        self.log_sip_message(
            "outbound",
            "NOTIFY",
            Some(call.id.to_string().as_str()),
            Some(&target_dialog.id),
            cseq.as_ref().map(|v| v.as_str()),
        );
            let peer_resp = match uac
                .send_notify(&sub, state, body.as_deref())
                .await
            {
                Ok(r) => {
                    info!(status = r.start.code, "Forwarded NOTIFY to peer");
                    self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "NOTIFY"), ("result", "ok")]);
                    self.metrics.observe_ms(
                        "fcp_sip_mid_dialog_forward_ms",
                        start.elapsed().as_millis(),
                        &[("method", "NOTIFY"), ("result", "ok")],
                    );
                    r
                }
                Err(e) => {
                    warn!(error = %e, "Failed to forward NOTIFY to peer");
                    self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "NOTIFY"), ("result", "error")]);
                    cleanup_after_forward_failure(
                        &self.call_manager,
                        &self.dialog_manager,
                        &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                        &call.id,
                        &target_dialog.id,
                        "NOTIFY forward failed",
                    );
                    return Ok(self.create_simple_response(
                        request,
                        481,
                        "Subscription Does Not Exist",
                    ));
                }
            };

            if peer_resp.start.code == 481 {
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &call.id,
                    &target_dialog.id,
                    "Peer returned 481 to NOTIFY",
                );
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "NOTIFY"), ("result", "481")]);
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
                Some(&target_dialog.id),
                cseq.as_ref().map(|v| v.as_str()),
            );
            let peer_resp = match uac
                .send_notify(&sub, state, body.as_deref())
                .await
            {
                Ok(r) => {
                    info!(status = r.start.code, "Forwarded NOTIFY (transient sub) to peer");
                    self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "NOTIFY"), ("result", "ok")]);
                    self.metrics.observe_ms(
                        "fcp_sip_mid_dialog_forward_ms",
                        start.elapsed().as_millis(),
                        &[("method", "NOTIFY"), ("result", "ok")],
                    );
                    r
                }
                Err(e) => {
                    warn!(error = %e, "Failed to forward NOTIFY (transient) to peer");
                    self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "NOTIFY"), ("result", "error")]);
                    cleanup_after_forward_failure(
                        &self.call_manager,
                        &self.dialog_manager,
                        &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                        &call.id,
                        &target_dialog.id,
                        "NOTIFY forward failed",
                    );
                    return Ok(self.create_simple_response(
                        request,
                        481,
                        "Subscription Does Not Exist",
                    ));
                }
            };

            if peer_resp.start.code == 481 {
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &call.id,
                    &target_dialog.id,
                    "Peer returned 481 to NOTIFY",
                );
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "NOTIFY"), ("result", "481")]);
            }
            return Ok(self.build_in_dialog_response(request, &peer_resp));
        }

        Ok(self.create_simple_response(request, 481, "Subscription Does Not Exist"))
    }

    /// Handle in-dialog UPDATE by forwarding to the opposite leg.
    async fn handle_update(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling UPDATE");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id).await;
        let cseq = sip_parse::header(&request.headers, "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id.clone())
        } else {
            call.b_leg.as_ref().and_then(|b| b.dialog.as_ref().map(|d| d.id.clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "UPDATE",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id,
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
            src_dialog_id
                .as_ref()
                .unwrap_or(&target_dialog.id),
            self.settings.update_rate_limit,
        ) {
            warn!(call_id = %call.id, "UPDATE rate limited");
            return Ok(self.create_simple_response(request, 429, "Too Many Requests"));
        }
        let body = validate_sdp_body_static(request, prior_sdp)?;

        self.log_sip_message(
            "outbound",
            "UPDATE",
            Some(call.id.to_string().as_str()),
            Some(&target_dialog.id),
            cseq.as_ref().map(|v| v.as_str()),
        );
        let peer_resp = match uac.send_update(&mut target_dialog, body.as_deref()).await {
            Ok(r) => {
                info!(status = r.start.code, "Forwarded UPDATE to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "UPDATE"), ("result", "ok")]);
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "UPDATE"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward UPDATE to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "UPDATE"), ("result", "error")]);
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &call.id,
                    &target_dialog.id,
                    "UPDATE forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start.code == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                &call.id,
                &target_dialog.id,
                "Peer returned 481 to UPDATE",
            );
            self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "UPDATE"), ("result", "481")]);
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
            if let Some((se, refresher)) = extract_session_timer(&peer_resp) {
                src_dialog.session_expires = Some(se);
                src_dialog.refresher = refresher;
                self.persist_updated_dialog(&call.id, is_a_leg_src, src_dialog.clone());
                self.start_session_timer(&src_dialog);
            } else if peer_resp.start.code == 422 {
                if let Some(min_se) = extract_min_se(&peer_resp) {
                    src_dialog.session_expires = Some(min_se);
                    debug!(call_id = %call.id, dialog_id = ?src_dialog.id, min_se = ?min_se, "Applied Min-SE from 422 to originating leg dialog");
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
        let _guard = self.acquire_mid_dialog_lock(&target_dialog.id).await;
        let cseq = sip_parse::header(&request.headers, "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id.clone())
        } else {
            call.b_leg.as_ref().and_then(|b| b.dialog.as_ref().map(|d| d.id.clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "PRACK",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id,
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

        self.log_sip_message(
            "outbound",
            "PRACK",
            Some(call.id.to_string().as_str()),
            Some(&target_dialog.id),
            cseq.as_ref().map(|v| v.as_str()),
        );
        let peer_resp = match uac.send_prack(&mut target_dialog, &reliable).await {
            Ok(r) => {
                info!(status = r.start.code, "Forwarded PRACK to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "PRACK"), ("result", "ok")]);
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "PRACK"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward PRACK to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "PRACK"), ("result", "error")]);
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &call.id,
                    &target_dialog.id,
                    "PRACK forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start.code == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                &call.id,
                &target_dialog.id,
                "Peer returned 481 to PRACK",
            );
            self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "PRACK"), ("result", "481")]);
        }
        self.persist_updated_dialog(&call.id, !is_a_leg, target_dialog);

        Ok(self.build_in_dialog_response(request, &peer_resp))
    }

    /// Handle in-dialog INFO by forwarding to the opposite leg.
    async fn handle_info(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling INFO");
        let (call, is_a_leg) = self.lookup_call_for_in_dialog(request)?;
        let target_dialog = self.get_opposite_dialog(&call, is_a_leg)?;
        let mut target_dialog = target_dialog.clone();
        let cseq = sip_parse::header(&request.headers, "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id.clone())
        } else {
            call.b_leg.as_ref().and_then(|b| b.dialog.as_ref().map(|d| d.id.clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "INFO",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id,
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for INFO forwarding"))?;

        let content_type = sip_parse::header(&request.headers, "Content-Type")
            .ok_or_else(|| anyhow::anyhow!("Missing Content-Type for INFO"))?;
        let body = String::from_utf8_lossy(&request.body).to_string();

        self.check_body_size(request, self.settings.info_max_body_bytes)?;

        // Record body size histogram
        self.metrics.record_histogram(
            "fcp_sip_info_body_bytes",
            request.body.len() as f64,
            &[("content_type", content_type.as_str())],
        );

        self.check_info_content_type(content_type.as_str())?;
        if self.check_rate_limit(
            "INFO",
            src_dialog_id
                .as_ref()
                .unwrap_or(&target_dialog.id),
            self.settings.info_rate_limit,
        ) {
            warn!(call_id = %call.id, "INFO rate limited");
            return Ok(self.create_simple_response(request, 429, "Too Many Requests"));
        }
        self.log_sip_message(
            "outbound",
            "INFO",
            Some(call.id.to_string().as_str()),
            Some(&target_dialog.id),
            cseq.as_ref().map(|v| v.as_str()),
        );
        let peer_resp = match uac.send_info(&mut target_dialog, content_type.as_str(), &body).await {
            Ok(r) => {
                info!(status = r.start.code, "Forwarded INFO to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "INFO"), ("result", "ok")]);
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "INFO"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward INFO to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "INFO"), ("result", "error")]);
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &call.id,
                    &target_dialog.id,
                    "INFO forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start.code == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                &call.id,
                &target_dialog.id,
                "Peer returned 481 to INFO",
            );
            self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "INFO"), ("result", "481")]);
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
        let cseq = sip_parse::header(&request.headers, "CSeq");
        let src_dialog_id = if is_a_leg {
            call.a_leg.dialog.as_ref().map(|d| d.id.clone())
        } else {
            call.b_leg.as_ref().and_then(|b| b.dialog.as_ref().map(|d| d.id.clone()))
        };
        let span = tracing::info_span!(
            "mid_dialog_forward",
            method = "REFER",
            call_id = %call.id,
            src_dialog = ?src_dialog_id,
            dst_dialog = ?target_dialog.id,
            cseq = ?cseq
        );
        let _guard_span = span.enter();
        let start = std::time::Instant::now();

        let uac = self
            .uac
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No UAC configured for REFER forwarding"))?;

        let refer_to_raw = sip_parse::header(&request.headers, "Refer-To")
            .ok_or_else(|| anyhow::anyhow!("Missing Refer-To"))?;
        let refer_uri_str = refer_to_raw
            .split('<')
            .nth(1)
            .and_then(|s| s.split('>').next())
            .unwrap_or(refer_to_raw.as_str())
            .trim();

        let refer_uri =
            sip_core::SipUri::parse(refer_uri_str).ok_or_else(|| anyhow::anyhow!("Invalid Refer-To URI"))?;

        if !self.settings.allow_refer {
            warn!(call_id = %call.id, "REFER not allowed by policy");
            return Ok(self.create_simple_response(request, 403, "Refer Not Allowed"));
        }
        self.log_sip_message(
            "outbound",
            "REFER",
            Some(call.id.to_string().as_str()),
            Some(&target_dialog.id),
            cseq.as_ref().map(|v| v.as_str()),
        );
        let (peer_resp, _) = match uac
            .send_refer(&mut target_dialog, &refer_uri, None)
            .await
        {
            Ok(r) => {
                info!(status = r.0.start.code, "Forwarded REFER to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "REFER"), ("result", "ok")]);
                self.metrics.observe_ms(
                    "fcp_sip_mid_dialog_forward_ms",
                    start.elapsed().as_millis(),
                    &[("method", "REFER"), ("result", "ok")],
                );
                r
            }
            Err(e) => {
                warn!(error = %e, "Failed to forward REFER to peer");
                self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "REFER"), ("result", "error")]);
                cleanup_after_forward_failure(
                    &self.call_manager,
                    &self.dialog_manager,
                    &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                    &call.id,
                    &target_dialog.id,
                    "REFER forward failed",
                );
                return Ok(self.create_simple_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                ));
            }
        };

        if peer_resp.start.code == 481 {
            cleanup_after_forward_failure(
                &self.call_manager,
                &self.dialog_manager,
                &self.session_timer_manager,
                    &self.rate_limits,
                    &self.mid_dialog_lock,
                    &self.subscription_index,
                &call.id,
                &target_dialog.id,
                "Peer returned 481 to REFER",
            );
            self.metrics.inc_counter("fcp_sip_mid_dialog_forward_total", &[("method", "REFER"), ("result", "481")]);
        }
        self.persist_updated_dialog(&call.id, !is_a_leg, target_dialog);

        Ok(self.build_in_dialog_response(request, &peer_resp))
    }

    /// Handle a REGISTER request for endpoint registration.
    ///
    /// If a registrar is configured, delegates to the registrar for processing.
    /// Otherwise, returns 501 Not Implemented.
    async fn handle_register(&self, request: &sip_core::Request) -> sip_core::Response {
        debug!("Handling REGISTER request");

        match &self.registrar {
            Some(registrar) => {
                // Use registrar to handle the request
                let response = registrar.handle_register(request).await;

                // Log success/failure
                if response.start.code == 200 {
                    info!(
                        aor = %sip_parse::header(&request.headers, "To")
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                        "REGISTER handled successfully"
                    );
                } else if response.start.code == 401 {
                    debug!("REGISTER requires authentication");
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
        if let Some(via) = request.headers.get("Via") {
            headers.push(SmolStr::new("Via"), via.clone());
        }
        if let Some(from) = request.headers.get("From") {
            headers.push(SmolStr::new("From"), from.clone());
        }
        if let Some(to) = request.headers.get("To") {
            let mut to_val = to.clone();
            if !to_val.contains(";tag=") {
                let tag = Uuid::new_v4().simple().to_string();
                to_val = SmolStr::new(format!("{};tag={}", to_val, tag));
            }
            headers.push(SmolStr::new("To"), to_val);
        }
        if let Some(call_id) = request.headers.get("Call-ID") {
            headers.push(SmolStr::new("Call-ID"), call_id.clone());
        }
        if let Some(cseq) = request.headers.get("CSeq") {
            headers.push(SmolStr::new("CSeq"), cseq.clone());
        }

        // Add Contact for INVITE/dialog forming responses if absent
        if request.start.method == sip_core::Method::Invite && headers.get("Contact").is_none() {
            headers.push(
                SmolStr::new("Contact"),
                SmolStr::new(format!("<{}>", self.uas.contact_uri.as_str())),
            );
        }

        // Content-Length: 0 (no body)
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        let mut resp = sip_core::Response::new(
            StatusLine::new(code, SmolStr::new(reason)),
            headers,
            Bytes::new(),
        );
        self.ensure_response_hygiene(request, &mut resp);
        self.log_sip_message(
            "outbound",
            &code.to_string(),
            sip_parse::header(&request.headers, "Call-ID").map(|s| s.as_str()),
            None,
            sip_parse::header(&request.headers, "CSeq").map(|s| s.as_str()),
        );
        resp
    }

    /// Send BYE with a single retry/backoff and clean up dialog state on success.
    fn spawn_bye_with_retry(
        &self,
        uac: Arc<SipUserAgentClient>,
        dialog: sip_dialog::Dialog,
        call_id: crate::call::CallId,
        is_a_leg: bool,
    ) {
        let dialog_manager = Arc::clone(&self.dialog_manager);
        let call_manager = self.call_manager.clone();
        let stm = Arc::clone(&self.session_timer_manager);
        let subscription_index = Arc::clone(&self.subscription_index);
        tokio::spawn(async move {
            let mut attempts = 0;
            loop {
                attempts += 1;
                match uac.bye(&dialog).await {
                    Ok(_) => {
                        debug!(call_id = %call_id, attempt = attempts, "Successfully sent BYE to peer leg");
                        stm.stop_timer(&dialog.id);
                        dialog_manager.remove(&dialog.id);
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
                        let dialog_id = dialog.id.clone();
                        let keys: Vec<_> = subscription_index
                            .iter()
                            .filter(|entry| entry.key().0 == dialog_id)
                            .map(|entry| entry.key().clone())
                            .collect();
                        for key in keys {
                            subscription_index.remove(&key);
                        }
                        break;
                    }
                    Err(e) => {
                        if attempts >= 2 {
                            error!(call_id = %call_id, error = %e, "Failed to send BYE after retry");
                            // Clean up dialog even on failure to prevent memory leak
                            dialog_manager.remove(&dialog.id);
                            let _ = call_manager.update_call(&call_id, |call| {
                                if is_a_leg {
                                    call.a_leg.dialog = None;
                                } else if let Some(b) = &mut call.b_leg {
                                    b.dialog = None;
                                }
                            });
                            // Clean subscription_index entries for this dialog (error path)
                            let dialog_id = dialog.id.clone();
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
    fn create_cancelled_response(&self, invite_req: &sip_core::Request) -> sip_core::Response {
        let mut response =
            sip_uas::UserAgentServer::create_response(invite_req, 487, "Request Terminated");

        // Ensure Contact header is present
        if response.headers.get("Contact").is_none() {
            response.headers.push(
                SmolStr::new("Contact"),
                SmolStr::new(format!("<{}>", self.uas.contact_uri.as_str())),
            );
        }

        // Copy Allow/Supported if present on the INVITE so capabilities are preserved
        for name in ["Allow", "Supported"] {
            if let Some(val) = invite_req.headers.get(name) {
                response.headers.push(SmolStr::new(name), val.clone());
            }
        }

        // Ensure To header has a tag; generate if missing
        if let Some(to) = response.headers.get("To") {
            if !to.contains(";tag=") {
                let tag = Uuid::new_v4().simple().to_string();
                let new_to = format!("{};tag={}", to.as_str(), tag);

                for header in response.headers.iter_mut() {
                    if header.name.as_str() == "To" {
                        header.value = SmolStr::new(new_to);
                        break;
                    }
                }
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
        if let Some(via) = a_leg_invite.headers.get("Via") {
            headers.push(SmolStr::new("Via"), via.clone());
        }
        if let Some(from) = a_leg_invite.headers.get("From") {
            headers.push(SmolStr::new("From"), from.clone());
        }
        if let Some(to) = a_leg_invite.headers.get("To") {
            // Preserve the To-tag from B-leg provisional if present
            let to_with_tag = if let Some(tag) = extract_tag_value(b_leg_resp.headers.get("To")) {
                format!("{};tag={}", to.as_str(), tag)
            } else {
                to.to_string()
            };
            headers.push(SmolStr::new("To"), SmolStr::new(to_with_tag));
        }
        if let Some(call_id) = a_leg_invite.headers.get("Call-ID") {
            headers.push(SmolStr::new("Call-ID"), call_id.clone());
        }
        if let Some(cseq) = a_leg_invite.headers.get("CSeq") {
            headers.push(SmolStr::new("CSeq"), cseq.clone());
        }

        // Copy Contact from B-leg provisional (remote target)
        if let Some(contact) = b_leg_resp.headers.get("Contact") {
            headers.push(SmolStr::new("Contact"), contact.clone());
        }

        // Propagate Record-Route for correct route set
        for rr in b_leg_resp.headers.get_all("Record-Route") {
            headers.push(SmolStr::new("Record-Route"), rr.clone());
        }

        // Propagate capability/signaling headers
        for name in ["Require", "Supported", "Allow", "RSeq", "Session-Expires", "Content-Type"] {
            for val in b_leg_resp.headers.get_all(name) {
                headers.push(SmolStr::new(name), val.clone());
            }
        }

        // Body (e.g., SDP for early media)
        let body = b_leg_resp.body.clone();
        headers.push(
            SmolStr::new("Content-Length"),
            SmolStr::new(body.len().to_string()),
        );

        sip_core::Response::new(
            StatusLine::new(b_leg_resp.start.code, b_leg_resp.start.reason.clone()),
            headers,
            body,
        )
    }

    /// Build an in-dialog response for the requester based on the peer response.
    fn build_in_dialog_response(
        &self,
        request: &sip_core::Request,
        peer_response: &sip_core::Response,
    ) -> sip_core::Response {
        let mut response = if peer_response.start.code == 200 {
            // Use UAS helper to ensure Contact/To-tag are present
            let body = if peer_response.body.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&peer_response.body).to_string())
            };
            self.uas.create_ok(request, body.as_deref())
        } else {
            let mut r = self.create_simple_response(
                request,
                peer_response.start.code,
                peer_response.start.reason.as_str(),
            );
            if !peer_response.body.is_empty() {
                r.body = peer_response.body.clone();
                if let Some(ct) = peer_response.headers.get("Content-Type") {
                    r.headers.push(SmolStr::new("Content-Type"), ct.clone());
                }
                r.headers
                    .push(SmolStr::new("Content-Length"), SmolStr::new(r.body.len().to_string()));
            }
            r
        };

        // Propagate capability headers from peer
        for name in ["Supported", "Allow", "Session-Expires", "Require"] {
            for val in peer_response.headers.get_all(name) {
                response
                    .headers
                    .push(SmolStr::new(name), val.clone());
            }
        }
        // Propagate Record-Route/Contact from peer if present
        for val in peer_response.headers.get_all("Record-Route") {
            response
                .headers
                .push(SmolStr::new("Record-Route"), val.clone());
        }
        if response.headers.get("Contact").is_none() {
            if let Some(ct) = peer_response.headers.get("Contact") {
                response
                    .headers
                    .push(SmolStr::new("Contact"), ct.clone());
            }
        }

        // Ensure Content-Length matches body
        response.headers.push(
            SmolStr::new("Content-Length"),
            SmolStr::new(response.body.len().to_string()),
        );

        self.ensure_response_hygiene(request, &mut response);
        self.log_sip_message(
            "outbound",
            &response.start.code.to_string(),
            sip_parse::header(&request.headers, "Call-ID").map(|s| s.as_str()),
            None,
            sip_parse::header(&request.headers, "CSeq").map(|s| s.as_str()),
        );

        response
    }

    /// Validate SDP content for offer/answer forwarding.
    fn validate_sdp_body(
        &self,
        request: &sip_core::Request,
        prior_sdp: Option<&str>,
    ) -> Result<Option<String>> {
        if request.body.is_empty() {
            // Allow empty body only if we have prior SDP (late-offer/answerless re-INVITE)
            if prior_sdp.is_some() {
                return Ok(None);
            } else {
                return Err(anyhow::anyhow!("No SDP present and no prior offer/answer"));
            }
        }

        // Require application/sdp when body present
        let ctype = sip_parse::header(&request.headers, "Content-Type")
            .ok_or_else(|| anyhow::anyhow!("Missing Content-Type"))?;
        if ctype.to_lowercase() != "application/sdp" {
            return Err(anyhow::anyhow!("Unsupported Content-Type"));
        }

        let body_str = String::from_utf8_lossy(&request.body).to_string();
        let sdp = parse_sdp(&body_str).map_err(|e| anyhow::anyhow!("Invalid SDP: {}", e))?;

        // Basic checks: at least one media line (audio or video)
        if sdp.media.is_empty() {
            return Err(anyhow::anyhow!("SDP missing media descriptions"));
        }

        // Optional: ensure audio present
        let has_audio = sdp
            .media
            .iter()
            .any(|m| matches!(m.media_type, sip_sdp::MediaType::Audio));
        if !has_audio {
            return Err(anyhow::anyhow!("SDP missing audio media"));
        }

        // Accept hold/resume attributes (sendonly/inactive) without rejecting
        Ok(Some(body_str))
    }

    /// Find call and which leg the request belongs to.
    fn lookup_call_for_in_dialog(
        &self,
        request: &sip_core::Request,
    ) -> Result<(crate::call::Call, bool)> {
        let dialog_call_id = sip_parse::header(&request.headers, "Call-ID")
            .ok_or_else(|| anyhow::anyhow!("Missing Call-ID"))?;

        let call = self
            .call_manager
            .find_by_dialog_call_id(dialog_call_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("Dialog not found"))?;

        let is_a_leg = call
            .a_leg
            .dialog
            .as_ref()
            .map(|d| d.id.call_id.as_str() == dialog_call_id.as_str())
            .unwrap_or(false);

        Ok((call, is_a_leg))
    }

    /// Get the opposite dialog (target leg) for forwarding.
    fn get_opposite_dialog<'a>(
        &self,
        call: &'a crate::call::Call,
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
        call: &crate::call::Call,
        is_a_leg: bool,
    ) -> Option<sip_core::Response> {
        let list = if is_a_leg {
            call.b_leg
                .as_ref()
                .map(|b| &b.reliable_provisionals)
        } else {
            Some(&call.a_leg.reliable_provisionals)
        }?;

        // Use the latest reliable provisional (highest RSeq)
        list.iter()
            .max_by_key(|resp| {
                sip_parse::header(&resp.headers, "RSeq")
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(0)
            })
            .cloned()
    }

    /// Persist updated dialog to CallManager and dialog_manager.
    fn persist_updated_dialog(
        &self,
        call_id: &crate::call::CallId,
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

        self.dialog_manager.insert(dialog_clone);
    }

    fn log_sip_message(
        &self,
        direction: &str,
        method_or_status: &str,
        call_id: Option<&str>,
        dialog_id: Option<&sip_dialog::DialogId>,
        cseq: Option<&str>,
    ) {
        debug!(
            direction = direction,
            method_or_status = method_or_status,
            call_id = call_id.unwrap_or("unknown"),
            dialog_id = ?dialog_id,
            cseq = cseq.unwrap_or(""),
            "SIP message trace"
        );
        let mut labels = vec![
            ("direction", direction),
            ("method", method_or_status),
        ];
        if let Some(id) = dialog_id {
            labels.push(("dialog_id", id.call_id.as_str()));
        }
        self.metrics.inc_counter("fcp_sip_message_total", &labels);

        // Send to background writer via channel (non-blocking)
        if let (Some(ref tx), Some(cid)) = (&self.transcript_tx, call_id) {
            let entry = TranscriptEntry {
                call_id: cid.to_string(),
                timestamp: std::time::SystemTime::now(),
                direction: direction.to_string(),
                method_or_status: method_or_status.to_string(),
                dialog_id_str: dialog_id.map(|d| format!("{:?}", d)).unwrap_or_else(|| "none".to_string()),
                cseq: cseq.unwrap_or("").to_string(),
            };
            // Fire and forget - if channel is full/closed, log is dropped (non-critical)
            let _ = tx.send(entry);
        }
    }

    fn check_body_size(
        &self,
        request: &sip_core::Request,
        max_bytes: usize,
    ) -> Result<()> {
        if request.body.len() > max_bytes {
            return Err(anyhow::anyhow!("Body too large"));
        }
        Ok(())
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
            s.id.call_id == dialog_id.call_id
                && s.id.from_tag == dialog_id.local_tag
                && s.id.to_tag == dialog_id.remote_tag
                && s.id.event.as_str() == event
        })
    }

    fn index_subscription(&self, sub: sip_dialog::Subscription) {
        let key = (
            sip_dialog::DialogId::new(
                sub.id.call_id.clone(),
                sub.id.from_tag.clone(),
                sub.id.to_tag.clone(),
            ),
            sub.id.event.clone(),
        );
        self.subscription_manager.insert(sub.clone());
        self.subscription_index.insert(key, sub.id.clone());
    }

    fn dialog_is_refresher(dialog: &sip_dialog::Dialog) -> bool {
        dialog
            .refresher
            .map(|r| {
                (r == sip_core::RefresherRole::Uac && dialog.is_uac)
                    || (r == sip_core::RefresherRole::Uas && !dialog.is_uac)
            })
            .unwrap_or(false)
    }

    fn start_session_timer(&self, dialog: &sip_dialog::Dialog) {
        if let Some(expires) = dialog.session_expires {
            let is_refresher = Self::dialog_is_refresher(dialog);
            self.session_timer_manager
                .start_timer(dialog.id.clone(), expires, is_refresher);
        }
    }

    fn create_transient_subscription(
        &self,
        dialog: &sip_dialog::Dialog,
        event: &str,
    ) -> Option<sip_dialog::Subscription> {
        let local_uri = dialog.local_uri.clone();
        let remote_uri = dialog.remote_uri.clone();
        let contact = dialog.remote_target.clone();
        let expires = std::time::Duration::from_secs(60);
        Some(sip_dialog::Subscription {
            id: sip_dialog::SubscriptionId::new(
                dialog.id.call_id.clone(),
                dialog.id.local_tag.clone(),
                dialog.id.remote_tag.clone(),
                smol_str::SmolStr::new(event.to_string()),
            ),
            state: sip_dialog::SubscriptionState::Active,
            local_uri,
            remote_uri,
            contact,
            expires,
            local_cseq: 1,
            remote_cseq: 0,
        })
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
    ) -> Result<()> {
        debug!(
            transport = ?transport,
            peer = %peer,
            size = payload.len(),
            "Handling inbound SIP message"
        );

        // Try parsing as request first, then response
        if let Some(req) = sip_parse::parse_request(&payload) {
            self.log_sip_message(
                "inbound",
                req.start.method.as_str(),
                sip_parse::header(&req.headers, "Call-ID").map(|s| s.as_str()),
                self.dialog_manager.find_by_request(&req).map(|d| d.id.clone()).as_ref(),
                sip_parse::header(&req.headers, "CSeq").map(|s| s.as_str()),
            );
            debug!(
                method = ?req.start.method,
                uri = %req.start.uri,
                "Received SIP request"
            );

            // Special handling for ACK - it doesn't create a server transaction
            // ACK for 2xx responses is handled outside the transaction layer (RFC 3261 §17.1.1.3)
            if req.start.method == sip_core::Method::Ack {
                if let Err(e) = self.handle_ack(&req) {
                    error!("Failed to handle ACK: {}", e);
                }
                return Ok(());
            }

            // Create transport context
            let ctx = sip_transaction::TransportContext {
                transport,
                peer,
                stream,
                server_name: None,
                ws_uri: None,
            };

            // Create server transaction
            let handle = self
                .transaction_manager
                .receive_request(req.clone(), ctx)
                .await;

            // Handle request and send response(s)
            if let Err(e) = self.handle_request(&req, &handle, peer).await {
                error!("Failed to handle request: {}", e);
                // Send error response
                let response = self.create_simple_response(&req, 500, "Internal Server Error");
                handle.send_final(response).await;
            }
        } else if let Some(resp) = sip_parse::parse_response(&payload) {
            self.log_sip_message(
                "inbound",
                &resp.start.code.to_string(),
                sip_parse::header(&resp.headers, "Call-ID").map(|s| s.as_str()),
                None,
                sip_parse::header(&resp.headers, "CSeq").map(|s| s.as_str()),
            );
            debug!(
                status = resp.start.code,
                reason = %resp.start.reason,
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

/// SDP validation helper (exposed for tests).
fn validate_sdp_body_static(
    request: &sip_core::Request,
    prior_sdp: Option<&str>,
) -> Result<Option<String>> {
    if request.body.is_empty() {
        if prior_sdp.is_some() {
            return Ok(None);
        } else {
            return Err(anyhow::anyhow!("No SDP present and no prior offer/answer"));
        }
    }

    let ctype = sip_parse::header(&request.headers, "Content-Type")
        .ok_or_else(|| anyhow::anyhow!("Missing Content-Type"))?;
    if ctype.to_lowercase() != "application/sdp" {
        return Err(anyhow::anyhow!("Unsupported Content-Type"));
    }

    let body_str = String::from_utf8_lossy(&request.body).to_string();
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
    call_manager: &crate::call_manager::CallManager,
    call_id: &crate::call::CallId,
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
    let se_header = sip_parse::header(&resp.headers, "Session-Expires")?;
    sip_parse::parse_session_expires(se_header).map(|se| {
        (
            Duration::from_secs(se.delta_seconds as u64),
            se.refresher,
        )
    })
}

fn extract_min_se(resp: &sip_core::Response) -> Option<Duration> {
    sip_parse::header(&resp.headers, "Min-SE")
        .and_then(|v| v.parse::<u32>().ok())
        .map(|secs| Duration::from_secs(secs as u64))
}

async fn handle_refresh_needed(
    call_manager: &crate::call_manager::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    uac: Option<Arc<SipUserAgentClient>>,
    dialog_id: sip_dialog::DialogId,
) -> Result<()> {
    let (mut dialog, call_id, is_a_leg) =
        find_call_and_dialog(call_manager, &dialog_id).ok_or_else(|| anyhow::anyhow!("Dialog not found"))?;

    // Only refresh if we are the refresher
    let is_refresher = SipMessageHandler::dialog_is_refresher(&dialog);
    if !is_refresher {
        return Ok(());
    }

    let se_secs = dialog
        .session_expires
        .map(|d| d.as_secs() as u32)
        .unwrap_or(1800);

    let refresher_str = if dialog.refresher == Some(sip_core::RefresherRole::Uac) {
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

        if result.start.code == 200 {
            break result;
        }

        if result.start.code == 422 {
            if attempts >= max_attempts {
                break result;
            }
            if let Some(min_se) = sip_parse::header(&result.headers, "Min-SE")
                .and_then(|v| v.parse::<u32>().ok())
            {
                current_se = current_se.max(min_se);
                dialog.session_expires = Some(Duration::from_secs(current_se as u64));
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

    if resp.start.code != 200 {
        // Best effort BYE then terminate dialog on failed refresh
        let _ = uac.bye(&dialog).await;
        terminate_dialog(call_manager, dialog_manager, session_timer_manager, &call_id, &dialog_id);
        return Err(anyhow::anyhow!("Session refresh rejected: {}", resp.start.code));
    }

    dialog.update_from_response(&resp);
    // Restart timer with updated info
    if let Some(se) = dialog.session_expires {
        let is_refresher = dialog
            .refresher
            .map(|r| (r == sip_core::RefresherRole::Uac && dialog.is_uac) || (r == sip_core::RefresherRole::Uas && !dialog.is_uac))
            .unwrap_or(false);
        session_timer_manager.start_timer(dialog.id.clone(), se, is_refresher);
    }
    // Persist updated dialog
    let _ = call_manager.update_call(&call_id, |c| {
        if is_a_leg {
            c.a_leg.dialog = Some(dialog.clone());
        } else if let Some(b) = &mut c.b_leg {
            b.dialog = Some(dialog.clone());
        }
    });
    dialog_manager.insert(dialog);
    Ok(())
}

async fn handle_session_expired(
    call_manager: &crate::call_manager::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    uac: Option<Arc<SipUserAgentClient>>,
    dialog_id: sip_dialog::DialogId,
) -> Result<()> {
    let (dialog, call_id, _is_a_leg) =
        find_call_and_dialog(call_manager, &dialog_id).ok_or_else(|| anyhow::anyhow!("Dialog not found"))?;
    // Attempt BYE to remote
    if let Some(uac) = uac {
        let _ = uac.bye(&dialog).await;
    }
    terminate_dialog(call_manager, dialog_manager, session_timer_manager, &call_id, &dialog_id);
    Ok(())
}

fn terminate_dialog(
    call_manager: &crate::call_manager::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    call_id: &crate::call::CallId,
    dialog_id: &sip_dialog::DialogId,
) {
    session_timer_manager.stop_timer(dialog_id);
    dialog_manager.remove(dialog_id);
    let _ = call_manager.update_call(call_id, |call| {
        if let Some(a) = call.a_leg.dialog.as_ref() {
            if &a.id == dialog_id {
                call.a_leg.dialog = None;
            }
        }
        if let Some(b) = &mut call.b_leg {
            if let Some(d) = b.dialog.as_ref() {
                if &d.id == dialog_id {
                    b.dialog = None;
                }
            }
        }
    });
    let _ = call_manager.terminate_call(call_id, Some("Session expired".to_string()));
}

fn terminate_dialog_with_cleanup(
    call_manager: &crate::call_manager::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    rate_limits: &dashmap::DashMap<sip_dialog::DialogId, RateWindow>,
    mid_dialog_lock: &dashmap::DashMap<sip_dialog::DialogId, Arc<tokio::sync::Mutex<()>>>,
    subscription_index: &dashmap::DashMap<(sip_dialog::DialogId, smol_str::SmolStr), sip_dialog::SubscriptionId>,
    call_id: &crate::call::CallId,
    dialog_id: &sip_dialog::DialogId,
) {
    // Perform standard dialog termination
    terminate_dialog(call_manager, dialog_manager, session_timer_manager, call_id, dialog_id);

    // Cleanup per-dialog state to prevent memory leaks
    rate_limits.remove(dialog_id);
    mid_dialog_lock.remove(dialog_id);

    // Clean subscription_index entries for this dialog
    let keys: Vec<_> = subscription_index
        .iter()
        .filter(|entry| entry.key().0 == *dialog_id)
        .map(|entry| entry.key().clone())
        .collect();
    for key in keys {
        subscription_index.remove(&key);
    }

    debug!(dialog_id = ?dialog_id, "Cleaned up dialog state (rate limits, locks, subscriptions)");
}

fn cleanup_after_forward_failure(
    call_manager: &crate::call_manager::CallManager,
    dialog_manager: &sip_dialog::DialogManager,
    session_timer_manager: &SessionTimerManager,
    rate_limits: &dashmap::DashMap<sip_dialog::DialogId, RateWindow>,
    mid_dialog_lock: &dashmap::DashMap<sip_dialog::DialogId, Arc<tokio::sync::Mutex<()>>>,
    subscription_index: &dashmap::DashMap<(sip_dialog::DialogId, smol_str::SmolStr), sip_dialog::SubscriptionId>,
    call_id: &crate::call::CallId,
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
        call_id,
        dialog_id,
    );
}

fn find_call_and_dialog(
    call_manager: &crate::call_manager::CallManager,
    dialog_id: &sip_dialog::DialogId,
) -> Option<(sip_dialog::Dialog, crate::call::CallId, bool)> {
    for call in call_manager.all_calls() {
        if let Some(a) = &call.a_leg.dialog {
            if a.id == *dialog_id {
                return Some((a.clone(), call.id, true));
            }
        }
        if let Some(b_leg) = &call.b_leg {
            if let Some(d) = &b_leg.dialog {
                if d.id == *dialog_id {
                    return Some((d.clone(), call.id, false));
                }
            }
        }
    }
    None
}

fn stop_timers_for_call(
    session_timer_manager: &SessionTimerManager,
    call: &crate::call::Call,
) {
    if let Some(a) = &call.a_leg.dialog {
        session_timer_manager.stop_timer(&a.id);
    }
    if let Some(b_leg) = &call.b_leg {
        if let Some(d) = &b_leg.dialog {
            session_timer_manager.stop_timer(&d.id);
        }
    }
}


/// Helper to extract SIP URI from From header.
fn extract_from_uri(request: &sip_core::Request) -> anyhow::Result<sip_core::SipUri> {
    let from_header = sip_parse::header(&request.headers, "From")
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
        Some(sip_core::Uri::Sip(uri)) => Ok(uri),
        Some(sip_core::Uri::Tel(_)) => Err(anyhow::anyhow!("Tel URI in From header not supported for dialog creation")),
        None => Err(anyhow::anyhow!("Failed to parse From URI")),
    }
}

/// Helper to normalize a From URI that may have a complex username.
///
/// Some SIP phones put the full AOR in the username part, like:
///   sip:bob%40192.168.1.81@192.168.1.81 (username is bob@192.168.1.81)
///
/// This creates routing problems. We normalize to: sip:bob@192.168.1.81
fn normalize_from_uri(uri: sip_core::SipUri) -> sip_core::SipUri {
    if let Some(ref user) = uri.user {
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
            let normalized_str = format!("sip:{}@{}", simple_user, uri.host);

            // Parse and return normalized URI, or return original if parsing fails
            if let Some(sip_core::Uri::Sip(normalized)) = sip_core::Uri::parse(&normalized_str) {
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
    let from_header = sip_parse::header(&request.headers, "From")?;

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
    if let Some(sip_core::Uri::Sip(uri)) = sip_core::Uri::parse(uri_str) {
        if let Some(user) = uri.user {
            // URL decode the username (e.g., "bob%40192.168.1.81" -> "bob@192.168.1.81")
            let decoded = urlencoding::decode(&user).ok()?.into_owned();
            return Some(decoded);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registrar::FcpRegistrar;
    use fcp_core::{FileBackedStorage, RegistrationStore};
    use sip_core::{Headers, Method, RequestLine, SipUri};
    use std::sync::Arc;
    use sip_core::SipVersion;
    use bytes::Bytes as StdBytes;
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
        let call_manager = Arc::new(crate::call_manager::CallManager::new());

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
        );

        // Create a REGISTER request
        let mut headers = Headers::new();
        headers.push("Via".into(), "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123".into());
        headers.push("From".into(), "<sip:alice@example.com>;tag=abc123".into());
        headers.push("To".into(), "<sip:alice@example.com>".into());
        headers.push("Call-ID".into(), "test-call-id".into());
        headers.push("CSeq".into(), "1 REGISTER".into());
        headers.push("Contact".into(), "<sip:alice@127.0.0.1:5060>".into());
        headers.push("Content-Length".into(), "0".into());

        let request = sip_core::Request::new(
            RequestLine::new(Method::Register, SipUri::parse("sip:example.com").unwrap()),
            headers,
            bytes::Bytes::new(),
        );

        // Handle the REGISTER request
        let response = handler.handle_register(&request).await;

        // Should return 501 Not Implemented (no registrar configured)
        assert_eq!(response.start.code, 501);
        assert_eq!(response.start.reason.as_str(), "Not Implemented");
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
        let call_manager = Arc::new(crate::call_manager::CallManager::new());

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
        );

        let mut headers = Headers::new();
        headers.push("Via".into(), "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123".into());
        headers.push("From".into(), "<sip:alice@example.com>;tag=abc123".into());
        headers.push("To".into(), "<sip:bob@example.com>".into());
        headers.push("Call-ID".into(), "test-call-id".into());
        headers.push("CSeq".into(), "1 INVITE".into());
        headers.push("Content-Length".into(), "0".into());

        let request = sip_core::Request::new(
            RequestLine::new(Method::Invite, SipUri::parse("sip:example.com").unwrap()),
            headers,
            bytes::Bytes::new(),
        );

        let response = handler.create_simple_response(&request, 488, "Not Acceptable Here");
        assert_eq!(response.start.code, 488);
        let to = response.headers.get("To").unwrap();
        assert!(to.contains("tag="));
        assert!(response.headers.get("Contact").is_some());
        assert!(response.headers.get("Allow").is_some());
        assert!(response.headers.get("Supported").is_some());
        assert_eq!(
            response.headers.get("Content-Length").unwrap().as_str(),
            response.body.len().to_string()
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
        let call_manager = Arc::new(crate::call_manager::CallManager::new());

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
        );

        let mut headers = Headers::new();
        headers.push("Via".into(), "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bKxyz".into());
        headers.push("From".into(), "<sip:alice@example.com>;tag=abc123".into());
        headers.push("To".into(), "<sip:bob@example.com>;tag=bbb".into());
        headers.push("Call-ID".into(), "test-call-id-2".into());
        headers.push("CSeq".into(), "2 INVITE".into());
        headers.push("Content-Length".into(), "0".into());

        let request = sip_core::Request::new(
            RequestLine::new(Method::Invite, SipUri::parse("sip:example.com").unwrap()),
            headers,
            bytes::Bytes::new(),
        );

        let mut peer_response = handler.create_simple_response(&request, 481, "Call/Transaction Does Not Exist");
        // Add a dummy body to ensure Content-Length is recalculated
        peer_response.body = bytes::Bytes::from_static(b"oops");
        let response = handler.build_in_dialog_response(&request, &peer_response);
        assert_eq!(response.start.code, 481);
        assert!(response.headers.get("Contact").is_some());
        assert!(response.headers.get("Allow").is_some());
        assert!(response.headers.get("Supported").is_some());
        assert_eq!(
            response.headers.get("Content-Length").unwrap().as_str(),
            response.body.len().to_string()
        );
    }

    #[tokio::test]
    async fn session_timer_emits_refresh_for_refresher() {
        let manager = SessionTimerManager::new();
        let dialog_id = sip_dialog::DialogId::new("call1", "local", "remote");
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
        let dialog_id = sip_dialog::DialogId::new("call2", "local", "remote");
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

    #[test]
    fn dialog_refresher_role_matches_leg() {
        let uri = SipUri::parse("sip:alice@example.com").unwrap();
        let id = sip_dialog::DialogId::new("call3", "local", "remote");
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
        uas_dialog.refresher = Some(sip_core::RefresherRole::Uas);
        uas_dialog.is_uac = false;
        assert!(SipMessageHandler::dialog_is_refresher(&uas_dialog));

        let mut mismatch = base_dialog.clone();
        mismatch.refresher = Some(sip_core::RefresherRole::Uas);
        mismatch.is_uac = true;
        assert!(!SipMessageHandler::dialog_is_refresher(&mismatch));
    }

    #[tokio::test]
    async fn test_register_with_registrar() {
        // Create storage
        let temp_dir = std::env::temp_dir().join("fcp-test-handler-register");
        let storage = Arc::new(FileBackedStorage::new(temp_dir.clone()).await.unwrap());

        // Create registrar (without authentication)
        let registrar = Arc::new(FcpRegistrar::new(storage.clone(), None, None));

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
        let call_manager = Arc::new(crate::call_manager::CallManager::new());

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
        );

        // Create a REGISTER request
        let mut headers = Headers::new();
        headers.push("Via".into(), "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123".into());
        headers.push("From".into(), "<sip:alice@example.com>;tag=abc123".into());
        headers.push("To".into(), "<sip:alice@example.com>".into());
        headers.push("Call-ID".into(), "test-call-id".into());
        headers.push("CSeq".into(), "1 REGISTER".into());
        headers.push("Contact".into(), "<sip:alice@127.0.0.1:5060>;expires=3600".into());
        headers.push("Expires".into(), "3600".into());
        headers.push("Content-Length".into(), "0".into());

        let request = sip_core::Request::new(
            RequestLine::new(Method::Register, SipUri::parse("sip:example.com").unwrap()),
            headers,
            bytes::Bytes::new(),
        );

        // Handle the REGISTER request
        let response = handler.handle_register(&request).await;

        // Should return 200 OK (registration successful)
        assert_eq!(response.start.code, 200);
        assert_eq!(response.start.reason.as_str(), "OK");

        // Verify the registration was stored
        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].contact, "sip:alice@127.0.0.1:5060");

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir).await.ok();
    }

    fn build_invite_with_sdp(call_id: &str, has_audio: bool) -> sip_core::Request {
        let mut headers = Headers::new();
        headers.push("Via".into(), "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK123".into());
        headers.push("From".into(), "<sip:alice@example.com>;tag=abc123".into());
        headers.push("To".into(), "<sip:bob@example.com>".into());
        headers.push("Call-ID".into(), call_id.into());
        headers.push("CSeq".into(), "1 INVITE".into());
        headers.push("Contact".into(), "<sip:alice@127.0.0.1:5060>".into());
        headers.push("Content-Type".into(), "application/sdp".into());

        let sdp = if has_audio {
            "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 4000 RTP/AVP 0\r\n".to_string()
        } else {
            "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n".to_string()
        };
        headers.push("Content-Length".into(), format!("{}", sdp.len()).into());

        sip_core::Request {
            start: RequestLine {
                method: Method::Invite,
                uri: SipUri::parse("sip:bob@example.com").unwrap().into(),
                version: SipVersion::V2,
            },
            headers,
            body: StdBytes::from(sdp.into_bytes()),
        }
    }

    #[tokio::test]
    async fn test_store_sdp_on_reinvite() {
        let call = crate::call::Call::from_invite(
            build_invite_with_sdp("call-a", true),
            "127.0.0.1:5060".parse().unwrap(),
        );
        let call_id = call.id;
        let cm = crate::call_manager::CallManager::new();
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
        let mut req = build_invite_with_sdp("call-d", true);
        req.body = StdBytes::new();
        req.headers.push("Content-Length".into(), "0".into());
        let result = validate_sdp_body_static(&req, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_sdp_body_empty_with_prior_passes() {
        let mut req = build_invite_with_sdp("call-e", true);
        req.body = StdBytes::new();
        req.headers.push("Content-Length".into(), "0".into());
        let result = validate_sdp_body_static(&req, Some("v=0\r\ns=-\r\n"));
        assert!(result.is_ok());
    }
}
