//! Transport layer integration.
//!
//! This module provides type conversions and utilities for working with
//! siphon-rs transports through fcp-core abstractions.

use anyhow::Result;
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, error};

// Re-export from siphon-rs
pub use sip_transport::pool::ConnectionPool;

// Re-export TLS helpers when TLS feature is enabled
#[cfg(feature = "tls")]
pub use sip_transport::TlsConfig;

// Re-export conversion functions from adapters module
pub use crate::adapters::{to_fcp_transport_type, to_sip_transport_kind};

use crate::prelude::*;

/// Real transport dispatcher that uses siphon-rs transport layer.
///
/// This dispatcher handles UDP, TCP, and TLS transports with connection pooling
/// for stream-based transports.
pub struct RealTransportDispatcher {
    /// UDP socket for sending datagrams
    udp_socket: Option<Arc<UdpSocket>>,

    /// TCP connection pool
    tcp_pool: Arc<ConnectionPool>,

    /// TLS configuration for secure connections
    #[cfg(feature = "tls")]
    tls_config: Option<Arc<tokio_rustls::rustls::ClientConfig>>,
}

impl RealTransportDispatcher {
    /// Create a new dispatcher with optional UDP socket.
    ///
    /// If udp_socket is None, UDP sends will fail.
    pub fn new(udp_socket: Option<Arc<UdpSocket>>) -> Self {
        Self {
            udp_socket,
            tcp_pool: Arc::new(ConnectionPool::new()),
            #[cfg(feature = "tls")]
            tls_config: None,
        }
    }

    /// Create a dispatcher with a UDP socket bound to the given address.
    pub async fn with_udp(bind_addr: SocketAddr) -> Result<Self> {
        // Create socket with SO_REUSEADDR to allow port sharing with listener
        use socket2::{Domain, Protocol, Socket, Type};

        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&bind_addr.into())?;

        let std_socket: std::net::UdpSocket = socket.into();
        let socket = UdpSocket::from_std(std_socket)?;

        Ok(Self::new(Some(Arc::new(socket))))
    }

    /// Set TLS client configuration for secure connections.
    #[cfg(feature = "tls")]
    pub fn with_tls_config(
        mut self,
        config: Arc<tokio_rustls::rustls::ClientConfig>,
    ) -> Self {
        self.tls_config = Some(config);
        self
    }

    /// Register an inbound packet sink for client-initiated TCP/TLS connections.
    /// This allows responses on outbound streams to be routed back into the SIP handler.
    pub async fn set_inbound_tx(&self, tx: mpsc::Sender<sip_transport::InboundPacket>) {
        self.tcp_pool.set_inbound_tx(tx.clone()).await;
        #[cfg(feature = "tls")]
        {
            // For symmetry, TLS pool will reuse the same sink once TLS pooling is added.
            let _ = &self.tls_config;
        }
    }

    /// Load TLS client configuration from system root certificates.
    ///
    /// This creates a client config suitable for outbound SIPS connections.
    #[cfg(feature = "tls")]
    pub fn load_tls_client_config() -> Result<Arc<tokio_rustls::rustls::ClientConfig>> {
        use tokio_rustls::rustls::{
            pki_types::CertificateDer, ClientConfig, RootCertStore,
        };

        // Use system root certificates
        let mut root_store = RootCertStore::empty();

        match rustls_native_certs::load_native_certs() {
            Ok(certs) => {
                for cert in certs {
                    // Convert from rustls_native_certs format to CertificateDer
                    root_store.add(CertificateDer::from(cert)).ok();
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load some native certificates: {}", e);
            }
        }

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(Arc::new(config))
    }
}

#[async_trait::async_trait]
impl TransportDispatcher for RealTransportDispatcher {
    async fn dispatch(&self, ctx: &TransportContext, payload: Bytes) -> Result<()> {
        use sip_transaction::TransportKind as TxnTransportKind;

        let peer = ctx.peer();
        let transport = ctx.transport();

        // Extract first line for logging (to identify message type)
        let first_line = payload
            .split(|&b| b == b'\r' || b == b'\n')
            .next()
            .and_then(|line| std::str::from_utf8(line).ok())
            .unwrap_or("<invalid>");

        debug!(
            transport = ?transport,
            peer = %peer,
            size = payload.len(),
            first_line = %first_line,
            has_stream = ctx.stream().is_some(),
            "dispatching SIP message"
        );

        // Check if we have a stream sender (for existing TCP/TLS connection)
        if let Some(stream_tx) = ctx.stream() {
            debug!(
                first_line = %first_line,
                peer = %peer,
                "using existing stream connection"
            );
            if stream_tx.send(payload.clone()).await.is_ok() {
                debug!(
                    first_line = %first_line,
                    peer = %peer,
                    "stream send successful"
                );
                return Ok(());
            }
            // If send failed, fall through to create new connection
            debug!(
                first_line = %first_line,
                peer = %peer,
                "existing stream connection failed, creating new connection"
            );
        }

        match transport {
            TxnTransportKind::Udp => {
                if let Some(socket) = &self.udp_socket {
                    sip_transport::send_udp(socket.as_ref(), &peer, &payload).await?;
                } else {
                    error!("UDP socket not configured");
                    anyhow::bail!("UDP socket not configured");
                }
            }
            TxnTransportKind::Tcp => {
                // Use connection pool for TCP
                debug!(
                    first_line = %first_line,
                    peer = %peer,
                    "sending via tcp_pool"
                );
                self.tcp_pool.send_tcp(peer, payload).await?;
            }
            TxnTransportKind::Tls => {
                #[cfg(feature = "tls")]
                {
                    if let Some(tls_config) = &self.tls_config {
                        // Extract server name from context or use peer IP as string
                        let server_name = match ctx.server_name() {
                            Some(name) => name.to_string(),
                            None => {
                                // Fall back to IP address as server name
                                peer.ip().to_string()
                            }
                        };

                        let tls_cfg = sip_transport::TlsConfig::new(server_name, tls_config.clone());

                        sip_transport::send_tls(&peer, &payload, &tls_cfg).await?;
                    } else {
                        error!("TLS configuration not set");
                        anyhow::bail!("TLS configuration not set");
                    }
                }
                #[cfg(not(feature = "tls"))]
                {
                    error!("TLS feature not enabled");
                    anyhow::bail!("TLS feature not enabled at compile time");
                }
            }
            TxnTransportKind::Ws | TxnTransportKind::Wss => {
                error!("WebSocket transports not yet implemented");
                anyhow::bail!("WebSocket transports not yet implemented");
            }
            TxnTransportKind::Sctp | TxnTransportKind::TlsSctp => {
                error!("SCTP transports not yet implemented");
                anyhow::bail!("SCTP transports not yet implemented");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::TransportType as FcpTransportType;
    use sip_transport::TransportKind as SipTransportKind;

    #[test]
    fn test_transport_conversion() {
        assert_eq!(
            to_sip_transport_kind(FcpTransportType::Udp),
            SipTransportKind::Udp
        );
        assert_eq!(
            to_fcp_transport_type(SipTransportKind::Tcp),
            FcpTransportType::Tcp
        );
    }
}
