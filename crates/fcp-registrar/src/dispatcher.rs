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
) {
    let call_id = header(request.headers(), "Call-ID")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    match request.method() {
        Method::Register => {
            let response = registrar.handle_register(request).await;

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
