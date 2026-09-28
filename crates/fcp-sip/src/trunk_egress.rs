//! What a trunk does to the call it carries (`docs/TENANT_TRUNKS.md` §6).
//!
//! The route says which trunk a leg goes out on; this says what that trunk
//! makes of the INVITE — the identity it presents, the session timer it
//! asks for, the DTMF it takes. The number transform and the destination
//! are the route's (the router applies both when it builds the target);
//! what is left is everything the B-leg's INVITE carries.

use fcp_b2bua_core::{DtmfCarriage, DtmfWay};
use fcp_core::trunk::TrunkDtmfMode;
use fcp_core::Trunk;
use sip_core::SipUri;

/// RFC 4028 §4: a session interval below 90 seconds is not asked for.
pub const MIN_SESSION_EXPIRES: u32 = 90;

/// The STIR/SHAKEN signing a trunk asks for (`docs/TENANT_TRUNKS.md` §6,
/// RFC 8224). The key and the certificate it names are the node's; which
/// calls are signed, and at what attestation, is the trunk's to say.
pub trait TrunkCallSigner: Send + Sync {
    /// The `Identity` header value for a call from `orig_tn` to `dest_tn`,
    /// or `None` when this node cannot sign it.
    fn identity(
        &self,
        orig_tn: &str,
        dest_tn: &str,
        attestation: fcp_core::trunk::AttestationLevel,
    ) -> Option<String>;
}

/// The header manipulation rules a trunk names (`docs/TENANT_TRUNKS.md`
/// §6). The engine that compiles and runs them is the SBC's, in
/// `fcp-sbc-core`; a node that has one hands it to its handler, and a node
/// without one applies no rules.
///
/// The rules act on a request this node *builds*, so they run on the
/// finished INVITE — after the Via and Contact, before it is sent.
pub trait TrunkHeaderRules: Send + Sync {
    /// Apply the egress rules `names` names to a request going out.
    fn apply_egress(&self, names: &[String], request: &mut sip_core::Request);

    /// Apply the ingress rules `names` names to a request that arrived on
    /// the trunk (`docs/TENANT_TRUNKS.md` §7).
    fn apply_ingress(&self, names: &[String], request: &mut sip_core::Request);
}

/// The INVITE a trunk wants, over the one the B2BUA would have sent.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TrunkEgress {
    /// The From URI for this leg, when the trunk names one of its own.
    /// `None` leaves the caller's own identity in place, which is what a
    /// B2BUA does by default.
    pub from: Option<String>,
    /// Headers added to the INVITE, in order.
    pub headers: Vec<(String, String)>,
    /// Whether RFC 2833 `telephone-event` stays in the offer this leg
    /// sends. A trunk that takes DTMF as SIP INFO, or takes none, is not
    /// offered it.
    pub offer_telephone_event: bool,
    /// The header manipulation rules the trunk names, run on the finished
    /// INVITE by whatever [`TrunkHeaderRules`] the node has.
    pub header_rules: Vec<String>,
    /// The caller's identity is withheld: the From says `Anonymous`.
    pub anonymous: bool,
}

/// The From URI of a withheld caller (RFC 3323 §4.1.1.3).
pub const ANONYMOUS_URI: &str = "sip:anonymous@anonymous.invalid";

/// A URI without its angle brackets and header parameters.
fn bare_uri(uri: &str) -> &str {
    let uri = uri.trim();
    match (uri.find('<'), uri.find('>')) {
        (Some(start), Some(end)) if start < end => &uri[start + 1..end],
        _ => uri.split(';').next().unwrap_or(uri),
    }
}

impl TrunkEgress {
    /// What `trunk` makes of a leg whose caller is `caller_from` (the From
    /// URI the B2BUA would have used).
    ///
    /// An emergency call's identity is the location's callback number and
    /// outranks anything the trunk says (`docs/E911.md`), so `emergency`
    /// keeps the caller's identity as it is; the rest of the trunk — its
    /// session timer, its DTMF — still applies.
    pub fn of(trunk: &Trunk, caller_from: Option<&str>, emergency: bool) -> Self {
        let from = if emergency {
            None
        } else {
            trunk_from(trunk, caller_from)
        };

        let mut headers = Vec::new();
        // The identity the trunk asserts is the one it presents: a carrier
        // that bills by P-Asserted-Identity must see the trunk's number,
        // not the extension behind it. An emergency call has asserted its
        // callback number already.
        if let Some(from) = &from {
            headers.push(("P-Asserted-Identity".to_string(), format!("<{from}>")));
        }

        // RFC 4028: ask for the session refresh the trunk wants, and say
        // this leg understands the answer.
        if trunk.session_timer_secs > 0 {
            let expires = trunk.session_timer_secs.max(MIN_SESSION_EXPIRES);
            headers.push((
                "Session-Expires".to_string(),
                format!("{expires};refresher=uac"),
            ));
            headers.push(("Min-SE".to_string(), MIN_SESSION_EXPIRES.to_string()));
            headers.push(("Supported".to_string(), "timer".to_string()));
        }

        Self {
            from,
            headers,
            offer_telephone_event: matches!(trunk.dtmf.mode, TrunkDtmfMode::Relay),
            header_rules: trunk.header_rules.clone(),
            anonymous: false,
        }
    }

    /// Present `uri` — the caller's line (`docs/ENTERPRISE_CALLING.md`
    /// §3.7) — when the trunk named no identity of its own.
    pub fn present_as(&mut self, uri: &str) {
        if self.from.is_some() {
            return;
        }
        self.from = Some(uri.to_string());
        self.headers
            .insert(0, ("P-Asserted-Identity".to_string(), format!("<{uri}>")));
    }

    /// Withhold the caller's identity (RFC 3323, RFC 3325): an anonymous
    /// From, `Privacy: id`, and the identity the leg would have presented
    /// kept in `P-Asserted-Identity` for the carrier, which must not pass
    /// it on.
    pub fn withhold(&mut self, caller_from: Option<&str>) {
        let identity = self
            .from
            .clone()
            .or_else(|| caller_from.map(|c| bare_uri(c).to_string()));
        self.headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("P-Asserted-Identity"));
        if let Some(identity) = identity {
            self.headers.insert(
                0,
                ("P-Asserted-Identity".to_string(), format!("<{identity}>")),
            );
        }
        self.headers.push(("Privacy".to_string(), "id".to_string()));
        self.from = Some(ANONYMOUS_URI.to_string());
        self.anonymous = true;
    }

    /// Nothing: a leg that no trunk carries.
    pub fn none() -> Self {
        Self {
            from: None,
            headers: Vec::new(),
            offer_telephone_event: true,
            header_rules: Vec::new(),
            anonymous: false,
        }
    }

    /// Sign the call this leg carries, when the trunk asks for it
    /// (RFC 8224). The `Identity` header covers the numbers as the carrier
    /// will see them — after the trunk's caller ID and its digit plan —
    /// because those are the numbers it will verify.
    ///
    /// A trunk that asks for signing on a node with no signer, or a call
    /// with no number at one end, goes out unsigned: an `Identity` header
    /// that does not match the call is worse than none.
    pub fn sign(
        &mut self,
        trunk: &Trunk,
        signer: Option<&std::sync::Arc<dyn TrunkCallSigner>>,
        orig_tn: &str,
        dest_tn: &str,
    ) {
        if trunk.stir_shaken.sign_mode != fcp_core::trunk::StirSignMode::Enabled {
            return;
        }
        let (Some(signer), false, false) = (signer, orig_tn.is_empty(), dest_tn.is_empty()) else {
            return;
        };
        if let Some(identity) = signer.identity(orig_tn, dest_tn, trunk.stir_shaken.attestation) {
            self.headers.push(("Identity".to_string(), identity));
        }
    }

    /// The From URI as a parsed SIP URI, for the INVITE's options.
    pub fn from_uri(&self) -> Option<SipUri> {
        self.from.as_deref().and_then(|f| SipUri::parse(f).ok())
    }

    /// The last look at the INVITE this trunk's rules want, when it names
    /// any and the node has an engine to run them.
    pub fn request_filter(
        &self,
        rules: Option<&std::sync::Arc<dyn TrunkHeaderRules>>,
    ) -> Option<sip_uac::RequestFilter> {
        let rules = rules
            .filter(|_| !self.header_rules.is_empty())
            .map(std::sync::Arc::clone);
        if rules.is_none() && !self.anonymous {
            return None;
        }
        let names = self.header_rules.clone();
        let anonymous = self.anonymous;
        Some(sip_uac::RequestFilter::new(move |request| {
            if anonymous {
                anonymous_display(request);
            }
            if let Some(rules) = &rules {
                rules.apply_egress(&names, request)
            }
        }))
    }
}

/// Name the From `Anonymous`, keeping its URI and tag: the UAC writes a
/// display name of its own, which a withheld caller must not carry.
fn anonymous_display(request: &mut sip_core::Request) {
    let Some(from) = request.headers().get("From").map(|v| v.to_string()) else {
        return;
    };
    let params = match (from.rfind('>'), from.find(';')) {
        (Some(end), _) => from[end + 1..].to_string(),
        (None, Some(semi)) => from[semi..].to_string(),
        (None, None) => String::new(),
    };
    let value = format!("\"Anonymous\" <{ANONYMOUS_URI}>{params}");
    request.headers_mut().remove("From");
    let _ = request.headers_mut().push(
        smol_str::SmolStr::new("From"),
        smol_str::SmolStr::new(value),
    );
}

/// The content type SIP INFO carries a digit in.
pub const DTMF_RELAY: &str = "application/dtmf-relay";

/// How long a digit is said to last when its sender did not say.
const DEFAULT_DIGIT_MS: u32 = 160;

/// How each leg of a call `trunk` carries takes DTMF, from the caller's
/// offer and the trunk's answer (`docs/TENANT_TRUNKS.md` §6).
///
/// A leg takes RFC 2833 only when its SDP has `telephone-event`: a trunk
/// set to RFC 2833 that answers without it takes digits in the audio, if
/// at all. A caller that did not offer it is left to send and receive
/// digits as it did before — INFO passes it unchanged.
pub fn dtmf_carriage(trunk: &Trunk, a_offer: Option<&str>, b_answer: &str) -> DtmfCarriage {
    let takes_2833 = |sdp: &str| fcp_media::extract_telephone_event_pt(sdp).is_some();
    let a = if a_offer.is_some_and(takes_2833) {
        DtmfWay::Rfc2833
    } else {
        DtmfWay::Info
    };
    let b = match trunk.dtmf.mode {
        TrunkDtmfMode::Relay if takes_2833(b_answer) => DtmfWay::Rfc2833,
        TrunkDtmfMode::Relay | TrunkDtmfMode::Disabled => DtmfWay::Inband,
        TrunkDtmfMode::Detect => DtmfWay::Info,
    };
    DtmfCarriage { a, b }
}

/// The `application/dtmf-relay` body for one digit.
pub fn dtmf_relay_body(digit: char, duration_ms: Option<u32>) -> String {
    format!(
        "Signal={digit}\r\nDuration={}\r\n",
        duration_ms.unwrap_or(DEFAULT_DIGIT_MS)
    )
}

/// The digit and duration an `application/dtmf-relay` body carries.
pub fn parse_dtmf_relay(body: &str) -> Option<(char, u32)> {
    let mut digit = None;
    let mut duration = DEFAULT_DIGIT_MS;
    for line in body.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "signal" => {
                digit = value
                    .chars()
                    .next()
                    .map(|c| c.to_ascii_uppercase())
                    .filter(|c| c.is_ascii_digit() || matches!(c, '*' | '#' | 'A'..='D'));
            }
            "duration" => duration = value.parse().unwrap_or(DEFAULT_DIGIT_MS),
            _ => {}
        }
    }
    Some((digit?, duration))
}

/// The From URI `trunk` presents for a caller whose own From is
/// `caller_from`: `caller_id_override` replaces the user part (or the
/// whole URI when it is one), `from_domain` replaces the host. `None` when
/// the trunk says neither.
fn trunk_from(trunk: &Trunk, caller_from: Option<&str>) -> Option<String> {
    let override_id = trunk
        .caller_id_override
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let domain = trunk
        .from_domain
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    if override_id.is_none() && domain.is_none() {
        return None;
    }

    // A caller ID given as a whole URI is used as it stands, except that
    // the trunk's own domain still wins over the one it names.
    let (caller_user, caller_host) = split_uri(caller_from);
    let (override_user, override_host) = match override_id {
        Some(id) if id.contains('@') || id.starts_with("sip:") || id.starts_with("sips:") => {
            let (u, h) = split_uri(Some(id));
            (u, h)
        }
        Some(id) => (Some(id.to_string()), None),
        None => (None, None),
    };

    let user = override_user.or(caller_user)?;
    let host = domain
        .map(str::to_string)
        .or(override_host)
        .or(caller_host)?;
    Some(format!("sip:{user}@{host}"))
}

/// The user and host of a SIP URI, in whatever shape it arrived
/// (`sip:alice@example.com`, `<sip:alice@example.com>`, `alice@host`).
fn split_uri(uri: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(uri) = uri else {
        return (None, None);
    };
    let uri = uri
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    let uri = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .unwrap_or(uri);
    let uri = uri.split(';').next().unwrap_or(uri);
    match uri.split_once('@') {
        Some((user, host)) => (
            (!user.is_empty()).then(|| user.to_string()),
            (!host.is_empty()).then(|| host.to_string()),
        ),
        None => ((!uri.is_empty()).then(|| uri.to_string()), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::trunk::TrunkDtmfConfig;
    use fcp_core::TenantId;

    const WITH_TE: &str = "v=0\r\no=- 1 1 IN IP4 10.0.0.1\r\ns=-\r\nc=IN IP4 10.0.0.1\r\nt=0 0\r\n\
        m=audio 4000 RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\n";
    const WITHOUT_TE: &str =
        "v=0\r\no=- 1 1 IN IP4 10.0.0.1\r\ns=-\r\nc=IN IP4 10.0.0.1\r\nt=0 0\r\n\
        m=audio 4000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

    fn trunk_taking(mode: TrunkDtmfMode) -> Trunk {
        let mut t = trunk();
        t.dtmf = TrunkDtmfConfig { mode };
        t
    }

    #[test]
    fn an_info_trunk_takes_info_whatever_the_caller_sends() {
        let t = trunk_taking(TrunkDtmfMode::Detect);
        assert_eq!(
            dtmf_carriage(&t, Some(WITH_TE), WITHOUT_TE),
            DtmfCarriage {
                a: DtmfWay::Rfc2833,
                b: DtmfWay::Info
            }
        );
        assert_eq!(
            dtmf_carriage(&t, Some(WITHOUT_TE), WITHOUT_TE),
            DtmfCarriage {
                a: DtmfWay::Info,
                b: DtmfWay::Info
            }
        );
    }

    #[test]
    fn an_rfc2833_trunk_takes_it_only_when_its_answer_does() {
        let t = trunk_taking(TrunkDtmfMode::Relay);
        assert_eq!(dtmf_carriage(&t, None, WITH_TE).b, DtmfWay::Rfc2833);
        assert_eq!(dtmf_carriage(&t, None, WITH_TE).a, DtmfWay::Info);
        assert_eq!(dtmf_carriage(&t, None, WITHOUT_TE).b, DtmfWay::Inband);
    }

    #[test]
    fn an_inband_trunk_takes_neither() {
        let t = trunk_taking(TrunkDtmfMode::Disabled);
        assert_eq!(dtmf_carriage(&t, Some(WITH_TE), WITH_TE).b, DtmfWay::Inband);
    }

    #[test]
    fn dtmf_relay_bodies_round_trip() {
        let body = dtmf_relay_body('5', Some(250));
        assert_eq!(body, "Signal=5\r\nDuration=250\r\n");
        assert_eq!(parse_dtmf_relay(&body), Some(('5', 250)));
        assert_eq!(parse_dtmf_relay("Signal=#\r\n"), Some(('#', 160)));
        assert_eq!(
            parse_dtmf_relay("signal = a\nduration = 90"),
            Some(('A', 90))
        );
        assert_eq!(parse_dtmf_relay("Signal=x\r\nDuration=100\r\n"), None);
        assert_eq!(parse_dtmf_relay("Duration=100\r\n"), None);
    }

    fn trunk() -> Trunk {
        Trunk::new("carrier-a".to_string(), TenantId::new())
    }

    #[test]
    fn a_line_is_presented_unless_the_trunk_names_its_own() {
        let mut egress = TrunkEgress::of(&trunk(), Some("sip:alice@pbx.local"), false);
        egress.present_as("sip:+15185550100@pbx.local");
        assert_eq!(egress.from.as_deref(), Some("sip:+15185550100@pbx.local"));
        assert_eq!(
            header(&egress, "P-Asserted-Identity"),
            Some("<sip:+15185550100@pbx.local>")
        );
        let mut t = trunk();
        t.caller_id_override = Some("+15125550100".into());
        t.from_domain = Some("carrier.example".into());
        let mut egress = TrunkEgress::of(&t, Some("sip:alice@pbx.local"), false);
        egress.present_as("sip:+15185550100@pbx.local");
        assert_eq!(
            egress.from.as_deref(),
            Some("sip:+15125550100@carrier.example")
        );
    }

    #[test]
    fn a_withheld_caller_is_anonymous_with_the_identity_asserted() {
        let mut egress = TrunkEgress::of(&trunk(), Some("<sip:alice@pbx.local>;tag=1"), false);
        egress.withhold(Some("<sip:alice@pbx.local>;tag=1"));
        assert_eq!(egress.from.as_deref(), Some(ANONYMOUS_URI));
        assert_eq!(header(&egress, "Privacy"), Some("id"));
        assert_eq!(
            header(&egress, "P-Asserted-Identity"),
            Some("<sip:alice@pbx.local>")
        );
        assert!(egress.anonymous);
        assert!(egress.request_filter(None).is_some(), "the From is renamed");
    }

    fn header<'a>(egress: &'a TrunkEgress, name: &str) -> Option<&'a str> {
        egress
            .headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn a_trunk_that_says_nothing_leaves_the_caller_alone() {
        let egress = TrunkEgress::of(&trunk(), Some("sip:alice@pbx.local"), false);
        assert_eq!(egress.from, None);
        assert!(egress.headers.is_empty());
        assert!(egress.offer_telephone_event, "the default is RFC 2833");
    }

    #[test]
    fn a_caller_id_override_replaces_the_user_and_a_from_domain_the_host() {
        let mut trunk = trunk();
        trunk.caller_id_override = Some("+15125550100".to_string());
        let egress = TrunkEgress::of(&trunk, Some("sip:alice@pbx.local"), false);
        assert_eq!(egress.from.as_deref(), Some("sip:+15125550100@pbx.local"));
        assert_eq!(
            header(&egress, "P-Asserted-Identity"),
            Some("<sip:+15125550100@pbx.local>"),
            "the carrier is told which identity the trunk asserts"
        );

        trunk.from_domain = Some("carrier.example".to_string());
        let egress = TrunkEgress::of(&trunk, Some("sip:alice@pbx.local"), false);
        assert_eq!(
            egress.from.as_deref(),
            Some("sip:+15125550100@carrier.example")
        );

        // A domain alone keeps the caller's own number.
        trunk.caller_id_override = None;
        let egress = TrunkEgress::of(&trunk, Some("sip:alice@pbx.local"), false);
        assert_eq!(egress.from.as_deref(), Some("sip:alice@carrier.example"));
    }

    #[test]
    fn a_caller_id_given_as_a_uri_is_used_whole() {
        let mut trunk = trunk();
        trunk.caller_id_override = Some("sip:+15125550100@billing.example".to_string());
        let egress = TrunkEgress::of(&trunk, Some("<sip:alice@pbx.local>;tag=x"), false);
        assert_eq!(
            egress.from.as_deref(),
            Some("sip:+15125550100@billing.example")
        );
        assert!(egress.from_uri().is_some(), "and it parses as a SIP URI");
    }

    #[test]
    fn an_emergency_call_keeps_its_callback_number() {
        let mut trunk = trunk();
        trunk.caller_id_override = Some("+15125550100".to_string());
        trunk.session_timer_secs = 1800;
        let egress = TrunkEgress::of(&trunk, Some("sip:+15185550199@pbx.local"), true);
        assert_eq!(
            egress.from, None,
            "the location's callback number outranks the trunk's caller ID"
        );
        assert!(
            header(&egress, "P-Asserted-Identity").is_none(),
            "and the emergency identity is asserted by the emergency path"
        );
        assert_eq!(
            header(&egress, "Session-Expires"),
            Some("1800;refresher=uac"),
            "the rest of the trunk still applies"
        );
    }

    #[test]
    fn a_session_timer_is_asked_for_and_never_below_the_minimum() {
        let mut trunk = trunk();
        trunk.session_timer_secs = 1800;
        let egress = TrunkEgress::of(&trunk, None, false);
        assert_eq!(
            header(&egress, "Session-Expires"),
            Some("1800;refresher=uac")
        );
        assert_eq!(header(&egress, "Min-SE"), Some("90"));
        assert_eq!(header(&egress, "Supported"), Some("timer"));

        trunk.session_timer_secs = 30;
        let egress = TrunkEgress::of(&trunk, None, false);
        assert_eq!(
            header(&egress, "Session-Expires"),
            Some("90;refresher=uac"),
            "RFC 4028 has a floor of 90 seconds"
        );

        trunk.session_timer_secs = 0;
        let egress = TrunkEgress::of(&trunk, None, false);
        assert!(
            header(&egress, "Session-Expires").is_none(),
            "a trunk that asks for no timer is sent none"
        );
    }

    /// A signer that says yes to everything, so the test is about what the
    /// trunk asks for rather than about cryptography.
    struct FakeSigner;

    impl TrunkCallSigner for FakeSigner {
        fn identity(
            &self,
            orig_tn: &str,
            dest_tn: &str,
            attestation: fcp_core::trunk::AttestationLevel,
        ) -> Option<String> {
            Some(format!(
                "signed {orig_tn}->{dest_tn} attest={attestation:?}"
            ))
        }
    }

    fn signer() -> std::sync::Arc<dyn TrunkCallSigner> {
        std::sync::Arc::new(FakeSigner)
    }

    #[test]
    fn a_trunk_is_signed_only_when_it_asks_to_be() {
        use fcp_core::trunk::{AttestationLevel, StirSignMode, TrunkStirShakenConfig};

        let mut trunk = trunk();
        let signing = std::sync::Arc::clone(&signer());

        // A trunk that says nothing is not signed.
        let mut egress = TrunkEgress::of(&trunk, None, false);
        egress.sign(&trunk, Some(&signing), "15125550100", "15125559999");
        assert!(header(&egress, "Identity").is_none());

        // One that asks is, over the numbers the carrier will see.
        trunk.stir_shaken = TrunkStirShakenConfig {
            sign_mode: StirSignMode::Enabled,
            attestation: AttestationLevel::B,
            ..Default::default()
        };
        let mut egress = TrunkEgress::of(&trunk, None, false);
        egress.sign(&trunk, Some(&signing), "15125550100", "15125559999");
        assert_eq!(
            header(&egress, "Identity"),
            Some("signed 15125550100->15125559999 attest=B")
        );

        // A node with no signer sends the call unsigned rather than
        // claiming an attestation it cannot make.
        let mut egress = TrunkEgress::of(&trunk, None, false);
        egress.sign(&trunk, None, "15125550100", "15125559999");
        assert!(header(&egress, "Identity").is_none());

        // And a call with no number at one end cannot be signed at all.
        let mut egress = TrunkEgress::of(&trunk, None, false);
        egress.sign(&trunk, Some(&signing), "15125550100", "");
        assert!(header(&egress, "Identity").is_none());
    }

    #[test]
    fn a_trunk_that_takes_dtmf_as_info_is_not_offered_telephone_event() {
        let mut trunk = trunk();
        trunk.dtmf = TrunkDtmfConfig {
            mode: TrunkDtmfMode::Detect,
        };
        assert!(!TrunkEgress::of(&trunk, None, false).offer_telephone_event);

        trunk.dtmf = TrunkDtmfConfig {
            mode: TrunkDtmfMode::Disabled,
        };
        assert!(!TrunkEgress::of(&trunk, None, false).offer_telephone_event);

        trunk.dtmf = TrunkDtmfConfig {
            mode: TrunkDtmfMode::Relay,
        };
        assert!(TrunkEgress::of(&trunk, None, false).offer_telephone_event);
    }
}
