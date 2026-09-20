//! SBC enforcement handler — decorator around `SipMessageHandler`.
//!
//! Implements `InboundMessageHandler` and enforces security policies
//! (IP ACL, message size, rate limiting, admission control) before
//! delegating to the inner B2BUA handler.
//!
//! SIP responses (starting with "SIP/") bypass security enforcement — they
//! are replies to our own outbound requests and must reach the transaction
//! layer to avoid timeouts. However, ingress header manipulation rules are
//! still applied to responses so that trunk-specific header stripping
//! (e.g. removing `Server` headers) works for both requests and responses.

use crate::metrics::SbcRuntimeMetrics;
use crate::stir_shaken::{StirMetrics, StirVerifier, VerificationResult};
use bytes::Bytes;
use fcp_sbc_core::admission::AdmissionController;
use fcp_sbc_core::security::{AclAction, MalformedSipPolicy, SecurityConfig};
use fcp_sbc_core::stir_shaken::{StirVerifyMode, TrunkStirShakenConfig};
use fcp_sbc_core::HeaderManipulationEngine;
use fcp_sip::listener::InboundMessageHandler;
use fcp_sip::transport::RealTransportDispatcher;
use sip_ratelimit::RateLimiterSet;
use sip_transaction::{TransportContext, TransportDispatcher};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Enforcement decorator wrapping the inner SIP message handler.
///
/// Enforces (in order):
/// 1. Response bypass (SIP responses skip security enforcement but get
///    ingress HMR applied before delegation)
/// 2. IP ACL check
/// 3. Message size limit
/// 4. Rate limiting (per-IP via `sip-ratelimit`)
/// 5. Malformed SIP structure check
/// 6. Method-specific rate limiting + admission control
/// 7. Apply ingress header manipulation rules (requests and responses)
/// 8. Delegate to inner `SipMessageHandler`
pub struct SbcEnforcementHandler {
    inner: Arc<dyn InboundMessageHandler>,
    security: SecurityConfig,
    rate_limiter: Arc<RateLimiterSet>,
    admission: Arc<AdmissionController>,
    metrics: Arc<SbcRuntimeMetrics>,
    dispatcher: Arc<RealTransportDispatcher>,
    hmr_engine: Option<Arc<HeaderManipulationEngine>>,
    /// Maps peer IP to trunk name for trunk identification on ingress.
    peer_trunk_map: HashMap<IpAddr, String>,
    /// STIR/SHAKEN verifier for inbound calls.
    stir_verifier: Option<Arc<StirVerifier>>,
    /// Per-trunk STIR/SHAKEN verification configuration.
    trunk_stir_configs: HashMap<String, TrunkStirShakenConfig>,
    /// STIR/SHAKEN metrics.
    stir_metrics: Option<Arc<StirMetrics>>,
}

impl SbcEnforcementHandler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        inner: Arc<dyn InboundMessageHandler>,
        security: SecurityConfig,
        rate_limiter: Arc<RateLimiterSet>,
        admission: Arc<AdmissionController>,
        metrics: Arc<SbcRuntimeMetrics>,
        dispatcher: Arc<RealTransportDispatcher>,
        hmr_engine: Option<Arc<HeaderManipulationEngine>>,
        peer_trunk_map: HashMap<IpAddr, String>,
        stir_verifier: Option<Arc<StirVerifier>>,
        trunk_stir_configs: HashMap<String, TrunkStirShakenConfig>,
        stir_metrics: Option<Arc<StirMetrics>>,
    ) -> Self {
        Self {
            inner,
            security,
            rate_limiter,
            admission,
            metrics,
            dispatcher,
            hmr_engine,
            peer_trunk_map,
            stir_verifier,
            trunk_stir_configs,
            stir_metrics,
        }
    }

    /// Look up the trunk name for a given peer address.
    fn identify_trunk_by_peer(&self, peer: SocketAddr) -> Option<&str> {
        self.peer_trunk_map.get(&peer.ip()).map(|s| s.as_str())
    }

    /// Send a SIP rejection response back to the peer.
    ///
    /// Uses the TCP stream if available, otherwise sends via the dispatcher.
    async fn send_rejection(
        &self,
        transport: sip_transaction::TransportKind,
        peer: SocketAddr,
        payload: &[u8],
        stream: &Option<mpsc::Sender<Bytes>>,
        code: u16,
        reason: &str,
    ) {
        if let Some(response_bytes) = build_rejection_response(payload, code, reason) {
            if let Some(tx) = stream {
                // TCP/TLS — use the existing stream
                let _ = tx.send(response_bytes).await;
            } else {
                // UDP — send via dispatcher
                let ctx = TransportContext::new(transport, peer, None);
                let _ = self.dispatcher.dispatch(&ctx, response_bytes).await;
            }
        }
        // If we can't build a response (truly malformed), silent drop is correct
    }
}

#[async_trait::async_trait]
impl InboundMessageHandler for SbcEnforcementHandler {
    async fn handle_inbound(
        &self,
        transport: sip_transaction::TransportKind,
        peer: SocketAddr,
        payload: Bytes,
        stream: Option<mpsc::Sender<Bytes>>,
        peer_identity: Option<Arc<sip_transport::PeerIdentity>>,
    ) -> anyhow::Result<()> {
        // 0. SIP responses bypass security enforcement — they are replies to
        //    our own outbound requests and must reach the transaction layer.
        //    However, ingress HMR is still applied so trunk-specific header
        //    stripping works on responses too.
        if payload.starts_with(b"SIP/") {
            let payload = if let Some(ref engine) = self.hmr_engine {
                let trunk = self.identify_trunk_by_peer(peer);
                if let Some(mut resp) = sip_parse::parse_response(&payload) {
                    engine.apply_response_ingress(&mut resp, trunk);
                    sip_parse::serialize_response(&resp)
                } else {
                    payload
                }
            } else {
                payload
            };
            return self
                .inner
                .handle_inbound(transport, peer, payload, stream, peer_identity)
                .await;
        }

        let peer_ip = peer.ip();

        // 1. IP ACL check
        if self.security.signaling_acl.evaluate(peer_ip) == AclAction::Deny {
            warn!(%peer, "SBC: ACL denied");
            self.metrics.record_security_rejection();
            self.metrics.record_per_ip_security(peer_ip);
            self.send_rejection(transport, peer, &payload, &stream, 403, "Forbidden")
                .await;
            return Ok(());
        }

        // 2. Message size check
        if payload.len() > self.security.max_message_size {
            warn!(%peer, len = payload.len(), max = self.security.max_message_size, "SBC: message too large");
            self.metrics.record_security_rejection();
            self.metrics.record_per_ip_security(peer_ip);
            self.send_rejection(transport, peer, &payload, &stream, 513, "Message Too Large")
                .await;
            return Ok(());
        }

        // 3. Rate limit check — connections limiter as a general per-IP gate
        let ip_str = peer_ip.to_string();
        if !self.rate_limiter.connections().check_rate_limit(&ip_str) {
            debug!(%peer, "SBC: rate limited (connections)");
            self.metrics.record_rate_limit_rejection();
            self.metrics.record_per_ip_rate_limit(peer_ip);
            self.send_rejection(transport, peer, &payload, &stream, 429, "Too Many Requests")
                .await;
            return Ok(());
        }

        // 4. Malformed SIP structure check
        if !validate_sip_structure(&payload) {
            match self.security.malformed_sip_policy {
                MalformedSipPolicy::Drop => {
                    self.metrics.record_security_rejection();
                    self.metrics.record_per_ip_security(peer_ip);
                    return Ok(());
                }
                MalformedSipPolicy::Reject => {
                    warn!(%peer, "SBC: malformed SIP request — rejecting with 400");
                    self.metrics.record_security_rejection();
                    self.metrics.record_per_ip_security(peer_ip);
                    self.send_rejection(transport, peer, &payload, &stream, 400, "Bad Request")
                        .await;
                    return Ok(());
                }
                MalformedSipPolicy::LogAndDrop => {
                    warn!(%peer, "SBC: malformed SIP request — dropping");
                    self.metrics.record_security_rejection();
                    self.metrics.record_per_ip_security(peer_ip);
                    return Ok(());
                }
            }
        }

        // 5. Method-specific rate limiting: quick-parse the first line to extract method
        if let Some(method) = extract_method_from_payload(&payload) {
            let allowed = match method {
                "INVITE" => self.rate_limiter.invite().check_rate_limit(&ip_str),
                "REGISTER" => self.rate_limiter.register().check_rate_limit(&ip_str),
                _ => true, // Other methods pass through (covered by connections limiter)
            };
            if !allowed {
                debug!(%peer, method, "SBC: rate limited (method)");
                self.metrics.record_rate_limit_rejection();
                self.metrics.record_per_ip_rate_limit(peer_ip);
                self.send_rejection(transport, peer, &payload, &stream, 429, "Too Many Requests")
                    .await;
                return Ok(());
            }

            // 6a. Admission control for INVITE (new call attempts)
            if method == "INVITE" {
                self.metrics.record_call_attempt();
                self.metrics.record_per_ip_call_attempt(peer_ip);
                match self.admission.try_admit_or_queue() {
                    fcp_sbc_core::admission::AdmitResult::Admitted => {}
                    fcp_sbc_core::admission::AdmitResult::Rejected => {
                        warn!(%peer, "SBC: admission control rejected (overloaded)");
                        self.metrics.record_admission_rejection();
                        self.metrics.record_per_ip_rejection(peer_ip);
                        self.send_rejection(
                            transport,
                            peer,
                            &payload,
                            &stream,
                            503,
                            "Service Unavailable",
                        )
                        .await;
                        return Ok(());
                    }
                    fcp_sbc_core::admission::AdmitResult::Queued(rx) => {
                        let cfg = self.admission.config_snapshot();
                        let timeout_dur = std::time::Duration::from_secs(cfg.queue_timeout_secs);
                        match tokio::time::timeout(timeout_dur, rx).await {
                            Ok(Ok(true)) => {
                                // Admitted from queue
                            }
                            _ => {
                                // Timeout or channel closed or rejected
                                warn!(%peer, "SBC: queued call timed out");
                                self.metrics.record_admission_rejection();
                                self.metrics.record_per_ip_rejection(peer_ip);
                                self.send_rejection(
                                    transport,
                                    peer,
                                    &payload,
                                    &stream,
                                    503,
                                    "Service Unavailable",
                                )
                                .await;
                                return Ok(());
                            }
                        }
                    }
                }
            }

            // 6b. Admission control for REGISTER
            if method == "REGISTER" && !self.admission.try_admit_registration() {
                warn!(%peer, "SBC: registration admission rejected");
                self.metrics.record_admission_rejection();
                self.send_rejection(
                    transport,
                    peer,
                    &payload,
                    &stream,
                    503,
                    "Service Unavailable",
                )
                .await;
                return Ok(());
            }
        }

        // 7. Apply ingress header manipulation rules (if enabled)
        let payload = if let Some(ref engine) = self.hmr_engine {
            let trunk = self.identify_trunk_by_peer(peer);
            if let Some(mut req) = sip_parse::parse_request(&payload) {
                engine.apply_request_ingress(&mut req, trunk);
                sip_parse::serialize_request(&req)
            } else if let Some(mut resp) = sip_parse::parse_response(&payload) {
                engine.apply_response_ingress(&mut resp, trunk);
                sip_parse::serialize_response(&resp)
            } else {
                payload // unparseable, pass through
            }
        } else {
            payload
        };

        // 7b. STIR/SHAKEN verification (INVITE only)
        if let Some(ref verifier) = self.stir_verifier {
            if let Some(method) = extract_method_from_payload(&payload) {
                if method == "INVITE" {
                    let trunk_name = self.identify_trunk_by_peer(peer);
                    let trunk_config =
                        trunk_name.and_then(|name| self.trunk_stir_configs.get(name));
                    let verify_mode = trunk_config
                        .map(|c| c.verify_mode)
                        .unwrap_or(StirVerifyMode::Disabled);

                    if verify_mode != StirVerifyMode::Disabled {
                        let max_iat_age = trunk_config.map(|c| c.max_iat_age_secs).unwrap_or(60);

                        let identity = fcp_sbc_core::stir_shaken::extract_header_from_payload(
                            &payload, "Identity",
                        );
                        let from = fcp_sbc_core::stir_shaken::extract_header_from_payload(
                            &payload, "From",
                        )
                        .or_else(|| {
                            fcp_sbc_core::stir_shaken::extract_header_from_payload(&payload, "f")
                        })
                        .unwrap_or_default();
                        let to =
                            fcp_sbc_core::stir_shaken::extract_header_from_payload(&payload, "To")
                                .or_else(|| {
                                    fcp_sbc_core::stir_shaken::extract_header_from_payload(
                                        &payload, "t",
                                    )
                                })
                                .unwrap_or_default();

                        if let Some(identity_value) = identity {
                            let result = verifier
                                .verify(&identity_value, &from, &to, max_iat_age)
                                .await;

                            match result {
                                VerificationResult::Verified { attestation } => {
                                    info!(
                                        %peer,
                                        attestation = attestation.as_str(),
                                        "STIR/SHAKEN verification succeeded"
                                    );
                                    if let Some(ref m) = self.stir_metrics {
                                        m.record_verification_success();
                                    }
                                }
                                VerificationResult::Failed(ref err) => {
                                    warn!(
                                        %peer,
                                        error = %err,
                                        mode = ?verify_mode,
                                        "STIR/SHAKEN verification failed"
                                    );
                                    if let Some(ref m) = self.stir_metrics {
                                        m.record_verification_failure();
                                    }
                                    if verify_mode == StirVerifyMode::Enforce {
                                        self.send_rejection(
                                            transport,
                                            peer,
                                            &payload,
                                            &stream,
                                            438,
                                            "Invalid Identity Header",
                                        )
                                        .await;
                                        return Ok(());
                                    }
                                    // LogOnly: warn logged above, continue
                                }
                                VerificationResult::NoIdentity => {
                                    // Should not happen here since we checked identity above
                                    if let Some(ref m) = self.stir_metrics {
                                        m.record_verification_no_identity();
                                    }
                                }
                            }
                        } else {
                            debug!(%peer, "STIR/SHAKEN: no Identity header present");
                            if let Some(ref m) = self.stir_metrics {
                                m.record_verification_no_identity();
                            }
                        }
                    }
                }
            }
        }

        // 8. Delegate to inner handler
        self.inner
            .handle_inbound(transport, peer, payload, stream, peer_identity)
            .await
    }
}

/// Build a minimal SIP rejection response from a raw request payload.
///
/// Extracts the mandatory response headers (Via, From, To, Call-ID, CSeq)
/// from the raw bytes and constructs a minimal response. Returns `None` if
/// the payload is too malformed to extract headers from.
fn build_rejection_response(payload: &[u8], code: u16, reason: &str) -> Option<Bytes> {
    let payload_str = std::str::from_utf8(payload).ok()?;

    let mut via = None;
    let mut from = None;
    let mut to = None;
    let mut call_id = None;
    let mut cseq = None;

    // Skip request line, parse headers
    for line in payload_str.lines().skip(1) {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.is_empty() {
            break; // End of headers
        }
        let lower = trimmed.to_ascii_lowercase();
        if via.is_none() && (lower.starts_with("via:") || lower.starts_with("v:")) {
            via = Some(trimmed);
        } else if from.is_none() && (lower.starts_with("from:") || lower.starts_with("f:")) {
            from = Some(trimmed);
        } else if to.is_none() && (lower.starts_with("to:") || lower.starts_with("t:")) {
            to = Some(trimmed);
        } else if call_id.is_none() && (lower.starts_with("call-id:") || lower.starts_with("i:")) {
            call_id = Some(trimmed);
        } else if cseq.is_none() && lower.starts_with("cseq:") {
            cseq = Some(trimmed);
        }
    }

    // All five headers are mandatory for a valid response
    let response = format!(
        "SIP/2.0 {} {}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\nContent-Length: 0\r\n\r\n",
        code, reason, via?, from?, to?, call_id?, cseq?
    );

    Some(Bytes::from(response))
}

/// Lightweight structural validation of a raw SIP request.
///
/// Checks that the payload has:
/// 1. A recognizable request line (METHOD SP URI SP SIP/2.0)
/// 2. The five mandatory headers: Via, From, To, Call-ID, CSeq
///
/// This does NOT perform a full SIP parse — it only scans for header
/// presence to reject obviously malformed messages before delegation.
fn validate_sip_structure(payload: &[u8]) -> bool {
    // Must be valid UTF-8 to be a SIP message
    let text = match std::str::from_utf8(payload) {
        Ok(t) => t,
        Err(_) => return false,
    };

    let mut lines = text.lines();

    // Request line: METHOD SP URI SP SIP/2.0
    let request_line = match lines.next() {
        Some(l) => l.trim_end_matches('\r'),
        None => return false,
    };
    if !request_line.ends_with("SIP/2.0") {
        return false;
    }
    // Need at least three parts: METHOD uri SIP/2.0
    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return false;
    }

    // Scan headers for the 5 mandatory ones
    let mut has_via = false;
    let mut has_from = false;
    let mut has_to = false;
    let mut has_call_id = false;
    let mut has_cseq = false;

    for line in lines {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.is_empty() {
            break; // end of headers
        }
        let lower = trimmed.to_ascii_lowercase();
        if !has_via && (lower.starts_with("via:") || lower.starts_with("v:")) {
            has_via = true;
        } else if !has_from && (lower.starts_with("from:") || lower.starts_with("f:")) {
            has_from = true;
        } else if !has_to && (lower.starts_with("to:") || lower.starts_with("t:")) {
            has_to = true;
        } else if !has_call_id && (lower.starts_with("call-id:") || lower.starts_with("i:")) {
            has_call_id = true;
        } else if !has_cseq && lower.starts_with("cseq:") {
            has_cseq = true;
        }
    }

    has_via && has_from && has_to && has_call_id && has_cseq
}

/// Quick-extract the SIP method from the first line of a raw SIP message.
///
/// SIP requests start with `METHOD SP URI SP SIP/2.0\r\n`.
/// Responses start with `SIP/2.0 SP status SP reason\r\n`.
/// Returns `None` for responses or unparseable payloads.
fn extract_method_from_payload(payload: &[u8]) -> Option<&str> {
    // Skip if it looks like a response (starts with "SIP/")
    if payload.starts_with(b"SIP/") {
        return None;
    }

    // Find the first space — everything before it is the method
    let space_pos = payload.iter().position(|&b| b == b' ')?;
    if space_pos == 0 || space_pos > 20 {
        return None; // SIP methods are short (<= ~10 chars)
    }

    std::str::from_utf8(&payload[..space_pos]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_method_invite() {
        let payload = b"INVITE sip:bob@example.com SIP/2.0\r\n";
        assert_eq!(extract_method_from_payload(payload), Some("INVITE"));
    }

    #[test]
    fn test_extract_method_register() {
        let payload = b"REGISTER sip:example.com SIP/2.0\r\n";
        assert_eq!(extract_method_from_payload(payload), Some("REGISTER"));
    }

    #[test]
    fn test_extract_method_response() {
        let payload = b"SIP/2.0 200 OK\r\n";
        assert_eq!(extract_method_from_payload(payload), None);
    }

    #[test]
    fn test_extract_method_empty() {
        assert_eq!(extract_method_from_payload(b""), None);
    }

    #[test]
    fn test_extract_method_options() {
        let payload = b"OPTIONS sip:example.com SIP/2.0\r\n";
        assert_eq!(extract_method_from_payload(payload), Some("OPTIONS"));
    }

    #[test]
    fn test_build_rejection_response_basic() {
        let payload = b"INVITE sip:bob@example.com SIP/2.0\r\n\
            Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:bob@example.com>\r\n\
            Call-ID: test123@10.0.0.1\r\n\
            CSeq: 1 INVITE\r\n\
            Content-Length: 0\r\n\
            \r\n";

        let response = build_rejection_response(payload, 403, "Forbidden");
        assert!(response.is_some());
        let resp_str = String::from_utf8(response.unwrap().to_vec()).unwrap();
        assert!(resp_str.starts_with("SIP/2.0 403 Forbidden\r\n"));
        assert!(resp_str.contains("Via:"));
        assert!(resp_str.contains("From:"));
        assert!(resp_str.contains("To:"));
        assert!(resp_str.contains("Call-ID:"));
        assert!(resp_str.contains("CSeq:"));
        assert!(resp_str.contains("Content-Length: 0"));
    }

    #[test]
    fn test_build_rejection_response_malformed() {
        // Missing required headers
        let payload = b"INVITE sip:bob@example.com SIP/2.0\r\n\r\n";
        let response = build_rejection_response(payload, 403, "Forbidden");
        assert!(response.is_none());
    }

    #[test]
    fn test_build_rejection_response_compact_headers() {
        let payload = b"INVITE sip:bob@example.com SIP/2.0\r\n\
            v: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
            f: <sip:alice@example.com>;tag=abc\r\n\
            t: <sip:bob@example.com>\r\n\
            i: test123@10.0.0.1\r\n\
            CSeq: 1 INVITE\r\n\
            \r\n";

        let response = build_rejection_response(payload, 429, "Too Many Requests");
        assert!(response.is_some());
        let resp_str = String::from_utf8(response.unwrap().to_vec()).unwrap();
        assert!(resp_str.starts_with("SIP/2.0 429 Too Many Requests\r\n"));
    }

    // --- validate_sip_structure tests ---

    #[test]
    fn test_validate_valid_minimal_request() {
        let payload = b"INVITE sip:bob@example.com SIP/2.0\r\n\
            Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:bob@example.com>\r\n\
            Call-ID: test123@10.0.0.1\r\n\
            CSeq: 1 INVITE\r\n\
            \r\n";
        assert!(validate_sip_structure(payload));
    }

    #[test]
    fn test_validate_compact_headers() {
        let payload = b"OPTIONS sip:example.com SIP/2.0\r\n\
            v: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
            f: <sip:alice@example.com>;tag=abc\r\n\
            t: <sip:bob@example.com>\r\n\
            i: test123@10.0.0.1\r\n\
            CSeq: 1 OPTIONS\r\n\
            \r\n";
        assert!(validate_sip_structure(payload));
    }

    #[test]
    fn test_validate_missing_via() {
        let payload = b"INVITE sip:bob@example.com SIP/2.0\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:bob@example.com>\r\n\
            Call-ID: test123@10.0.0.1\r\n\
            CSeq: 1 INVITE\r\n\
            \r\n";
        assert!(!validate_sip_structure(payload));
    }

    #[test]
    fn test_validate_missing_cseq() {
        let payload = b"INVITE sip:bob@example.com SIP/2.0\r\n\
            Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:bob@example.com>\r\n\
            Call-ID: test123@10.0.0.1\r\n\
            \r\n";
        assert!(!validate_sip_structure(payload));
    }

    #[test]
    fn test_validate_empty_payload() {
        assert!(!validate_sip_structure(b""));
    }

    #[test]
    fn test_validate_bad_request_line() {
        let payload = b"GARBAGE LINE HERE\r\n\
            Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:bob@example.com>\r\n\
            Call-ID: test123@10.0.0.1\r\n\
            CSeq: 1 INVITE\r\n\
            \r\n";
        assert!(!validate_sip_structure(payload));
    }

    #[test]
    fn test_validate_non_utf8() {
        let payload: &[u8] = &[0xFF, 0xFE, 0x00, 0x01];
        assert!(!validate_sip_structure(payload));
    }

    #[test]
    fn test_validate_headers_only_no_request_line() {
        // Starts with a header, not a request line
        let payload = b"Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:bob@example.com>\r\n\
            Call-ID: test123@10.0.0.1\r\n\
            CSeq: 1 INVITE\r\n\
            \r\n";
        assert!(!validate_sip_structure(payload));
    }

    // --- Ingress HMR on SIP responses ---

    #[test]
    fn test_response_ingress_hmr_removes_server_header() {
        use fcp_sbc_core::hmr::*;
        use fcp_sbc_core::HeaderManipulationEngine;

        // Build an HMR rule that removes the Server header on ingress responses
        let cfg = HeaderManipulationConfig {
            enabled: true,
            rules: vec![HmrRule {
                name: "strip-server".into(),
                priority: 10,
                enabled: true,
                direction: HmrDirection::Ingress,
                message_type: HmrMessageType::Response,
                methods: vec![],
                conditions: vec![],
                actions: vec![HmrAction {
                    action_type: HmrActionType::Remove,
                    header_name: "Server".into(),
                    value: None,
                    pattern: None,
                }],
            }],
        };
        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();

        // Build a raw SIP response with a Server header
        let raw = Bytes::from(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
             From: <sip:alice@example.com>;tag=abc\r\n\
             To: <sip:bob@example.com>;tag=xyz\r\n\
             Call-ID: test123@10.0.0.1\r\n\
             CSeq: 1 INVITE\r\n\
             Server: BadServer/1.0\r\n\
             Content-Length: 0\r\n\
             \r\n",
        );

        // Parse → apply HMR → serialize (same pipeline as enforcement handler)
        let mut resp = sip_parse::parse_response(&raw).expect("should parse response");
        assert!(resp.headers().get("Server").is_some());

        engine.apply_response_ingress(&mut resp, None);
        assert!(resp.headers().get("Server").is_none());

        let serialized = sip_parse::serialize_response(&resp);
        let text = std::str::from_utf8(&serialized).unwrap();
        assert!(text.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(!text.contains("Server:"));
        assert!(!text.contains("BadServer"));
    }

    // ===================================================================
    // Integration tests with MockInnerHandler
    // ===================================================================

    /// Mock inner handler that records all calls for inspection.
    struct MockInnerHandler {
        calls: SentMessages,
    }

    /// Messages captured by the mock handler, shared with the assertions.
    type SentMessages =
        Arc<tokio::sync::Mutex<Vec<(sip_transaction::TransportKind, SocketAddr, Bytes)>>>;

    impl MockInnerHandler {
        fn new() -> (Self, SentMessages) {
            let calls = Arc::new(tokio::sync::Mutex::new(Vec::new()));
            (
                Self {
                    calls: Arc::clone(&calls),
                },
                calls,
            )
        }
    }

    #[async_trait::async_trait]
    impl InboundMessageHandler for MockInnerHandler {
        async fn handle_inbound(
            &self,
            transport: sip_transaction::TransportKind,
            peer: SocketAddr,
            payload: Bytes,
            _stream: Option<mpsc::Sender<Bytes>>,
            _peer_identity: Option<Arc<sip_transport::PeerIdentity>>,
        ) -> anyhow::Result<()> {
            self.calls.lock().await.push((transport, peer, payload));
            Ok(())
        }
    }

    /// A valid SIP INVITE payload for testing.
    fn valid_invite() -> Bytes {
        Bytes::from(
            "INVITE sip:bob@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
             From: <sip:alice@example.com>;tag=abc\r\n\
             To: <sip:bob@example.com>\r\n\
             Call-ID: test123@10.0.0.1\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\
             \r\n",
        )
    }

    /// A valid SIP REGISTER payload for testing.
    fn valid_register() -> Bytes {
        Bytes::from(
            "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
             From: <sip:alice@example.com>;tag=abc\r\n\
             To: <sip:alice@example.com>\r\n\
             Call-ID: reg456@10.0.0.1\r\n\
             CSeq: 1 REGISTER\r\n\
             Content-Length: 0\r\n\
             \r\n",
        )
    }

    /// A valid SIP OPTIONS payload for testing.
    fn valid_options() -> Bytes {
        Bytes::from(
            "OPTIONS sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
             From: <sip:alice@example.com>;tag=abc\r\n\
             To: <sip:bob@example.com>\r\n\
             Call-ID: opt789@10.0.0.1\r\n\
             CSeq: 1 OPTIONS\r\n\
             Content-Length: 0\r\n\
             \r\n",
        )
    }

    /// Build a test enforcement handler with configurable options.
    fn build_test_handler(
        inner: Arc<dyn InboundMessageHandler>,
        security: SecurityConfig,
        admission: fcp_sbc_core::AdmissionConfig,
        rate_limiter: Option<Arc<RateLimiterSet>>,
        hmr_engine: Option<Arc<HeaderManipulationEngine>>,
    ) -> SbcEnforcementHandler {
        let admission = Arc::new(fcp_sbc_core::admission::AdmissionController::new(admission));
        build_test_handler_with(inner, security, admission, rate_limiter, hmr_engine)
    }

    /// As [`build_test_handler`], with a controller the test has already
    /// loaded — a node that is carrying calls or holding registrations.
    fn build_test_handler_with(
        inner: Arc<dyn InboundMessageHandler>,
        security: SecurityConfig,
        admission: Arc<AdmissionController>,
        rate_limiter: Option<Arc<RateLimiterSet>>,
        hmr_engine: Option<Arc<HeaderManipulationEngine>>,
    ) -> SbcEnforcementHandler {
        let dispatcher = Arc::new(fcp_sip::transport::RealTransportDispatcher::new(None));
        let rate_limiter = rate_limiter.unwrap_or_else(|| Arc::new(RateLimiterSet::disabled()));
        let metrics = Arc::new(SbcRuntimeMetrics::new());
        SbcEnforcementHandler::new(
            inner,
            security,
            rate_limiter,
            admission,
            metrics,
            dispatcher,
            hmr_engine,
            HashMap::new(),
            None,
            HashMap::new(),
            None,
        )
    }

    /// Default peer address for tests.
    fn test_peer() -> SocketAddr {
        "10.0.0.1:5060".parse().unwrap()
    }

    #[tokio::test]
    async fn test_acl_deny_sends_403() {
        use fcp_sbc_core::security::{IpAcl, IpAclEntry};

        let (mock, calls) = MockInnerHandler::new();
        let security = SecurityConfig {
            signaling_acl: IpAcl {
                rules: vec![IpAclEntry {
                    cidr: "10.0.0.0/8".into(),
                    action: AclAction::Deny,
                    description: None,
                }],
                default_action: AclAction::Allow,
            },
            ..Default::default()
        };

        let handler = build_test_handler(
            Arc::new(mock),
            security,
            fcp_sbc_core::AdmissionConfig::default(),
            None,
            None,
        );

        // Use a TCP stream channel to capture the rejection response
        let (tx, mut rx) = mpsc::channel::<Bytes>(8);
        handler
            .handle_inbound(
                sip_transaction::TransportKind::Tcp,
                test_peer(),
                valid_invite(),
                Some(tx),
                None,
            )
            .await
            .unwrap();

        // Inner should NOT be called
        assert!(calls.lock().await.is_empty());

        // Should receive a 403 response
        let resp = rx.recv().await.expect("should receive rejection response");
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("SIP/2.0 403 Forbidden"));
    }

    #[tokio::test]
    async fn test_acl_allow_delegates() {
        let (mock, calls) = MockInnerHandler::new();
        let handler = build_test_handler(
            Arc::new(mock),
            SecurityConfig::default(), // default allows all
            fcp_sbc_core::AdmissionConfig::default(),
            None,
            None,
        );

        handler
            .handle_inbound(
                sip_transaction::TransportKind::Udp,
                test_peer(),
                valid_options(),
                None,
                None,
            )
            .await
            .unwrap();

        // Inner should be called with the payload
        let recorded = calls.lock().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].1, test_peer());
    }

    #[tokio::test]
    async fn test_message_too_large_sends_513() {
        let (mock, calls) = MockInnerHandler::new();
        let security = SecurityConfig {
            max_message_size: 100, // very small limit
            ..Default::default()
        };

        let handler = build_test_handler(
            Arc::new(mock),
            security,
            fcp_sbc_core::AdmissionConfig::default(),
            None,
            None,
        );

        let (tx, mut rx) = mpsc::channel::<Bytes>(8);
        handler
            .handle_inbound(
                sip_transaction::TransportKind::Tcp,
                test_peer(),
                valid_invite(), // > 100 bytes
                Some(tx),
                None,
            )
            .await
            .unwrap();

        assert!(calls.lock().await.is_empty());

        let resp = rx.recv().await.expect("should receive 513 response");
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("SIP/2.0 513 Message Too Large"));
    }

    #[tokio::test]
    async fn test_rate_limit_sends_429() {
        use sip_ratelimit::RateLimitConfig;

        let (mock, calls) = MockInnerHandler::new();
        // 1 request per 60 seconds — very restrictive
        let connections_cfg = RateLimitConfig::new(1, 60).unwrap().with_enabled(true);
        let rate_limiter = Arc::new(RateLimiterSet::with_configs(
            RateLimitConfig::disabled(),
            RateLimitConfig::disabled(),
            RateLimitConfig::disabled(),
            connections_cfg,
        ));

        let handler = build_test_handler(
            Arc::new(mock),
            SecurityConfig::default(),
            fcp_sbc_core::AdmissionConfig::default(),
            Some(rate_limiter),
            None,
        );

        // First request should pass (consumes the 1 token)
        handler
            .handle_inbound(
                sip_transaction::TransportKind::Udp,
                test_peer(),
                valid_options(),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(calls.lock().await.len(), 1);

        // Second request should be rate limited
        let (tx, mut rx) = mpsc::channel::<Bytes>(8);
        handler
            .handle_inbound(
                sip_transaction::TransportKind::Tcp,
                test_peer(),
                valid_options(),
                Some(tx),
                None,
            )
            .await
            .unwrap();

        // Inner should still only have 1 call
        assert_eq!(calls.lock().await.len(), 1);

        let resp = rx.recv().await.expect("should receive 429 response");
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("SIP/2.0 429 Too Many Requests"));
    }

    #[tokio::test]
    async fn test_malformed_sip_drop() {
        let (mock, calls) = MockInnerHandler::new();
        let security = SecurityConfig {
            malformed_sip_policy: MalformedSipPolicy::Drop,
            ..Default::default()
        };

        let handler = build_test_handler(
            Arc::new(mock),
            security,
            fcp_sbc_core::AdmissionConfig::default(),
            None,
            None,
        );

        // Payload with bad request line (no SIP/2.0)
        let bad_payload = Bytes::from("GARBAGE\r\nVia: x\r\n\r\n");
        handler
            .handle_inbound(
                sip_transaction::TransportKind::Udp,
                test_peer(),
                bad_payload,
                None,
                None,
            )
            .await
            .unwrap();

        // Inner should NOT be called (silently dropped)
        assert!(calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_malformed_sip_reject_400() {
        let (mock, calls) = MockInnerHandler::new();
        let security = SecurityConfig {
            malformed_sip_policy: MalformedSipPolicy::Reject,
            ..Default::default()
        };

        let handler = build_test_handler(
            Arc::new(mock),
            security,
            fcp_sbc_core::AdmissionConfig::default(),
            None,
            None,
        );

        // Valid enough to extract headers but missing mandatory headers
        let bad_payload = Bytes::from(
            "INVITE sip:bob@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
             From: <sip:alice@example.com>;tag=abc\r\n\
             \r\n",
        );

        let (tx, mut rx) = mpsc::channel::<Bytes>(8);
        handler
            .handle_inbound(
                sip_transaction::TransportKind::Tcp,
                test_peer(),
                bad_payload,
                Some(tx),
                None,
            )
            .await
            .unwrap();

        assert!(calls.lock().await.is_empty());

        // The 400 response may or may not be buildable (missing To/Call-ID/CSeq).
        // The handler sends a rejection if it can, otherwise silently drops.
        // For this payload, build_rejection_response returns None (missing headers),
        // so we just verify inner was NOT called.
        let _resp = rx.try_recv(); // may or may not have response
    }

    #[tokio::test]
    async fn test_admission_invite_503() {
        let (mock, calls) = MockInnerHandler::new();
        // Zero is no limit now, so the node is given a limit of one and
        // the one call it allows (`docs/CAPACITY_AND_SURVIVABILITY.md` §4).
        let admission = Arc::new(AdmissionController::new(fcp_sbc_core::AdmissionConfig {
            max_concurrent_calls: 1,
            ..Default::default()
        }));
        admission.call_started();

        let handler = build_test_handler_with(
            Arc::new(mock),
            SecurityConfig::default(),
            admission,
            None,
            None,
        );

        let (tx, mut rx) = mpsc::channel::<Bytes>(8);
        handler
            .handle_inbound(
                sip_transaction::TransportKind::Tcp,
                test_peer(),
                valid_invite(),
                Some(tx),
                None,
            )
            .await
            .unwrap();

        assert!(calls.lock().await.is_empty());

        let resp = rx.recv().await.expect("should receive 503 response");
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("SIP/2.0 503 Service Unavailable"));
    }

    #[tokio::test]
    async fn test_admission_register_503() {
        let (mock, calls) = MockInnerHandler::new();
        // Zero is no limit now: the node allows one registration and
        // already holds it.
        let admission = Arc::new(AdmissionController::new(fcp_sbc_core::AdmissionConfig {
            max_concurrent_registrations: 1,
            ..Default::default()
        }));
        admission.registration_added();
        let handler = build_test_handler_with(
            Arc::new(mock),
            SecurityConfig::default(),
            admission,
            None,
            None,
        );

        let (tx, mut rx) = mpsc::channel::<Bytes>(8);
        handler
            .handle_inbound(
                sip_transaction::TransportKind::Tcp,
                test_peer(),
                valid_register(),
                Some(tx),
                None,
            )
            .await
            .unwrap();

        assert!(calls.lock().await.is_empty());

        let resp = rx.recv().await.expect("should receive 503 response");
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("SIP/2.0 503 Service Unavailable"));
    }

    #[tokio::test]
    async fn test_response_bypass() {
        let (mock, calls) = MockInnerHandler::new();
        let handler = build_test_handler(
            Arc::new(mock),
            SecurityConfig::default(),
            fcp_sbc_core::AdmissionConfig::default(),
            None,
            None,
        );

        let response_payload = Bytes::from(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
             From: <sip:alice@example.com>;tag=abc\r\n\
             To: <sip:bob@example.com>;tag=xyz\r\n\
             Call-ID: test123@10.0.0.1\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\
             \r\n",
        );

        handler
            .handle_inbound(
                sip_transaction::TransportKind::Udp,
                test_peer(),
                response_payload,
                None,
                None,
            )
            .await
            .unwrap();

        // Response should bypass enforcement and reach inner handler
        let recorded = calls.lock().await;
        assert_eq!(recorded.len(), 1);
    }

    #[tokio::test]
    async fn test_hmr_applied_to_request() {
        use fcp_sbc_core::hmr::*;
        use fcp_sbc_core::HeaderManipulationEngine;

        let (mock, calls) = MockInnerHandler::new();

        // Build HMR rule that removes X-Custom header on ingress requests
        let cfg = HeaderManipulationConfig {
            enabled: true,
            rules: vec![HmrRule {
                name: "strip-x-custom".into(),
                priority: 10,
                enabled: true,
                direction: HmrDirection::Ingress,
                message_type: HmrMessageType::Request,
                methods: vec![],
                conditions: vec![],
                actions: vec![HmrAction {
                    action_type: HmrActionType::Remove,
                    header_name: "X-Custom".into(),
                    value: None,
                    pattern: None,
                }],
            }],
        };
        let engine = Arc::new(HeaderManipulationEngine::new(&cfg, &[]).unwrap());

        let handler = build_test_handler(
            Arc::new(mock),
            SecurityConfig::default(),
            fcp_sbc_core::AdmissionConfig::default(),
            None,
            Some(engine),
        );

        // Payload with an X-Custom header that should be stripped
        let payload = Bytes::from(
            "OPTIONS sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776\r\n\
             From: <sip:alice@example.com>;tag=abc\r\n\
             To: <sip:bob@example.com>\r\n\
             Call-ID: opt789@10.0.0.1\r\n\
             CSeq: 1 OPTIONS\r\n\
             X-Custom: should-be-removed\r\n\
             Content-Length: 0\r\n\
             \r\n",
        );

        handler
            .handle_inbound(
                sip_transaction::TransportKind::Udp,
                test_peer(),
                payload,
                None,
                None,
            )
            .await
            .unwrap();

        let recorded = calls.lock().await;
        assert_eq!(recorded.len(), 1);
        let forwarded = std::str::from_utf8(&recorded[0].2).unwrap();
        assert!(!forwarded.contains("X-Custom"));
        assert!(!forwarded.contains("should-be-removed"));
    }

    #[test]
    fn test_extract_header_from_payload() {
        let payload = b"INVITE sip:bob@example.com SIP/2.0\r\n\
            Identity: token;info=<url>\r\n\
            From: <sip:+15551001@example.com>;tag=abc\r\n\
            To: <sip:+15551002@example.com>\r\n\
            Call-ID: test123\r\n\
            CSeq: 1 INVITE\r\n\
            \r\n";

        let identity = fcp_sbc_core::stir_shaken::extract_header_from_payload(payload, "Identity");
        assert!(identity.is_some());
        assert!(identity.unwrap().starts_with("token"));

        let missing = fcp_sbc_core::stir_shaken::extract_header_from_payload(payload, "X-Missing");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_full_pipeline_happy_path() {
        let (mock, calls) = MockInnerHandler::new();
        let handler = build_test_handler(
            Arc::new(mock),
            SecurityConfig::default(),
            fcp_sbc_core::AdmissionConfig::default(),
            None,
            None,
        );

        handler
            .handle_inbound(
                sip_transaction::TransportKind::Udp,
                test_peer(),
                valid_invite(),
                None,
                None,
            )
            .await
            .unwrap();

        let recorded = calls.lock().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, sip_transaction::TransportKind::Udp);
        assert_eq!(recorded[0].1, test_peer());
    }
}
