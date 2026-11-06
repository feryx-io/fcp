//! Transport listeners for receiving inbound SIP messages.
//!
//! This module provides a `TransportListener` that manages UDP, TCP, TLS, WS,
//! and WSS receive loops and routes incoming packets to a message handler.

use anyhow::{Context, Result};
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use crate::lanes::CallLanes;

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

    /// PEM bundle of the CA(s) a client certificate on the TLS and WSS
    /// listeners must chain to (mutual TLS). `None` never asks a peer
    /// for a certificate. A bundle that fails to load is a startup
    /// error, never a fallback to an unauthenticated listener.
    #[cfg(feature = "tls")]
    pub tls_client_ca: Option<String>,

    /// With `tls_client_ca`: whether a peer that presents no certificate
    /// is admitted with no identity (`Optional`) or refused at the
    /// handshake (`Required`, the default).
    #[cfg(feature = "tls")]
    pub tls_client_auth: Option<sip_transport::ClientAuthMode>,

    /// WebSocket bind address
    pub ws_bind: Option<SocketAddr>,

    /// Secure WebSocket bind address
    pub wss_bind: Option<SocketAddr>,
}

/// Handler trait for processing incoming SIP messages.
#[async_trait::async_trait]
pub trait InboundMessageHandler: Send + Sync {
    /// Handle an inbound SIP message.
    ///
    /// The handler should parse the message, route it to the appropriate
    /// component (transaction manager, dialog manager, etc.), and send
    /// any necessary responses.
    ///
    /// `peer_identity` is the verified certificate identity of the
    /// connection the message arrived on (mutual TLS), `None` on any
    /// other transport or when the peer presented no certificate.
    async fn handle_inbound(
        &self,
        transport: sip_transaction::TransportKind,
        peer: SocketAddr,
        payload: Bytes,
        stream: Option<mpsc::Sender<Bytes>>,
        peer_identity: Option<Arc<sip_transport::PeerIdentity>>,
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

    /// WS listener task
    #[cfg(feature = "ws")]
    ws_task: Option<JoinHandle<()>>,

    /// WSS listener task
    #[cfg(all(feature = "ws", feature = "tls"))]
    wss_task: Option<JoinHandle<()>>,

    /// Each call's messages, handled in the order they arrived
    /// (`crate::lanes`), whichever transport they came on.
    lanes: CallLanes,

    /// Shutdown signal sender (broadcast to all listeners)
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl TransportListener {
    /// Create a new transport listener.
    pub fn new(config: ListenerConfig, handler: Arc<dyn InboundMessageHandler>) -> Self {
        Self {
            config,
            handler,
            lanes: CallLanes::default(),
            udp_task: None,
            tcp_task: None,
            #[cfg(feature = "tls")]
            tls_task: None,
            #[cfg(feature = "ws")]
            ws_task: None,
            #[cfg(all(feature = "ws", feature = "tls"))]
            wss_task: None,
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
                use socket2::{Domain, Protocol, Socket, Type};

                let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
                    .context("Failed to create socket")?;
                socket
                    .set_reuse_address(true)
                    .context("Failed to set SO_REUSEADDR")?;
                socket
                    .set_nonblocking(true)
                    .context("Failed to set non-blocking")?;
                socket
                    .bind(&udp_bind.into())
                    .context("Failed to bind UDP socket")?;

                let std_socket: std::net::UdpSocket = socket.into();
                UdpSocket::from_std(std_socket).context("Failed to convert to tokio UdpSocket")?
            };
            let socket = Arc::new(socket);

            let handler = Arc::clone(&self.handler);

            let lanes = self.lanes.clone();
            let mut shutdown = shutdown_rx.resubscribe();

            let task = tokio::spawn(async move {
                if let Err(e) = Self::run_udp_listener(socket, handler, lanes, &mut shutdown).await
                {
                    error!("UDP listener error: {}", e);
                }
            });

            self.udp_task = Some(task);
        }

        // Start TCP listener
        if let Some(tcp_bind) = self.config.tcp_bind {
            info!(%tcp_bind, "Starting TCP listener");

            let handler = Arc::clone(&self.handler);

            let lanes = self.lanes.clone();
            let mut shutdown = shutdown_rx.resubscribe();

            let task = tokio::spawn(async move {
                if let Err(e) =
                    Self::run_tcp_listener(tcp_bind, handler, lanes, &mut shutdown).await
                {
                    error!("TCP listener error: {}", e);
                }
            });

            self.tcp_task = Some(task);
        }

        // Start TLS listener
        #[cfg(feature = "tls")]
        if let Some(tls_bind) = self.config.tls_bind {
            if let (Some(cert_path), Some(key_path)) = (&self.config.tls_cert, &self.config.tls_key)
            {
                info!(%tls_bind, "Starting TLS listener");

                let tls_config = self.load_server_config(cert_path, key_path, "TLS")?;

                let handler = Arc::clone(&self.handler);

                let lanes = self.lanes.clone();
                let mut shutdown = shutdown_rx.resubscribe();

                let task = tokio::spawn(async move {
                    if let Err(e) =
                        Self::run_tls_listener(tls_bind, tls_config, handler, lanes, &mut shutdown)
                            .await
                    {
                        error!("TLS listener error: {}", e);
                    }
                });

                self.tls_task = Some(task);
            } else {
                error!("TLS bind address configured but missing cert/key paths");
            }
        }

        // Start WS listener
        #[cfg(feature = "ws")]
        if let Some(ws_bind) = self.config.ws_bind {
            info!(%ws_bind, "Starting WS listener");

            let handler = Arc::clone(&self.handler);

            let lanes = self.lanes.clone();
            let mut shutdown = shutdown_rx.resubscribe();

            let task = tokio::spawn(async move {
                if let Err(e) = Self::run_ws_listener(ws_bind, handler, lanes, &mut shutdown).await
                {
                    error!("WS listener error: {}", e);
                }
            });

            self.ws_task = Some(task);
        }

        // Start WSS listener
        #[cfg(all(feature = "ws", feature = "tls"))]
        if let Some(wss_bind) = self.config.wss_bind {
            if let (Some(cert_path), Some(key_path)) = (&self.config.tls_cert, &self.config.tls_key)
            {
                info!(%wss_bind, "Starting WSS listener");

                let tls_config = self.load_server_config(cert_path, key_path, "WSS")?;

                let handler = Arc::clone(&self.handler);

                let lanes = self.lanes.clone();
                let mut shutdown = shutdown_rx.resubscribe();

                let task = tokio::spawn(async move {
                    if let Err(e) =
                        Self::run_wss_listener(wss_bind, tls_config, handler, lanes, &mut shutdown)
                            .await
                    {
                        error!("WSS listener error: {}", e);
                    }
                });

                self.wss_task = Some(task);
            } else {
                error!("WSS bind address configured but missing cert/key paths");
            }
        }

        Ok(())
    }

    /// Run UDP listener loop.
    async fn run_udp_listener(
        socket: Arc<UdpSocket>,
        handler: Arc<dyn InboundMessageHandler>,
        lanes: CallLanes,
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
                        transport = ?packet.transport(),
                        peer = %packet.peer(),
                        size = packet.payload().len(),
                        "Received inbound packet"
                    );

                    dispatch(&lanes, &handler, packet, "UDP");
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
        lanes: CallLanes,
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
                        transport = ?packet.transport(),
                        peer = %packet.peer(),
                        size = packet.payload().len(),
                        "Received TCP packet"
                    );

                    dispatch(&lanes, &handler, packet, "TCP");
                }
                _ = shutdown.recv() => {
                    info!("TCP listener shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Build the rustls `ServerConfig` for a TLS-family listener: the
    /// listener's own certificate, plus client-certificate verification
    /// against `tls_client_ca` when configured (mutual TLS).
    #[cfg(feature = "tls")]
    fn load_server_config(
        &self,
        cert_path: &str,
        key_path: &str,
        what: &str,
    ) -> Result<Arc<tokio_rustls::rustls::ServerConfig>> {
        crate::transport::ensure_crypto_provider();
        match &self.config.tls_client_ca {
            Some(client_ca) => {
                let mode = self
                    .config
                    .tls_client_auth
                    .unwrap_or(sip_transport::ClientAuthMode::Required);
                info!(client_ca = %client_ca, %mode, "{what} listener verifies client certificates (mutual TLS)");
                sip_transport::load_rustls_server_config_with_client_auth(
                    cert_path, key_path, client_ca, mode,
                )
                .with_context(|| format!("Failed to load {what} server config with client auth"))
            }
            None => sip_transport::load_rustls_server_config(cert_path, key_path)
                .with_context(|| format!("Failed to load {what} server config")),
        }
    }

    /// Run TLS listener loop.
    #[cfg(feature = "tls")]
    async fn run_tls_listener(
        bind_addr: SocketAddr,
        tls_config: Arc<tokio_rustls::rustls::ServerConfig>,
        handler: Arc<dyn InboundMessageHandler>,
        lanes: CallLanes,
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
                        transport = ?packet.transport(),
                        peer = %packet.peer(),
                        size = packet.payload().len(),
                        "Received TLS packet"
                    );

                    dispatch(&lanes, &handler, packet, "TLS");
                }
                _ = shutdown.recv() => {
                    info!("TLS listener shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Run WS listener loop.
    #[cfg(feature = "ws")]
    async fn run_ws_listener(
        bind_addr: SocketAddr,
        handler: Arc<dyn InboundMessageHandler>,
        lanes: CallLanes,
        shutdown: &mut tokio::sync::broadcast::Receiver<()>,
    ) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<sip_transport::InboundPacket>(1000);

        let bind_str = bind_addr.to_string();
        tokio::spawn(async move {
            if let Err(e) = sip_transport::run_ws(&bind_str, tx).await {
                error!("WS accept loop error: {}", e);
            }
        });

        loop {
            tokio::select! {
                Some(packet) = rx.recv() => {
                    debug!(
                        transport = ?packet.transport(),
                        peer = %packet.peer(),
                        size = packet.payload().len(),
                        "Received WS packet"
                    );

                    dispatch(&lanes, &handler, packet, "WS");
                }
                _ = shutdown.recv() => {
                    info!("WS listener shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Run WSS listener loop.
    #[cfg(all(feature = "ws", feature = "tls"))]
    async fn run_wss_listener(
        bind_addr: SocketAddr,
        tls_config: Arc<tokio_rustls::rustls::ServerConfig>,
        handler: Arc<dyn InboundMessageHandler>,
        lanes: CallLanes,
        shutdown: &mut tokio::sync::broadcast::Receiver<()>,
    ) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<sip_transport::InboundPacket>(1000);

        let bind_str = bind_addr.to_string();
        tokio::spawn(async move {
            if let Err(e) = sip_transport::run_wss(&bind_str, tls_config, tx).await {
                error!("WSS accept loop error: {}", e);
            }
        });

        loop {
            tokio::select! {
                Some(packet) = rx.recv() => {
                    debug!(
                        transport = ?packet.transport(),
                        peer = %packet.peer(),
                        size = packet.payload().len(),
                        "Received WSS packet"
                    );

                    dispatch(&lanes, &handler, packet, "WSS");
                }
                _ = shutdown.recv() => {
                    info!("WSS listener shutting down");
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

        #[cfg(feature = "ws")]
        if let Some(task) = self.ws_task.take() {
            let _ = task.await;
        }

        #[cfg(all(feature = "ws", feature = "tls"))]
        if let Some(task) = self.wss_task.take() {
            let _ = task.await;
        }

        info!("All transport listeners stopped");
        Ok(())
    }
}

/// Hand a packet to the handler on a task of its own, after the message
/// before it in its call (`crate::lanes`): the listener goes straight on to
/// the next packet, which a B2BUA needs to take a B-leg's responses while a
/// request waits on them.
fn dispatch(
    lanes: &CallLanes,
    handler: &Arc<dyn InboundMessageHandler>,
    packet: sip_transport::InboundPacket,
    what: &'static str,
) {
    let transport = convert_transport_kind(packet.transport());
    let handler = Arc::clone(handler);
    let payload = packet.payload().clone();
    lanes.dispatch(&payload.clone(), async move {
        if let Err(e) = handler
            .handle_inbound(
                transport,
                packet.peer(),
                payload,
                packet.stream().cloned(),
                packet.peer_identity().cloned(),
            )
            .await
        {
            error!("Failed to handle {what} message: {}", e);
        }
    });
}

/// Convert sip_transport::TransportKind to sip_transaction::TransportKind.
fn convert_transport_kind(
    transport: sip_transport::TransportKind,
) -> sip_transaction::TransportKind {
    use sip_transaction::TransportKind as TxnT;
    use sip_transport::TransportKind as SipT;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    /// Records each message's call and number as it is handled; the first
    /// message of every call takes a while.
    struct Recorder {
        handled: Mutex<Vec<(String, u32)>>,
    }

    #[async_trait::async_trait]
    impl InboundMessageHandler for Recorder {
        async fn handle_inbound(
            &self,
            _transport: sip_transaction::TransportKind,
            _peer: SocketAddr,
            payload: Bytes,
            _stream: Option<mpsc::Sender<Bytes>>,
            _peer_identity: Option<Arc<sip_transport::PeerIdentity>>,
        ) -> Result<()> {
            let text = String::from_utf8_lossy(&payload).to_string();
            let call_id = crate::lanes::call_id_of(&payload).expect("a Call-ID");
            let cseq: u32 = text
                .lines()
                .find_map(|line| line.strip_prefix("CSeq: "))
                .and_then(|cseq| cseq.split(' ').next())
                .and_then(|n| n.parse().ok())
                .expect("a CSeq");
            if cseq == 1 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            self.handled.lock().unwrap().push((call_id, cseq));
            Ok(())
        }
    }

    fn info(call: usize, cseq: u32) -> String {
        format!(
            "INFO sip:node@127.0.0.1 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5999;branch=z9hG4bK-{call}-{cseq}\r\n\
             From: <sip:a@x>;tag=a{call}\r\n\
             To: <sip:b@x>;tag=b{call}\r\n\
             Call-ID: call-{call}@test\r\n\
             CSeq: {cseq} INFO\r\n\
             Max-Forwards: 70\r\n\
             Content-Length: 0\r\n\r\n"
        )
    }

    fn free_port() -> SocketAddr {
        std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .expect("a free port")
    }

    fn config(udp_bind: Option<SocketAddr>, tcp_bind: Option<SocketAddr>) -> ListenerConfig {
        ListenerConfig {
            udp_bind,
            tcp_bind,
            #[cfg(feature = "tls")]
            tls_bind: None,
            #[cfg(feature = "tls")]
            tls_cert: None,
            #[cfg(feature = "tls")]
            tls_key: None,
            #[cfg(feature = "tls")]
            tls_client_ca: None,
            #[cfg(feature = "tls")]
            tls_client_auth: None,
            ws_bind: None,
            wss_bind: None,
        }
    }

    const CALLS: usize = 20;
    const PER_CALL: u32 = 4;

    /// Every call's messages were handled in the order they were sent,
    /// though each call's first took longer than the rest (#313).
    async fn handled_in_order(recorder: &Recorder) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while recorder.handled.lock().unwrap().len() < CALLS * PER_CALL as usize {
            assert!(
                tokio::time::Instant::now() < deadline,
                "handled {} of {} messages",
                recorder.handled.lock().unwrap().len(),
                CALLS * PER_CALL as usize
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let handled = recorder.handled.lock().unwrap();
        for call in 0..CALLS {
            let call_id = format!("call-{call}@test");
            let order: Vec<u32> = handled
                .iter()
                .filter(|(id, _)| *id == call_id)
                .map(|(_, cseq)| *cseq)
                .collect();
            assert_eq!(order, (1..=PER_CALL).collect::<Vec<_>>(), "{call_id}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_calls_messages_over_udp_are_handled_in_the_order_they_came() {
        let bind = free_port();
        let recorder = Arc::new(Recorder {
            handled: Mutex::new(Vec::new()),
        });
        let mut listener = TransportListener::new(config(Some(bind), None), recorder.clone());
        listener.start().await.expect("the listener starts");
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        for call in 0..CALLS {
            for cseq in 1..=PER_CALL {
                socket
                    .send_to(info(call, cseq).as_bytes(), bind)
                    .await
                    .unwrap();
            }
        }
        handled_in_order(&recorder).await;
        listener.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_calls_messages_over_tcp_are_handled_in_the_order_they_came() {
        let bind = free_port();
        let recorder = Arc::new(Recorder {
            handled: Mutex::new(Vec::new()),
        });
        let mut listener = TransportListener::new(config(None, Some(bind)), recorder.clone());
        listener.start().await.expect("the listener starts");
        let mut stream = loop {
            match tokio::net::TcpStream::connect(bind).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        };
        let mut all = String::new();
        for call in 0..CALLS {
            for cseq in 1..=PER_CALL {
                all.push_str(&info(call, cseq));
            }
        }
        stream.write_all(all.as_bytes()).await.unwrap();
        handled_in_order(&recorder).await;
        listener.stop().await.unwrap();
    }
}
