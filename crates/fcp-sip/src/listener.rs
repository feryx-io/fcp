//! Transport listeners for receiving inbound SIP messages.
//!
//! This module provides a `TransportListener` that manages UDP, TCP, and TLS
//! receive loops and routes incoming packets to a message handler.

use anyhow::{Context, Result};
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

/// Configuration for transport listeners.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// UDP bind address
    pub udp_bind: Option<SocketAddr>,

    /// TCP bind address
    pub tcp_bind: Option<SocketAddr>,

    /// TLS bind address
    #[cfg(feature = "tls")]
    pub tls_bind: Option<SocketAddr>,

    /// TLS certificate path (for TLS server)
    #[cfg(feature = "tls")]
    pub tls_cert: Option<String>,

    /// TLS private key path (for TLS server)
    #[cfg(feature = "tls")]
    pub tls_key: Option<String>,
}

/// Handler trait for processing incoming SIP messages.
#[async_trait::async_trait]
pub trait InboundMessageHandler: Send + Sync {
    /// Handle an inbound SIP message.
    ///
    /// The handler should parse the message, route it to the appropriate
    /// component (transaction manager, dialog manager, etc.), and send
    /// any necessary responses.
    async fn handle_inbound(
        &self,
        transport: sip_transaction::TransportKind,
        peer: SocketAddr,
        payload: Bytes,
        stream: Option<mpsc::Sender<Bytes>>,
    ) -> Result<()>;
}

/// Manages transport listeners for receiving SIP messages.
pub struct TransportListener {
    /// Configuration
    config: ListenerConfig,

    /// Message handler
    handler: Arc<dyn InboundMessageHandler>,

    /// UDP listener task
    udp_task: Option<JoinHandle<()>>,

    /// TCP listener task
    tcp_task: Option<JoinHandle<()>>,

    /// TLS listener task
    #[cfg(feature = "tls")]
    tls_task: Option<JoinHandle<()>>,

    /// Shutdown signal sender (broadcast to all listeners)
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl TransportListener {
    /// Create a new transport listener.
    pub fn new(config: ListenerConfig, handler: Arc<dyn InboundMessageHandler>) -> Self {
        Self {
            config,
            handler,
            udp_task: None,
            tcp_task: None,
            #[cfg(feature = "tls")]
            tls_task: None,
            shutdown_tx: None,
        }
    }

    /// Start all configured transport listeners.
    pub async fn start(&mut self) -> Result<()> {
        use tokio::sync::broadcast;
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_rx = shutdown_tx.subscribe();
        // Store the broadcast sender to keep it alive - this prevents premature shutdown
        self.shutdown_tx = Some(shutdown_tx);

        // Start UDP listener
        if let Some(udp_bind) = self.config.udp_bind {
            info!(%udp_bind, "Starting UDP listener");

            // Create socket with SO_REUSEADDR to allow port reuse
            let socket = {
                use socket2::{Socket, Domain, Type, Protocol};

                let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
                    .context("Failed to create socket")?;
                socket.set_reuse_address(true)
                    .context("Failed to set SO_REUSEADDR")?;
                socket.set_nonblocking(true)
                    .context("Failed to set non-blocking")?;
                socket.bind(&udp_bind.into())
                    .context("Failed to bind UDP socket")?;

                let std_socket: std::net::UdpSocket = socket.into();
                UdpSocket::from_std(std_socket)
                    .context("Failed to convert to tokio UdpSocket")?
            };
            let socket = Arc::new(socket);

            let handler = Arc::clone(&self.handler);
            let mut shutdown = shutdown_rx.resubscribe();

            let task = tokio::spawn(async move {
                if let Err(e) = Self::run_udp_listener(socket, handler, &mut shutdown).await {
                    error!("UDP listener error: {}", e);
                }
            });

            self.udp_task = Some(task);
        }

        // Start TCP listener
        if let Some(tcp_bind) = self.config.tcp_bind {
            info!(%tcp_bind, "Starting TCP listener");

            let handler = Arc::clone(&self.handler);
            let mut shutdown = shutdown_rx.resubscribe();

            let task = tokio::spawn(async move {
                if let Err(e) = Self::run_tcp_listener(tcp_bind, handler, &mut shutdown).await {
                    error!("TCP listener error: {}", e);
                }
            });

            self.tcp_task = Some(task);
        }

        // Start TLS listener
        #[cfg(feature = "tls")]
        if let Some(tls_bind) = self.config.tls_bind {
            if let (Some(cert_path), Some(key_path)) =
                (&self.config.tls_cert, &self.config.tls_key)
            {
                info!(%tls_bind, "Starting TLS listener");

                // Load TLS server config
                let tls_config = sip_transport::load_rustls_server_config(cert_path, key_path)
                    .context("Failed to load TLS server config")?;

                let handler = Arc::clone(&self.handler);
                let mut shutdown = shutdown_rx.resubscribe();

                let task = tokio::spawn(async move {
                    if let Err(e) = Self::run_tls_listener(
                        tls_bind,
                        tls_config,
                        handler,
                        &mut shutdown
                    ).await {
                        error!("TLS listener error: {}", e);
                    }
                });

                self.tls_task = Some(task);
            } else {
                error!("TLS bind address configured but missing cert/key paths");
            }
        }

        Ok(())
    }

    /// Run UDP listener loop.
    async fn run_udp_listener(
        socket: Arc<UdpSocket>,
        handler: Arc<dyn InboundMessageHandler>,
        shutdown: &mut tokio::sync::broadcast::Receiver<()>,
    ) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<sip_transport::InboundPacket>(1000);

        // Spawn siphon-rs UDP receive loop
        let socket_clone = Arc::clone(&socket);
        tokio::spawn(async move {
            if let Err(e) = sip_transport::run_udp(socket_clone, tx).await {
                error!("UDP receive loop error: {}", e);
            }
        });

        // Process incoming packets
        loop {
            tokio::select! {
                Some(packet) = rx.recv() => {
                    debug!(
                        transport = ?packet.transport,
                        peer = %packet.peer,
                        size = packet.payload.len(),
                        "Received inbound packet"
                    );

                    // Convert sip_transport::TransportKind to sip_transaction::TransportKind
                    let txn_transport = convert_transport_kind(packet.transport);

                    // IMPORTANT: Spawn message handling in separate task to avoid blocking
                    // This allows the listener to immediately process the next packet,
                    // which is critical for B2BUA operation where responses must be
                    // processed while waiting for B-leg responses
                    let handler = Arc::clone(&handler);
                    tokio::spawn(async move {
                        if let Err(e) = handler.handle_inbound(
                            txn_transport,
                            packet.peer,
                            packet.payload,
                            packet.stream,
                        ).await {
                            error!("Failed to handle inbound message: {}", e);
                        }
                    });
                }
                _ = shutdown.recv() => {
                    info!("UDP listener shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Run TCP listener loop.
    async fn run_tcp_listener(
        bind_addr: SocketAddr,
        handler: Arc<dyn InboundMessageHandler>,
        shutdown: &mut tokio::sync::broadcast::Receiver<()>,
    ) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<sip_transport::InboundPacket>(1000);

        // Spawn siphon-rs TCP accept loop
        let bind_str = bind_addr.to_string();
        tokio::spawn(async move {
            if let Err(e) = sip_transport::run_tcp(&bind_str, tx).await {
                error!("TCP accept loop error: {}", e);
            }
        });

        // Process incoming packets
        loop {
            tokio::select! {
                Some(packet) = rx.recv() => {
                    debug!(
                        transport = ?packet.transport,
                        peer = %packet.peer,
                        size = packet.payload.len(),
                        "Received TCP packet"
                    );

                    let txn_transport = convert_transport_kind(packet.transport);

                    // IMPORTANT: Spawn message handling in separate task to avoid blocking
                    let handler = Arc::clone(&handler);
                    tokio::spawn(async move {
                        if let Err(e) = handler.handle_inbound(
                            txn_transport,
                            packet.peer,
                            packet.payload,
                            packet.stream,
                        ).await {
                            error!("Failed to handle TCP message: {}", e);
                        }
                    });
                }
                _ = shutdown.recv() => {
                    info!("TCP listener shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Run TLS listener loop.
    #[cfg(feature = "tls")]
    async fn run_tls_listener(
        bind_addr: SocketAddr,
        tls_config: Arc<tokio_rustls::rustls::ServerConfig>,
        handler: Arc<dyn InboundMessageHandler>,
        shutdown: &mut tokio::sync::broadcast::Receiver<()>,
    ) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<sip_transport::InboundPacket>(1000);

        // Spawn siphon-rs TLS accept loop
        let bind_str = bind_addr.to_string();
        tokio::spawn(async move {
            if let Err(e) = sip_transport::run_tls(&bind_str, tls_config, tx).await {
                error!("TLS accept loop error: {}", e);
            }
        });

        // Process incoming packets
        loop {
            tokio::select! {
                Some(packet) = rx.recv() => {
                    debug!(
                        transport = ?packet.transport,
                        peer = %packet.peer,
                        size = packet.payload.len(),
                        "Received TLS packet"
                    );

                    let txn_transport = convert_transport_kind(packet.transport);

                    if let Err(e) = handler.handle_inbound(
                        txn_transport,
                        packet.peer,
                        packet.payload,
                        packet.stream,
                    ).await {
                        error!("Failed to handle TLS message: {}", e);
                    }
                }
                _ = shutdown.recv() => {
                    info!("TLS listener shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Stop all listeners.
    pub async fn stop(&mut self) -> Result<()> {
        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Wait for tasks to complete
        if let Some(task) = self.udp_task.take() {
            let _ = task.await;
        }

        if let Some(task) = self.tcp_task.take() {
            let _ = task.await;
        }

        #[cfg(feature = "tls")]
        if let Some(task) = self.tls_task.take() {
            let _ = task.await;
        }

        info!("All transport listeners stopped");
        Ok(())
    }
}

/// Convert sip_transport::TransportKind to sip_transaction::TransportKind.
fn convert_transport_kind(
    transport: sip_transport::TransportKind,
) -> sip_transaction::TransportKind {
    use sip_transport::TransportKind as SipT;
    use sip_transaction::TransportKind as TxnT;

    match transport {
        SipT::Udp => TxnT::Udp,
        SipT::Tcp => TxnT::Tcp,
        SipT::Tls => TxnT::Tls,
        SipT::Sctp => TxnT::Sctp,
        SipT::TlsSctp => TxnT::TlsSctp,
        SipT::Ws => TxnT::Ws,
        SipT::Wss => TxnT::Wss,
    }
}
