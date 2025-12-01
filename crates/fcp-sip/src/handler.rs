//! Message handler for routing inbound SIP messages.

use anyhow::Result;
use bytes::Bytes;
use smol_str::SmolStr;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

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
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        dialog_manager: Arc<DialogManager>,
        local_uri: sip_core::SipUri,
        media_profile: Option<sip_sdp::profiles::MediaProfileBuilder>,
        local_addr: String,
        rtp_port: u16,
        registrar: Option<Arc<FcpRegistrar>>,
        routing_service: Option<Arc<fcp_routing::RoutingService>>,
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

    /// Handle an INVITE request by creating a dialog and sending responses.
    async fn handle_invite(
        &self,
        request: &sip_core::Request,
        handle: &ServerTransactionHandle,
        peer: SocketAddr,
    ) -> Result<()> {
        debug!("Handling INVITE request");

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

        // Send 180 Ringing
        let ringing = self.uas.create_ringing(request);
        handle.send_provisional(ringing).await;

        // Generate SDP answer if we have a media profile
        let sdp_answer = self.generate_sdp_answer(request)?;

        // Accept the INVITE and create dialog
        match self.uas.accept_invite(request, sdp_answer.as_deref()) {
            Ok((response, dialog)) => {
                debug!(
                    call_id = %dialog.id.call_id,
                    local_tag = %dialog.id.local_tag,
                    remote_tag = %dialog.id.remote_tag,
                    has_sdp = sdp_answer.is_some(),
                    "Created dialog for INVITE"
                );
                handle.send_final(response).await;
            }
            Err(e) => {
                error!("Failed to accept INVITE: {}", e);
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
    fn handle_ack(&self, request: &sip_core::Request) -> Result<()> {
        debug!("Received ACK");

        // Extract Call-ID and tags to find the dialog
        let call_id = sip_parse::header(&request.headers, "Call-ID")
            .ok_or_else(|| anyhow::anyhow!("Missing Call-ID in ACK"))?;

        // ACK confirms the dialog is established
        // The dialog was already created when we sent the 200 OK
        // Just log it for now
        debug!(call_id = %call_id, "ACK received for dialog");

        Ok(())
    }

    /// Handle a BYE request by terminating the dialog.
    fn handle_bye(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        debug!("Handling BYE request");

        // Find the dialog from the request headers (Call-ID + tags)
        let dialog = self
            .dialog_manager
            .find_by_request(request)
            .ok_or_else(|| anyhow::anyhow!("Dialog not found for BYE"))?;

        // Handle BYE using UAS helper
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
