use crate::Result;
use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    payload: T,

    /// Remote peer address
    remote: SocketAddr,

    /// Local address where message was received/will be sent
    local: SocketAddr,

    /// Transport protocol used
    transport: TransportType,
}

impl<T> TransportMessage<T> {
    /// Create a new transport message.
    pub fn new(
        payload: T,
        remote: SocketAddr,
        local: SocketAddr,
        transport: TransportType,
    ) -> Self {
        Self {
            payload,
            remote,
            local,
            transport,
        }
    }

    /// Borrow the message payload.
    pub fn payload(&self) -> &T {
        &self.payload
    }

    /// Consume and return the payload.
    pub fn into_payload(self) -> T {
        self.payload
    }

    /// Remote peer address.
    pub fn remote(&self) -> SocketAddr {
        self.remote
    }

    /// Local socket address.
    pub fn local(&self) -> SocketAddr {
        self.local
    }

    /// Transport used to deliver this message.
    pub fn transport(&self) -> TransportType {
        self.transport
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
    id: ConnectionId,

    /// Local address
    local: SocketAddr,

    /// Remote peer address
    remote: SocketAddr,

    /// Transport type
    transport: TransportType,

    /// Connection state
    state: ConnectionState,

    /// Timestamp when connection was established
    established_at: std::time::Instant,

    /// Number of messages sent over this connection
    messages_sent: u64,

    /// Number of messages received over this connection
    messages_received: u64,

    /// Total bytes sent
    bytes_sent: u64,

    /// Total bytes received
    bytes_received: u64,
}

impl ConnectionMetadata {
    /// Create a new metadata snapshot for a connection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ConnectionId,
        local: SocketAddr,
        remote: SocketAddr,
        transport: TransportType,
        state: ConnectionState,
        established_at: std::time::Instant,
        messages_sent: u64,
        messages_received: u64,
        bytes_sent: u64,
        bytes_received: u64,
    ) -> Self {
        Self {
            id,
            local,
            remote,
            transport,
            state,
            established_at,
            messages_sent,
            messages_received,
            bytes_sent,
            bytes_received,
        }
    }

    pub fn id(&self) -> ConnectionId {
        self.id
    }

    pub fn local(&self) -> SocketAddr {
        self.local
    }

    pub fn remote(&self) -> SocketAddr {
        self.remote
    }

    pub fn transport(&self) -> TransportType {
        self.transport
    }

    pub fn state(&self) -> ConnectionState {
        self.state
    }

    pub fn established_at(&self) -> std::time::Instant {
        self.established_at
    }

    pub fn messages_sent(&self) -> u64 {
        self.messages_sent
    }

    pub fn messages_received(&self) -> u64 {
        self.messages_received
    }

    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent
    }

    pub fn bytes_received(&self) -> u64 {
        self.bytes_received
    }

    pub fn set_state(&mut self, state: ConnectionState) {
        self.state = state;
    }

    pub fn record_sent(&mut self, bytes: u64) {
        self.messages_sent = self.messages_sent.saturating_add(1);
        self.bytes_sent = self.bytes_sent.saturating_add(bytes);
    }

    pub fn record_received(&mut self, bytes: u64) {
        self.messages_received = self.messages_received.saturating_add(1);
        self.bytes_received = self.bytes_received.saturating_add(bytes);
    }
}

/// Rate limiter for connection-based traffic control.
///
/// Uses a sliding window algorithm to track requests over time.
#[derive(Debug)]
pub struct RateLimiter {
    /// Maximum requests allowed per window
    max_requests: u32,

    /// Maximum bytes allowed per window
    max_bytes: u64,

    /// Time window for rate limiting
    window: Duration,

    /// Request timestamps (sliding window)
    request_times: Mutex<VecDeque<Instant>>,

    /// Byte counts with timestamps (sliding window)
    byte_window: Mutex<VecDeque<(Instant, u64)>>,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// # Arguments
    /// * `max_requests` - Maximum number of requests allowed per window
    /// * `max_bytes` - Maximum bytes allowed per window
    /// * `window` - Time window duration (e.g., 1 minute)
    pub fn new(max_requests: u32, max_bytes: u64, window: Duration) -> Self {
        Self {
            max_requests,
            max_bytes,
            window,
            request_times: Mutex::new(VecDeque::new()),
            byte_window: Mutex::new(VecDeque::new()),
        }
    }

    /// Create a default rate limiter with reasonable limits.
    ///
    /// Default: 100 requests/minute, 1MB/minute
    pub fn default_limits() -> Self {
        Self::new(100, 1_048_576, Duration::from_secs(60))
    }

    /// Check if a request is allowed (does not consume the limit).
    ///
    /// Returns `true` if the request would be allowed, `false` if rate limited.
    pub fn check_request(&self, bytes: u64) -> bool {
        let now = Instant::now();
        let cutoff = now - self.window;

        // Check request rate
        let mut request_times = self.request_times.lock().unwrap();

        // Remove old entries
        while let Some(&time) = request_times.front() {
            if time < cutoff {
                request_times.pop_front();
            } else {
                break;
            }
        }

        if request_times.len() >= self.max_requests as usize {
            return false;
        }

        // Check byte rate
        let mut byte_window = self.byte_window.lock().unwrap();

        // Remove old entries and sum remaining bytes
        let mut total_bytes = 0u64;
        while let Some(&(time, _)) = byte_window.front() {
            if time < cutoff {
                byte_window.pop_front();
            } else {
                break;
            }
        }

        for (_, b) in byte_window.iter() {
            total_bytes = total_bytes.saturating_add(*b);
        }

        total_bytes.saturating_add(bytes) <= self.max_bytes
    }

    /// Record a request (consumes the limit).
    ///
    /// Should only be called after `check_request()` returns `true`.
    pub fn record_request(&self, bytes: u64) {
        let now = Instant::now();

        let mut request_times = self.request_times.lock().unwrap();
        request_times.push_back(now);

        let mut byte_window = self.byte_window.lock().unwrap();
        byte_window.push_back((now, bytes));
    }

    /// Check and record a request atomically.
    ///
    /// Returns `true` if allowed and recorded, `false` if rate limited.
    pub fn allow_request(&self, bytes: u64) -> bool {
        if self.check_request(bytes) {
            self.record_request(bytes);
            true
        } else {
            false
        }
    }

    /// Get current request count in the window.
    pub fn current_request_count(&self) -> usize {
        let now = Instant::now();
        let cutoff = now - self.window;

        let mut request_times = self.request_times.lock().unwrap();

        // Remove old entries
        while let Some(&time) = request_times.front() {
            if time < cutoff {
                request_times.pop_front();
            } else {
                break;
            }
        }

        request_times.len()
    }

    /// Get current byte count in the window.
    pub fn current_byte_count(&self) -> u64 {
        let now = Instant::now();
        let cutoff = now - self.window;

        let mut byte_window = self.byte_window.lock().unwrap();

        // Remove old entries and sum
        let mut total = 0u64;
        while let Some(&(time, _)) = byte_window.front() {
            if time < cutoff {
                byte_window.pop_front();
            } else {
                break;
            }
        }

        for (_, bytes) in byte_window.iter() {
            total = total.saturating_add(*bytes);
        }

        total
    }

    /// Reset the rate limiter (clear all history).
    pub fn reset(&self) {
        self.request_times.lock().unwrap().clear();
        self.byte_window.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_rate_limiter_basic() {
        let limiter = RateLimiter::new(3, 1000, Duration::from_secs(1));

        // Should allow first 3 requests
        assert!(limiter.allow_request(100));
        assert!(limiter.allow_request(100));
        assert!(limiter.allow_request(100));

        // Should deny 4th request
        assert!(!limiter.allow_request(100));
    }

    #[test]
    fn test_rate_limiter_byte_limit() {
        let limiter = RateLimiter::new(100, 500, Duration::from_secs(1));

        // Should allow requests up to byte limit
        assert!(limiter.allow_request(200));
        assert!(limiter.allow_request(200));

        // Should deny request that would exceed byte limit
        assert!(!limiter.allow_request(200));
    }

    #[test]
    fn test_rate_limiter_window_expiry() {
        let limiter = RateLimiter::new(2, 1000, Duration::from_millis(100));

        // Fill the limit
        assert!(limiter.allow_request(100));
        assert!(limiter.allow_request(100));
        assert!(!limiter.allow_request(100));

        // Wait for window to expire
        thread::sleep(Duration::from_millis(150));

        // Should allow again
        assert!(limiter.allow_request(100));
    }

    #[test]
    fn test_rate_limiter_stats() {
        let limiter = RateLimiter::new(10, 1000, Duration::from_secs(1));

        limiter.allow_request(100);
        limiter.allow_request(200);
        limiter.allow_request(150);

        assert_eq!(limiter.current_request_count(), 3);
        assert_eq!(limiter.current_byte_count(), 450);
    }

    #[test]
    fn test_rate_limiter_reset() {
        let limiter = RateLimiter::new(2, 1000, Duration::from_secs(1));

        limiter.allow_request(100);
        limiter.allow_request(100);
        assert!(!limiter.allow_request(100));

        limiter.reset();

        // Should allow again after reset
        assert!(limiter.allow_request(100));
    }
}

/// The address a node advertises for a listener bound to `bind`: the bind
/// as it is when its IP is concrete, otherwise the node's routable address
/// (`[sip] local_ip`, or what the binary detected) with the bind's port.
/// Placement, migration and cross-node control send callers to what the
/// heartbeat says, so a node bound to `0.0.0.0` must not say `0.0.0.0`
/// (`docs/DEPLOYMENT_GAPS.md` §1.1). With nothing routable known the bind
/// is advertised as-is — the record is then honest about not knowing.
pub fn advertised_address(bind: SocketAddr, local_ip: Option<&str>) -> SocketAddr {
    if !bind.ip().is_unspecified() {
        return bind;
    }
    match local_ip.and_then(|ip| ip.trim().parse::<std::net::IpAddr>().ok()) {
        Some(ip) => SocketAddr::new(ip, bind.port()),
        None => bind,
    }
}

#[cfg(test)]
mod advertised_address_tests {
    use super::advertised_address;

    #[test]
    fn a_concrete_bind_is_advertised_as_is() {
        let bind = "10.0.0.5:5060".parse().unwrap();
        assert_eq!(advertised_address(bind, Some("10.0.0.20")), bind);
        assert_eq!(advertised_address(bind, None), bind);
    }

    #[test]
    fn an_unspecified_bind_takes_local_ip_and_keeps_its_port() {
        let bind = "0.0.0.0:5074".parse().unwrap();
        assert_eq!(
            advertised_address(bind, Some("10.0.0.20")),
            "10.0.0.20:5074".parse().unwrap()
        );
        let v6 = "[::]:5061".parse().unwrap();
        assert_eq!(
            advertised_address(v6, Some("2001:db8::7")),
            "[2001:db8::7]:5061".parse().unwrap()
        );
    }

    #[test]
    fn an_unspecified_bind_with_nothing_routable_stays_unspecified() {
        let bind = "0.0.0.0:5060".parse().unwrap();
        assert_eq!(advertised_address(bind, None), bind);
        assert_eq!(advertised_address(bind, Some("not an ip")), bind);
        assert_eq!(advertised_address(bind, Some("")), bind);
    }
}
