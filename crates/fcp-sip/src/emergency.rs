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
use fcp_core::e911::{dialed_user_part, resolve_for_user, EmergencyCallRecord, EmergencyConfig};
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
        delivery: info.delivery().map(str::to_string),
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
}
