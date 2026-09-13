//! The call manager's half of participant migration off a draining
//! conference node (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5b).
//!
//! A conference node that drains sends the B2BUA a REFER on the B-leg of
//! each conference call, its `Refer-To` naming the room on another node
//! with a migration ticket as a URI header. The B2BUA does not relay that
//! REFER to the caller's phone (which would dial the node directly, past
//! the call manager); it executes the move itself: a fresh B-leg to the
//! new node with the ticket as a request header, the caller re-INVITEd
//! with the new node's answer (or the anchored media session re-pointed),
//! the legs swapped, the outcome reported to the old node with a sipfrag
//! NOTIFY, the old leg dropped. The SIP is in `handler.rs`; here are the
//! parts with none: reading the `Refer-To`, deciding whether the caller's
//! answer moved its media, and the sipfrag body.

use crate::uac::SipUserAgentClient;
use fcp_core::MIGRATION_TICKET_HEADER;
use sip_core::SipUri;
use sip_dialog::Dialog;
use std::time::Duration;
use tracing::debug;

/// What a migration `Refer-To` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationRefer {
    /// The room on the new node, with the URI headers stripped: what the
    /// INVITE is addressed to.
    pub target: SipUri,
    /// The ticket, to go in the INVITE's `X-FCP-Migrate` header.
    pub ticket: String,
}

/// Read a `Refer-To` header value. `None` when it is not a migration
/// (no `X-FCP-Migrate` URI header): an ordinary transfer, handled as
/// before. The ticket is the only URI header copied anywhere; the rest
/// of that channel is dropped as RFC 3261 §19.1.5 asks.
pub fn parse_migration_refer(refer_to: &str) -> Option<MigrationRefer> {
    let uri = refer_to
        .split('<')
        .nth(1)
        .and_then(|s| s.split('>').next())
        .unwrap_or(refer_to)
        .trim();
    let uri = SipUri::parse(uri).ok()?;
    let ticket = uri
        .headers()
        .get(MIGRATION_TICKET_HEADER.to_ascii_lowercase().as_str())?
        .to_string();
    if ticket.is_empty() {
        return None;
    }
    Some(MigrationRefer {
        target: uri.without_uri_headers(),
        ticket,
    })
}

/// Where an SDP sends its media: the session-level and media-level
/// connection addresses and each `m=` line's port, in order.
fn media_endpoints(sdp: &str) -> Vec<String> {
    sdp.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("c=") || l.starts_with("m="))
        .map(|l| {
            if let Some(m) = l.strip_prefix("m=") {
                // `m=audio 4000 RTP/AVP 0 8`: the kind and the port.
                let mut parts = m.split_whitespace();
                let kind = parts.next().unwrap_or("");
                let port = parts.next().unwrap_or("");
                format!("m={kind} {port}")
            } else {
                l.to_string()
            }
        })
        .collect()
}

/// Whether the caller's answer to the re-INVITE sends its media somewhere
/// other than the offer the new node was given: a different address or
/// port on any section. When it does, the new node has to be told with a
/// re-INVITE of its own; when it does not (the usual case: a phone keeps
/// its ports), the new node already has the right endpoint.
pub fn media_endpoint_changed(offered: &str, answered: &str) -> bool {
    media_endpoints(offered) != media_endpoints(answered)
}

/// The `message/sipfrag` body reporting a REFER's outcome (RFC 3515).
pub fn sipfrag(code: u16, reason: &str) -> String {
    format!("SIP/2.0 {code} {reason}\r\n")
}

/// How long the report may take to be answered.
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// Report a REFER's outcome on the dialog it arrived in: the implicit
/// `refer` subscription's one NOTIFY (RFC 3515 §2.4.5), a sipfrag body
/// with `code` and `Subscription-State: terminated`. `local_uri` is what
/// the notifier signs as. Returns the far end's status.
pub async fn report_refer_outcome(
    uac: &SipUserAgentClient,
    local_uri: &SipUri,
    dialog: &mut Dialog,
    code: u16,
    reason: &str,
) -> anyhow::Result<u16> {
    let id = dialog.id().clone();
    let sub_id =
        sip_dialog::SubscriptionId::new(id.call_id(), id.remote_tag(), id.local_tag(), "refer")
            .map_err(|e| anyhow::anyhow!("refer subscription id: {e}"))?;
    let mut sub = sip_dialog::Subscription::unchecked_new(
        sub_id,
        sip_dialog::SubscriptionState::Terminated,
        dialog.local_uri().clone(),
        dialog.remote_uri().clone(),
        dialog.remote_target().clone(),
        Duration::from_secs(0),
        dialog.local_cseq(),
        0,
    );
    let uas = sip_uas::UserAgentServer::new(local_uri.clone(), local_uri.clone());
    let request = uas
        .create_notify_sipfrag(&mut sub, code, reason)
        .map_err(|e| anyhow::anyhow!("refer NOTIFY: {e}"))?;
    let response = tokio::time::timeout(
        NOTIFY_TIMEOUT,
        uac.inner().send_in_dialog_non_invite(dialog, request),
    )
    .await
    .map_err(|_| anyhow::anyhow!("refer NOTIFY unanswered"))??;
    let status = response.start_line().code();
    debug!(code, status, "refer NOTIFY answered");
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_migration_refer_to_yields_the_target_and_the_ticket() {
        let r = parse_migration_refer("<sip:8000@10.0.0.21:5074?X-FCP-Migrate=abc.def>").unwrap();
        assert_eq!(r.ticket, "abc.def");
        assert_eq!(r.target.as_str(), "sip:8000@10.0.0.21:5074");
        assert!(r.target.headers().is_empty());
        // Without angle brackets, and with a display name.
        assert!(parse_migration_refer("sip:8000@10.0.0.21?X-FCP-Migrate=t").is_some());
        assert!(parse_migration_refer("\"Room\" <sip:8000@10.0.0.21?X-FCP-Migrate=t>").is_some());
    }

    #[test]
    fn an_ordinary_refer_to_is_not_a_migration() {
        assert_eq!(parse_migration_refer("<sip:2001@example.com>"), None);
        assert_eq!(
            parse_migration_refer(
                "<sip:2001@example.com?Replaces=abc%3Bto-tag%3D1%3Bfrom-tag%3D2>"
            ),
            None
        );
        assert_eq!(parse_migration_refer("<sip:8000@h?X-FCP-Migrate=>"), None);
        assert_eq!(parse_migration_refer("garbage"), None);
    }

    const OFFER: &str = "v=0\r\no=- 1 1 IN IP4 10.1.1.5\r\ns=-\r\nc=IN IP4 10.1.1.5\r\nt=0 0\r\nm=audio 4000 RTP/AVP 0 8\r\na=rtpmap:0 PCMU/8000\r\nm=video 4002 RTP/AVP 96\r\n";

    #[test]
    fn the_same_ports_are_not_a_change_but_a_new_port_or_address_is() {
        let same_with_other_attributes =
            OFFER.replace("a=rtpmap:0 PCMU/8000\r\n", "a=sendrecv\r\n");
        assert!(!media_endpoint_changed(OFFER, &same_with_other_attributes));
        assert!(media_endpoint_changed(
            OFFER,
            &OFFER.replace("m=audio 4000", "m=audio 4010")
        ));
        assert!(media_endpoint_changed(
            OFFER,
            &OFFER.replace("c=IN IP4 10.1.1.5", "c=IN IP4 10.1.1.9")
        ));
        // A section dropped is a change too.
        assert!(media_endpoint_changed(
            OFFER,
            OFFER.split("m=video").next().unwrap()
        ));
    }

    #[test]
    fn sipfrag_is_one_status_line() {
        assert_eq!(sipfrag(200, "OK"), "SIP/2.0 200 OK\r\n");
    }
}
