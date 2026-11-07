//! The address a service on this host gives another one on it (#335).
//!
//! On a cloud VM whose public address is a 1:1 NAT at the provider's edge
//! (GCE, an AWS elastic IP), no interface holds that address and nothing
//! the host sends to it comes back (`crate::self_reach`). Every service
//! advertises it in Contact and SDP, so whatever one service on the host
//! sends another by it inside a call — a BYE to voicemail, the media
//! between them — is lost. Kamailio answers this with an advertised
//! address per socket; here it is per peer: a message to a peer on this
//! host has the unreachable address in its Contact, Record-Route and SDP
//! swapped for the one the peer is at, which this host does reach. A peer
//! elsewhere is sent the message as it was.

use bytes::Bytes;
use dashmap::DashMap;
use std::net::IpAddr;

/// What [`crate::transport::RealTransportDispatcher`] swaps in a message to
/// a peer on this host.
#[derive(Debug)]
pub struct SiblingRewrite {
    advertised: IpAddr,
    advertised_text: String,
    /// Whether each peer address seen is one of this host's own.
    local: DashMap<IpAddr, bool>,
}

impl SiblingRewrite {
    /// For a host that does not reach `advertised`.
    pub fn new(advertised: IpAddr) -> Self {
        Self {
            advertised,
            advertised_text: advertised.to_string(),
            local: DashMap::new(),
        }
    }

    pub fn advertised(&self) -> IpAddr {
        self.advertised
    }

    /// `payload` as it should go to `peer`, or `None` when it goes as it
    /// is: the peer is not on this host, or the message does not name the
    /// advertised address. `bound` is the address this service's socket is
    /// bound to; when it is a specific one, that is the one advertised.
    pub fn apply(&self, peer: IpAddr, bound: Option<IpAddr>, payload: &[u8]) -> Option<Bytes> {
        if peer == self.advertised || !self.is_this_host(peer) {
            return None;
        }
        let to = match bound {
            Some(ip) if !ip.is_unspecified() => ip,
            _ => peer,
        };
        if to.is_ipv4() != self.advertised.is_ipv4() {
            return None;
        }
        rewrite(payload, &self.advertised_text, &to.to_string())
    }

    /// A loopback address, or one an interface of this host holds: an
    /// address a socket can be bound to.
    fn is_this_host(&self, ip: IpAddr) -> bool {
        if ip.is_loopback() {
            return true;
        }
        *self
            .local
            .entry(ip)
            .or_insert_with(|| std::net::UdpSocket::bind((ip, 0)).is_ok())
    }
}

/// `message` with `from` replaced by `to` where it names where to reach
/// the sender: the Contact and Record-Route headers, and the SDP's `c=`,
/// `o=` and `a=rtcp:` lines. Content-Length follows the body. `None` when
/// nothing was replaced.
pub fn rewrite(message: &[u8], from: &str, to: &str) -> Option<Bytes> {
    let (head, sep, body) = split_message(message);
    let head = std::str::from_utf8(head).ok()?;
    let ipv6 = from.contains(':');

    let mut changed = false;
    let mut new_body = Vec::with_capacity(body.len());
    let body_changed = match std::str::from_utf8(body) {
        Ok(text) => {
            for line in text.split_inclusive('\n') {
                if line.starts_with("c=") || line.starts_with("o=") || line.starts_with("a=rtcp:") {
                    if let Some(new) = replace_address(line, from, to, ipv6) {
                        new_body.extend_from_slice(new.as_bytes());
                        changed = true;
                        continue;
                    }
                }
                new_body.extend_from_slice(line.as_bytes());
            }
            changed
        }
        // A body that is not text names no address of ours.
        Err(_) => false,
    };
    if !body_changed {
        new_body = body.to_vec();
    }

    let mut new_head = String::with_capacity(head.len());
    for (i, line) in head.split("\r\n").enumerate() {
        if i > 0 {
            new_head.push_str("\r\n");
        }
        let name = line.split(':').next().unwrap_or("").trim();
        let rewritten = if i > 0
            && ["contact", "m", "record-route"]
                .iter()
                .any(|h| name.eq_ignore_ascii_case(h))
        {
            replace_address(line, from, to, ipv6)
        } else {
            None
        };
        match rewritten {
            Some(new) => {
                new_head.push_str(&new);
                changed = true;
            }
            None if body_changed && is_content_length(name) && i > 0 => {
                let (header, _) = line.split_once(':').unwrap_or((line, ""));
                new_head.push_str(&format!("{header}: {}", new_body.len()));
            }
            None => new_head.push_str(line),
        }
    }
    if !changed {
        return None;
    }
    let mut out = Vec::with_capacity(new_head.len() + sep.len() + new_body.len());
    out.extend_from_slice(new_head.as_bytes());
    out.extend_from_slice(sep);
    out.extend_from_slice(&new_body);
    Some(Bytes::from(out))
}

fn is_content_length(name: &str) -> bool {
    name.eq_ignore_ascii_case("content-length") || name.eq_ignore_ascii_case("l")
}

/// The header block, the blank line after it, and the body.
fn split_message(message: &[u8]) -> (&[u8], &[u8], &[u8]) {
    match message.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(at) => (&message[..at], &message[at..at + 4], &message[at + 4..]),
        None => (message, &[], &[]),
    }
}

/// `text` with each whole occurrence of the address `from` replaced: one
/// not run into a longer address (`10.0.0.1` is not in `10.0.0.12`).
fn replace_address(text: &str, from: &str, to: &str, ipv6: bool) -> Option<String> {
    let part_of = |c: char| {
        if ipv6 {
            c.is_ascii_hexdigit() || c == ':'
        } else {
            c.is_ascii_digit() || c == '.'
        }
    };
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(at) = rest.find(from) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + from.len()..].chars().next();
        out.push_str(&rest[..at]);
        if before.is_some_and(part_of) || after.is_some_and(part_of) {
            out.push_str(from);
        } else {
            out.push_str(to);
            changed = true;
        }
        rest = &rest[at + from.len()..];
    }
    out.push_str(rest);
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLIC: &str = "203.0.113.7";

    fn invite(body: &str) -> String {
        format!(
            "INVITE sip:vm@203.0.113.7:5080 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 203.0.113.7:5060;branch=z9hG4bK1\r\n\
             From: <sip:alice@203.0.113.7>;tag=a\r\n\
             To: <sip:vm@203.0.113.7:5080>\r\n\
             Contact: <sip:fcp@203.0.113.7:5060>\r\n\
             Record-Route: <sip:203.0.113.7:5060;lr>\r\n\
             Call-ID: c1@203.0.113.7\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    const SDP: &str = "v=0\r\n\
        o=- 1 1 IN IP4 203.0.113.7\r\n\
        s=-\r\n\
        c=IN IP4 203.0.113.7\r\n\
        t=0 0\r\n\
        m=audio 30000 RTP/AVP 0\r\n\
        a=rtcp:30001 IN IP4 203.0.113.7\r\n";

    #[test]
    fn contact_record_route_and_sdp_name_the_local_address() {
        let out = rewrite(invite(SDP).as_bytes(), PUBLIC, "127.0.0.1").unwrap();
        let out = std::str::from_utf8(&out).unwrap();
        assert!(out.contains("Contact: <sip:fcp@127.0.0.1:5060>"), "{out}");
        assert!(
            out.contains("Record-Route: <sip:127.0.0.1:5060;lr>"),
            "{out}"
        );
        assert!(out.contains("c=IN IP4 127.0.0.1\r\n"), "{out}");
        assert!(out.contains("o=- 1 1 IN IP4 127.0.0.1\r\n"), "{out}");
        assert!(out.contains("a=rtcp:30001 IN IP4 127.0.0.1\r\n"), "{out}");
        // What names where the request goes, or who it is, stays.
        assert!(out.starts_with("INVITE sip:vm@203.0.113.7:5080 SIP/2.0\r\n"));
        assert!(out.contains("Via: SIP/2.0/UDP 203.0.113.7:5060;"));
        assert!(out.contains("From: <sip:alice@203.0.113.7>"));
        assert!(out.contains("Call-ID: c1@203.0.113.7"));
        // The length is the new body's.
        let (_, body) = out.split_once("\r\n\r\n").unwrap();
        assert!(
            out.contains(&format!("Content-Length: {}\r\n", body.len())),
            "{out}"
        );
        assert_eq!(body, SDP.replace(PUBLIC, "127.0.0.1"));
    }

    #[test]
    fn a_longer_address_and_a_message_without_ours_are_left() {
        let other = invite(SDP).replace(PUBLIC, "203.0.113.70");
        assert!(rewrite(other.as_bytes(), PUBLIC, "127.0.0.1").is_none());
        let bye = "BYE sip:vm@203.0.113.7 SIP/2.0\r\nCall-ID: x\r\nContent-Length: 0\r\n\r\n";
        assert!(rewrite(bye.as_bytes(), PUBLIC, "127.0.0.1").is_none());
    }

    #[test]
    fn a_compact_contact_without_a_body_keeps_its_length() {
        let ok = "SIP/2.0 200 OK\r\nm: <sip:203.0.113.7:5080>\r\nl: 0\r\n\r\n";
        let out = rewrite(ok.as_bytes(), PUBLIC, "10.128.0.5").unwrap();
        assert_eq!(
            std::str::from_utf8(&out).unwrap(),
            "SIP/2.0 200 OK\r\nm: <sip:10.128.0.5:5080>\r\nl: 0\r\n\r\n"
        );
    }

    #[test]
    fn only_a_peer_on_this_host_is_given_the_local_address() {
        let sibling = SiblingRewrite::new(PUBLIC.parse().unwrap());
        let message = invite(SDP);
        let to_loopback = sibling
            .apply("127.0.0.1".parse().unwrap(), None, message.as_bytes())
            .unwrap();
        assert!(std::str::from_utf8(&to_loopback)
            .unwrap()
            .contains("c=IN IP4 127.0.0.1"));
        // A socket bound to one address advertises that one.
        let bound = sibling
            .apply(
                "127.0.0.1".parse().unwrap(),
                Some("127.0.0.2".parse().unwrap()),
                message.as_bytes(),
            )
            .unwrap();
        assert!(std::str::from_utf8(&bound)
            .unwrap()
            .contains("Contact: <sip:fcp@127.0.0.2:5060>"));
        // TEST-NET-2: no interface here holds it.
        assert!(sibling
            .apply("198.51.100.4".parse().unwrap(), None, message.as_bytes())
            .is_none());
    }
}
