//! What a REGISTER's Contact says about its phone beyond where it is
//! (`docs/PUSH.md` §2.1): its instance and reg-id (RFC 5626, Contact header
//! parameters) and, for an app that can be woken, its push address
//! (RFC 8599, `pn-*` parameters of the Contact URI).

use fcp_core::push::PushAddress;

/// The parts of one Contact header value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactParts {
    /// `+sip.instance`, without its quotes (`<urn:uuid:…>`).
    pub instance: Option<String>,
    pub reg_id: Option<u32>,
    pub push: PushRequest,
}

/// What the Contact URI asks of push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushRequest {
    /// No `pn-provider`: an ordinary phone.
    None,
    /// A provider, the app at it and the device's token.
    Address(PushAddress),
    /// `pn-provider` without `pn-param` or `pn-prid`.
    Incomplete,
}

/// The RFC 8599 parameters of a Contact URI, which say where to push and
/// are not part of where the phone is.
const PUSH_PARAMS: [&str; 4] = ["pn-provider", "pn-param", "pn-prid", "pn-purr"];

/// Whether a URI parameter's name is one of RFC 8599's.
pub fn is_push_param(name: &str) -> bool {
    PUSH_PARAMS.iter().any(|p| p.eq_ignore_ascii_case(name))
}

/// Split on `;` outside double quotes.
fn split_params(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

fn name_value(param: &str) -> (String, Option<&str>) {
    let param = param.trim();
    match param.split_once('=') {
        Some((n, v)) => (n.trim().to_ascii_lowercase(), Some(v.trim())),
        None => (param.to_ascii_lowercase(), None),
    }
}

/// Read one Contact header value: `<uri;uri-params>;header-params`, or a
/// bare URI whose parameters are the header's.
pub fn parse_contact(value: &str) -> ContactParts {
    let value = value.trim();
    let (uri, header) = match (value.find('<'), value.find('>')) {
        (Some(open), Some(close)) if open < close => (&value[open + 1..close], &value[close + 1..]),
        _ => {
            // addr-spec: RFC 3261 gives its parameters to the header.
            match value.split_once(';') {
                Some((u, h)) => (u, h),
                None => (value, ""),
            }
        }
    };

    let mut instance = None;
    let mut reg_id = None;
    for param in split_params(header) {
        match name_value(param) {
            (n, Some(v)) if n == "+sip.instance" => {
                instance = Some(v.trim_matches('"').to_string());
            }
            (n, Some(v)) if n == "reg-id" => reg_id = v.parse().ok(),
            _ => {}
        }
    }

    let (mut provider, mut param, mut prid) = (None, None, None);
    for p in split_params(uri).into_iter().skip(1) {
        match name_value(p) {
            (n, Some(v)) if n == "pn-provider" => provider = Some(v.to_ascii_lowercase()),
            (n, Some(v)) if n == "pn-param" => param = Some(v.to_string()),
            (n, Some(v)) if n == "pn-prid" => prid = Some(v.to_string()),
            _ => {}
        }
    }
    let push = match (provider, param, prid) {
        (None, _, _) => PushRequest::None,
        (Some(provider), Some(param), Some(prid)) if !param.is_empty() && !prid.is_empty() => {
            PushRequest::Address(PushAddress {
                provider,
                param,
                prid,
            })
        }
        _ => PushRequest::Incomplete,
    };

    ContactParts {
        instance,
        reg_id,
        push,
    }
}

/// `Feature-Caps` values naming providers this node pushes through
/// (RFC 8599 §13.2): one per provider.
pub fn feature_caps(providers: &[&str]) -> Vec<String> {
    providers
        .iter()
        .map(|p| format!("*;+sip.pns=\"{p}\""))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_names_its_instance_and_where_to_push() {
        let parts = parse_contact(
            "<sip:app-3f9c@10.0.0.7:52311;transport=tls;pn-provider=APNS;\
             pn-param=ABCDE12345.com.thevoiceguy.anvil.voip;pn-prid=8f2c00a91>\
             ;+sip.instance=\"<urn:uuid:00000000-0000-0000-0000-000000000001>\";reg-id=1;expires=604800",
        );
        assert_eq!(
            parts.instance.as_deref(),
            Some("<urn:uuid:00000000-0000-0000-0000-000000000001>")
        );
        assert_eq!(parts.reg_id, Some(1));
        assert_eq!(
            parts.push,
            PushRequest::Address(PushAddress {
                provider: "apns".into(),
                param: "ABCDE12345.com.thevoiceguy.anvil.voip".into(),
                prid: "8f2c00a91".into(),
            })
        );
    }

    #[test]
    fn a_desk_phone_has_neither() {
        let parts = parse_contact("<sip:1001@192.0.2.10:5060>;expires=3600");
        assert_eq!(parts.instance, None);
        assert_eq!(parts.reg_id, None);
        assert_eq!(parts.push, PushRequest::None);
        let bare = parse_contact("sip:1001@192.0.2.10;expires=60");
        assert_eq!(bare.push, PushRequest::None);
    }

    #[test]
    fn a_provider_alone_is_incomplete() {
        let parts = parse_contact("<sip:a@h;pn-provider=fcm>");
        assert_eq!(parts.push, PushRequest::Incomplete);
        let empty = parse_contact("<sip:a@h;pn-provider=fcm;pn-param=p;pn-prid=>");
        assert_eq!(empty.push, PushRequest::Incomplete);
    }

    #[test]
    fn a_quoted_semicolon_does_not_split() {
        let parts = parse_contact("<sip:a@h>;+sip.instance=\"<urn:x;y>\";reg-id=2");
        assert_eq!(parts.instance.as_deref(), Some("<urn:x;y>"));
        assert_eq!(parts.reg_id, Some(2));
    }

    #[test]
    fn feature_caps_name_each_provider() {
        assert_eq!(
            feature_caps(&["apns", "fcm"]),
            ["*;+sip.pns=\"apns\"", "*;+sip.pns=\"fcm\""]
        );
        assert!(is_push_param("PN-PRID"));
        assert!(!is_push_param("transport"));
    }
}
