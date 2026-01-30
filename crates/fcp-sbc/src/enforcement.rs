//! SBC enforcement handler — decorator around `SipMessageHandler`.
//!
//! Implements `InboundMessageHandler` and enforces security policies
//! (IP ACL, message size, rate limiting, admission control) before
//! delegating to the inner B2BUA handler.
//!
//! SIP responses (starting with "SIP/") bypass enforcement — they are
//! replies to our own outbound requests and must reach the transaction
//! layer to avoid timeouts.

use crate::admission::AdmissionController;
use crate::metrics::SbcRuntimeMetrics;
use bytes::Bytes;
use fcp_sbc_core::security::{AclAction, SecurityConfig};
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::listener::InboundMessageHandler;
use fcp_sip::transport::RealTransportDispatcher;
use sip_ratelimit::RateLimiterSet;
use sip_transaction::{TransportContext, TransportDispatcher};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Enforcement decorator wrapping the inner SIP message handler.
///
/// Enforces (in order):
/// 1. Response bypass (SIP responses skip enforcement)
/// 2. IP ACL check
/// 3. Message size limit
/// 4. Rate limiting (per-IP via `sip-ratelimit`)
/// 5. Admission control (CPS + concurrent limits)
/// 6. Delegate to inner `SipMessageHandler`
pub struct SbcEnforcementHandler {
    inner: Arc<SipMessageHandler>,
    security: SecurityConfig,
    rate_limiter: Arc<RateLimiterSet>,
    admission: Arc<AdmissionController>,
    metrics: Arc<SbcRuntimeMetrics>,
    dispatcher: Arc<RealTransportDispatcher>,
}

impl SbcEnforcementHandler {
    pub fn new(
        inner: Arc<SipMessageHandler>,
        security: SecurityConfig,
        rate_limiter: Arc<RateLimiterSet>,
        admission: Arc<AdmissionController>,
        metrics: Arc<SbcRuntimeMetrics>,
        dispatcher: Arc<RealTransportDispatcher>,
    ) -> Self {
        Self {
            inner,
            security,
            rate_limiter,
            admission,
            metrics,
            dispatcher,
        }
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
    ) -> anyhow::Result<()> {
        // 0. SIP responses bypass enforcement — they are replies to our own
        //    outbound requests and must reach the transaction layer.
        if payload.starts_with(b"SIP/") {
            return self
                .inner
                .handle_inbound(transport, peer, payload, stream)
                .await;
        }

        let peer_ip = peer.ip();

        // 1. IP ACL check
        if self.security.signaling_acl.evaluate(peer_ip) == AclAction::Deny {
            warn!(%peer, "SBC: ACL denied");
            self.metrics.record_security_rejection();
            self.send_rejection(transport, peer, &payload, &stream, 403, "Forbidden")
                .await;
            return Ok(());
        }

        // 2. Message size check
        if payload.len() > self.security.max_message_size {
            warn!(%peer, len = payload.len(), max = self.security.max_message_size, "SBC: message too large");
            self.metrics.record_security_rejection();
            self.send_rejection(
                transport,
                peer,
                &payload,
                &stream,
                513,
                "Message Too Large",
            )
            .await;
            return Ok(());
        }

        // 3. Rate limit check — connections limiter as a general per-IP gate
        let ip_str = peer_ip.to_string();
        if !self.rate_limiter.connections().check_rate_limit(&ip_str) {
            debug!(%peer, "SBC: rate limited (connections)");
            self.metrics.record_rate_limit_rejection();
            self.send_rejection(
                transport,
                peer,
                &payload,
                &stream,
                429,
                "Too Many Requests",
            )
            .await;
            return Ok(());
        }

        // Method-specific rate limiting: quick-parse the first line to extract method
        if let Some(method) = extract_method_from_payload(&payload) {
            let allowed = match method {
                "INVITE" => self.rate_limiter.invite().check_rate_limit(&ip_str),
                "REGISTER" => self.rate_limiter.register().check_rate_limit(&ip_str),
                _ => true, // Other methods pass through (covered by connections limiter)
            };
            if !allowed {
                debug!(%peer, method, "SBC: rate limited (method)");
                self.metrics.record_rate_limit_rejection();
                self.send_rejection(
                    transport,
                    peer,
                    &payload,
                    &stream,
                    429,
                    "Too Many Requests",
                )
                .await;
                return Ok(());
            }

            // 4a. Admission control for INVITE (new call attempts)
            if method == "INVITE" {
                self.metrics.record_call_attempt();
                if !self.admission.try_admit_call() {
                    warn!(%peer, "SBC: admission control rejected (overloaded)");
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

            // 4b. Admission control for REGISTER
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

        // 5. Delegate to inner handler
        self.inner
            .handle_inbound(transport, peer, payload, stream)
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
        } else if call_id.is_none()
            && (lower.starts_with("call-id:") || lower.starts_with("i:"))
        {
            call_id = Some(trimmed);
        } else if cseq.is_none() && lower.starts_with("cseq:") {
            cseq = Some(trimmed);
        }
    }

    // All five headers are mandatory for a valid response
    let response = format!(
        "SIP/2.0 {} {}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\nContent-Length: 0\r\n\r\n",
        code,
        reason,
        via?,
        from?,
        to?,
        call_id?,
        cseq?
    );

    Some(Bytes::from(response))
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
}
