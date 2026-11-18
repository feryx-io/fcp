//! The SIP the gateway receives: responses to its own INVITEs and BYEs,
//! which the transaction layer matches to the calls waiting on them, and
//! in-dialog requests from the far end — a BYE ends the session; an INFO
//! of `application/vnd.fcp.content+json` is the conference server's word
//! on the shared screen's floor (§15.6, 7c); OPTIONS is answered;
//! anything else is refused, since the gateway places calls and never
//! takes them.

use crate::session::{ContentNotice, SessionManager};
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

/// The media type of the conference server's floor notices.
pub const CONTENT_NOTICE_TYPE: &str = "application/vnd.fcp.content+json";

/// The floor notice an INFO carries, if it is one: the type must match
/// (parameters aside) and the body parse.
pub fn parse_content_notice(request: &Request) -> Option<ContentNotice> {
    let content_type = request.headers().get("Content-Type")?;
    let media_type = content_type.split(';').next()?.trim();
    if !media_type.eq_ignore_ascii_case(CONTENT_NOTICE_TYPE) {
        return None;
    }
    serde_json::from_slice(request.body()).ok()
}

/// An INFO of the audience's voice in a room (`docs/WEBINARS.md` §2.5).
pub fn is_qa_info(request: &Request) -> bool {
    request
        .headers()
        .get("Content-Type")
        .and_then(|t| t.split(';').next())
        .is_some_and(|t| t.trim().eq_ignore_ascii_case(fcp_core::QA_INFO_TYPE))
}

/// An INFO of chat in a room (`docs/WEBINARS.md` §2.5).
pub fn is_chat_info(request: &Request) -> bool {
    request
        .headers()
        .get("Content-Type")
        .and_then(|t| t.split(';').next())
        .is_some_and(|t| t.trim().eq_ignore_ascii_case(fcp_core::CHAT_INFO_TYPE))
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
        let _ = headers.push("Allow", "BYE, INFO, NOTIFY, OPTIONS");
    }
    if code == 415 {
        let _ = headers.push("Accept", CONTENT_NOTICE_TYPE);
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
                Method::Info if is_chat_info(&request) => {
                    match fcp_core::chat::ChatMessage::parse(request.body()) {
                        Ok(message) if self.sessions.chat_notice(&call_id, &message).await => {
                            handle
                                .send_final(simple_response(&request, 200, "OK"))
                                .await;
                        }
                        Ok(_) => {
                            handle
                                .send_final(simple_response(
                                    &request,
                                    481,
                                    "Call/Transaction Does Not Exist",
                                ))
                                .await;
                        }
                        Err(e) => {
                            debug!(%peer, call_id = %call_id, error = %e, "fcp-chat INFO refused");
                            handle
                                .send_final(simple_response(&request, 400, "Bad Request"))
                                .await;
                        }
                    }
                }
                Method::Info if is_qa_info(&request) => {
                    match fcp_core::qa::QaMessage::parse(request.body()) {
                        Ok(message) if self.sessions.qa_notice(&call_id, &message).await => {
                            handle
                                .send_final(simple_response(&request, 200, "OK"))
                                .await;
                        }
                        Ok(_) => {
                            handle
                                .send_final(simple_response(
                                    &request,
                                    481,
                                    "Call/Transaction Does Not Exist",
                                ))
                                .await;
                        }
                        Err(e) => {
                            debug!(%peer, call_id = %call_id, error = %e, "fcp-qa INFO refused");
                            handle
                                .send_final(simple_response(&request, 400, "Bad Request"))
                                .await;
                        }
                    }
                }
                Method::Info => match parse_content_notice(&request) {
                    Some(notice) => {
                        if self.sessions.content_notice(&call_id, &notice).await {
                            handle
                                .send_final(simple_response(&request, 200, "OK"))
                                .await;
                        } else {
                            debug!(%peer, call_id = %call_id, "floor notice for an unknown call");
                            handle
                                .send_final(simple_response(
                                    &request,
                                    481,
                                    "Call/Transaction Does Not Exist",
                                ))
                                .await;
                        }
                    }
                    None => {
                        debug!(%peer, call_id = %call_id, "INFO of a type the gateway does not take");
                        handle
                            .send_final(simple_response(&request, 415, "Unsupported Media Type"))
                            .await;
                    }
                },
                // The room's roster, which a forwarding session subscribed
                // to in the call: whose stream is on each of its tracks.
                Method::Notify
                    if request
                        .headers()
                        .get("Event")
                        .is_some_and(|e| e.trim().starts_with("conference")) =>
                {
                    let code = if self.sessions.roster_notice(&call_id, request.body()).await {
                        (200, "OK")
                    } else {
                        (481, "Call/Transaction Does Not Exist")
                    };
                    handle
                        .send_final(simple_response(&request, code.0, code.1))
                        .await;
                }
                Method::Refer => {
                    // The conference node is draining and names another
                    // node for this session (phase 5c). Any other REFER is
                    // a transfer, which the gateway does not do.
                    let refer_to = request
                        .headers()
                        .get("Refer-To")
                        .map(str::to_string)
                        .unwrap_or_default();
                    match fcp_sip::conference_migration::parse_migration_refer(&refer_to) {
                        Some(refer) => {
                            use crate::session::MigrationStart;
                            let (code, reason) =
                                match self.sessions.begin_migration(&call_id, refer).await {
                                    MigrationStart::Accepted => (202, "Accepted"),
                                    MigrationStart::Unknown => {
                                        (481, "Call/Transaction Does Not Exist")
                                    }
                                    MigrationStart::Busy => (491, "Request Pending"),
                                    MigrationStart::NotReady => (480, "Temporarily Unavailable"),
                                };
                            debug!(%peer, call_id = %call_id, code, "REFER to move the session");
                            handle
                                .send_final(simple_response(&request, code, reason))
                                .await;
                        }
                        None => {
                            debug!(%peer, call_id = %call_id, "REFER that is not a move; the gateway does not transfer");
                            handle
                                .send_final(simple_response(&request, 603, "Decline"))
                                .await;
                        }
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
        assert_eq!(
            refused.headers().get("Allow"),
            Some("BYE, INFO, NOTIFY, OPTIONS")
        );
        let unsupported = simple_response(&tagged, 415, "Unsupported Media Type");
        assert_eq!(
            unsupported.headers().get("Accept"),
            Some(CONTENT_NOTICE_TYPE)
        );
    }

    fn info(content_type: &str, body: &str) -> Request {
        let raw = format!(
            "INFO sip:webrtc-gw@10.0.0.2 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.7:5060;branch=z9hG4bK777\r\n\
             From: <sip:8001@10.0.0.7>;tag=abc\r\n\
             To: <sip:webrtc-gw@10.0.0.2>;tag=xyz\r\n\
             Call-ID: call-1@10.0.0.7\r\n\
             CSeq: 3 INFO\r\n\
             Content-Type: {content_type}\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        sip_parse::parse_request(&Bytes::from(raw)).expect("parses")
    }

    #[test]
    fn a_floor_notice_is_read_off_an_info_of_its_type() {
        let req = info(
            "application/vnd.fcp.content+json; charset=utf-8",
            r#"{"floor":"granted","mine":false,"presenter":"Ann","participant_id":"p1"}"#,
        );
        let notice = parse_content_notice(&req).expect("a notice");
        assert_eq!(notice.floor, "granted");
        assert!(!notice.mine);
        assert_eq!(notice.presenter.as_deref(), Some("Ann"));
        // DTMF by INFO, or anything else, is not one.
        assert!(parse_content_notice(&info("application/dtmf-relay", "Signal=1")).is_none());
        assert!(parse_content_notice(&info("application/vnd.fcp.content+json", "nope")).is_none());
    }
}
