//! Message handler for routing inbound SIP messages.

use anyhow::Result;
use bytes::Bytes;
use smol_str::SmolStr;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::prelude::*;
use crate::registrar::FcpRegistrar;
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
        // Create User Agent Server helper with shared dialog manager
        let mut uas = sip_uas::UserAgentServer::new(local_uri.clone(), local_uri);
        uas.dialog_manager = Arc::clone(&dialog_manager);

        Self {
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
    ) -> Result<()> {
        use sip_core::Method;

        match request.start.method {
            Method::Invite => {
                self.handle_invite(request, handle, peer).await?;
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
        let trying = sip_uas::UserAgentServer::create_trying(request);
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

        // Use the first registered contact as destination
        let destination_contact = &destination_contacts[0];
        let target_uri = destination_contact.contact.to_string();

        info!(
            call_id = %call_id,
            target = %target_uri,
            "Forwarding INVITE to B-leg"
        );

        // Update call state to Inviting
        let _ = self.call_manager.update_call(&call_id, |call| {
            call.state = crate::call::CallState::Inviting;
        });

        // Extract SDP from A-leg INVITE to forward to B-leg
        let a_leg_sdp = if !request.body.is_empty() {
            Some(String::from_utf8_lossy(&request.body).to_string())
        } else {
            None
        };

        // Create outbound INVITE to B-leg using UAC
        debug!(
            call_id = %call_id,
            target = %target_uri,
            has_sdp = a_leg_sdp.is_some(),
            "Creating B-leg INVITE"
        );

        let mut b_leg_call = match uac.invite(&target_uri, a_leg_sdp.as_deref()).await {
            Ok(call) => call,
            Err(e) => {
                error!(
                    call_id = %call_id,
                    target = %target_uri,
                    error = %e,
                    "Failed to create B-leg INVITE"
                );
                let response = self.create_simple_response(request, 500, "Failed to forward call");
                a_leg_handle.send_final(response).await;
                let _ = self.call_manager.terminate_call(&call_id, Some(format!("Failed to create B-leg INVITE: {}", e)));
                return Ok(());
            }
        };

        info!(
            call_id = %call_id,
            target = %target_uri,
            "B-leg INVITE sent, awaiting responses"
        );

        // Update call state to Proceeding
        let _ = self.call_manager.update_call(&call_id, |call| {
            call.state = crate::call::CallState::Proceeding;
        });

        // Relay provisional responses from B-leg to A-leg until we get a 180 Ringing
        info!(call_id = %call_id, "Waiting for provisional responses from B-leg");
        while let Some(provisional) = b_leg_call.await_provisional().await {
            let status_code = provisional.start.code;
            debug!(
                call_id = %call_id,
                code = status_code,
                "Received provisional response from B-leg"
            );

            // Forward 18x responses to A-leg (180 Ringing, 183 Session Progress)
            if status_code >= 180 && status_code < 200 {
                info!(
                    call_id = %call_id,
                    code = status_code,
                    "Relaying provisional response to A-leg"
                );

                // Create provisional response to relay to A-leg
                let response = if status_code == 180 {
                    self.uas.create_ringing(request)
                } else {
                    // For other 18x responses, create appropriate response
                    self.create_simple_response(request, status_code, provisional.start.reason.as_str())
                };

                info!(call_id = %call_id, code = status_code, "Sending provisional to A-leg");
                a_leg_handle.send_provisional(response).await;

                // After forwarding 180 Ringing, move on to wait for final response
                if status_code == 180 {
                    info!(call_id = %call_id, "Sent 180 Ringing, now waiting for final response");
                    break;
                }
            }
        }

        // Clone the B-leg dialog before consuming CallHandle
        let b_leg_dialog = b_leg_call.dialog.clone();

        // Wait for final response from B-leg
        info!(call_id = %call_id, "Waiting for final response from B-leg");
        let b_leg_final = match b_leg_call.await_final().await {
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

            // Accept the INVITE on A-leg and create A-leg dialog
            match self.uas.accept_invite(request, b_leg_sdp.as_deref()) {
                Ok((a_leg_response, a_leg_dialog)) => {
                    info!(
                        call_id = %call_id,
                        "Call established - both legs connected"
                    );

                    // Update call with both dialogs
                    let _ = self.call_manager.update_call(&call_id, |call| {
                        call.a_leg.dialog = Some(a_leg_dialog.clone());
                        // Store B-leg dialog information
                        if let Some(b_leg) = &mut call.b_leg {
                            b_leg.dialog = Some(b_leg_dialog.clone());
                        }
                        call.state = crate::call::CallState::Established;
                        call.establish();
                    });

                    // Send 200 OK to A-leg
                    a_leg_handle.send_final(a_leg_response).await;

                    // Note: ACK to B-leg is handled automatically by the UAC transaction layer
                    debug!(call_id = %call_id, "ACK will be sent automatically to B-leg");
                }
                Err(e) => {
                    error!(
                        call_id = %call_id,
                        error = %e,
                        "Failed to accept INVITE on A-leg"
                    );
                    let response = self.create_simple_response(request, 500, "Internal Server Error");
                    a_leg_handle.send_final(response).await;

                    // TODO: Send BYE to B-leg to terminate the established call
                    // This requires storing the UAC reference or B-leg dialog for later BYE
                    warn!(call_id = %call_id, "B-leg call established but A-leg failed - B-leg cleanup needed");

                    let _ = self.call_manager.terminate_call(&call_id, Some(format!("Failed to accept on A-leg: {}", e)));
                }
            }
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
        let ringing = self.uas.create_ringing(request);
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

                // Update call with A-leg dialog
                let _ = self.call_manager.update_call(&call_id, |call| {
                    call.a_leg.dialog = Some(dialog.clone());
                    call.state = crate::call::CallState::Established;
                    call.establish();
                });

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

                            // Spawn a task to send BYE to B-leg (don't wait for response)
                            let uac_clone = Arc::clone(uac);
                            let b_leg_dialog_clone = b_leg_dialog.clone();
                            let call_id = call.id;
                            tokio::spawn(async move {
                                match uac_clone.bye(&b_leg_dialog_clone).await {
                                    Ok(_) => {
                                        debug!(call_id = %call_id, "Successfully sent BYE to B-leg");
                                    }
                                    Err(e) => {
                                        error!(call_id = %call_id, error = %e, "Failed to send BYE to B-leg");
                                    }
                                }
                            });
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

                        // Spawn a task to send BYE to A-leg (don't wait for response)
                        let uac_clone = Arc::clone(uac);
                        let a_leg_dialog_clone = a_leg_dialog.clone();
                        let call_id = call.id;
                        tokio::spawn(async move {
                            match uac_clone.bye(&a_leg_dialog_clone).await {
                                Ok(_) => {
                                    debug!(call_id = %call_id, "Successfully sent BYE to A-leg");
                                }
                                Err(e) => {
                                    error!(call_id = %call_id, error = %e, "Failed to send BYE to A-leg");
                                }
                            }
                        });
                    } else {
                        debug!(call_id = %call.id, "A-leg has no dialog, skipping BYE forward");
                    }
                }
            } else {
                debug!(call_id = %call.id, "No UAC available, cannot forward BYE");
            }

            // Terminate the call in CallManager
            let _ = self.call_manager.terminate_call(&call.id, None);
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
            headers.push(SmolStr::new("To"), to.clone());
        }
        if let Some(call_id) = request.headers.get("Call-ID") {
            headers.push(SmolStr::new("Call-ID"), call_id.clone());
        }
        if let Some(cseq) = request.headers.get("CSeq") {
            headers.push(SmolStr::new("CSeq"), cseq.clone());
        }

        // Content-Length: 0 (no body)
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        sip_core::Response::new(
            StatusLine::new(code, SmolStr::new(reason)),
            headers,
            Bytes::new(),
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registrar::FcpRegistrar;
    use fcp_core::{FileBackedStorage, RegistrationStore};
    use sip_core::{Headers, Method, RequestLine, SipUri};
    use std::sync::Arc;

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
}
