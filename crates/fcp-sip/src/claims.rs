//! What a peer may claim about a request (`docs/EDGE_SECURITY.md` §2.3).
//!
//! A SIP request carries headers that decide things: which record a leg is
//! filed under, who the network says is calling, how the callee's phone
//! should alert. Every one of them is text a peer put in a packet, and
//! believing them because they arrived is how a caller chooses whose call
//! their leg is billed against, presents themselves to a room under
//! somebody else's name, or makes a desk phone answer itself.
//!
//! This module is the one place that says which of those a peer has
//! standing to assert. It runs at **ingress**, before anything reads the
//! request, so a reader written later is gated by construction rather than
//! by somebody remembering to gate it. A header a peer may not claim is
//! removed outright: the request then looks the way it would have looked
//! if the peer had never sent it.
//!
//! Headers with a gate of their own keep it and are not listed here:
//! `X-FCP-Caller-Tenant`, `X-FCP-Caller-User` and `X-FCP-Client-Address`
//! are checked against `[e911] trusted_edges` where they are read
//! ([`crate::emergency`]), `X-FCP-Trunk-Node` against the peer's client
//! certificate, and `X-FCP-Join` and `X-FCP-Migrate` carry their own HMAC.

use sip_core::Request;
use tracing::debug;

/// The headers a peer may not assert for itself.
///
/// - `X-FCP-Call-Id` and `X-FCP-Transferred-From` decide which call a
///   leg's record is filed under and which call it came from. Records are
///   rated, billed, rolled up per tenant and exported for compliance, so
///   choosing the id is choosing whose call you are correlated with.
/// - `P-Asserted-Identity`, `P-Preferred-Identity` and `Remote-Party-ID`
///   are the network's word for who is calling (RFC 3325). A peer that is
///   not the network does not get to say it; a carrier's is re-asserted at
///   the trunk, never passed through.
/// - `Alert-Info` and `Call-Info` tell the callee's phone how to alert,
///   and `;info=alert-autoanswer` / `;answer-after=0` tell it to answer
///   itself — which is exactly what this platform's own intercom sends
///   (`fcp-pbx/src/intercom.rs`). How a callee's phone alerts is the
///   node's decision, not the caller's.
/// - `X-FCP-Intercom-Mode` mutes the caller's own audio on the far leg,
///   which turns a forced answer into a one-way listen.
/// - `X-FCP-Forwarded-For` spares a leg the node placed to itself the
///   caller's class of service; believed, like a transfer origin, only
///   when it names a call this node has.
/// - `X-FCP-Own-Leg` admits an INVITE as one this node placed to itself
///   ([`crate::own_leg`]); kept only when it carries this process's token.
/// - `X-FCP-Flow-Dial` turns a user's voicemail into a refusal; kept only
///   on a leg that carries that token.
/// - `X-FCP-Own-Leg-Tenant` makes the call another tenant's; kept only on
///   a leg that carries that token.
pub const CLAIMED_HEADERS: &[&str] = &[
    fcp_core::CALL_ID_HEADER,
    crate::handler::TRANSFERRED_FROM_HEADER,
    crate::handler::FORWARDED_FOR_HEADER,
    "P-Asserted-Identity",
    "P-Preferred-Identity",
    "Remote-Party-ID",
    "Alert-Info",
    "Call-Info",
    "X-FCP-Intercom-Mode",
    crate::own_leg::OWN_LEG_HEADER,
    crate::handler::FLOW_DIAL_HEADER,
    crate::own_leg::OWN_LEG_TENANT_HEADER,
];

/// Whether this node believes what a peer says about a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerTrust {
    /// A trusted edge, an authenticated inter-node peer, or this node's
    /// own work. What it claims is believed.
    Trusted,
    /// A phone, a carrier, or whatever else reached the port.
    Untrusted,
}

impl PeerTrust {
    /// `Trusted` when `trusted`, which reads better at a call site than a
    /// bare `if`.
    pub fn of(trusted: bool) -> Self {
        if trusted {
            Self::Trusted
        } else {
            Self::Untrusted
        }
    }
}

/// What this node will believe of one request.
pub struct Claims<'a> {
    trust: PeerTrust,
    knows_call: Option<&'a dyn Fn(&str) -> bool>,
}

impl<'a> Claims<'a> {
    /// Claims from a peer of this standing.
    pub fn from(trust: PeerTrust) -> Self {
        Self {
            trust,
            knows_call: None,
        }
    }

    /// Believe a transfer origin naming a call this node knows, even from
    /// an untrusted peer.
    ///
    /// A blind transfer puts the call's id in the `Refer-To` URI, and the
    /// transferred phone copies it into its new INVITE (RFC 3261 §19.1.5)
    /// — so an ordinary phone carries one legitimately, and the value is a
    /// UUID this node minted and handed out. Asking whether the call
    /// exists keeps the transfer correlated while an invented id still
    /// names nothing. A transfer that lands on another node of a cluster
    /// loses its correlation rather than trusting an id nobody here
    /// issued.
    pub fn knowing_calls(mut self, knows_call: &'a dyn Fn(&str) -> bool) -> Self {
        self.knows_call = Some(knows_call);
        self
    }

    /// The request with every claim this peer cannot make removed, or
    /// `None` when it made none and the original will do.
    ///
    /// Trusted peers are returned `None` too: nothing of theirs is taken
    /// away.
    pub fn strip(&self, request: &Request) -> Option<Request> {
        if self.trust == PeerTrust::Trusted {
            return None;
        }

        let claimed: Vec<&str> = CLAIMED_HEADERS
            .iter()
            .copied()
            .filter(|name| request.headers().contains(name))
            .filter(|name| !self.believable(request, name))
            .collect();
        if claimed.is_empty() {
            return None;
        }

        let mut stripped = request.clone();
        for name in &claimed {
            stripped.headers_mut().remove(name);
        }
        debug!(
            headers = ?claimed,
            "Ignoring what an untrusted peer claims about this request"
        );
        Some(stripped)
    }

    /// Whether this one header survives despite the peer being untrusted.
    fn believable(&self, request: &Request, name: &str) -> bool {
        // A shared line's phone names the appearance it calls from or picks
        // up (`docs/ENTERPRISE_CALLING.md` §3.9). That is the phone's to
        // say — the node checks it against the line's appearances — and is
        // never passed to another leg; it forces no answer.
        if name.eq_ignore_ascii_case("Call-Info") {
            let values = request.headers().get_all(name);
            let mut any = false;
            for value in values {
                let value = value.to_ascii_lowercase();
                if !value.contains("appearance-index") || value.contains("answer-after") {
                    return false;
                }
                any = true;
            }
            return any;
        }
        if name.eq_ignore_ascii_case(crate::own_leg::OWN_LEG_HEADER)
            || name.eq_ignore_ascii_case(crate::handler::FLOW_DIAL_HEADER)
            || name.eq_ignore_ascii_case(crate::own_leg::OWN_LEG_TENANT_HEADER)
        {
            return crate::own_leg::proves(request);
        }
        if !name.eq_ignore_ascii_case(crate::handler::TRANSFERRED_FROM_HEADER)
            && !name.eq_ignore_ascii_case(crate::handler::FORWARDED_FOR_HEADER)
        {
            return false;
        }
        let Some(knows_call) = self.knows_call else {
            return false;
        };
        sip_parse::header(request.headers(), name)
            .map(|value| knows_call(value.as_str().trim()))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A phone naming its appearance keeps it; one asking the far end to
    /// answer by itself does not, even beside one.
    #[test]
    fn an_appearance_is_the_phones_to_name_and_an_auto_answer_is_not() {
        let claims = Claims::from(PeerTrust::Untrusted);
        let seizing = request(&["Call-Info: <sip:10.0.0.1>;appearance-index=2"]);
        assert!(claims.strip(&seizing).is_none());
        let forcing = request(&[
            "Call-Info: <sip:10.0.0.1>;appearance-index=2",
            "Call-Info: <sip:10.0.0.1>;answer-after=0",
        ]);
        let stripped = claims.strip(&forcing).expect("stripped");
        assert!(!has(&stripped, "Call-Info"));
        let sneaking = request(&["Call-Info: <sip:10.0.0.1>;appearance-index=2;answer-after=0"]);
        assert!(claims.strip(&sneaking).is_some());
    }

    /// A leg this node placed to itself keeps its own-leg token; one that
    /// guessed at it does not, and is not admitted as the node's own.
    #[test]
    fn only_this_process_s_own_leg_token_survives() {
        let claims = Claims::from(PeerTrust::Untrusted);
        let own = request(&[&format!("X-FCP-Own-Leg: {}", crate::own_leg::token())]);
        assert!(claims.strip(&own).is_none());
        assert!(crate::own_leg::proves(&own));
        let forged = request(&["X-FCP-Own-Leg: 0123456789abcdef"]);
        let stripped = claims.strip(&forged).expect("stripped");
        assert!(!has(&stripped, "X-FCP-Own-Leg"));
        assert!(!crate::own_leg::proves(&forged));
    }

    /// A queue's leg to this node names the tenant it rings for; a phone
    /// that names one is not believed (docs/TENANT_SCOPE.md §4b).
    #[test]
    fn only_an_own_leg_names_its_tenant() {
        let claims = Claims::from(PeerTrust::Untrusted);
        let tenant = fcp_core::TenantId::new();
        let own = request(&[
            &format!("X-FCP-Own-Leg: {}", crate::own_leg::token()),
            &format!("X-FCP-Own-Leg-Tenant: {tenant}"),
        ]);
        assert!(claims.strip(&own).is_none());
        assert_eq!(crate::own_leg::tenant(&own), Some(tenant));
        let forged = request(&[&format!("X-FCP-Own-Leg-Tenant: {tenant}")]);
        let stripped = claims.strip(&forged).expect("stripped");
        assert!(!has(&stripped, "X-FCP-Own-Leg-Tenant"));
        assert_eq!(crate::own_leg::tenant(&forged), None);
    }

    /// A request carrying `headers`, each a `Name: value` line.
    fn request(headers: &[&str]) -> Request {
        let extra: String = headers.iter().map(|h| format!("{h}\r\n")).collect();
        let raw = format!(
            "INVITE sip:bob@fcp.local SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.9:5060;branch=z9hG4bK1\r\n\
             From: <sip:alice@fcp.local>;tag=1\r\n\
             To: <sip:bob@fcp.local>\r\n\
             Call-ID: call-1\r\n\
             CSeq: 1 INVITE\r\n\
             {extra}\
             Content-Length: 0\r\n\r\n"
        );
        sip_parse::parse_request(&bytes::Bytes::from(raw)).expect("test request parses")
    }

    fn has(request: &Request, name: &str) -> bool {
        request.headers().contains(name)
    }

    #[test]
    fn an_untrusted_peer_does_not_choose_the_record_its_leg_is_filed_under() {
        // `docs/EDGE_SECURITY.md` §2.3: records are rated and billed, so
        // choosing the id is choosing whose call you are correlated with.
        let claimed = request(&["X-FCP-Call-Id: call-11111111-1111-1111-1111-111111111111"]);
        let stripped = Claims::from(PeerTrust::Untrusted)
            .strip(&claimed)
            .expect("the claim is taken away");
        assert!(!has(&stripped, "X-FCP-Call-Id"));
        // The rest of the request is untouched.
        assert!(has(&stripped, "From") && has(&stripped, "CSeq"));
    }

    #[test]
    fn a_trusted_edge_still_names_the_record() {
        let claimed = request(&["X-FCP-Call-Id: call-11111111-1111-1111-1111-111111111111"]);
        assert!(Claims::from(PeerTrust::Trusted).strip(&claimed).is_none());
    }

    #[test]
    fn a_caller_does_not_make_the_callees_phone_answer_itself() {
        // The platform's own intercom sends exactly these
        // (`fcp-pbx/src/intercom.rs`); forwarding a caller's copy of them
        // turns any phone into a listening device, and the intercom mode
        // header mutes the caller so nobody hears it happen.
        let claimed = request(&[
            "Alert-Info: <http://x/>;info=alert-autoanswer;delay=0",
            "Call-Info: <sip:>;answer-after=0",
            "X-FCP-Intercom-Mode: oneway",
        ]);
        let stripped = Claims::from(PeerTrust::Untrusted)
            .strip(&claimed)
            .expect("the claims are taken away");
        assert!(!has(&stripped, "Alert-Info"));
        assert!(!has(&stripped, "Call-Info"));
        assert!(!has(&stripped, "X-FCP-Intercom-Mode"));
    }

    #[test]
    fn a_peer_is_not_the_network_that_asserts_who_is_calling() {
        let claimed = request(&[
            "P-Asserted-Identity: <sip:+15125550100@carrier.example>",
            "P-Preferred-Identity: <sip:+15125550100@carrier.example>",
            "Remote-Party-ID: <sip:+15125550100@carrier.example>;party=calling",
        ]);
        let stripped = Claims::from(PeerTrust::Untrusted)
            .strip(&claimed)
            .expect("the claims are taken away");
        assert!(!has(&stripped, "P-Asserted-Identity"));
        assert!(!has(&stripped, "P-Preferred-Identity"));
        assert!(!has(&stripped, "Remote-Party-ID"));
    }

    #[test]
    fn every_copy_of_a_claimed_header_goes() {
        let claimed = request(&[
            "Alert-Info: <http://one/>",
            "Alert-Info: <http://two/>;info=alert-autoanswer",
        ]);
        let stripped = Claims::from(PeerTrust::Untrusted)
            .strip(&claimed)
            .expect("the claims are taken away");
        assert!(!has(&stripped, "Alert-Info"));
    }

    #[test]
    fn a_claim_is_matched_however_it_is_spelled() {
        let claimed = request(&["alert-info: <http://x/>;info=alert-autoanswer"]);
        assert!(Claims::from(PeerTrust::Untrusted).strip(&claimed).is_some());
    }

    #[test]
    fn a_request_claiming_nothing_is_left_alone() {
        assert!(Claims::from(PeerTrust::Untrusted)
            .strip(&request(&[]))
            .is_none());
    }

    #[test]
    fn a_transfer_origin_this_node_minted_survives() {
        // The transferred phone copies it out of the Refer-To this node
        // built (RFC 3261 §19.1.5), so an ordinary phone carries one.
        let known = "call-11111111-1111-1111-1111-111111111111";
        let claimed = request(&[&format!("X-FCP-Transferred-From: {known}")]);
        let knows = |id: &str| id == known;
        assert!(Claims::from(PeerTrust::Untrusted)
            .knowing_calls(&knows)
            .strip(&claimed)
            .is_none());
    }

    #[test]
    fn a_transfer_origin_naming_nothing_does_not() {
        let claimed =
            request(&["X-FCP-Transferred-From: call-22222222-2222-2222-2222-222222222222"]);
        let knows = |_: &str| false;
        let stripped = Claims::from(PeerTrust::Untrusted)
            .knowing_calls(&knows)
            .strip(&claimed)
            .expect("an id this node never issued is taken away");
        assert!(!has(&stripped, "X-FCP-Transferred-From"));
    }

    #[test]
    fn a_node_that_cannot_check_takes_the_transfer_origin_away() {
        let claimed =
            request(&["X-FCP-Transferred-From: call-22222222-2222-2222-2222-222222222222"]);
        let stripped = Claims::from(PeerTrust::Untrusted)
            .strip(&claimed)
            .expect("unverifiable is not believed");
        assert!(!has(&stripped, "X-FCP-Transferred-From"));
    }

    #[test]
    fn the_exception_is_only_for_the_transfer_origin() {
        // A `knows_call` that says yes to everything must not rescue the
        // other claims.
        let claimed = request(&[
            "X-FCP-Call-Id: call-11111111-1111-1111-1111-111111111111",
            "Alert-Info: <http://x/>;info=alert-autoanswer",
        ]);
        let yes = |_: &str| true;
        let stripped = Claims::from(PeerTrust::Untrusted)
            .knowing_calls(&yes)
            .strip(&claimed)
            .expect("the claims are taken away");
        assert!(!has(&stripped, "X-FCP-Call-Id"));
        assert!(!has(&stripped, "Alert-Info"));
    }
}
