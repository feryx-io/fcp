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
use tracing::{debug, error, info};

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
/// This dispatcher handles UDP, TCP, TLS, WS, and WSS transports with
/// connection pooling for stream-based transports where available.
pub struct RealTransportDispatcher {
    /// UDP socket for sending datagrams
    udp_socket: Option<Arc<UdpSocket>>,

    /// TCP connection pool
    tcp_pool: Arc<ConnectionPool>,

    /// TLS configuration for secure connections
    #[cfg(feature = "tls")]
    tls_config: Option<Arc<tokio_rustls::rustls::ClientConfig>>,

    /// TLS connection pool. Keeps outbound TLS connections open so the
    /// responses to a request we sent (and in-dialog requests the peer
    /// sends back down the same connection) reach the inbound sink,
    /// exactly as the TCP pool does.
    #[cfg(feature = "tls")]
    tls_pool: Arc<sip_transport::pool::TlsPool>,

    /// Set when this host does not reach its advertised address: a message
    /// to a peer on this host names the one the peer is at (#335).
    sibling: std::sync::OnceLock<crate::sibling::SiblingRewrite>,

    /// Told of every message once it is sent (`crate::wire_trace`).
    tap: std::sync::OnceLock<Arc<dyn crate::wire_trace::WireTap>>,
}

impl RealTransportDispatcher {
    /// Create a new dispatcher with optional UDP socket.
    ///
    /// This creates an internal ConnectionPool. For better resource sharing,
    /// consider using `new_with_pool()` to provide an external pool.
    ///
    /// If udp_socket is None, UDP sends will fail.
    pub fn new(udp_socket: Option<Arc<UdpSocket>>) -> Self {
        Self {
            udp_socket,
            tcp_pool: Arc::new(ConnectionPool::new()),
            #[cfg(feature = "tls")]
            tls_config: None,
            #[cfg(feature = "tls")]
            tls_pool: Arc::new(sip_transport::pool::TlsPool::new()),
            sibling: std::sync::OnceLock::new(),
            tap: std::sync::OnceLock::new(),
        }
    }

    /// Create a new dispatcher with optional UDP socket and external TCP connection pool.
    ///
    /// **Recommended for production**: This allows sharing a single ConnectionPool
    /// between the dispatcher and transaction manager, avoiding duplicate connections
    /// and better resource utilization at scale.
    ///
    /// # Example
    /// ```ignore
    /// let pool = Arc::new(ConnectionPool::new());
    /// let dispatcher = RealTransportDispatcher::new_with_pool(udp_socket, pool);
    /// ```
    pub fn new_with_pool(
        udp_socket: Option<Arc<UdpSocket>>,
        tcp_pool: Arc<ConnectionPool>,
    ) -> Self {
        Self {
            udp_socket,
            tcp_pool,
            #[cfg(feature = "tls")]
            tls_config: None,
            #[cfg(feature = "tls")]
            tls_pool: Arc::new(sip_transport::pool::TlsPool::new()),
            sibling: std::sync::OnceLock::new(),
            tap: std::sync::OnceLock::new(),
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
    pub fn with_tls_config(mut self, config: Arc<tokio_rustls::rustls::ClientConfig>) -> Self {
        self.tls_config = Some(config);
        self
    }

    /// This host does not reach `advertised`, the address its services
    /// put in Contact and SDP (`crate::self_reach`): from now on a message
    /// to a peer on this host names the address the peer is at instead
    /// (`crate::sibling`). The first call wins.
    pub fn advertise_to_siblings(&self, advertised: std::net::IpAddr) {
        if self
            .sibling
            .set(crate::sibling::SiblingRewrite::new(advertised))
            .is_ok()
        {
            info!(
                %advertised,
                "Services on this host are told this host's own address for it, not the advertised one"
            );
        }
    }

    /// Tell `tap` of every message sent from now on, as it was sent. The
    /// first call wins.
    pub fn trace_to(&self, tap: Arc<dyn crate::wire_trace::WireTap>) {
        let _ = self.tap.set(tap);
    }

    /// Register an inbound packet sink for client-initiated TCP/TLS connections.
    /// This allows responses on outbound streams to be routed back into the SIP handler.
    pub async fn set_inbound_tx(&self, tx: mpsc::Sender<sip_transport::InboundPacket>) {
        self.tcp_pool.set_inbound_tx(tx.clone()).await;
        #[cfg(feature = "tls")]
        self.tls_pool.set_inbound_tx(tx).await;
    }

    /// Load TLS client configuration from system root certificates.
    ///
    /// This creates a client config suitable for outbound SIPS connections.
    #[cfg(feature = "tls")]
    pub fn load_tls_client_config() -> Result<Arc<tokio_rustls::rustls::ClientConfig>> {
        Self::load_tls_client_config_with(None, None)
    }

    /// Load TLS client configuration for outbound SIPS connections.
    ///
    /// Servers are verified against the system roots. With
    /// `private_ca` (a PEM bundle), a server whose chain reaches that
    /// CA is also accepted under *any DNS name its certificate carries*:
    /// cluster peers are dialed by the address they advertise in
    /// membership, while their certificates name the node, so the
    /// usual name check would fail for exactly the peers the private CA
    /// exists to identify. Public roots keep the strict check.
    ///
    /// `client_identity` is `(cert_path, key_path)` presented to a
    /// server that asks for a client certificate (mutual TLS).
    #[cfg(feature = "tls")]
    pub fn load_tls_client_config_with(
        private_ca: Option<&str>,
        client_identity: Option<(&str, &str)>,
    ) -> Result<Arc<tokio_rustls::rustls::ClientConfig>> {
        use anyhow::Context as _;
        use tokio_rustls::rustls::{client::WebPkiServerVerifier, ClientConfig, RootCertStore};

        ensure_crypto_provider();

        // Use system root certificates
        let mut root_store = RootCertStore::empty();

        let result = rustls_native_certs::load_native_certs();
        for err in &result.errors {
            tracing::warn!("Failed to load some native certificates: {}", err);
        }
        for cert in result.certs {
            root_store.add(cert).ok();
        }

        let identity = match client_identity {
            Some((cert, key)) => Some(
                sip_transport::ClientIdentity::load(cert, key)
                    .context("Failed to load TLS client certificate")?,
            ),
            None => None,
        };

        let Some(private_ca) = private_ca else {
            return sip_transport::build_rustls_client_config(root_store, identity)
                .context("Failed to build TLS client config");
        };

        let private_roots = sip_transport::load_client_ca_roots(private_ca)
            .context("Failed to load private CA bundle")?;
        let private = WebPkiServerVerifier::builder(Arc::new(private_roots))
            .build()
            .context("Failed to build private-CA verifier")?;
        let public = if root_store.is_empty() {
            None
        } else {
            Some(
                WebPkiServerVerifier::builder(Arc::new(root_store))
                    .build()
                    .context("Failed to build system-roots verifier")?,
            )
        };
        let verifier = Arc::new(PrivateCaServerVerifier { public, private });

        let builder = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier);
        let config = match identity {
            Some(identity) => {
                let (certs, key) = identity.into_parts();
                builder
                    .with_client_auth_cert(certs, key)
                    .context("Failed to install TLS client certificate")?
            }
            None => builder.with_no_client_auth(),
        };
        Ok(Arc::new(config))
    }
}

/// Server-certificate verifier for a mixed world: public servers under
/// the system roots with the ordinary name check, cluster peers under a
/// private CA under any DNS name their certificate names. See
/// [`RealTransportDispatcher::load_tls_client_config_with`].
#[cfg(feature = "tls")]
#[derive(Debug)]
struct PrivateCaServerVerifier {
    public: Option<Arc<tokio_rustls::rustls::client::WebPkiServerVerifier>>,
    private: Arc<tokio_rustls::rustls::client::WebPkiServerVerifier>,
}

#[cfg(feature = "tls")]
impl tokio_rustls::rustls::client::danger::ServerCertVerifier for PrivateCaServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        ocsp_response: &[u8],
        now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> std::result::Result<
        tokio_rustls::rustls::client::danger::ServerCertVerified,
        tokio_rustls::rustls::Error,
    > {
        use tokio_rustls::rustls::client::danger::ServerCertVerified;
        use tokio_rustls::rustls::pki_types::ServerName;
        use tokio_rustls::rustls::{CertificateError, Error};

        match self.private.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => return Ok(verified),
            Err(Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => {
                // Chain, validity and usage all passed under the private
                // CA; only the dialed name is not in the certificate.
                // Accept the certificate under a name it does carry.
                if let Ok(identity) = sip_transport::PeerIdentity::from_der(end_entity.as_ref()) {
                    for name in identity.dns_names() {
                        let Ok(candidate) = ServerName::try_from(name.clone()) else {
                            continue;
                        };
                        if self
                            .private
                            .verify_server_cert(
                                end_entity,
                                intermediates,
                                &candidate,
                                ocsp_response,
                                now,
                            )
                            .is_ok()
                        {
                            tracing::debug!(
                                dialed = ?server_name,
                                certificate_name = %name,
                                "accepted private-CA server certificate under its own name"
                            );
                            return Ok(ServerCertVerified::assertion());
                        }
                    }
                }
            }
            Err(_) => {}
        }

        match &self.public {
            Some(public) => public.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ),
            None => Err(Error::InvalidCertificate(CertificateError::UnknownIssuer)),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> std::result::Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        self.private.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> std::result::Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        self.private.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        self.private.supported_verify_schemes()
    }
}

/// Make sure rustls has a process-level [`CryptoProvider`].
///
/// The workspace links rustls with both the `ring` and `aws-lc-rs` backends
/// (pulled in by different dependencies), so rustls cannot pick one on its
/// own and panics the first time a `ClientConfig`/`ServerConfig` builder is
/// used. Installing a default is idempotent; the first caller wins, so a
/// binary that wants a specific backend can install it before touching
/// SIP transports.
///
/// [`CryptoProvider`]: tokio_rustls::rustls::crypto::CryptoProvider
#[cfg(feature = "tls")]
pub fn ensure_crypto_provider() {
    use tokio_rustls::rustls::crypto::{aws_lc_rs, CryptoProvider};
    if CryptoProvider::get_default().is_none() {
        // Err only when another thread installed one first, which is fine.
        let _ = aws_lc_rs::default_provider().install_default();
    }
}

impl RealTransportDispatcher {
    /// Send `payload` as it is: on the stream it belongs to, else by its
    /// transport.
    async fn deliver(&self, ctx: &TransportContext, payload: Bytes) -> Result<()> {
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
        // If a stream exists, this is a response to an inbound connection and MUST use the stream.
        // We should NOT fall back to creating a new outbound connection.
        if let Some(stream_tx) = ctx.stream() {
            debug!(
                first_line = %first_line,
                peer = %peer,
                "using existing stream connection"
            );
            match stream_tx.send(payload.clone()).await {
                Ok(()) => {
                    debug!(
                        first_line = %first_line,
                        peer = %peer,
                        "stream send successful"
                    );
                    return Ok(());
                }
                Err(_) => {
                    // Stream channel closed - connection is dead
                    error!(
                        first_line = %first_line,
                        peer = %peer,
                        "stream channel closed, connection terminated"
                    );
                    anyhow::bail!(
                        "Failed to send via stream channel (connection closed) to {}",
                        peer
                    );
                }
            }
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

                        self.tls_pool
                            .send_tls(peer, server_name, tls_config.clone(), payload)
                            .await?;
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
                #[cfg(feature = "ws")]
                {
                    let secure = matches!(transport, TxnTransportKind::Wss);
                    let url = ctx
                        .ws_uri()
                        .map(str::to_owned)
                        .unwrap_or_else(|| websocket_url(ctx, secure));

                    debug!(
                        first_line = %first_line,
                        peer = %peer,
                        url = %url,
                        secure,
                        "sending via pooled websocket transport"
                    );

                    // Use the connection pool so repeated sends to the same
                    // peer reuse the existing WS connection instead of opening
                    // a new handshake per message.
                    self.tcp_pool.send_ws(&url, payload).await?;
                }
                #[cfg(not(feature = "ws"))]
                {
                    error!("WebSocket feature not enabled");
                    anyhow::bail!("WebSocket feature not enabled at compile time");
                }
            }
            TxnTransportKind::Sctp | TxnTransportKind::TlsSctp => {
                error!("SCTP transports not yet implemented");
                anyhow::bail!("SCTP transports not yet implemented");
            }
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl TransportDispatcher for RealTransportDispatcher {
    async fn dispatch(&self, ctx: &TransportContext, payload: Bytes) -> Result<()> {
        let peer = ctx.peer();
        let payload = match self.sibling.get() {
            Some(sibling) => {
                let bound = self
                    .udp_socket
                    .as_ref()
                    .and_then(|s| s.local_addr().ok())
                    .map(|a| a.ip());
                sibling.apply(peer.ip(), bound, &payload).unwrap_or(payload)
            }
            None => payload,
        };
        // Timed as it leaves, filed once it has: the far end's answer can
        // be read before this send returns, and must not file before it.
        let tap = self.tap.get().map(|tap| {
            (
                Arc::clone(tap),
                payload.clone(),
                std::time::SystemTime::now(),
            )
        });
        self.deliver(ctx, payload).await?;
        if let Some((tap, payload, at)) = tap {
            tap.sent(&format!("{:?}", ctx.transport()), peer, &payload, at);
        }
        Ok(())
    }
}

#[cfg(feature = "ws")]
pub fn websocket_url(ctx: &TransportContext, secure: bool) -> String {
    let host = ctx.server_name().map(str::to_owned).unwrap_or_else(|| {
        let ip = ctx.peer().ip();
        if ip.is_ipv6() {
            format!("[{}]", ip)
        } else {
            ip.to_string()
        }
    });
    let scheme = if secure { "wss" } else { "ws" };
    format!("{}://{}:{}/", scheme, host, ctx.peer().port())
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
