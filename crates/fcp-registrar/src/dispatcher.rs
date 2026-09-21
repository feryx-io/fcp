//! REGISTER-only request dispatcher.
//!
//! Accepts only REGISTER requests and returns 405 for everything else.

use bytes::Bytes;
use fcp_sip::registrar::FcpRegistrar;
use sip_core::{Headers, Method, Request, Response, StatusLine};
use sip_parse::header;
use sip_transaction::ServerTransactionHandle;
use smol_str::SmolStr;
use std::sync::Arc;
use tracing::{info, warn};

/// Dispatch an incoming SIP request.
///
/// REGISTER requests are handled by FcpRegistrar.
/// All other methods receive 405 Method Not Allowed.
pub async fn dispatch_request(
    request: &Request,
    handle: ServerTransactionHandle,
    registrar: &Arc<FcpRegistrar>,
    admission: Option<&Arc<dyn fcp_sip::admission::NodeAdmission>>,
    source: std::net::SocketAddr,
) {
    let call_id = header(request.headers(), "Call-ID")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    match request.method() {
        Method::Register => {
            // What this node accepts, before any credential work, and what
            // a draining node grants — the gate every registrar on this
            // platform runs (`docs/CAPACITY_AND_SURVIVABILITY.md` §5, §6).
            let aor = header(request.headers(), "To")
                .map(|to| {
                    let to = to.as_str();
                    to.split('<')
                        .nth(1)
                        .and_then(|s| s.split('>').next())
                        .unwrap_or(to)
                        .trim()
                        .to_string()
                })
                .unwrap_or_default();
            let shortened;
            let request =
                match fcp_sip::admission::register_gate(admission, request, source.ip(), &aor) {
                    fcp_sip::admission::RegisterGate::Pass => request,
                    fcp_sip::admission::RegisterGate::Shortened(capped) => {
                        shortened = *capped;
                        &shortened
                    }
                    fcp_sip::admission::RegisterGate::Refuse {
                        refusal,
                        retry_after_secs,
                    } => {
                        warn!(
                            call_id = %call_id,
                            aor = %aor,
                            source = %source,
                            reason = %refusal.as_str(),
                            "Refusing REGISTER: {}",
                            refusal.detail()
                        );
                        handle
                            .send_final(build_refusal(request, retry_after_secs))
                            .await;
                        return;
                    }
                };
            // The binding remembers where the device is, as the call
            // manager's does (`docs/E911_BLOCK_2.md` §6.2).
            let response = registrar
                .handle_register_from(request, None, Some(source))
                .await;

            match response.start_line().code() {
                200 => info!(call_id = %call_id, "Registration successful"),
                401 => info!(call_id = %call_id, "Authentication challenge sent"),
                code => warn!(call_id = %call_id, code, "Registration failed"),
            }

            handle.send_final(response).await;
        }
        Method::Options => {
            let response = build_options_response(request);
            handle.send_final(response).await;
        }
        other => {
            warn!(
                method = ?other,
                call_id = %call_id,
                "Rejecting non-REGISTER request with 405"
            );
            let response = build_405_response(request);
            handle.send_final(response).await;
        }
    }
}

/// Build a 405 Method Not Allowed response with Allow: REGISTER header.
fn build_405_response(request: &Request) -> Response {
    let mut headers = Headers::new();
    copy_mandatory_headers(request, &mut headers);
    let _ = headers.push(SmolStr::new("Allow"), SmolStr::new("REGISTER, OPTIONS"));
    let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

    Response::new(
        StatusLine::new(405, "Method Not Allowed").expect("valid status line"),
        headers,
        Bytes::new(),
    )
    .expect("valid response")
}

/// Build the `503` a refused REGISTER gets: the same answer the call
/// manager gives, with `Retry-After` so a device that means no harm comes
/// back rather than giving up (`docs/CAPACITY_AND_SURVIVABILITY.md` §5).
fn build_refusal(request: &Request, retry_after_secs: u64) -> Response {
    let mut headers = Headers::new();
    copy_mandatory_headers(request, &mut headers);
    let _ = headers.push(
        SmolStr::new("Retry-After"),
        SmolStr::new(retry_after_secs.to_string()),
    );
    let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

    Response::new(
        StatusLine::new(503, "Service Unavailable").expect("valid status line"),
        headers,
        Bytes::new(),
    )
    .expect("valid response")
}

/// Build a 200 OK response for OPTIONS requests.
fn build_options_response(request: &Request) -> Response {
    let mut headers = Headers::new();
    copy_mandatory_headers(request, &mut headers);
    let _ = headers.push(SmolStr::new("Allow"), SmolStr::new("REGISTER, OPTIONS"));
    let _ = headers.push(SmolStr::new("Accept"), SmolStr::new("application/sdp"));
    let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

    Response::new(
        StatusLine::new(200, "OK").expect("valid status line"),
        headers,
        Bytes::new(),
    )
    .expect("valid response")
}

/// Copy mandatory SIP headers from request to response (RFC 3261).
fn copy_mandatory_headers(request: &Request, headers: &mut Headers) {
    if let Some(via) = request.headers().get_smol("Via") {
        let _ = headers.push(SmolStr::new("Via"), via.clone());
    }
    if let Some(from) = request.headers().get_smol("From") {
        let _ = headers.push(SmolStr::new("From"), from.clone());
    }
    if let Some(to) = request.headers().get_smol("To") {
        let _ = headers.push(SmolStr::new("To"), to.clone());
    }
    if let Some(call_id) = request.headers().get_smol("Call-ID") {
        let _ = headers.push(SmolStr::new("Call-ID"), call_id.clone());
    }
    if let Some(cseq) = request.headers().get_smol("CSeq") {
        let _ = headers.push(SmolStr::new("CSeq"), cseq.clone());
    }
}
