//! Transport layer initialization for fcp-registrar.
//!
//! Simplified from siphond's transport.rs — only UDP and TCP, no TLS/WS.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::Bytes;
use sip_transaction::{TransportContext, TransportDispatcher, TransportKind};
use sip_transport::{
    pool::ConnectionPool, run_tcp, run_udp, send_stream, send_udp, DefaultTransportPolicy,
    InboundPacket, TransportPolicy,
};
use std::sync::Arc;
use tokio::{net::UdpSocket, sync::mpsc};
use tracing::info;

/// Start UDP and TCP transport layers.
///
/// Returns the transport dispatcher and UDP socket reference.
pub async fn start_transports(
    udp_bind: &str,
    tcp_bind: &str,
    tx: mpsc::Sender<InboundPacket>,
) -> Result<(Arc<dyn TransportDispatcher>, Arc<UdpSocket>)> {
    let udp_socket = Arc::new(UdpSocket::bind(udp_bind).await?);
    let recv_socket = Arc::clone(&udp_socket);

    let tcp_pool = Arc::new(ConnectionPool::new());

    let dispatcher = Arc::new(RegistrarTransportDispatcher::new(
        Arc::clone(&udp_socket),
        Arc::new(DefaultTransportPolicy::default()),
        tcp_pool,
    ));

    // Spawn UDP listener
    tokio::spawn({
        let tx = tx.clone();
        async move {
            if let Err(e) = run_udp(recv_socket, tx).await {
                tracing::error!(%e, "UDP listener exited");
            }
        }
    });

    // Spawn TCP listener
    tokio::spawn({
        let bind = tcp_bind.to_string();
        let tx = tx.clone();
        async move {
            if let Err(e) = run_tcp(&bind, tx).await {
                tracing::error!(%e, "TCP listener exited");
            }
        }
    });

    info!(udp = %udp_bind, tcp = %tcp_bind, "SIP transports started");
    Ok((dispatcher, udp_socket))
}

/// Transport dispatcher for the registrar (UDP + TCP only).
struct RegistrarTransportDispatcher {
    udp_socket: Arc<UdpSocket>,
    policy: Arc<dyn TransportPolicy>,
    tcp_pool: Arc<ConnectionPool>,
}

impl RegistrarTransportDispatcher {
    fn new(
        udp_socket: Arc<UdpSocket>,
        policy: Arc<dyn TransportPolicy>,
        tcp_pool: Arc<ConnectionPool>,
    ) -> Self {
        Self {
            udp_socket,
            policy,
            tcp_pool,
        }
    }
}

#[async_trait]
impl TransportDispatcher for RegistrarTransportDispatcher {
    async fn dispatch(&self, ctx: &TransportContext, payload: Bytes) -> Result<()> {
        let desired = to_sip_transport(ctx.transport());
        let selected = self.policy.choose(desired, payload.len(), false);

        match selected {
            sip_transport::TransportKind::Udp => {
                send_udp(self.udp_socket.as_ref(), &ctx.peer(), &payload).await?;
            }
            sip_transport::TransportKind::Tcp => {
                if let Some(writer) = &ctx.stream() {
                    send_stream(selected, writer, payload).await?;
                } else {
                    self.tcp_pool.send_tcp(ctx.peer(), payload).await?;
                }
            }
            other => {
                return Err(anyhow!(
                    "Transport {:?} not supported by registrar",
                    other
                ));
            }
        }

        Ok(())
    }
}

fn to_sip_transport(kind: TransportKind) -> sip_transport::TransportKind {
    match kind {
        TransportKind::Udp => sip_transport::TransportKind::Udp,
        TransportKind::Tcp => sip_transport::TransportKind::Tcp,
        TransportKind::Tls => sip_transport::TransportKind::Tls,
        TransportKind::Ws => sip_transport::TransportKind::Ws,
        TransportKind::Wss => sip_transport::TransportKind::Wss,
        TransportKind::Sctp => sip_transport::TransportKind::Sctp,
        TransportKind::TlsSctp => sip_transport::TransportKind::TlsSctp,
    }
}

/// Map sip-transport TransportKind to sip-transaction TransportKind.
pub fn map_transport(kind: sip_transport::TransportKind) -> TransportKind {
    match kind {
        sip_transport::TransportKind::Udp => TransportKind::Udp,
        sip_transport::TransportKind::Tcp => TransportKind::Tcp,
        sip_transport::TransportKind::Tls => TransportKind::Tls,
        sip_transport::TransportKind::Sctp => TransportKind::Sctp,
        sip_transport::TransportKind::TlsSctp => TransportKind::TlsSctp,
        sip_transport::TransportKind::Ws => TransportKind::Ws,
        sip_transport::TransportKind::Wss => TransportKind::Wss,
    }
}
