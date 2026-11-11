//! A conference room reached through the call manager
//! (`docs/SOFTPHONE_VIDEO.md` §2.6, F2b).
//!
//! A phone's call to a room is two legs: the phone's to this node and this
//! node's to the conference node. What the phone and the room say to each
//! other inside the call crosses here:
//!
//! - the INVITE carries a join link's token (`X-FCP-Join`) and whether the
//!   phone takes the floor's notices (`Recv-Info: fcp-content`) on to the
//!   room, which does what it does for a browser through the gateway;
//! - a `conference` SUBSCRIBE in the phone's dialog is sent in the room's
//!   dialog, and the room's NOTIFYs back in the phone's, with their
//!   `Subscription-State`, type and body as they are (RFC 4575, RFC 6665);
//! - the room's floor INFO (`application/vnd.fcp.content+json`) is sent on
//!   to the phone.
//!
//! The node's general in-dialog relay is not used for these: it opens a new
//! subscription to the far leg's contact rather than sending in the dialog,
//! says every NOTIFY is `active`, and takes only the INFO types it is
//! configured for.

use anyhow::Result;
use sip_core::{Headers, Method, Request, RequestLine};
use sip_dialog::Dialog;
use tracing::info;

/// The largest roster NOTIFY relayed: a full roster of a large room. The
/// conference node is this node's own; the general relay's NOTIFY limit is
/// for a phone's.
pub(crate) const MAX_ROSTER_BYTES: usize = 1024 * 1024;

/// The headers a SUBSCRIBE for the roster takes from the phone to the room.
pub(crate) const SUBSCRIBE_HEADERS: &[&str] = &["Event", "Expires", "Accept"];

/// The headers a roster NOTIFY takes from the room to the phone.
pub(crate) const NOTIFY_HEADERS: &[&str] = &["Event", "Subscription-State", "Content-Type"];

/// The headers a floor INFO takes from the room to the phone.
pub(crate) const INFO_HEADERS: &[&str] = &["Info-Package", "Content-Type"];

/// What the room is told of a phone's INVITE beyond what every conference
/// leg carries: the join link's token, when it is for the room this call
/// was routed to in the caller's own tenant (the call manager placed the
/// call by that room; a token for another would be admitted elsewhere than
/// it was placed), and `Recv-Info: fcp-content` when the phone takes the
/// floor's notices.
pub(crate) fn join_headers(
    request: &Request,
    call: &fcp_b2bua_core::Call,
) -> Vec<(&'static str, String)> {
    let mut headers = Vec::new();
    if let Some(token) = request.headers().get(fcp_core::JOIN_TOKEN_HEADER) {
        let fits = fcp_core::JoinToken::peek_unverified(token).is_some_and(|t| {
            t.tenant_id == call.tenant_id
                && call.conference_room.as_deref() == Some(t.room_number.as_str())
        });
        if fits {
            headers.push((fcp_core::JOIN_TOKEN_HEADER, token.trim().to_string()));
        } else {
            info!(
                call_id = %call.id,
                "A join token for another room or tenant; joining without it"
            );
        }
    }
    let recv: Vec<&str> = request.headers().get_all("Recv-Info").collect();
    if fcp_core::recv_info_has(&recv, fcp_core::CONTENT_INFO_PACKAGE) {
        headers.push(("Recv-Info", fcp_core::CONTENT_INFO_PACKAGE.to_string()));
    }
    headers
}

/// A request for the room's conference package (RFC 4575): its `Event`
/// names `conference`, with or without parameters.
pub(crate) fn is_roster(request: &Request) -> bool {
    request.headers().get("Event").is_some_and(|e| {
        e.split(';')
            .next()
            .is_some_and(|p| p.trim().eq_ignore_ascii_case("conference"))
    })
}

/// An INFO telling the room's floor.
pub(crate) fn is_floor_info(request: &Request) -> bool {
    request.headers().get("Content-Type").is_some_and(|t| {
        t.split(';')
            .next()
            .is_some_and(|t| t.trim().eq_ignore_ascii_case(fcp_core::CONTENT_INFO_TYPE))
    })
}

/// `request`'s headers among `names`, in order, for the other leg.
pub(crate) fn carried(request: &Request, names: &[&str]) -> Vec<(String, String)> {
    names
        .iter()
        .flat_map(|name| {
            request
                .headers()
                .get_all(name)
                .map(move |v| (name.to_string(), v.to_string()))
        })
        .collect()
}

/// A request in `dialog` carrying `headers` and `body`. The CSeq, route
/// set, Request-URI and Via are the UAC's to set as it sends
/// (`send_in_dialog_non_invite`).
pub fn in_dialog_request(
    dialog: &Dialog,
    method: Method,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<Request> {
    let mut h = Headers::new();
    let mut push = |name: &str, value: String| -> Result<()> {
        h.push(name, value)
            .map_err(|e| anyhow::anyhow!("{name}: {e}"))
    };
    push(
        "Via",
        format!(
            "SIP/2.0/UDP placeholder;branch=z9hG4bK{}",
            uuid::Uuid::new_v4().simple()
        ),
    )?;
    push("Max-Forwards", "70".into())?;
    let id = dialog.id();
    push(
        "From",
        format!("<{}>;tag={}", dialog.local_uri().as_str(), id.local_tag()),
    )?;
    push(
        "To",
        format!("<{}>;tag={}", dialog.remote_uri().as_str(), id.remote_tag()),
    )?;
    push("Call-ID", id.call_id().to_string())?;
    push("CSeq", format!("1 {}", method.as_str()))?;
    for (name, value) in headers {
        push(name, value.clone())?;
    }
    push("Content-Length", body.len().to_string())?;
    Request::new(
        RequestLine::new(method.clone(), dialog.remote_target().clone()),
        h,
        bytes::Bytes::copy_from_slice(body),
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", method.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(extra: &str) -> Request {
        let raw = format!(
            "SUBSCRIBE sip:3000@cm.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.1;branch=z9hG4bK1\r\n\
             From: <sip:alice@example.com>;tag=a\r\n\
             To: <sip:3000@cm.example.com>;tag=b\r\n\
             Call-ID: c1\r\n\
             CSeq: 2 SUBSCRIBE\r\n\
             {extra}\
             Content-Length: 0\r\n\r\n"
        );
        sip_parse::parse_request(&bytes::Bytes::from(raw)).unwrap()
    }

    #[test]
    fn the_conference_package_and_the_floor_info_are_recognised() {
        assert!(is_roster(&request("Event: conference\r\n")));
        assert!(is_roster(&request("Event: Conference;id=7\r\n")));
        assert!(!is_roster(&request("Event: dialog\r\n")));
        assert!(!is_roster(&request("")));
        assert!(is_floor_info(&request(
            "Content-Type: application/vnd.fcp.content+json; charset=utf-8\r\n"
        )));
        assert!(!is_floor_info(&request(
            "Content-Type: application/dtmf-relay\r\n"
        )));
    }

    #[test]
    fn only_the_named_headers_are_carried() {
        let r = request("Event: conference\r\nExpires: 600\r\nAccept: application/conference-info+xml\r\nX-Other: 1\r\n");
        assert_eq!(
            carried(&r, SUBSCRIBE_HEADERS),
            vec![
                ("Event".to_string(), "conference".to_string()),
                ("Expires".to_string(), "600".to_string()),
                (
                    "Accept".to_string(),
                    "application/conference-info+xml".to_string()
                ),
            ]
        );
    }
}
