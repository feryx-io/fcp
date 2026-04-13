//! Transport layer initialization for fcp-registrar.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use sip_transaction::{TransportContext, TransportDispatcher, TransportKind};
use sip_transport::{
    pool::ConnectionPool, run_tcp, run_tls, run_udp, send_stream, send_udp, DefaultTransportPolicy,
    InboundPacket, TransportPolicy,
};
use std::sync::Arc;
use tokio::{net::UdpSocket, sync::mpsc};
use tokio_rustls::rustls::ServerConfig;
use tracing::info;

#[derive(Debug, Clone)]
pub struct RegistrarTransportConfig {
    pub udp_bind: String,
    pub tcp_bind: String,
    pub tls_bind: Option<String>,
    pub ws_bind: Option<String>,
    pub wss_bind: Option<String>,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
}

/// Start the configured SIP transport layers.
///
/// Returns the transport dispatcher and UDP socket reference.
pub async fn start_transports(
    config: &RegistrarTransportConfig,
    tx: mpsc::Sender<InboundPacket>,
) -> Result<(Arc<dyn TransportDispatcher>, Arc<UdpSocket>)> {
    let udp_socket = Arc::new(UdpSocket::bind(&config.udp_bind).await?);
    let recv_socket = Arc::clone(&udp_socket);

    let tcp_pool = Arc::new(ConnectionPool::new());
    let tls_client_config = fcp_sip::transport::RealTransportDispatcher::load_tls_client_config()
        .context("failed to load TLS client configuration for registrar")?;

    let dispatcher = Arc::new(RegistrarTransportDispatcher::new(
        Arc::clone(&udp_socket),
        Arc::new(DefaultTransportPolicy::default()),
        tcp_pool,
        tls_client_config,
    ));

    tokio::spawn({
        let tx = tx.clone();
        async move {
            if let Err(e) = run_udp(recv_socket, tx).await {
                tracing::error!(%e, "UDP listener exited");
            }
        }
    });

    tokio::spawn({
        let bind = config.tcp_bind.clone();
        let tx = tx.clone();
        async move {
            if let Err(e) = run_tcp(&bind, tx).await {
                tracing::error!(%e, "TCP listener exited");
            }
        }
    });

    if let Some(ref bind) = config.tls_bind {
        let server_config = load_tls_server_config(
            config.tls_cert_path.as_deref(),
            config.tls_key_path.as_deref(),
        )
        .context("failed to load TLS server configuration for registrar")?;
        let bind = bind.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = run_tls(&bind, server_config, tx).await {
                tracing::error!(%e, "TLS listener exited");
            }
        });
    }

    if let Some(ref bind) = config.ws_bind {
        let bind = bind.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = sip_transport::run_ws(&bind, tx).await {
                tracing::error!(%e, "WS listener exited");
            }
        });
    }

    if let Some(ref bind) = config.wss_bind {
        let server_config = load_tls_server_config(
            config.tls_cert_path.as_deref(),
            config.tls_key_path.as_deref(),
        )
        .context("failed to load WSS server configuration for registrar")?;
        let bind = bind.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = sip_transport::run_wss(&bind, server_config, tx).await {
                tracing::error!(%e, "WSS listener exited");
            }
        });
    }

    info!(
        udp = %config.udp_bind,
        tcp = %config.tcp_bind,
        tls = ?config.tls_bind,
        ws = ?config.ws_bind,
        wss = ?config.wss_bind,
        "SIP transports started"
    );

    Ok((dispatcher, udp_socket))
}

struct RegistrarTransportDispatcher {
    udp_socket: Arc<UdpSocket>,
    policy: Arc<dyn TransportPolicy>,
    tcp_pool: Arc<ConnectionPool>,
    tls_client_config: Arc<tokio_rustls::rustls::ClientConfig>,
}

impl RegistrarTransportDispatcher {
    fn new(
        udp_socket: Arc<UdpSocket>,
        policy: Arc<dyn TransportPolicy>,
        tcp_pool: Arc<ConnectionPool>,
        tls_client_config: Arc<tokio_rustls::rustls::ClientConfig>,
    ) -> Self {
        Self {
            udp_socket,
            policy,
            tcp_pool,
            tls_client_config,
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
            sip_transport::TransportKind::Tls => {
                if let Some(writer) = &ctx.stream() {
                    send_stream(selected, writer, payload).await?;
                } else {
                    let server_name = ctx
                        .server_name()
                        .map(str::to_owned)
                        .unwrap_or_else(|| ctx.peer().ip().to_string());
                    let tls_config = sip_transport::TlsConfig::new(
                        server_name,
                        Arc::clone(&self.tls_client_config),
                    );
                    sip_transport::send_tls(&ctx.peer(), &payload, &tls_config).await?;
                }
            }
            sip_transport::TransportKind::Ws | sip_transport::TransportKind::Wss => {
                if let Some(writer) = &ctx.stream() {
                    send_stream(selected, writer, payload).await?;
                } else {
                    let secure = matches!(selected, sip_transport::TransportKind::Wss);
                    let url = ctx
                        .ws_uri()
                        .map(str::to_owned)
                        .unwrap_or_else(|| fcp_sip::transport::websocket_url(ctx, secure));
                    self.tcp_pool.send_ws(&url, payload).await?;
                }
            }
            other => {
                return Err(anyhow!("Transport {:?} not supported by registrar", other));
            }
        }

        Ok(())
    }
}

fn load_tls_server_config(
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> Result<Arc<ServerConfig>> {
    let cert_path = cert_path.ok_or_else(|| anyhow!("TLS certificate path is required"))?;
    let key_path = key_path.ok_or_else(|| anyhow!("TLS private key path is required"))?;
    sip_transport::load_rustls_server_config(cert_path, key_path)
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
