//! SBC enforcement handler — decorator around `SipMessageHandler`.
//!
//! Implements `InboundMessageHandler` and enforces security policies
//! (IP ACL, message size, rate limiting, admission control) before
//! delegating to the inner B2BUA handler.

use crate::admission::AdmissionController;
use crate::metrics::SbcRuntimeMetrics;
use bytes::Bytes;
use fcp_sbc_core::security::{AclAction, SecurityConfig};
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::listener::InboundMessageHandler;
use sip_ratelimit::RateLimiterSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Enforcement decorator wrapping the inner SIP message handler.
///
/// Enforces (in order):
/// 1. IP ACL check
/// 2. Message size limit
/// 3. Rate limiting (per-IP via `sip-ratelimit`)
/// 4. Admission control (CPS + concurrent limits)
/// 5. Delegate to inner `SipMessageHandler`
pub struct SbcEnforcementHandler {
    inner: Arc<SipMessageHandler>,
    security: SecurityConfig,
    rate_limiter: Arc<RateLimiterSet>,
    admission: Arc<AdmissionController>,
    metrics: Arc<SbcRuntimeMetrics>,
}

impl SbcEnforcementHandler {
    pub fn new(
        inner: Arc<SipMessageHandler>,
        security: SecurityConfig,
        rate_limiter: Arc<RateLimiterSet>,
        admission: Arc<AdmissionController>,
        metrics: Arc<SbcRuntimeMetrics>,
    ) -> Self {
        Self {
            inner,
            security,
            rate_limiter,
            admission,
            metrics,
        }
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
        let peer_ip = peer.ip();

        // 1. IP ACL check
        if self.security.signaling_acl.evaluate(peer_ip) == AclAction::Deny {
            warn!(%peer, "SBC: ACL denied");
            self.metrics.record_security_rejection();
            return Ok(());
        }

        // 2. Message size check
        if payload.len() > self.security.max_message_size {
            warn!(%peer, len = payload.len(), max = self.security.max_message_size, "SBC: message too large");
            self.metrics.record_security_rejection();
            return Ok(());
        }

        // 3. Rate limit check (per-IP, using INVITE bucket as general limiter)
        //    Quick check: parse request line to determine method for method-specific limiting.
        //    For simplicity in v1, use the invite limiter for INVITE, register for REGISTER,
        //    and the connections limiter as a general per-IP gate.
        let ip_str = peer_ip.to_string();
        if !self.rate_limiter.connections().check_rate_limit(&ip_str) {
            debug!(%peer, "SBC: rate limited (connections)");
            self.metrics.record_rate_limit_rejection();
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
                return Ok(());
            }

            // 4. Admission control (only for INVITE — new call attempts)
            if method == "INVITE" {
                self.metrics.record_call_attempt();
                if !self.admission.try_admit_call() {
                    warn!(%peer, "SBC: admission control rejected (overloaded)");
                    self.metrics.record_admission_rejection();
                    return Ok(());
                }
            }
        }

        // 5. Delegate to inner handler
        self.inner
            .handle_inbound(transport, peer, payload, stream)
            .await
    }
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
}
