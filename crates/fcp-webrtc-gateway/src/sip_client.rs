//! SIP UAC factory for outbound call initiation.
//!
//! Extracted from the fcp-call-manager pattern: creates a transport dispatcher
//! and IntegratedUAC for sending SIP INVITEs to remote phones.

use crate::config::GatewayConfig;
use anyhow::{Context, Result};
use fcp_sip::prelude::*;
use fcp_sip::transport::ConnectionPool;
use std::sync::Arc;
use tracing::info;

/// Create a transport dispatcher with a shared TCP connection pool.
pub async fn create_transport_dispatcher(
    config: &GatewayConfig,
    tcp_pool: Arc<ConnectionPool>,
) -> Result<Arc<fcp_sip::transport::RealTransportDispatcher>> {
    use fcp_sip::transport::RealTransportDispatcher;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid SIP UDP bind address")?;

    // Create UDP socket with SO_REUSEADDR
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .context("Failed to create UDP socket")?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&bind_addr.into())?;

    let std_socket: std::net::UdpSocket = socket.into();
    let udp_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    info!(bind = %bind_addr, "SIP UDP transport bound");

    let dispatcher = RealTransportDispatcher::new_with_pool(Some(Arc::new(udp_socket)), tcp_pool);

    Ok(Arc::new(dispatcher))
}

/// Create a SIP UAC for outbound calls.
pub async fn create_uac(
    config: &GatewayConfig,
    transaction_manager: Arc<TransactionManager>,
    dispatcher: Arc<fcp_sip::transport::RealTransportDispatcher>,
) -> Result<Arc<SipUserAgentClient>> {
    let local_uri = create_local_uri(config)?;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid SIP UDP bind address")?;

    let local_addr = if let Some(ref local_ip) = config.sip.local_ip {
        let port = if bind_addr.port() == 0 {
            5060
        } else {
            bind_addr.port()
        };
        format!("{}:{}", local_ip, port)
    } else {
        let port = if bind_addr.port() == 0 {
            5060
        } else {
            bind_addr.port()
        };
        format!("{}:{}", bind_addr.ip(), port)
    };

    let resolver = Arc::new(SipResolver::from_system().context("Failed to create DNS resolver")?);

    let builder = IntegratedUAC::builder()
        .local_uri(local_uri.as_str())
        .local_addr(&local_addr)?
        .via_advertised_addr(&local_addr)?
        .transaction_manager(transaction_manager)
        .resolver(resolver)
        .dispatcher(dispatcher);

    let integrated_uac = builder.build().context("Failed to build IntegratedUAC")?;

    info!(local_uri = local_uri.as_str(), local_addr = %local_addr, "SIP UAC created");

    Ok(Arc::new(SipUserAgentClient::new(Arc::new(integrated_uac))))
}

/// Create a local SIP URI from configuration.
fn create_local_uri(config: &GatewayConfig) -> Result<SipUri> {
    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid SIP UDP bind address")?;

    // Use explicit local_uri if configured
    if let Some(ref uri_str) = config.sip.local_uri {
        let uri = Uri::parse(uri_str).context("Failed to parse configured sip.local_uri")?;
        return uri
            .as_sip()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Configured sip.local_uri is not a SIP URI"));
    }

    // Build from local_ip or bind address
    let domain = if bind_addr.ip().is_unspecified() {
        config
            .sip
            .local_ip
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string()))
    } else {
        bind_addr.ip().to_string()
    };

    let uri_str = if bind_addr.port() != 0 && bind_addr.port() != 5060 {
        format!("sip:webrtc-gw@{}:{}", domain, bind_addr.port())
    } else {
        format!("sip:webrtc-gw@{}", domain)
    };

    let uri = Uri::parse(&uri_str).context("Failed to parse local URI")?;
    uri.as_sip()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

/// Derive a usable local IP from the bind address.
pub fn derive_local_ip(bind_addr: &str) -> String {
    if let Ok(addr) = bind_addr.parse::<std::net::SocketAddr>() {
        if !addr.ip().is_unspecified() {
            return addr.ip().to_string();
        }
    }
    auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Auto-detect the local LAN IP by asking the OS which interface routes to the internet.
pub fn auto_detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:53").ok()?;
    let addr = socket.local_addr().ok()?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    let ip = addr.ip().to_string();
    info!(ip = %ip, "Auto-detected local IP");
    Some(ip)
}
