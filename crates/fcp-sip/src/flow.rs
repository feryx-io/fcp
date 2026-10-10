//! RFC 5626 Flow Token Registry.
//!
//! This module provides a registry for mapping flow tokens to TLS/TCP connection
//! writers. Flow tokens enable connection reuse for NAT'd clients by allowing
//! the server to send requests back through the same connection the client
//! established during registration.
//!
//! # Background
//!
//! When a SIP client behind NAT registers over TLS, the server accepts the
//! inbound connection. To send requests (like INVITE) to that client, the
//! server must use the same TLS connection - opening a new connection to the
//! client's ephemeral port will fail.
//!
//! RFC 5626 defines the "Outbound" mechanism where:
//! 1. Client registers with +sip.instance and reg-id parameters
//! 2. Server generates a flow token identifying the connection
//! 3. Server stores the flow token with the registration
//! 4. When routing to the client, server uses the stored connection

use bytes::Bytes;
use dashmap::DashMap;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{debug, info};
use uuid::Uuid;

/// A flow token uniquely identifies a TLS/TCP connection.
pub type FlowToken = String;

/// Information about a flow connection.
#[derive(Clone)]
pub struct FlowInfo {
    /// The connection writer channel for sending data.
    pub stream: mpsc::Sender<Bytes>,
    /// The peer's socket address.
    pub peer_addr: SocketAddr,
    /// Whose binding it carries, once its REGISTER succeeded: the tenant
    /// and the address of record. What another node's relay of a call down
    /// this flow is trusted for (`docs/PUSH.md` §2.4).
    pub owner: Option<(fcp_core::TenantId, String)>,
}

/// Registry for RFC 5626 flow tokens mapping to connection writers.
///
/// This enables connection reuse for NAT'd clients by storing the
/// mpsc::Sender<Bytes> that can be used to send data on the original
/// inbound connection.
#[derive(Default)]
pub struct FlowRegistry {
    /// Map from flow token to flow info (stream + peer address).
    flows: DashMap<FlowToken, FlowInfo>,
}

impl FlowRegistry {
    /// Create a new flow registry.
    pub fn new() -> Self {
        Self {
            flows: DashMap::new(),
        }
    }

    /// Register a new flow and return its token.
    ///
    /// # Arguments
    /// * `stream` - The connection writer channel for sending data
    /// * `peer_addr` - The peer's socket address
    ///
    /// # Returns
    /// A unique flow token that can be used to retrieve the connection later.
    pub fn register(&self, stream: mpsc::Sender<Bytes>, peer_addr: SocketAddr) -> FlowToken {
        let token = format!("flow-{}", Uuid::new_v4().simple());
        debug!(flow_token = %token, peer = %peer_addr, "Registered new flow");
        self.flows.insert(
            token.clone(),
            FlowInfo {
                stream,
                peer_addr,
                owner: None,
            },
        );
        token
    }

    /// Look up flow info by flow token.
    ///
    /// # Arguments
    /// * `token` - The flow token to look up
    ///
    /// # Returns
    /// The flow info (stream + peer address) if found and still valid.
    pub fn lookup(&self, token: &str) -> Option<FlowInfo> {
        self.flows.get(token).map(|entry| entry.value().clone())
    }

    /// The flow carries this tenant's binding at `aor` (its REGISTER
    /// succeeded).
    pub fn set_owner(&self, token: &str, tenant: fcp_core::TenantId, aor: String) {
        if let Some(mut entry) = self.flows.get_mut(token) {
            entry.owner = Some((tenant, aor));
        }
    }

    /// Look up just the connection writer by flow token.
    pub fn lookup_stream(&self, token: &str) -> Option<mpsc::Sender<Bytes>> {
        self.flows
            .get(token)
            .map(|entry| entry.value().stream.clone())
    }

    /// Remove a flow from the registry.
    ///
    /// This should be called when the connection closes or the registration expires.
    pub fn remove(&self, token: &str) {
        if self.flows.remove(token).is_some() {
            debug!(flow_token = %token, "Removed flow from registry");
        }
    }

    /// Check if a flow token exists and the connection is still alive.
    ///
    /// This checks if the channel is closed (connection terminated).
    pub fn is_valid(&self, token: &str) -> bool {
        if let Some(entry) = self.flows.get(token) {
            // Check if the channel is still open
            !entry.value().stream.is_closed()
        } else {
            false
        }
    }

    /// Remove stale flows where the connection has closed.
    ///
    /// Returns the number of flows removed.
    pub fn cleanup_stale(&self) -> usize {
        let mut removed = 0;
        self.flows.retain(|token, flow_info| {
            if flow_info.stream.is_closed() {
                debug!(flow_token = %token, "Removing stale flow (connection closed)");
                removed += 1;
                false
            } else {
                true
            }
        });
        if removed > 0 {
            info!(removed = removed, "Cleaned up stale flows");
        }
        removed
    }

    /// Get the current number of registered flows.
    pub fn len(&self) -> usize {
        self.flows.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }

    /// Send data on a flow by token.
    ///
    /// # Arguments
    /// * `token` - The flow token
    /// * `data` - The data to send
    ///
    /// # Returns
    /// Ok(()) if the data was sent, Err if the flow doesn't exist or the connection closed.
    pub async fn send(&self, token: &str, data: Bytes) -> anyhow::Result<()> {
        let flow_info = self
            .lookup(token)
            .ok_or_else(|| anyhow::anyhow!("Flow token not found: {}", token))?;

        flow_info
            .stream
            .send(data)
            .await
            .map_err(|_| anyhow::anyhow!("Flow connection closed: {}", token))?;

        debug!(flow_token = %token, "Sent data on flow");
        Ok(())
    }
}

impl std::fmt::Debug for FlowRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowRegistry")
            .field("flow_count", &self.flows.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_addr() -> SocketAddr {
        "192.168.1.100:5060".parse().unwrap()
    }

    #[tokio::test]
    async fn test_register_and_lookup() {
        let registry = FlowRegistry::new();
        let (tx, _rx) = mpsc::channel(10);

        let token = registry.register(tx, test_addr());
        assert!(token.starts_with("flow-"));

        // Should be able to look up the flow
        let flow_info = registry.lookup(&token);
        assert!(flow_info.is_some());
        assert_eq!(flow_info.unwrap().peer_addr, test_addr());

        // Should not find non-existent token
        assert!(registry.lookup("flow-nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_remove() {
        let registry = FlowRegistry::new();
        let (tx, _rx) = mpsc::channel(10);

        let token = registry.register(tx, test_addr());
        assert_eq!(registry.len(), 1);

        registry.remove(&token);
        assert_eq!(registry.len(), 0);
        assert!(registry.lookup(&token).is_none());
    }

    #[tokio::test]
    async fn test_is_valid() {
        let registry = FlowRegistry::new();
        let (tx, rx) = mpsc::channel::<Bytes>(10);

        let token = registry.register(tx, test_addr());
        assert!(registry.is_valid(&token));

        // Drop receiver to close the channel
        drop(rx);

        // Channel is closed, should be invalid
        assert!(!registry.is_valid(&token));
    }

    #[tokio::test]
    async fn test_cleanup_stale() {
        let registry = FlowRegistry::new();

        // Create some flows
        let (tx1, rx1) = mpsc::channel::<Bytes>(10);
        let (tx2, _rx2) = mpsc::channel::<Bytes>(10);

        let token1 = registry.register(tx1, test_addr());
        let _token2 = registry.register(tx2, "192.168.1.101:5060".parse().unwrap());

        assert_eq!(registry.len(), 2);

        // Close first channel by dropping receiver
        drop(rx1);

        // Cleanup should remove the stale flow
        let removed = registry.cleanup_stale();
        assert_eq!(removed, 1);
        assert_eq!(registry.len(), 1);
        assert!(registry.lookup(&token1).is_none());
    }

    #[tokio::test]
    async fn test_send() {
        let registry = FlowRegistry::new();
        let (tx, mut rx) = mpsc::channel::<Bytes>(10);

        let token = registry.register(tx, test_addr());

        // Send data
        let data = Bytes::from("test data");
        registry.send(&token, data.clone()).await.unwrap();

        // Receive and verify
        let received = rx.recv().await.unwrap();
        assert_eq!(received, data);
    }

    #[tokio::test]
    async fn test_send_closed_channel() {
        let registry = FlowRegistry::new();
        let (tx, rx) = mpsc::channel::<Bytes>(10);

        let token = registry.register(tx, test_addr());
        drop(rx); // Close channel

        // Send should fail
        let result = registry.send(&token, Bytes::from("test")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_nonexistent() {
        let registry = FlowRegistry::new();

        let result = registry.send("flow-nonexistent", Bytes::from("test")).await;
        assert!(result.is_err());
    }
}
