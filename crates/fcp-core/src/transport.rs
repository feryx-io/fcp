use std::net::SocketAddr;

/// Transport protocol supported by the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    Udp,
    Tcp,
    Tls,
    Ws,
    Wss,
}

impl TransportType {
    pub fn is_reliable(self) -> bool {
        !matches!(self, TransportType::Udp)
    }
}

/// Basic envelope used when passing SIP messages through the transport layer.
#[derive(Debug, Clone)]
pub struct TransportMessage<T> {
    pub payload: T,
    pub remote: SocketAddr,
    pub local: SocketAddr,
    pub transport: TransportType,
}
