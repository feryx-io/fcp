//! What a trunk does to the call it carries (`docs/TENANT_TRUNKS.md` §6).
//!
//! The route says which trunk a leg goes out on; this says what that trunk
//! makes of the INVITE — the identity it presents, the session timer it
//! asks for, the DTMF it takes. The number transform and the destination
//! are the route's (the router applies both when it builds the target);
//! what is left is everything the B-leg's INVITE carries.

use fcp_core::trunk::TrunkDtmfMode;
use fcp_core::Trunk;
use sip_core::SipUri;

/// RFC 4028 §4: a session interval below 90 seconds is not asked for.
pub const MIN_SESSION_EXPIRES: u32 = 90;

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
        }
    }

    /// Nothing: a leg that no trunk carries.
    pub fn none() -> Self {
        Self {
            from: None,
            headers: Vec::new(),
            offer_telephone_event: true,
        }
    }

    /// The From URI as a parsed SIP URI, for the INVITE's options.
    pub fn from_uri(&self) -> Option<SipUri> {
        self.from.as_deref().and_then(|f| SipUri::parse(f).ok())
    }
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

    fn trunk() -> Trunk {
        Trunk::new("carrier-a".to_string(), TenantId::new())
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
