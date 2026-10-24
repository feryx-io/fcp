//! Phones behind NAT.
//!
//! A phone behind NAT names an address in its Contact and its SDP that
//! nobody outside its network can reach: its LAN address, or loopback when
//! it knows no better. What the network saw — where its REGISTER and its
//! INVITE came from — is where it is. A UDP phone without an RFC 5626 flow
//! is rung at the address it registered from, and an anchored call learns
//! such a leg's RTP address from its first packet, accepted only from the
//! address its SIP came from (`docs/SINGLE-NODE-SETUP.md` "Phones behind
//! NAT").

use std::net::{IpAddr, SocketAddr};

use fcp_core::storage::Registration;

/// An address only its own network can reach: private, loopback,
/// link-local, shared (CGNAT), unspecified, or IPv6 unique-local.
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (first & 0xfe00) == 0xfc00
                    || (first & 0xffc0) == 0xfe80
            }
        },
    }
}

/// Whether a peer that names `advertised` but was seen at `seen` is behind
/// NAT. A local address seen from a public one is; so is loopback seen
/// from anywhere else. A peer on the server's own network naming its own
/// LAN address is not, nor is a trunk whose media comes from another public
/// address than its signalling, nor `0.0.0.0` — an old-style hold, not an
/// address.
pub fn behind_nat(advertised: IpAddr, seen: IpAddr) -> bool {
    let (advertised, seen) = (canonical(advertised), canonical(seen));
    if advertised == seen || advertised.is_unspecified() {
        return false;
    }
    advertised.is_loopback() || (is_local(advertised) && !is_local(seen))
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

/// The address an SDP's audio is sent to: the audio section's `c=` line,
/// else the session's.
pub fn sdp_address(sdp: &str) -> Option<IpAddr> {
    let mut session = None;
    let mut in_audio = false;
    let mut seen_media = false;
    for line in sdp.lines().map(str::trim) {
        if let Some(media) = line.strip_prefix("m=") {
            in_audio = media.starts_with("audio");
            seen_media = true;
        } else if let Some(conn) = line.strip_prefix("c=") {
            let ip = conn
                .split_whitespace()
                .nth(2)
                .and_then(|a| a.split('/').next())
                .and_then(|a| a.parse::<IpAddr>().ok());
            if in_audio {
                if let Some(ip) = ip {
                    return Some(ip);
                }
            } else if !seen_media {
                session = ip;
            }
        }
    }
    session
}

/// Whether an anchored leg whose SDP is `sdp` and whose SIP came from
/// `seen` must have its RTP address learned rather than read off the SDP.
pub fn learns_media_address(sdp: &str, seen: IpAddr) -> bool {
    sdp_address(sdp).is_some_and(|advertised| behind_nat(advertised, seen))
}

/// The host of a SIP URI or Contact, when it is an address.
pub fn uri_address(uri: &str) -> Option<IpAddr> {
    host_span(uri).and_then(|(start, end)| fcp_core::e911::parse_address(&uri[start..end]))
}

/// The Contact a registration is reached at: as registered, or for a UDP
/// phone behind NAT without a flow, its URI's host and port replaced with
/// the address the REGISTER came from.
pub fn reachable_contact(registration: &Registration) -> String {
    nated_contact(registration).unwrap_or_else(|| registration.contact.clone())
}

fn nated_contact(registration: &Registration) -> Option<String> {
    if registration.flow_token.is_some() {
        return None;
    }
    let source: SocketAddr = registration.source_address.as_deref()?.parse().ok()?;
    seen_contact(&registration.contact, source)
}

/// A Contact that a peer seen at `seen` sent over UDP, with its host and
/// port replaced by `seen` when the peer is behind NAT; `None` when it is
/// right as it is.
fn seen_contact(contact: &str, seen: SocketAddr) -> Option<String> {
    if !over_udp(contact) {
        return None;
    }
    let (start, end) = host_span(contact)?;
    let advertised = fcp_core::e911::parse_address(&contact[start..end])?;
    if !behind_nat(advertised, seen.ip()) {
        return None;
    }
    // The port follows the host: the mapping NAT made is for both.
    let end = port_end(contact, end);
    Some(format!("{}{}{}", &contact[..start], seen, &contact[end..]))
}

/// Make the Contact of a request that arrived over UDP from `peer` one
/// that reaches it, so the dialog it starts or refreshes sends its
/// requests where the peer is. Not for REGISTER: the registrar keeps the
/// contact as sent, beside the address it came from
/// ([`reachable_contact`]).
pub fn fix_contact(headers: &mut sip_core::Headers, peer: SocketAddr) -> bool {
    let Some(fixed) = headers.get("Contact").and_then(|c| seen_contact(c, peer)) else {
        return false;
    };
    headers.replace_first("Contact", fixed).unwrap_or(false)
}

/// Whether a Contact is reached over UDP: `sip:` with no transport or
/// `transport=udp`.
fn over_udp(contact: &str) -> bool {
    let uri = uri_part(contact);
    if uri.trim_start().to_ascii_lowercase().starts_with("sips:") {
        return false;
    }
    uri.split(';').skip(1).all(|param| {
        let param = param.trim().to_ascii_lowercase();
        match param.strip_prefix("transport=") {
            Some(transport) => transport == "udp",
            None => true,
        }
    })
}

/// The URI inside a Contact's angle brackets, or the whole of it.
fn uri_part(contact: &str) -> &str {
    match (contact.find('<'), contact.find('>')) {
        (Some(start), Some(end)) if start < end => &contact[start + 1..end],
        _ => contact,
    }
}

/// Byte offsets of the host (without the port) in a Contact or URI.
fn host_span(contact: &str) -> Option<(usize, usize)> {
    let base = match (contact.find('<'), contact.find('>')) {
        (Some(start), Some(end)) if start < end => start + 1,
        _ => 0,
    };
    let uri = uri_part(contact);
    let limit = uri.find([';', '?', '>']).unwrap_or(uri.len());
    let addr = &uri[..limit];
    let after_scheme = addr.find(':').map(|i| i + 1).unwrap_or(0);
    // The user part may itself contain an `@`, so take the last one.
    let host_start = addr.rfind('@').map(|i| i + 1).unwrap_or(after_scheme);
    let host = &addr[host_start..];
    let host_len = if host.starts_with('[') {
        host.find(']')? + 1
    } else {
        host.find(':').unwrap_or(host.len())
    };
    if host_len == 0 {
        return None;
    }
    Some((base + host_start, base + host_start + host_len))
}

/// Where a host's `:port`, if any, ends.
fn port_end(contact: &str, host_end: usize) -> usize {
    let rest = &contact[host_end..];
    match rest.strip_prefix(':') {
        Some(port) => host_end + 1 + port.bytes().take_while(u8::is_ascii_digit).count(),
        None => host_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn registration(contact: &str, source: Option<&str>, flow: bool) -> Registration {
        Registration {
            tenant_id: fcp_core::TenantId::nil(),
            aor: "sip:bob@example.com".to_string(),
            contact: contact.to_string(),
            expires_at: SystemTime::now(),
            call_id: "c".to_string(),
            cseq: 1,
            user_agent: None,
            q: None,
            registered_at: SystemTime::now(),
            path: None,
            flow_token: flow.then(|| "t".to_string()),
            source_address: source.map(str::to_string),
            instance_id: None,
            reg_id: None,
            push: None,
            flow_node: None,
        }
    }

    #[test]
    fn a_local_address_seen_from_a_public_one_is_behind_nat() {
        assert!(behind_nat(ip("192.168.1.20"), ip("203.0.113.7")));
        assert!(behind_nat(ip("100.64.3.4"), ip("203.0.113.7")));
        assert!(behind_nat(ip("fd00::5"), ip("2001:db8::7")));
        // Loopback names nowhere, whoever sent it.
        assert!(behind_nat(ip("127.0.0.1"), ip("192.168.1.20")));
    }

    #[test]
    fn a_peer_on_the_servers_network_or_a_trunk_is_not() {
        assert!(!behind_nat(ip("192.168.1.20"), ip("192.168.1.20")));
        assert!(!behind_nat(ip("10.0.0.5"), ip("172.18.0.4")));
        // A carrier's media from another public address than its SIP.
        assert!(!behind_nat(ip("198.51.100.9"), ip("203.0.113.7")));
        assert!(!behind_nat(ip("10.0.0.5"), ip("::ffff:10.0.0.5")));
        // A hold the old way.
        assert!(!behind_nat(ip("0.0.0.0"), ip("203.0.113.7")));
    }

    #[test]
    fn the_sdp_address_is_the_audio_sections_else_the_sessions() {
        let session = "v=0\r\nc=IN IP4 127.0.0.1\r\nm=audio 5000 RTP/AVP 0\r\n";
        assert_eq!(sdp_address(session), Some(ip("127.0.0.1")));
        let audio = "v=0\r\nc=IN IP4 10.0.0.1\r\nm=video 6000 RTP/AVP 96\r\nc=IN IP4 10.0.0.2\r\n\
                     m=audio 5000 RTP/AVP 0\r\nc=IN IP4 10.0.0.3\r\n";
        assert_eq!(sdp_address(audio), Some(ip("10.0.0.3")));
        assert_eq!(sdp_address("v=0\r\nm=audio 5000 RTP/AVP 0\r\n"), None);
        assert!(learns_media_address(session, ip("203.0.113.7")));
        assert!(!learns_media_address(session, ip("127.0.0.1")));
    }

    #[test]
    fn a_udp_phone_behind_nat_is_reached_where_it_registered_from() {
        let reg = registration(
            "<sip:bob@127.0.0.1:56409>;+sip.instance=\"<urn:uuid:1>\"",
            Some("38.248.41.91:56409"),
            false,
        );
        assert_eq!(
            reachable_contact(&reg),
            "<sip:bob@38.248.41.91:56409>;+sip.instance=\"<urn:uuid:1>\""
        );
        let bare = registration(
            "sip:bob@192.168.1.20;transport=udp",
            Some("203.0.113.7:4000"),
            false,
        );
        assert_eq!(
            reachable_contact(&bare),
            "sip:bob@203.0.113.7:4000;transport=udp"
        );
        let v6 = registration(
            "<sip:bob@[fd00::5]:5060>",
            Some("[2001:db8::7]:4000"),
            false,
        );
        assert_eq!(reachable_contact(&v6), "<sip:bob@[2001:db8::7]:4000>");
    }

    #[test]
    fn other_contacts_are_kept_as_registered() {
        for (contact, source, flow) in [
            // A flow reaches it already.
            (
                "<sip:bob@192.168.1.20:5060>",
                Some("203.0.113.7:4000"),
                true,
            ),
            // Over TCP or TLS a new connection to the source goes nowhere.
            (
                "<sip:bob@192.168.1.20:5060;transport=tcp>",
                Some("203.0.113.7:4000"),
                false,
            ),
            (
                "<sips:bob@192.168.1.20:5061>",
                Some("203.0.113.7:4000"),
                false,
            ),
            // Public, a name, on the server's network, or nothing seen.
            (
                "<sip:alice@38.248.41.91:5060>",
                Some("38.248.41.91:5060"),
                false,
            ),
            (
                "<sip:bob@phone.example.com>",
                Some("203.0.113.7:4000"),
                false,
            ),
            (
                "<sip:bob@192.168.1.20:5060>",
                Some("192.168.1.1:5060"),
                false,
            ),
            ("<sip:bob@192.168.1.20:5060>", None, false),
        ] {
            let reg = registration(contact, source, flow);
            assert_eq!(reachable_contact(&reg), contact, "{contact}");
        }
    }

    #[test]
    fn a_requests_contact_is_made_where_the_peer_was_seen() {
        let peer: SocketAddr = "38.248.41.91:56409".parse().unwrap();
        let mut headers = sip_core::Headers::new();
        headers
            .push("Contact", "<sip:bob@127.0.0.1:56409;ob>")
            .unwrap();
        assert!(fix_contact(&mut headers, peer));
        assert_eq!(
            headers.get("Contact"),
            Some("<sip:bob@38.248.41.91:56409;ob>")
        );
        // Right already: left alone.
        assert!(!fix_contact(&mut headers, peer));
        let mut none = sip_core::Headers::new();
        assert!(!fix_contact(&mut none, peer));
    }

    #[test]
    fn a_uris_address_is_read_past_its_user_and_parameters() {
        assert_eq!(
            uri_address("sip:bob@38.248.41.91:56409"),
            Some(ip("38.248.41.91"))
        );
        assert_eq!(
            uri_address("<sip:bob@[2001:db8::7]:4000;transport=udp>"),
            Some(ip("2001:db8::7"))
        );
        assert_eq!(uri_address("sip:38.248.41.91"), Some(ip("38.248.41.91")));
        assert_eq!(uri_address("sip:bob@pbx.example.com"), None);
    }
}
