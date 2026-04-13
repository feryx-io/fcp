//! Shared SIP configuration validation helpers.
//!
//! Provides [`validate_sip_binds`] — a common validation routine used by all
//! FCP service crates that expose SIP transports.  Service-specific checks
//! (e.g. auth enforcement) remain in the individual crates.

use anyhow::{Context, Result};

/// Validate SIP bind addresses and TLS prerequisites.
///
/// Checks that every supplied bind string is a valid [`std::net::SocketAddr`]
/// and that `wss_bind` is only set when both `tls_cert_path` and
/// `tls_key_path` are also provided.
///
/// # Parameters
/// - `udp_bind` – required UDP bind address (e.g. `"0.0.0.0:5060"`)
/// - `tcp_bind` – required TCP bind address
/// - `ws_bind`  – optional WebSocket bind address
/// - `wss_bind` – optional secure WebSocket bind address
/// - `tls_cert_path` – path to TLS certificate (required when `wss_bind` is set)
/// - `tls_key_path`  – path to TLS private key (required when `wss_bind` is set)
pub fn validate_sip_binds(
    udp_bind: &str,
    tcp_bind: &str,
    ws_bind: Option<&str>,
    wss_bind: Option<&str>,
    tls_cert_path: Option<&str>,
    tls_key_path: Option<&str>,
) -> Result<()> {
    udp_bind
        .parse::<std::net::SocketAddr>()
        .context("Invalid SIP UDP bind address")?;
    tcp_bind
        .parse::<std::net::SocketAddr>()
        .context("Invalid SIP TCP bind address")?;
    if let Some(bind) = ws_bind {
        bind.parse::<std::net::SocketAddr>()
            .context("Invalid SIP WS bind address")?;
    }
    if let Some(bind) = wss_bind {
        bind.parse::<std::net::SocketAddr>()
            .context("Invalid SIP WSS bind address")?;
        if tls_cert_path.is_none() || tls_key_path.is_none() {
            anyhow::bail!("sip.wss_bind requires sip.tls_cert_path and sip.tls_key_path");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_udp_tcp() {
        assert!(validate_sip_binds("0.0.0.0:5060", "0.0.0.0:5060", None, None, None, None).is_ok());
    }

    #[test]
    fn invalid_udp_bind() {
        assert!(
            validate_sip_binds("not-an-addr", "0.0.0.0:5060", None, None, None, None).is_err()
        );
    }

    #[test]
    fn invalid_tcp_bind() {
        assert!(
            validate_sip_binds("0.0.0.0:5060", "not-an-addr", None, None, None, None).is_err()
        );
    }

    #[test]
    fn valid_ws_bind() {
        assert!(validate_sip_binds(
            "0.0.0.0:5060",
            "0.0.0.0:5060",
            Some("0.0.0.0:8080"),
            None,
            None,
            None
        )
        .is_ok());
    }

    #[test]
    fn invalid_ws_bind() {
        assert!(validate_sip_binds(
            "0.0.0.0:5060",
            "0.0.0.0:5060",
            Some("bad"),
            None,
            None,
            None
        )
        .is_err());
    }

    #[test]
    fn wss_requires_tls_paths() {
        // wss_bind set but no TLS paths — should fail
        assert!(validate_sip_binds(
            "0.0.0.0:5060",
            "0.0.0.0:5060",
            None,
            Some("0.0.0.0:8443"),
            None,
            None
        )
        .is_err());

        // wss_bind set with only cert — should fail
        assert!(validate_sip_binds(
            "0.0.0.0:5060",
            "0.0.0.0:5060",
            None,
            Some("0.0.0.0:8443"),
            Some("/etc/fcp/cert.pem"),
            None
        )
        .is_err());

        // wss_bind set with both TLS paths — should pass
        assert!(validate_sip_binds(
            "0.0.0.0:5060",
            "0.0.0.0:5060",
            None,
            Some("0.0.0.0:8443"),
            Some("/etc/fcp/cert.pem"),
            Some("/etc/fcp/key.pem")
        )
        .is_ok());
    }
}
