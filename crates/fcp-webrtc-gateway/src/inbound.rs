//! The SIP the gateway receives: responses to its own INVITEs and BYEs,
//! which the transaction layer matches to the calls waiting on them, and
//! in-dialog requests from the far end — a BYE ends the session; OPTIONS
//! is answered; anything else is refused, since the gateway places calls
//! and never takes them.

use crate::session::SessionManager;
use bytes::Bytes;
use fcp_sip::listener::InboundMessageHandler;
use sip_core::{Headers, Method, Request, Response, StatusLine};
use sip_transaction::{TransactionManager, TransportContext, TransportKind};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

/// Handles inbound SIP for the gateway.
pub struct GatewayInboundHandler {
    transaction_manager: Arc<TransactionManager>,
    sessions: Arc<SessionManager>,
}

impl GatewayInboundHandler {
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        sessions: Arc<SessionManager>,
    ) -> Self {
        Self {
            transaction_manager,
            sessions,
        }
    }
}

/// A bodyless response to `request` with the headers RFC 3261 §8.2.6
/// requires copied over, and a To tag when the request had none.
pub fn simple_response(request: &Request, code: u16, reason: &str) -> Response {
    let mut headers = Headers::new();
    for name in ["Via", "From", "Call-ID", "CSeq"] {
        if let Some(value) = request.headers().get(name) {
            let _ = headers.push(name, value);
        }
    }
    if let Some(to) = request.headers().get("To") {
        let value = if to.contains(";tag=") {
            to.to_string()
        } else {
            format!("{to};tag={}", uuid::Uuid::new_v4().simple())
        };
        let _ = headers.push("To", value);
    }
    let _ = headers.push("Content-Length", "0");
    if code == 405 {
        let _ = headers.push("Allow", "BYE, OPTIONS");
    }
    Response::new(
        StatusLine::new(code, reason).expect("valid status line"),
        headers,
        Bytes::new(),
    )
    .expect("valid response")
}

#[async_trait::async_trait]
impl InboundMessageHandler for GatewayInboundHandler {
    async fn handle_inbound(
        &self,
        transport: TransportKind,
        peer: SocketAddr,
        payload: Bytes,
        stream: Option<mpsc::Sender<Bytes>>,
        _peer_identity: Option<Arc<sip_transport::PeerIdentity>>,
    ) -> anyhow::Result<()> {
        if let Some(request) = sip_parse::parse_request(&payload) {
            let method = request.method().clone();
            if method == Method::Ack {
                // ACKs to our responses need no transaction work here.
                return Ok(());
            }
            let ctx = TransportContext::new(transport, peer, stream);
            let handle = self
                .transaction_manager
                .receive_request(request.clone(), ctx)
                .await;
            let call_id = request
                .headers()
                .get("Call-ID")
                .map(str::to_string)
                .unwrap_or_default();
            match method {
                Method::Bye => {
                    if self.sessions.remote_bye(&call_id).await {
                        debug!(%peer, call_id = %call_id, "BYE from the far end; session ended");
                        handle
                            .send_final(simple_response(&request, 200, "OK"))
                            .await;
                    } else {
                        debug!(%peer, call_id = %call_id, "BYE for an unknown call");
                        handle
                            .send_final(simple_response(
                                &request,
                                481,
                                "Call/Transaction Does Not Exist",
                            ))
                            .await;
                    }
                }
                Method::Options => {
                    handle
                        .send_final(simple_response(&request, 200, "OK"))
                        .await;
                }
                Method::Cancel => {
                    handle
                        .send_final(simple_response(
                            &request,
                            481,
                            "Call/Transaction Does Not Exist",
                        ))
                        .await;
                }
                other => {
                    warn!(%peer, method = %other.as_str(), "refusing inbound request; the gateway only places calls");
                    handle
                        .send_final(simple_response(&request, 405, "Method Not Allowed"))
                        .await;
                }
            }
            Ok(())
        } else if let Some(response) = sip_parse::parse_response(&payload) {
            trace!(%peer, code = response.start_line().code(), "SIP response");
            self.transaction_manager.receive_response(response).await;
            Ok(())
        } else {
            trace!(%peer, len = payload.len(), "ignoring unparseable SIP datagram");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, to: &str) -> Request {
        let raw = format!(
            "{method} sip:webrtc-gw@10.0.0.2 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.7:5060;branch=z9hG4bK776\r\n\
             From: <sip:8001@10.0.0.7>;tag=abc\r\n\
             To: {to}\r\n\
             Call-ID: call-1@10.0.0.7\r\n\
             CSeq: 2 {method}\r\n\
             Content-Length: 0\r\n\r\n"
        );
        sip_parse::parse_request(&Bytes::from(raw)).expect("parses")
    }

    #[test]
    fn simple_response_copies_the_required_headers_and_tags_to() {
        let req = request("BYE", "<sip:webrtc-gw@10.0.0.2>");
        let resp = simple_response(&req, 200, "OK");
        assert_eq!(resp.start_line().code(), 200);
        assert_eq!(resp.headers().get("Call-ID"), Some("call-1@10.0.0.7"));
        assert_eq!(resp.headers().get("CSeq"), Some("2 BYE"));
        assert!(resp.headers().get("To").unwrap().contains(";tag="));
        assert_eq!(resp.headers().get("Content-Length"), Some("0"));

        let tagged = request("BYE", "<sip:webrtc-gw@10.0.0.2>;tag=xyz");
        let resp = simple_response(&tagged, 481, "Call/Transaction Does Not Exist");
        assert_eq!(
            resp.headers().get("To"),
            Some("<sip:webrtc-gw@10.0.0.2>;tag=xyz")
        );
        let refused = simple_response(&tagged, 405, "Method Not Allowed");
        assert_eq!(refused.headers().get("Allow"), Some("BYE, OPTIONS"));
    }
}
