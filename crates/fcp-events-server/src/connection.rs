//! WebSocket connection management for event streaming.

use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use fcp_core::TenantId;
use fcp_events::{EventFilter, PlatformEvent};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Unique connection identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(Uuid);

impl ConnectionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "conn-{}", self.0)
    }
}

/// Connection metadata and state
#[derive(Debug)]
pub struct Connection {
    pub id: ConnectionId,
    pub tenant_id: TenantId,
    pub subscriptions: Vec<EventFilter>,
    pub last_activity: Instant,
    pub sender: mpsc::UnboundedSender<PlatformEvent>,
}

impl Connection {
    pub fn new(tenant_id: TenantId, sender: mpsc::UnboundedSender<PlatformEvent>) -> Self {
        Self {
            id: ConnectionId::new(),
            tenant_id,
            subscriptions: Vec::new(),
            last_activity: Instant::now(),
            sender,
        }
    }

    /// Check if this connection is interested in an event
    pub fn matches_event(&self, event: &PlatformEvent) -> bool {
        // If no subscriptions, receive all events
        if self.subscriptions.is_empty() {
            return true;
        }

        // Check if any subscription matches
        self.subscriptions
            .iter()
            .any(|filter| filter.matches(event))
    }

    /// Update activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Check if connection has timed out
    pub fn is_timed_out(&self, timeout: Duration) -> bool {
        self.last_activity.elapsed() > timeout
    }
}

/// Connection manager for tracking all active connections
pub struct ConnectionManager {
    connections: Arc<DashMap<ConnectionId, Connection>>,
    tenant_connections: Arc<DashMap<TenantId, Vec<ConnectionId>>>,
    heartbeat_interval: Duration,
    client_timeout: Duration,
    max_connections: usize,
    max_connections_per_tenant: usize,
}

impl ConnectionManager {
    pub fn new(
        heartbeat_interval: Duration,
        client_timeout: Duration,
        max_connections: usize,
        max_connections_per_tenant: usize,
    ) -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
            tenant_connections: Arc::new(DashMap::new()),
            heartbeat_interval,
            client_timeout,
            max_connections,
            max_connections_per_tenant,
        }
    }

    /// Register a new connection
    pub fn register(
        &self,
        tenant_id: TenantId,
        sender: mpsc::UnboundedSender<PlatformEvent>,
    ) -> Result<ConnectionId, String> {
        // Check global connection limit
        if self.connections.len() >= self.max_connections {
            return Err("Maximum connections reached".to_string());
        }

        // Check per-tenant connection limit
        let tenant_count = self
            .tenant_connections
            .get(&tenant_id)
            .map(|conns| conns.len())
            .unwrap_or(0);

        if tenant_count >= self.max_connections_per_tenant {
            return Err(format!(
                "Maximum connections per tenant ({}) reached",
                self.max_connections_per_tenant
            ));
        }

        let conn = Connection::new(tenant_id.clone(), sender);
        let conn_id = conn.id;

        info!(
            connection_id = %conn_id,
            tenant_id = %tenant_id,
            "Registering new WebSocket connection"
        );

        // Add to connections map
        self.connections.insert(conn_id, conn);

        // Add to tenant connections
        self.tenant_connections
            .entry(tenant_id)
            .or_insert_with(Vec::new)
            .push(conn_id);

        Ok(conn_id)
    }

    /// Unregister a connection
    pub fn unregister(&self, conn_id: ConnectionId) {
        if let Some((_, conn)) = self.connections.remove(&conn_id) {
            info!(
                connection_id = %conn_id,
                tenant_id = %conn.tenant_id,
                "Unregistering WebSocket connection"
            );

            // Remove from tenant connections
            if let Some(mut tenant_conns) = self.tenant_connections.get_mut(&conn.tenant_id) {
                tenant_conns.retain(|id| *id != conn_id);
            }
        }
    }

    /// Update connection subscriptions
    pub fn update_subscriptions(&self, conn_id: ConnectionId, subscriptions: Vec<EventFilter>) {
        if let Some(mut conn) = self.connections.get_mut(&conn_id) {
            debug!(
                connection_id = %conn_id,
                subscription_count = subscriptions.len(),
                "Updating connection subscriptions"
            );
            conn.subscriptions = subscriptions;
            conn.touch();
        }
    }

    /// Touch connection to update last activity
    pub fn touch(&self, conn_id: ConnectionId) {
        if let Some(mut conn) = self.connections.get_mut(&conn_id) {
            conn.touch();
        }
    }

    /// Publish event to all matching connections
    pub fn publish(&self, event: PlatformEvent) {
        let mut send_count = 0;
        let mut error_count = 0;

        for conn_ref in self.connections.iter() {
            let conn = conn_ref.value();
            if conn.matches_event(&event) {
                match conn.sender.send(event.clone()) {
                    Ok(_) => send_count += 1,
                    Err(e) => {
                        error!(
                            connection_id = %conn.id,
                            error = %e,
                            "Failed to send event to connection"
                        );
                        error_count += 1;
                    }
                }
            }
        }

        debug!(send_count, error_count, "Published event to connections");
    }

    /// Remove timed out connections
    pub fn remove_timed_out(&self) -> usize {
        let timed_out: Vec<ConnectionId> = self
            .connections
            .iter()
            .filter(|entry| entry.value().is_timed_out(self.client_timeout))
            .map(|entry| entry.key().clone())
            .collect();

        let count = timed_out.len();
        for conn_id in timed_out {
            warn!(connection_id = %conn_id, "Removing timed out connection");
            self.unregister(conn_id);
        }

        count
    }

    /// Get connection count
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Get connection count for tenant
    pub fn tenant_connection_count(&self, tenant_id: &TenantId) -> usize {
        self.tenant_connections
            .get(tenant_id)
            .map(|conns| conns.len())
            .unwrap_or(0)
    }

    pub fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }
}

/// Handle a WebSocket connection
pub async fn handle_websocket(
    socket: WebSocket,
    conn_id: ConnectionId,
    tenant_id: TenantId,
    manager: Arc<ConnectionManager>,
    mut event_rx: mpsc::UnboundedReceiver<PlatformEvent>,
    batch_size: usize,
    batch_timeout: Duration,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    info!(
        connection_id = %conn_id,
        tenant_id = %tenant_id,
        "WebSocket connection established"
    );

    // Send initial connection message
    let connect_msg = serde_json::json!({
        "type": "connected",
        "connection_id": conn_id.to_string(),
        "tenant_id": tenant_id.to_string(),
    });

    if let Ok(msg) = serde_json::to_string(&connect_msg) {
        let _ = ws_tx.send(Message::Text(msg)).await;
    }

    // Heartbeat timer
    let mut heartbeat_interval = tokio::time::interval(manager.heartbeat_interval());
    heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Event batching
    let mut event_batch: Vec<PlatformEvent> = Vec::with_capacity(batch_size);
    let mut batch_timer = tokio::time::interval(batch_timeout);
    batch_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // Receive WebSocket messages from client
            Some(msg_result) = ws_rx.next() => {
                match msg_result {
                    Ok(Message::Text(text)) => {
                        manager.touch(conn_id);
                        if let Err(e) = handle_client_message(&text, conn_id, &manager) {
                            error!(
                                connection_id = %conn_id,
                                error = %e,
                                "Failed to handle client message"
                            );
                        }
                    }
                    Ok(Message::Ping(data)) => {
                        manager.touch(conn_id);
                        let _ = ws_tx.send(Message::Pong(data)).await;
                    }
                    Ok(Message::Pong(_)) => {
                        manager.touch(conn_id);
                    }
                    Ok(Message::Close(_)) => {
                        info!(connection_id = %conn_id, "Client closed connection");
                        break;
                    }
                    Err(e) => {
                        error!(connection_id = %conn_id, error = %e, "WebSocket error");
                        break;
                    }
                    _ => {}
                }
            }

            // Send events to client
            Some(event) = event_rx.recv() => {
                event_batch.push(event);

                // Send batch if full
                if event_batch.len() >= batch_size {
                    if let Err(e) = send_event_batch(&mut ws_tx, &event_batch).await {
                        error!(connection_id = %conn_id, error = %e, "Failed to send event batch");
                        break;
                    }
                    event_batch.clear();
                }
            }

            // Batch timeout - send partial batch
            _ = batch_timer.tick() => {
                if !event_batch.is_empty() {
                    if let Err(e) = send_event_batch(&mut ws_tx, &event_batch).await {
                        error!(connection_id = %conn_id, error = %e, "Failed to send event batch");
                        break;
                    }
                    event_batch.clear();
                }
            }

            // Send heartbeat ping
            _ = heartbeat_interval.tick() => {
                if let Err(e) = ws_tx.send(Message::Ping(vec![])).await {
                    error!(connection_id = %conn_id, error = %e, "Failed to send heartbeat");
                    break;
                }
            }
        }
    }

    // Cleanup
    manager.unregister(conn_id);
    info!(connection_id = %conn_id, "WebSocket connection closed");
}

/// Handle client message (subscription updates, etc.)
fn handle_client_message(
    text: &str,
    conn_id: ConnectionId,
    manager: &ConnectionManager,
) -> Result<(), String> {
    let msg: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("Invalid JSON: {}", e))?;

    let msg_type = msg
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing 'type' field".to_string())?;

    match msg_type {
        "subscribe" => {
            let filters_json = msg
                .get("filters")
                .ok_or_else(|| "Missing 'filters' field".to_string())?;

            let filters: Vec<EventFilter> = serde_json::from_value(filters_json.clone())
                .map_err(|e| format!("Invalid filters: {}", e))?;

            manager.update_subscriptions(conn_id, filters);
            Ok(())
        }
        "ping" => {
            // Just a keepalive, already touched connection
            Ok(())
        }
        _ => Err(format!("Unknown message type: {}", msg_type)),
    }
}

/// Send batch of events to client
async fn send_event_batch(
    ws_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    events: &[PlatformEvent],
) -> Result<(), String> {
    let batch_msg = serde_json::json!({
        "type": "events",
        "count": events.len(),
        "events": events,
    });

    let text = serde_json::to_string(&batch_msg)
        .map_err(|e| format!("Failed to serialize events: {}", e))?;

    ws_tx
        .send(Message::Text(text))
        .await
        .map_err(|e| format!("Failed to send message: {}", e))?;

    Ok(())
}

/// Background task to cleanup timed out connections
pub async fn cleanup_task(manager: Arc<ConnectionManager>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        interval.tick().await;

        let removed = manager.remove_timed_out();
        if removed > 0 {
            info!(removed_count = removed, "Removed timed out connections");
        }

        debug!(
            total_connections = manager.connection_count(),
            "Connection cleanup completed"
        );
    }
}
