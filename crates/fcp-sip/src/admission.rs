//! What this node accepts.
//!
//! The call path asks this before it does the work of a call, and before a
//! REGISTER reaches the credential store. The implementation lives in
//! `fcp_sbc_core::admission::AdmissionController`, one controller for every
//! routing binary (`docs/CAPACITY_AND_SURVIVABILITY.md` §5); the interface
//! is here so the SIP layer can call it without depending on the SBC.
//!
//! Emergency calls never reach these checks: the handler skips them, as it
//! does for a trunk's limits (`docs/TENANT_TRUNKS.md` §7).

use fcp_core::TenantId;
use sip_core::Request;
use smol_str::SmolStr;
use std::net::IpAddr;
use std::sync::Arc;

/// Why this node would not take the call or the registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// The node is leaving service and takes no new calls
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    Draining,
    /// More calls started in the last second than the node allows.
    NodeCps,
    /// The node is already carrying as many calls as it allows.
    NodeCalls,
    /// The tenant is already at its `max_concurrent_calls`.
    TenantCalls,
    /// The node holds as many registrations as it allows.
    Registrations,
    /// This source address or this AOR is registering too often.
    RegisterRate,
}

impl Refusal {
    /// The label this refusal is counted under.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draining => "draining",
            Self::NodeCps => "node_cps",
            Self::NodeCalls => "node_calls",
            Self::TenantCalls => "tenant_calls",
            Self::Registrations => "registrations",
            Self::RegisterRate => "register_rate",
        }
    }

    /// What the far end is told, and what the record says.
    pub fn detail(&self) -> &'static str {
        match self {
            Self::Draining => "Node draining",
            Self::NodeCps => "Node over its calls per second",
            Self::NodeCalls => "Node at its concurrent call limit",
            Self::TenantCalls => "Tenant at its concurrent call limit",
            Self::Registrations => "Node at its registration limit",
            Self::RegisterRate => "Registering too often",
        }
    }
}

/// The limits this node keeps, and the counts behind them.
pub trait NodeAdmission: Send + Sync {
    /// Take a place for a call that is starting. Released by [`ended`] with
    /// the same `call_id`, however the call ends.
    ///
    /// [`ended`]: NodeAdmission::ended
    fn admit(&self, call_id: &str, tenant: &TenantId) -> Result<(), Refusal>;

    /// Give back the place a call held. Unknown ids are ignored, so this is
    /// safe to call for a call that was never admitted.
    fn ended(&self, call_id: &str);

    /// A REGISTER arriving from `source` for `aor`, before any credential
    /// work, so a flood cannot reach the store.
    fn admit_register(&self, source: IpAddr, aor: &str) -> Result<(), Refusal>;

    /// What to put in `Retry-After` when refusing.
    fn retry_after_secs(&self) -> u64 {
        5
    }

    /// Whether this node is leaving service. A draining node refuses new
    /// calls — emergency calls excepted, which never reach admission —
    /// and keeps the calls it has until its deadline.
    fn draining(&self) -> bool {
        false
    }

    /// Start or stop refusing new calls, which is what a drain does first
    /// and what calling one off undoes ([`crate::drain`]). A node with no
    /// admission of its own has nothing to refuse with.
    fn set_draining(&self, _draining: bool) {}

    /// The longest registration a draining node grants, in seconds, so a
    /// device comes back soon and is placed on a node that is serving.
    /// `None` while the node is serving, and for a deregistration.
    fn register_expiry_cap(&self) -> Option<u32> {
        None
    }
}

/// What this node does with a REGISTER before any credential work: refuse
/// a flood, shorten what a draining node grants, or let it through as it
/// arrived (`docs/CAPACITY_AND_SURVIVABILITY.md` §5, §6).
///
/// Every registrar on this platform runs this one gate — the call
/// manager's handler and the standalone registrar alike — so a device sees
/// the same thing wherever it registers.
pub enum RegisterGate {
    /// Let it through as it arrived.
    Pass,
    /// Let it through, but ask for no more than a draining node grants.
    Shortened(Box<Request>),
    /// Refuse it: `503`, with this many seconds in `Retry-After`.
    Refuse {
        refusal: Refusal,
        retry_after_secs: u64,
    },
}

/// Run the gate. A node with no admission of its own passes everything.
pub fn register_gate(
    admission: Option<&Arc<dyn NodeAdmission>>,
    request: &Request,
    source: IpAddr,
    aor: &str,
) -> RegisterGate {
    let Some(admission) = admission else {
        return RegisterGate::Pass;
    };
    if let Err(refusal) = admission.admit_register(source, aor) {
        return RegisterGate::Refuse {
            refusal,
            retry_after_secs: admission.retry_after_secs(),
        };
    }
    // A draining node keeps its devices, but not for long: it grants a
    // short registration so each one comes back soon and is placed on a
    // node that is serving. A deregistration is left exactly as it
    // arrived.
    match admission
        .register_expiry_cap()
        .and_then(|cap| cap_register_expiry(request, cap))
    {
        Some(shortened) => RegisterGate::Shortened(Box::new(shortened)),
        None => RegisterGate::Pass,
    }
}

/// A REGISTER with its expiry shortened to `cap` seconds, or `None` when
/// it already asks for no more than that — or asks to deregister, which a
/// draining node answers exactly as it arrived
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
///
/// Both places a device may ask are shortened: the `Expires` header and
/// each Contact's `expires` parameter. A request that names neither gets
/// an `Expires` header, since the registrar's own default may be longer.
pub fn cap_register_expiry(request: &sip_core::Request, cap: u32) -> Option<sip_core::Request> {
    let header_expiry = sip_parse::header(request.headers(), "Expires")
        .and_then(|v| v.as_str().trim().parse::<u32>().ok());
    // A deregistration: `Expires: 0`, or a wildcard contact.
    if header_expiry == Some(0)
        || request
            .headers()
            .get_all("Contact")
            .any(|c| c.trim() == "*" || contact_expires(c) == Some(0))
    {
        return None;
    }

    let contacts: Vec<String> = request
        .headers()
        .get_all("Contact")
        .map(|c| c.to_string())
        .collect();
    // A REGISTER with no Contact asks what this node holds; it grants
    // nothing, so there is nothing to shorten.
    if contacts.is_empty() {
        return None;
    }
    let contact_over_cap = contacts
        .iter()
        .any(|c| contact_expires(c).is_some_and(|secs| secs > cap));
    // A contact of its own binds for its own `expires` (RFC 3261 §10.2.4),
    // so the header matters only to a contact that names none — and then
    // the registrar's default, which may be longer, is what the header
    // has to beat.
    let header_over_cap = match header_expiry {
        Some(secs) => secs > cap,
        None => contacts.iter().any(|c| contact_expires(c).is_none()),
    };
    if !header_over_cap && !contact_over_cap {
        return None;
    }

    let mut shortened = request.clone();
    let headers = shortened.headers_mut();
    if header_over_cap {
        let _ = headers.set_or_push(SmolStr::new("Expires"), SmolStr::new(cap.to_string()));
    }
    if contact_over_cap {
        headers.remove("Contact");
        for contact in contacts {
            let capped = match contact_expires(&contact) {
                Some(secs) if secs > cap => {
                    let parts: Vec<String> = contact
                        .split(';')
                        .map(|part| {
                            if part.trim().to_ascii_lowercase().starts_with("expires=") {
                                format!("expires={cap}")
                            } else {
                                part.to_string()
                            }
                        })
                        .collect();
                    parts.join(";")
                }
                _ => contact,
            };
            let _ = headers.push(SmolStr::new("Contact"), SmolStr::new(capped));
        }
    }
    Some(shortened)
}

/// The `expires` parameter of one Contact header value, when it has one.
fn contact_expires(contact: &str) -> Option<u32> {
    contact.split(';').find_map(|part| {
        let part = part.trim();
        part.get(..8)
            .filter(|prefix| prefix.eq_ignore_ascii_case("expires="))
            .and_then(|_| part[8..].trim().parse::<u32>().ok())
    })
}

/// What a draining node does to the registration a device asks for
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
#[cfg(test)]
mod register_expiry_tests {
    use super::*;

    fn register(headers: &[(&str, &str)]) -> sip_core::Request {
        let mut raw = String::from("REGISTER sip:example.com SIP/2.0\r\n");
        raw.push_str("Via: SIP/2.0/UDP 198.51.100.5:5060;branch=z9hG4bK1\r\n");
        raw.push_str("From: <sip:alice@example.com>;tag=a\r\n");
        raw.push_str("To: <sip:alice@example.com>\r\n");
        raw.push_str("Call-ID: reg-1\r\n");
        raw.push_str("CSeq: 1 REGISTER\r\n");
        for (name, value) in headers {
            raw.push_str(&format!("{name}: {value}\r\n"));
        }
        raw.push_str("Content-Length: 0\r\n\r\n");
        sip_parse::parse_request(&bytes::Bytes::from(raw)).expect("a REGISTER")
    }

    /// An hour becomes a minute, in the header and in the contact.
    #[test]
    fn a_long_registration_is_shortened() {
        let request = register(&[
            ("Contact", "<sip:alice@198.51.100.5:5060>;expires=3600"),
            ("Expires", "3600"),
        ]);
        let capped = cap_register_expiry(&request, 60).expect("shortened");
        assert_eq!(capped.headers().get("Expires"), Some("60"));
        assert_eq!(
            capped.headers().get("Contact"),
            Some("<sip:alice@198.51.100.5:5060>;expires=60")
        );
    }

    /// A device that asks for less than the cap is left alone, and so is
    /// one that asks for exactly it.
    #[test]
    fn a_short_registration_is_left_alone() {
        assert!(cap_register_expiry(&register(&[("Expires", "30")]), 60).is_none());
        assert!(cap_register_expiry(&register(&[("Expires", "60")]), 60).is_none());
        assert!(cap_register_expiry(
            &register(&[("Contact", "<sip:alice@198.51.100.5:5060>;expires=45")]),
            60
        )
        .is_none());
    }

    /// A request that names no expiry at all gets one: the registrar's
    /// own default may be longer than the cap.
    #[test]
    fn a_registration_that_names_no_expiry_gets_the_cap() {
        let request = register(&[("Contact", "<sip:alice@198.51.100.5:5060>")]);
        let capped = cap_register_expiry(&request, 60).expect("shortened");
        assert_eq!(capped.headers().get("Expires"), Some("60"));
    }

    /// A deregistration is answered exactly as it arrived, however it is
    /// written: a device leaving must not be given a registration.
    #[test]
    fn a_deregistration_is_untouched() {
        assert!(cap_register_expiry(
            &register(&[
                ("Contact", "<sip:alice@198.51.100.5:5060>"),
                ("Expires", "0")
            ]),
            60
        )
        .is_none());
        assert!(cap_register_expiry(
            &register(&[("Contact", "<sip:alice@198.51.100.5:5060>;expires=0")]),
            60
        )
        .is_none());
        assert!(
            cap_register_expiry(&register(&[("Contact", "*"), ("Expires", "0")]), 60).is_none()
        );
    }

    /// Several contacts: each one's own expiry is shortened, and the
    /// others are kept as they were.
    #[test]
    fn every_contact_is_shortened() {
        let request = register(&[
            ("Contact", "<sip:alice@198.51.100.5:5060>;expires=3600"),
            (
                "Contact",
                "<sip:alice@198.51.100.6:5060>;q=0.5;expires=1800",
            ),
            ("Expires", "3600"),
        ]);
        let capped = cap_register_expiry(&request, 60).expect("shortened");
        let contacts: Vec<&str> = capped.headers().get_all("Contact").collect();
        assert_eq!(
            contacts,
            vec![
                "<sip:alice@198.51.100.5:5060>;expires=60",
                "<sip:alice@198.51.100.6:5060>;q=0.5;expires=60",
            ]
        );
    }

    /// The gate every registrar runs: a flood is refused, a draining node
    /// shortens what it grants, and a node with no limits passes
    /// everything through untouched.
    mod gate {
        use super::*;
        use std::sync::atomic::{AtomicBool, Ordering};

        struct Node {
            draining: AtomicBool,
            refuse: bool,
        }

        impl NodeAdmission for Node {
            fn admit(&self, _call_id: &str, _tenant: &TenantId) -> Result<(), Refusal> {
                Ok(())
            }
            fn ended(&self, _call_id: &str) {}
            fn admit_register(&self, _source: IpAddr, _aor: &str) -> Result<(), Refusal> {
                if self.refuse {
                    return Err(Refusal::RegisterRate);
                }
                Ok(())
            }
            fn retry_after_secs(&self) -> u64 {
                7
            }
            fn draining(&self) -> bool {
                self.draining.load(Ordering::SeqCst)
            }
            fn set_draining(&self, draining: bool) {
                self.draining.store(draining, Ordering::SeqCst);
            }
            fn register_expiry_cap(&self) -> Option<u32> {
                self.draining().then_some(60)
            }
        }

        fn node(refuse: bool, draining: bool) -> Arc<dyn NodeAdmission> {
            Arc::new(Node {
                draining: AtomicBool::new(draining),
                refuse,
            })
        }

        fn source() -> IpAddr {
            "198.51.100.5".parse().unwrap()
        }

        #[test]
        fn a_node_with_no_admission_passes_everything() {
            let request = register(&[("Contact", "<sip:alice@198.51.100.5:5060>;expires=3600")]);
            assert!(matches!(
                register_gate(None, &request, source(), "sip:alice@example.com"),
                RegisterGate::Pass
            ));
        }

        #[test]
        fn a_flood_is_refused_with_retry_after() {
            let request = register(&[("Contact", "<sip:alice@198.51.100.5:5060>")]);
            let admission = node(true, false);
            match register_gate(
                Some(&admission),
                &request,
                source(),
                "sip:alice@example.com",
            ) {
                RegisterGate::Refuse {
                    refusal,
                    retry_after_secs,
                } => {
                    assert_eq!(refusal, Refusal::RegisterRate);
                    assert_eq!(retry_after_secs, 7, "the node's own Retry-After");
                }
                _ => panic!("a flood must be refused"),
            }
        }

        #[test]
        fn a_draining_node_shortens_what_it_grants() {
            let request = register(&[
                ("Contact", "<sip:alice@198.51.100.5:5060>;expires=3600"),
                ("Expires", "3600"),
            ]);
            let admission = node(false, true);
            match register_gate(
                Some(&admission),
                &request,
                source(),
                "sip:alice@example.com",
            ) {
                RegisterGate::Shortened(capped) => {
                    assert_eq!(capped.headers().get("Expires"), Some("60"));
                }
                _ => panic!("a draining node grants a minute at most"),
            }
        }

        #[test]
        fn a_serving_node_leaves_the_request_as_it_arrived() {
            let request = register(&[("Contact", "<sip:alice@198.51.100.5:5060>;expires=3600")]);
            let admission = node(false, false);
            assert!(matches!(
                register_gate(
                    Some(&admission),
                    &request,
                    source(),
                    "sip:alice@example.com"
                ),
                RegisterGate::Pass
            ));
        }
    }
}
