use crate::Result;
use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;

/// Transport protocol supported by the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportType {
    Udp,
    Tcp,
    Tls,
    Ws,
    Wss,
}

impl TransportType {
    /// Check if this transport is connection-oriented and reliable.
    pub fn is_reliable(self) -> bool {
        !matches!(self, TransportType::Udp)
    }

    /// Check if this transport uses TLS/encryption.
    pub fn is_secure(self) -> bool {
        matches!(self, TransportType::Tls | TransportType::Wss)
    }

    /// Check if this is a WebSocket transport.
    pub fn is_websocket(self) -> bool {
        matches!(self, TransportType::Ws | TransportType::Wss)
    }

    /// Get the default port for this transport.
    pub fn default_port(self) -> u16 {
        match self {
            TransportType::Udp | TransportType::Tcp => 5060,
            TransportType::Tls => 5061,
            TransportType::Ws => 80,
            TransportType::Wss => 443,
        }
    }

    /// Get the transport as a string (lowercase).
    pub fn as_str(self) -> &'static str {
        match self {
            TransportType::Udp => "udp",
            TransportType::Tcp => "tcp",
            TransportType::Tls => "tls",
            TransportType::Ws => "ws",
            TransportType::Wss => "wss",
        }
    }
}

impl fmt::Display for TransportType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Message envelope for passing SIP messages through the transport layer.
#[derive(Debug, Clone)]
pub struct TransportMessage<T> {
    /// The message payload
    pub payload: T,

    /// Remote peer address
    pub remote: SocketAddr,

    /// Local address where message was received/will be sent
    pub local: SocketAddr,

    /// Transport protocol used
    pub transport: TransportType,
}

impl<T> TransportMessage<T> {
    /// Create a new transport message.
    pub fn new(payload: T, remote: SocketAddr, local: SocketAddr, transport: TransportType) -> Self {
        Self {
            payload,
            remote,
            local,
            transport,
        }
    }

    /// Map the payload to a different type.
    pub fn map<U, F>(self, f: F) -> TransportMessage<U>
    where
        F: FnOnce(T) -> U,
    {
        TransportMessage {
            payload: f(self.payload),
            remote: self.remote,
            local: self.local,
            transport: self.transport,
        }
    }
}

/// Trait for sending messages over a transport.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a message to a remote peer.
    async fn send(&self, data: Bytes, remote: SocketAddr) -> Result<()>;

    /// Get the local bind address.
    fn local_addr(&self) -> SocketAddr;

    /// Get the transport type.
    fn transport_type(&self) -> TransportType;

    /// Check if this transport is still active.
    fn is_active(&self) -> bool {
        true
    }
}

/// Trait for receiving messages from a transport.
#[async_trait]
pub trait TransportReceiver: Send + Sync {
    /// Receive the next message from this transport.
    async fn recv(&mut self) -> Result<TransportMessage<Bytes>>;
}

/// Connection handle for connection-oriented transports (TCP, TLS, WS, WSS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(u64);

impl ConnectionId {
    /// Create a new connection ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the inner ID value.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "conn-{}", self.0)
    }
}

/// Connection state for connection-oriented transports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Connection is being established
    Connecting,

    /// Connection is active and ready
    Connected,

    /// Connection is closing gracefully
    Closing,

    /// Connection is closed
    Closed,

    /// Connection failed
    Failed,
}

/// Connection metadata for monitoring and debugging.
#[derive(Debug, Clone)]
pub struct ConnectionMetadata {
    /// Unique connection identifier
    pub id: ConnectionId,

    /// Local address
    pub local: SocketAddr,

    /// Remote peer address
    pub remote: SocketAddr,

    /// Transport type
    pub transport: TransportType,

    /// Connection state
    pub state: ConnectionState,

    /// Timestamp when connection was established
    pub established_at: std::time::Instant,

    /// Number of messages sent over this connection
    pub messages_sent: u64,

    /// Number of messages received over this connection
    pub messages_received: u64,

    /// Total bytes sent
    pub bytes_sent: u64,

    /// Total bytes received
    pub bytes_received: u64,
}
