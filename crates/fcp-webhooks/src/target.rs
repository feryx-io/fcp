//! Where a webhook may be delivered (docs/DEVELOPER_SURFACE.md §3, phase 5).
//!
//! A tenant names the URL, so without a guard a webhook is a way to make
//! the platform send requests into its own network: the cloud's metadata
//! service at 169.254.169.254, the admin on localhost, a database on a
//! private address. A target must be HTTPS and must not be — or resolve to
//! — a loopback, private, link-local, shared or otherwise non-public
//! address, unless the platform allows private targets (a lab, a test).
//!
//! The URL is checked when a webhook is made and changed. At delivery the
//! dispatcher's resolver ([`GuardedResolver`]) drops forbidden addresses
//! from every lookup, so a name that resolves somewhere private at delivery
//! time (DNS rebinding) is refused too.

use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Why a target was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetError(pub String);

impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TargetError {}

/// Whether an address is one a webhook must not reach: anything not on the
/// public internet.
pub fn forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return forbidden_v4(v4);
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // fc00::/7, unique local
                || (first & 0xfe00) == 0xfc00
                // fe80::/10, link local
                || (first & 0xffc0) == 0xfe80
                // 2001:db8::/32, documentation
                || (first == 0x2001 && v6.segments()[1] == 0x0db8)
                // 64:ff9b::/96 NAT64 onto an address that is itself forbidden
                || (first == 0x0064
                    && v6.segments()[1] == 0xff9b
                    && forbidden_v4(Ipv4Addr::from(
                        (u32::from(v6.segments()[6]) << 16) | u32::from(v6.segments()[7]),
                    )))
        }
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // 0.0.0.0/8, "this network"
        || a == 0
        // 100.64.0.0/10, shared address space (carrier-grade NAT)
        || (a == 100 && (64..128).contains(&b))
        // 192.0.0.0/24, IETF protocol assignments
        || (a == 192 && b == 0 && ip.octets()[2] == 0)
        // 198.18.0.0/15, benchmarking
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4, reserved
        || a >= 240
}

/// Check a target URL's form: HTTPS (HTTP only where private targets are
/// allowed), a host, no credentials, and — for an address written into the
/// URL — a public one. A name is checked when it is resolved
/// ([`check_resolved`], [`GuardedResolver`]).
pub fn check_url(raw: &str, allow_private: bool) -> Result<Url, TargetError> {
    let url = Url::parse(raw.trim()).map_err(|e| TargetError(format!("not a URL: {e}")))?;
    match url.scheme() {
        "https" => {}
        "http" if allow_private => {}
        "http" => return Err(TargetError("a webhook must be HTTPS".into())),
        other => return Err(TargetError(format!("a webhook must be HTTPS, not {other}"))),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(TargetError(
            "credentials do not belong in a webhook URL: verify the signature instead".into(),
        ));
    }
    let host = url
        .host()
        .ok_or_else(|| TargetError("a webhook URL needs a host".into()))?;
    if !allow_private {
        let literal = match host {
            url::Host::Ipv4(v4) => Some(IpAddr::V4(v4)),
            url::Host::Ipv6(v6) => Some(IpAddr::V6(v6)),
            url::Host::Domain(name) => {
                let name = name.trim_end_matches('.').to_ascii_lowercase();
                if name == "localhost" || name.ends_with(".localhost") {
                    return Err(TargetError(format!("{name} is not a public host")));
                }
                None
            }
        };
        if let Some(ip) = literal {
            if forbidden_ip(ip) {
                return Err(TargetError(format!("{ip} is not a public address")));
            }
        }
    }
    Ok(url)
}

/// Check a target URL and, unless private targets are allowed, that every
/// address its host resolves to now is public.
pub async fn check_resolved(raw: &str, allow_private: bool) -> Result<Url, TargetError> {
    let url = check_url(raw, allow_private)?;
    if allow_private {
        return Ok(url);
    }
    if let Some(url::Host::Domain(name)) = url.host() {
        let port = url.port_or_known_default().unwrap_or(443);
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name, port))
            .await
            .map_err(|e| TargetError(format!("{name} does not resolve: {e}")))?
            .collect();
        if addrs.is_empty() {
            return Err(TargetError(format!("{name} does not resolve")));
        }
        if let Some(bad) = addrs.iter().find(|a| forbidden_ip(a.ip())) {
            return Err(TargetError(format!(
                "{name} resolves to {}, which is not a public address",
                bad.ip()
            )));
        }
    }
    Ok(url)
}

/// The dispatcher's resolver: the system's, with forbidden addresses
/// dropped, so a delivery can only connect to a public address.
#[derive(Debug, Clone, Copy)]
pub struct GuardedResolver {
    pub allow_private: bool,
}

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allow_private = self.allow_private;
        Box::pin(async move {
            let host = name.as_str().to_string();
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| allow_private || !forbidden_ip(a.ip()))
                .collect();
            if found.is_empty() {
                let err: Box<dyn std::error::Error + Send + Sync> =
                    Box::new(TargetError(format!("{host} resolves to no public address")));
                return Err(err);
            }
            let addrs: reqwest::dns::Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}

/// Whether a URL written as an address (not a name) may be delivered to:
/// the resolver never sees those.
pub fn literal_target_allowed(url: &str, allow_private: bool) -> bool {
    if allow_private {
        return true;
    }
    match Url::parse(url)
        .ok()
        .and_then(|u| u.host().map(|h| h.to_owned()))
    {
        Some(url::Host::Ipv4(v4)) => !forbidden_v4(v4),
        Some(url::Host::Ipv6(v6)) => !forbidden_ip(IpAddr::V6(v6)),
        Some(url::Host::Domain(_)) => true,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_loopback_link_local_and_metadata_are_refused() {
        for bad in [
            "https://127.0.0.1/hook",
            "https://10.1.2.3/hook",
            "https://172.16.0.9/hook",
            "https://192.168.1.1/hook",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1/hook",
            "https://0.0.0.0/hook",
            "https://[::1]/hook",
            "https://[fd00::1]/hook",
            "https://[fe80::1]/hook",
            "https://[::ffff:10.0.0.1]/hook",
            "https://localhost/hook",
            "https://api.localhost/hook",
        ] {
            assert!(check_url(bad, false).is_err(), "{bad} was allowed");
        }
    }

    #[test]
    fn a_public_https_target_is_allowed_and_http_is_not() {
        assert!(check_url("https://hooks.example.com/fcp", false).is_ok());
        assert!(check_url("https://93.184.216.34/fcp", false).is_ok());
        assert!(check_url("http://hooks.example.com/fcp", false).is_err());
        assert!(check_url("ftp://hooks.example.com/fcp", false).is_err());
        assert!(check_url("https://user:pw@hooks.example.com/fcp", false).is_err());
        assert!(check_url("not a url", false).is_err());
    }

    #[test]
    fn a_lab_may_allow_private_targets() {
        assert!(check_url("http://127.0.0.1:9000/hook", true).is_ok());
        assert!(literal_target_allowed("http://127.0.0.1:9000/hook", true));
        assert!(!literal_target_allowed("https://10.0.0.1/hook", false));
        assert!(literal_target_allowed(
            "https://hooks.example.com/hook",
            false
        ));
    }

    #[tokio::test]
    async fn a_name_resolving_to_loopback_is_refused_and_the_resolver_drops_it() {
        let err = check_resolved("https://localhost.example.invalid.localhost/x", false).await;
        assert!(err.is_err());
        // `localhost` resolves to loopback: the resolver leaves nothing.
        let resolver = GuardedResolver {
            allow_private: false,
        };
        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        let result = reqwest::dns::Resolve::resolve(&resolver, name).await;
        assert!(result.is_err(), "loopback survived the resolver");
        let allowed = GuardedResolver {
            allow_private: true,
        };
        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        assert!(reqwest::dns::Resolve::resolve(&allowed, name).await.is_ok());
    }
}
