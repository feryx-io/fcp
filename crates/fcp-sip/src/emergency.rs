//! One emergency path for every way a call is placed
//! (`docs/E911_BLOCK_2.md` §4).
//!
//! The B2BUA's INVITE path and a flow's `Dial` node classify the dialed
//! number, resolve the caller's location, put the callback number and
//! `Priority: emergency` on the outbound INVITE and write the record through
//! these functions, so neither can place an emergency call the other would
//! have handled differently.
//!
//! A trusted edge — the WebRTC gateway in front of browsers — names the
//! tenant user it places a call for in [`CALLER_TENANT_HEADER`] and
//! [`CALLER_USER_HEADER`]. The call manager believes those headers only from
//! the addresses in `[e911] trusted_edges`; from anyone else they are
//! ignored, and nothing forwards them.

use std::net::IpAddr;
use std::sync::Arc;

use fcp_b2bua_core::EmergencyCallInfo;
use fcp_core::e911::{
    dialed_user_part, resolve_for_user, EmergencyCallRecord, EmergencyConfig, LocationDelivery,
};
use fcp_core::storage::CredentialStore;
use fcp_core::{SiteId, TenantId};
use ipnetwork::IpNetwork;
use smol_str::SmolStr;
use tracing::{debug, warn};

pub use fcp_core::{
    CALLER_SITE_HEADER, CALLER_TENANT_HEADER, CALLER_USER_HEADER, CLIENT_ADDRESS_HEADER,
};

/// The caller a trusted edge vouched for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeCaller {
    pub tenant_id: TenantId,
    /// The tenant's user the call is placed for; none for a site's call,
    /// which a phone the site already authenticated placed.
    pub username: Option<String>,
    /// The site whose link the call came up, when the edge is the cloud
    /// SBC (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
    pub site_id: Option<SiteId>,
}

impl EdgeCaller {
    /// Whether this is a site's call: the tenant's own, with no user of
    /// the cloud's to check it against.
    pub fn is_site(&self) -> bool {
        self.site_id.is_some() && self.username.is_none()
    }
}

/// The addresses whose caller headers are believed.
#[derive(Debug, Clone, Default)]
pub struct TrustedEdges(Vec<IpNetwork>);

impl TrustedEdges {
    /// Parse addresses and CIDR ranges (`10.0.0.7`, `10.1.0.0/24`, `::1`).
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> anyhow::Result<Self> {
        entries
            .iter()
            .map(|e| {
                let e = e.as_ref().trim();
                e.parse::<IpNetwork>()
                    .or_else(|_| e.parse::<IpAddr>().map(IpNetwork::from))
                    .map_err(|err| anyhow::anyhow!("trusted edge {e:?}: {err}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .map(Self)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        // An IPv4 peer on a dual-stack socket arrives mapped into IPv6.
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
            v4 => v4,
        };
        self.0.iter().any(|net| net.contains(ip))
    }

    /// The address a trusted edge says its client is at
    /// (`X-FCP-Client-Address`, `docs/E911_BLOCK_2.md` §6.2). From a peer
    /// that is not a trusted edge the header is ignored, and so is the
    /// peer's own address: an edge that vouches for nobody places nobody.
    pub fn client_address(&self, request: &sip_core::Request, peer: IpAddr) -> Option<IpAddr> {
        let vouched = sip_parse::header(request.headers(), CLIENT_ADDRESS_HEADER);
        if !self.contains(peer) {
            if vouched.is_some() {
                warn!(peer = %peer, "Ignoring a client address from a peer that is not a trusted edge");
            }
            // The call came from the caller's own device.
            return Some(peer);
        }
        match vouched.and_then(|v| parse_client_address(v.as_str())) {
            Some(address) => Some(address),
            None => {
                debug!(peer = %peer, "A trusted edge named no client address; the caller is placed by other means");
                None
            }
        }
    }

    /// The caller an INVITE from `peer` was placed for: `None` when the
    /// request names none, and when `peer` is not a trusted edge (logged,
    /// since an untrusted peer naming a caller is either misconfigured or
    /// lying).
    pub fn caller(&self, request: &sip_core::Request, peer: IpAddr) -> Option<EdgeCaller> {
        let tenant = sip_parse::header(request.headers(), CALLER_TENANT_HEADER);
        let user = sip_parse::header(request.headers(), CALLER_USER_HEADER);
        let site = sip_parse::header(request.headers(), CALLER_SITE_HEADER);
        if tenant.is_none() && user.is_none() && site.is_none() {
            return None;
        }
        if !self.contains(peer) {
            warn!(peer = %peer, "Ignoring caller headers from a peer that is not a trusted edge");
            return None;
        }
        // A user, or a site (`docs/HYBRID_CONTROL_PLANE.md` §3.6), with
        // the tenant either way.
        let (Some(tenant), true) = (tenant, user.is_some() || site.is_some()) else {
            warn!(peer = %peer, "Ignoring incomplete caller headers from a trusted edge");
            return None;
        };
        let Ok(tenant_id) = tenant.as_str().trim().parse::<TenantId>() else {
            warn!(peer = %peer, tenant = %tenant, "Ignoring caller headers naming an unparsable tenant");
            return None;
        };
        let username = user
            .map(|u| u.as_str().trim().to_string())
            .filter(|u| !u.is_empty());
        let site_id = match site {
            Some(site) => match site.as_str().trim().parse::<SiteId>() {
                Ok(id) => Some(id),
                Err(_) => {
                    warn!(peer = %peer, site = %site, "Ignoring caller headers naming an unparsable site");
                    return None;
                }
            },
            None => None,
        };
        if username.is_none() && site_id.is_none() {
            return None;
        }
        Some(EdgeCaller {
            tenant_id,
            username,
            site_id,
        })
    }
}

/// An address, or an `address:port`, as a client address.
fn parse_client_address(value: &str) -> Option<IpAddr> {
    fcp_core::e911::parse_address(value)
}

/// The location a device sent with its INVITE, if it sent one this
/// platform can read (`docs/E911_BLOCK_2.md` §6.3). Whether it is used is
/// decided by what the tenant trusts the device to send.
pub fn supplied_location(request: &sip_core::Request) -> Option<fcp_core::E911Address> {
    let parts = crate::body::non_sdp_parts(request.headers(), request.body());
    let pidf = parts
        .iter()
        .find(|p| p.content_type == fcp_core::pidf_lo::CONTENT_TYPE)?;
    let xml = String::from_utf8_lossy(&pidf.body);
    let parsed = fcp_core::pidf_lo::parse(&xml)?;
    parsed.address.filter(|a| !a.address1.trim().is_empty())
}

/// Where a site's emergency call is: the location the site sent, with the
/// callback number it sent as the caller — a site places an emergency call
/// as its ELIN (`outbound_identity`). `None` when the site sent none.
pub fn site_location(
    supplied: Option<fcp_core::E911Address>,
    request: &sip_core::Request,
    client_address: Option<IpAddr>,
) -> Option<fcp_core::e911::DispatchableLocation> {
    let address = supplied?;
    let callback_number = sip_parse::header(request.headers(), "P-Asserted-Identity")
        .or_else(|| sip_parse::header(request.headers(), "From"))
        .and_then(|value| {
            let value = value.as_str();
            let uri = value
                .split('<')
                .nth(1)
                .and_then(|s| s.split('>').next())
                .unwrap_or(value);
            let user = uri.split(':').nth(1)?.split(['@', ';']).next()?.to_string();
            let digits = user.trim_start_matches('+');
            (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())).then_some(user)
        });
    Some(fcp_core::e911::DispatchableLocation {
        address,
        callback_number,
        source: fcp_core::e911::LocationSource::SiteSupplied,
        location_id: None,
        location_name: None,
        point: None,
        client_address: client_address.map(|a| a.to_string()),
        declared_age: None,
    })
}

/// The tenant's emergency configuration, or the US defaults.
pub async fn config_for(
    storage: Option<&Arc<fcp_core::DynStorage>>,
    tenant_id: &TenantId,
) -> EmergencyConfig {
    let stored = match storage.and_then(|s| s.emergency_store()) {
        Some(store) => store
            .get_emergency_config(tenant_id)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, "Failed to load emergency config; using defaults");
                None
            }),
        None => None,
    };
    stored.unwrap_or_else(|| EmergencyConfig::defaults(*tenant_id))
}

/// Classify `destination` (a URI or a bare number) for the tenant and, when
/// it is an emergency number, resolve the caller's dispatchable location
/// and callback number. `caller_aor` (`user@host`) picks the caller's
/// device. Never fails: a call with no location still goes out.
#[allow(clippy::too_many_arguments)]
pub async fn detect(
    storage: Option<&Arc<fcp_core::DynStorage>>,
    tenant_id: &TenantId,
    destination: &str,
    caller_username: Option<&str>,
    caller_aor: Option<&str>,
    client_address: Option<std::net::IpAddr>,
    supplied: Option<&fcp_core::E911Address>,
) -> Option<EmergencyCallInfo> {
    let config = config_for(storage, tenant_id).await;
    let kind = config.classify(destination)?;
    let mut info = EmergencyCallInfo {
        dialed: dialed_user_part(destination).to_string(),
        kind,
        callback_number: None,
        location: None,
        leg: None,
        delivered: Vec::new(),
        geolocation_error: None,
    };
    let Some(storage) = storage else {
        return Some(info);
    };
    let username = caller_username.unwrap_or_default();
    let user = if username.is_empty() {
        None
    } else {
        storage
            .get_user_by_sip_name(username, tenant_id)
            .await
            .ok()
            .flatten()
    };
    if let Some(user) = &user {
        // Where the caller was seen: the address a trusted edge vouched for
        // or the INVITE's own source, else where their device last
        // registered from (§6.2).
        info.location = resolve_for_user(
            storage.as_ref(),
            user,
            fcp_core::e911::CallerContext {
                prefer_device_aor: caller_aor,
                client_address,
                supplied,
                config: Some(&config),
            },
        )
        .await;
    }
    info.callback_number = info
        .location
        .as_ref()
        .and_then(|l| l.callback_number.clone());
    if info.location.is_none() {
        warn!(
            tenant_id = %tenant_id,
            caller = %username,
            dialed = %info.dialed,
            "Emergency call without a dispatchable location"
        );
    }
    Some(info)
}

/// The From URI and extra headers an emergency call goes out with: the
/// callback number as the caller (so the PSAP's lookup and callback both
/// work) and in `P-Asserted-Identity`, and `Priority: emergency`. `caller`
/// is the From the call would otherwise carry; `host` names the host the
/// callback number is written under when there is no caller.
pub fn outbound_identity(
    info: &EmergencyCallInfo,
    caller: Option<sip_core::SipUri>,
    host: &str,
) -> (Option<sip_core::SipUri>, sip_core::Headers) {
    let host = caller
        .as_ref()
        .map(|u| u.host().to_string())
        .unwrap_or_else(|| host.to_string());
    let from = match &info.callback_number {
        Some(cb) => sip_core::SipUri::parse(&format!("sip:{cb}@{host}"))
            .ok()
            .or(caller),
        None => caller,
    };
    let mut headers = sip_core::Headers::new();
    if let Some(cb) = &info.callback_number {
        let _ = headers.push(
            SmolStr::new("P-Asserted-Identity"),
            SmolStr::new(format!("<sip:{cb}@{host}>")),
        );
    }
    let _ = headers.push(SmolStr::new("Priority"), SmolStr::new("emergency"));
    (from, headers)
}

/// One emergency route to try and how the location goes on it
/// (`docs/E911_BLOCK_2.md` §5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyAttempt {
    pub target: String,
    pub trunk: Option<String>,
    /// Which trunk that is (`docs/TENANT_TRUNKS.md` §4).
    pub trunk_id: Option<fcp_core::TrunkId>,
    pub delivery: Vec<LocationDelivery>,
}

impl EmergencyAttempt {
    pub fn from_route(route: &crate::handler::OutboundRoute) -> Self {
        let delivery = if route.emergency_location().is_empty() {
            vec![LocationDelivery::Elin]
        } else {
            route.emergency_location()
        };
        Self {
            target: route.target_uri.clone(),
            trunk: route.trunk_name(),
            trunk_id: route.trunk_id(),
            delivery,
        }
    }

    /// The same route with the location by value dropped, when the trunk
    /// also takes ELIN: what follows a trunk refusing the location
    /// (§12.9). A trunk that takes only `pidf_lo` gives way to the next
    /// route instead.
    pub fn without_pidf_lo(&self) -> Option<Self> {
        (self.delivery.contains(&LocationDelivery::PidfLo)
            && self.delivery.contains(&LocationDelivery::Elin))
        .then(|| Self {
            delivery: vec![LocationDelivery::Elin],
            ..self.clone()
        })
    }

    /// This attempt carrying the location by value too: a site's call
    /// going on to the cloud's call manager, which places it by the
    /// location (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
    pub fn with_pidf_lo(mut self) -> Self {
        if !self.delivery.contains(&LocationDelivery::PidfLo) {
            self.delivery.push(LocationDelivery::PidfLo);
        }
        self
    }
}

/// The INVITE options one attempt goes out with, and the delivery methods
/// it actually carries.
///
/// Every attempt carries `sdp`, `from` (the callback number as caller) and
/// `headers` (`P-Asserted-Identity`, `Priority`). An attempt whose trunk
/// takes `pidf_lo`, for a call with a location, also carries a
/// `multipart/mixed` body — the SDP and the PIDF-LO, named by `Content-ID`
/// — a `Geolocation: <cid:…>` header and `Geolocation-Routing: yes`
/// (RFC 6442).
pub fn attempt_invite(
    info: &EmergencyCallInfo,
    attempt: &EmergencyAttempt,
    call_id: fcp_b2bua_core::CallId,
    sdp: Option<&str>,
    from: Option<sip_core::SipUri>,
    headers: Option<&sip_core::Headers>,
    host: &str,
) -> (sip_uac::InviteOptions, Vec<LocationDelivery>) {
    let mut extra = headers.cloned().unwrap_or_default();
    let mut delivered = Vec::new();
    if attempt.delivery.contains(&LocationDelivery::Elin) && info.callback_number.is_some() {
        delivered.push(LocationDelivery::Elin);
    }
    let host = from
        .as_ref()
        .map(|u| u.host().to_string())
        .unwrap_or_else(|| host.to_string());
    let pidf = match (
        &info.location,
        attempt.delivery.contains(&LocationDelivery::PidfLo),
    ) {
        (Some(location), true) => {
            let caller = info
                .callback_number
                .clone()
                .or_else(|| from.as_ref().and_then(|u| u.user().map(str::to_string)))
                .unwrap_or_else(|| "emergency".to_string());
            let entity = format!("pres:{caller}@{host}");
            let device_id = format!("urn:uuid:{}", call_id.as_uuid());
            Some(fcp_core::pidf_lo::document(
                location,
                &fcp_core::pidf_lo::Subject {
                    entity: &entity,
                    device_id: &device_id,
                    timestamp: std::time::SystemTime::now(),
                },
            ))
        }
        (None, true) => {
            warn!(call_id = %call_id, "Trunk takes the location by value, but the call has none");
            None
        }
        _ => None,
    };

    let body = match pidf {
        Some(document) => {
            let content_id = format!("{}.location@{}", call_id.as_uuid(), host);
            match located_body(sdp, document, &content_id) {
                Ok((body, geolocation)) => {
                    let _ = extra.push("Geolocation", geolocation);
                    let _ = extra.push(
                        "Geolocation-Routing",
                        sip_core::GeolocationRoutingHeader::allowed(true).to_string(),
                    );
                    delivered.push(LocationDelivery::PidfLo);
                    Some(body)
                }
                Err(e) => {
                    warn!(call_id = %call_id, error = %e, "Could not build the location body; sending without it");
                    sdp.map(sip_uac::InviteBody::sdp)
                }
            }
        }
        None => sdp.map(sip_uac::InviteBody::sdp),
    };
    (
        sip_uac::InviteOptions {
            body,
            extra_headers: extra,
            from,
            // An emergency INVITE is not rewritten by an operator's rules.
            filter: None,
            // A trunk's address, its name kept in the Request-URI.
            destination: crate::trunk_egress::take_destination(&attempt.target).1,
        },
        delivered,
    )
}

/// A `multipart/mixed` body of `sdp` and the PIDF-LO `document` under
/// `content_id`, and the `Geolocation` header value naming it (RFC 6442).
pub fn located_body(
    sdp: Option<&str>,
    document: String,
    content_id: &str,
) -> Result<(sip_uac::InviteBody, String), sip_core::MultipartError> {
    let pidf_part = sip_core::BodyPart::new(fcp_core::pidf_lo::CONTENT_TYPE, document)?
        .with_content_id(content_id)?;
    let mut multipart = sip_core::MultipartBody::mixed();
    if let Some(sdp) = sdp {
        multipart.push(sip_core::BodyPart::new("application/sdp", sdp.to_string())?)?;
    }
    multipart.push(pidf_part)?;
    let uri = sip_core::Uri::parse(&format!("cid:{content_id}"))
        .map_err(|e| sip_core::MultipartError::InvalidHeader(e.to_string()))?;
    let geolocation = sip_core::GeolocationHeader::single(sip_core::GeolocationValue::new(uri));
    Ok((
        sip_uac::InviteBody::multipart(&multipart),
        geolocation.to_string(),
    ))
}

/// What a response said about the location it was sent: its
/// `Geolocation-Error` header, or for a 424 (Bad Location Information,
/// RFC 6442 §4.3) without one, the status. `None` when it said nothing.
pub fn geolocation_error_of(response: &sip_core::Response) -> Option<String> {
    let header = sip_parse::header(response.headers(), "Geolocation-Error")
        .map(|v| v.as_str().trim().to_string())
        .filter(|v| !v.is_empty());
    match (header, response.start_line().code()) {
        (Some(value), _) => Some(value),
        (None, 424) => Some("424 Bad Location Information".to_string()),
        _ => None,
    }
}

/// The audit record of an emergency call as it is placed.
pub fn placement_record(
    call_id: fcp_b2bua_core::CallId,
    tenant_id: TenantId,
    sip_call_id: Option<String>,
    caller_uri: String,
    caller_username: Option<String>,
    info: &EmergencyCallInfo,
) -> EmergencyCallRecord {
    EmergencyCallRecord {
        id: 0,
        tenant_id,
        call_id: call_id.to_string(),
        sip_call_id,
        caller_uri,
        caller_username,
        dialed_number: info.dialed.clone(),
        kind: info.kind,
        callback_number: info.callback_number.clone(),
        location: info.location.clone(),
        trunk_name: None,
        trunk_id: None,
        outcome: "placed".to_string(),
        placed_at: std::time::SystemTime::now(),
        notified: false,
        sip_status: None,
        answered_at: None,
        ended_at: None,
        delivery: info.delivery(),
        location_structured: info
            .location
            .as_ref()
            .map(|l| l.address.civic.is_structured()),
        geolocation_error: None,
    }
}

/// Publish `call.emergency` for a call just placed and write its record;
/// the call manager completes the record when the call ends.
pub async fn announce(
    call_manager: &fcp_b2bua_core::CallManager,
    call_id: fcp_b2bua_core::CallId,
    record: EmergencyCallRecord,
) {
    // The event carries the plain targets it always has; the notifier
    // reads the typed contacts from the tenant's configuration (§8.1).
    let notify_contacts = config_for(call_manager.storage(), &record.tenant_id)
        .await
        .contact_targets();
    if let Some(call) = call_manager.get_call(&call_id) {
        call_manager.publish_emergency(&call, notify_contacts);
    }
    call_manager.record_emergency(call_id, record);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite(headers: &[(&str, &str)]) -> sip_core::Request {
        let mut h = sip_core::Headers::new();
        for (k, v) in headers {
            h.push(SmolStr::new(*k), SmolStr::new(*v)).unwrap();
        }
        sip_core::Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Invite,
                sip_core::SipUri::parse("sip:911@10.0.0.1").unwrap(),
            ),
            h,
            bytes::Bytes::new(),
        )
        .unwrap()
    }

    #[test]
    fn caller_headers_are_believed_only_from_trusted_edges() {
        let tenant = TenantId::new();
        let edges = TrustedEdges::parse(&["10.0.0.7", "10.1.0.0/24"]).unwrap();
        let tenant_str = tenant.to_string();
        let named = invite(&[
            (CALLER_TENANT_HEADER, tenant_str.as_str()),
            (CALLER_USER_HEADER, "alice"),
        ]);
        let expected = Some(EdgeCaller {
            tenant_id: tenant,
            username: Some("alice".into()),
            site_id: None,
        });
        assert_eq!(edges.caller(&named, "10.0.0.7".parse().unwrap()), expected);
        assert_eq!(
            edges.caller(&named, "10.1.0.200".parse().unwrap()),
            expected
        );
        assert_eq!(
            edges.caller(&named, "::ffff:10.0.0.7".parse().unwrap()),
            expected,
            "a mapped IPv4 peer is the IPv4 peer"
        );
        assert_eq!(edges.caller(&named, "10.0.0.8".parse().unwrap()), None);
        assert_eq!(
            TrustedEdges::default().caller(&named, "127.0.0.1".parse().unwrap()),
            None,
            "no edge is trusted unless configured"
        );
        let half = invite(&[(CALLER_USER_HEADER, "alice")]);
        assert_eq!(edges.caller(&half, "10.0.0.7".parse().unwrap()), None);
        let garbage = invite(&[
            (CALLER_TENANT_HEADER, "not-a-tenant"),
            (CALLER_USER_HEADER, "alice"),
        ]);
        assert_eq!(edges.caller(&garbage, "10.0.0.7".parse().unwrap()), None);
        assert!(TrustedEdges::parse(&["nonsense"]).is_err());
    }

    #[test]
    fn an_emergency_call_goes_out_as_its_callback_number() {
        let info = EmergencyCallInfo {
            dialed: "911".into(),
            kind: fcp_core::e911::EmergencyKind::Live,
            callback_number: Some("+15185550100".into()),
            location: None,
            leg: None,
            delivered: Vec::new(),
            geolocation_error: None,
        };
        let caller = sip_core::SipUri::parse("sip:alice@pbx.example").ok();
        let (from, headers) = outbound_identity(&info, caller, "10.0.0.1");
        assert_eq!(from.unwrap().as_str(), "sip:+15185550100@pbx.example");
        assert_eq!(
            sip_parse::header(&headers, "P-Asserted-Identity").map(|v| v.to_string()),
            Some("<sip:+15185550100@pbx.example>".into())
        );
        assert_eq!(
            sip_parse::header(&headers, "Priority").map(|v| v.to_string()),
            Some("emergency".into())
        );

        let unlocated = EmergencyCallInfo {
            callback_number: None,
            ..info
        };
        let (from, headers) = outbound_identity(&unlocated, None, "10.0.0.1");
        assert!(from.is_none());
        assert!(sip_parse::header(&headers, "P-Asserted-Identity").is_none());
        assert!(sip_parse::header(&headers, "Priority").is_some());
    }

    fn located_info() -> EmergencyCallInfo {
        let mut site = fcp_core::SiteLocation::new("HQ".into(), TenantId::nil());
        site.address_line1 = Some("100 State St".into());
        site.city = Some("Albany".into());
        site.state = Some("NY".into());
        site.postal_code = Some("12207".into());
        EmergencyCallInfo {
            dialed: "911".into(),
            kind: fcp_core::e911::EmergencyKind::Live,
            callback_number: Some("+15185550100".into()),
            location: Some(fcp_core::e911::DispatchableLocation {
                address: fcp_core::e911::site_address(&site).unwrap(),
                callback_number: Some("+15185550100".into()),
                source: fcp_core::e911::LocationSource::UserLocation,
                location_id: Some(site.id),
                location_name: Some(site.name.clone()),
                point: None,
                client_address: None,
                declared_age: None,
            }),
            leg: None,
            delivered: Vec::new(),
            geolocation_error: None,
        }
    }

    fn attempt(delivery: &[LocationDelivery]) -> EmergencyAttempt {
        EmergencyAttempt {
            target: "sip:911@10.0.0.9".into(),
            trunk: Some("psap".into()),
            trunk_id: None,
            delivery: delivery.to_vec(),
        }
    }

    #[test]
    fn an_attempt_carries_the_location_its_trunk_takes() {
        use LocationDelivery::{Elin, PidfLo};
        let info = located_info();
        let call_id = fcp_b2bua_core::CallId::new();
        let from = sip_core::SipUri::parse("sip:+15185550100@pbx.example").ok();
        let sdp = "v=0\r\n";

        let (options, delivered) = attempt_invite(
            &info,
            &attempt(&[Elin]),
            call_id,
            Some(sdp),
            from.clone(),
            None,
            "10.0.0.1",
        );
        assert_eq!(delivered, [Elin]);
        assert_eq!(options.body.unwrap().content_type, "application/sdp");
        assert!(sip_parse::header(&options.extra_headers, "Geolocation").is_none());

        let (options, delivered) = attempt_invite(
            &info,
            &attempt(&[PidfLo]),
            call_id,
            Some(sdp),
            from.clone(),
            None,
            "10.0.0.1",
        );
        assert_eq!(delivered, [PidfLo]);
        let body = options.body.unwrap();
        let parsed = sip_core::MultipartBody::parse(&body.content_type, &body.bytes).unwrap();
        let geolocation = sip_parse::header(&options.extra_headers, "Geolocation")
            .unwrap()
            .to_string();
        let pidf = parsed
            .part_by_content_id(geolocation.trim_start_matches('<').trim_end_matches('>'))
            .expect("Geolocation names the PIDF-LO part");
        assert!(String::from_utf8_lossy(pidf.body()).contains("pres:+15185550100@pbx.example"));
        assert_eq!(
            parsed.part_by_type("application/sdp").unwrap().body(),
            sdp.as_bytes()
        );
        assert_eq!(options.from, from);

        // No location: nothing to send by value, and it is not claimed.
        let unlocated = EmergencyCallInfo {
            location: None,
            ..located_info()
        };
        let (options, delivered) = attempt_invite(
            &unlocated,
            &attempt(&[Elin, PidfLo]),
            call_id,
            Some(sdp),
            from,
            None,
            "h",
        );
        assert_eq!(delivered, [Elin]);
        assert_eq!(options.body.unwrap().content_type, "application/sdp");
    }

    #[test]
    fn only_a_trunk_that_also_takes_elin_is_retried_without_the_location() {
        use LocationDelivery::{Elin, PidfLo};
        assert_eq!(
            attempt(&[Elin, PidfLo]).without_pidf_lo().unwrap().delivery,
            [Elin]
        );
        assert!(attempt(&[PidfLo]).without_pidf_lo().is_none());
        assert!(attempt(&[Elin]).without_pidf_lo().is_none());
    }
}
