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
use fcp_core::TenantId;
use ipnetwork::IpNetwork;
use smol_str::SmolStr;
use tracing::warn;

pub use fcp_core::{CALLER_TENANT_HEADER, CALLER_USER_HEADER};

/// The caller a trusted edge vouched for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeCaller {
    pub tenant_id: TenantId,
    pub username: String,
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

    /// The caller an INVITE from `peer` was placed for: `None` when the
    /// request names none, and when `peer` is not a trusted edge (logged,
    /// since an untrusted peer naming a caller is either misconfigured or
    /// lying).
    pub fn caller(&self, request: &sip_core::Request, peer: IpAddr) -> Option<EdgeCaller> {
        let tenant = sip_parse::header(request.headers(), CALLER_TENANT_HEADER);
        let user = sip_parse::header(request.headers(), CALLER_USER_HEADER);
        if tenant.is_none() && user.is_none() {
            return None;
        }
        if !self.contains(peer) {
            warn!(peer = %peer, "Ignoring caller headers from a peer that is not a trusted edge");
            return None;
        }
        let (Some(tenant), Some(user)) = (tenant, user) else {
            warn!(peer = %peer, "Ignoring incomplete caller headers from a trusted edge");
            return None;
        };
        let Ok(tenant_id) = tenant.as_str().trim().parse::<TenantId>() else {
            warn!(peer = %peer, tenant = %tenant, "Ignoring caller headers naming an unparsable tenant");
            return None;
        };
        let username = user.as_str().trim().to_string();
        if username.is_empty() {
            return None;
        }
        Some(EdgeCaller {
            tenant_id,
            username,
        })
    }
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
pub async fn detect(
    storage: Option<&Arc<fcp_core::DynStorage>>,
    tenant_id: &TenantId,
    destination: &str,
    caller_username: Option<&str>,
    caller_aor: Option<&str>,
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
            .get_user_in_tenant(username, tenant_id)
            .await
            .ok()
            .flatten()
    };
    if let Some(user) = &user {
        info.location = resolve_for_user(storage.as_ref(), user, caller_aor).await;
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
    pub delivery: Vec<LocationDelivery>,
}

impl EmergencyAttempt {
    pub fn from_route(route: &crate::handler::OutboundRoute) -> Self {
        let delivery = if route.emergency_location.is_empty() {
            vec![LocationDelivery::Elin]
        } else {
            route.emergency_location.clone()
        };
        Self {
            target: route.target_uri.clone(),
            trunk: route.trunk_name.clone(),
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
            let part = sip_core::BodyPart::new(fcp_core::pidf_lo::CONTENT_TYPE, document)
                .and_then(|p| p.with_content_id(&content_id));
            match part {
                Ok(pidf_part) => {
                    let mut multipart = sip_core::MultipartBody::mixed();
                    let sdp_part =
                        sdp.map(|s| sip_core::BodyPart::new("application/sdp", s.to_string()));
                    let built = match sdp_part {
                        Some(Ok(p)) => multipart.push(p).and_then(|_| multipart.push(pidf_part)),
                        Some(Err(e)) => Err(e),
                        None => multipart.push(pidf_part),
                    };
                    match built.and_then(|_| {
                        sip_core::Uri::parse(&format!("cid:{content_id}"))
                            .map_err(|e| sip_core::MultipartError::InvalidHeader(e.to_string()))
                    }) {
                        Ok(uri) => {
                            let geolocation = sip_core::GeolocationHeader::single(
                                sip_core::GeolocationValue::new(uri),
                            );
                            let _ = extra.push("Geolocation", geolocation.to_string());
                            let _ = extra.push(
                                "Geolocation-Routing",
                                sip_core::GeolocationRoutingHeader::allowed(true).to_string(),
                            );
                            delivered.push(LocationDelivery::PidfLo);
                            Some(sip_uac::InviteBody::multipart(&multipart))
                        }
                        Err(e) => {
                            warn!(call_id = %call_id, error = %e, "Could not build the location body; sending without it");
                            sdp.map(sip_uac::InviteBody::sdp)
                        }
                    }
                }
                Err(e) => {
                    warn!(call_id = %call_id, error = %e, "Could not build the location part; sending without it");
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
        },
        delivered,
    )
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
    let notify_contacts = config_for(call_manager.storage(), &record.tenant_id)
        .await
        .notify_contacts;
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
            username: "alice".into(),
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
