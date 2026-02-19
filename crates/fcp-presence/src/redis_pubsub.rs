//! Redis Pub/Sub for cross-node presence change propagation.
//!
//! When a presence state changes on one node, it publishes a message to a Redis
//! channel. Other nodes subscribe to that channel and trigger NOTIFY delivery
//! for their local watchers.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio_stream::Stream;
use tracing::{debug, warn};

use fcp_core::TenantId;

/// A presence change event broadcast via Redis Pub/Sub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceChangeEvent {
    /// The presentity AOR whose state changed.
    pub aor: String,
    /// Tenant ID for multi-tenancy.
    pub tenant_id: TenantId,
    /// Node ID of the node that originated the change (for self-filtering).
    pub node_id: String,
}

const CHANNEL: &str = "presence:changes";

/// Redis Pub/Sub handler for cross-node NOTIFY propagation.
pub struct RedisPresencePubSub {
    client: redis::Client,
    /// This node's unique identifier, used to filter out self-published messages.
    node_id: String,
}

impl RedisPresencePubSub {
    /// Create a new RedisPresencePubSub.
    ///
    /// If `node_id` is None, a random UUID is generated.
    pub fn new(redis_url: &str, node_id: Option<String>) -> Result<Self> {
        let client = redis::Client::open(redis_url)
            .context("Failed to create Redis client for pubsub")?;
        let node_id = node_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok(Self { client, node_id })
    }

    /// Get this node's ID.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Publish a presence change event to Redis.
    ///
    /// Called after a local presence state update to inform other nodes.
    pub async fn publish_change(&self, aor: &str, tenant_id: &TenantId) -> Result<()> {
        let event = PresenceChangeEvent {
            aor: aor.to_string(),
            tenant_id: tenant_id.clone(),
            node_id: self.node_id.clone(),
        };

        let payload = serde_json::to_string(&event)
            .context("Failed to serialize presence change event")?;

        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .context("Failed to get Redis connection for publish")?;

        redis::cmd("PUBLISH")
            .arg(CHANNEL)
            .arg(&payload)
            .exec_async(&mut conn)
            .await
            .context("Failed to publish presence change")?;

        debug!(aor = %aor, node_id = %self.node_id, "Published presence change to Redis");
        Ok(())
    }

    /// Subscribe to presence change events from other nodes.
    ///
    /// Returns a stream of `PresenceChangeEvent` that filters out messages
    /// originating from this node.
    pub async fn subscribe(
        &self,
    ) -> Result<impl Stream<Item = PresenceChangeEvent> + Send + 'static> {
        let mut pubsub = self.client
            .get_async_pubsub()
            .await
            .context("Failed to create Redis pubsub connection")?;

        pubsub
            .subscribe(CHANNEL)
            .await
            .context("Failed to subscribe to presence changes channel")?;

        let node_id = self.node_id.clone();

        let stream = async_stream::stream! {
            let mut msg_stream = pubsub.into_on_message();
            loop {
                use futures::StreamExt;
                match msg_stream.next().await {
                    Some(msg) => {
                        let payload: String = match msg.get_payload() {
                            Ok(p) => p,
                            Err(e) => {
                                warn!(error = %e, "Failed to get pubsub payload");
                                continue;
                            }
                        };

                        let event: PresenceChangeEvent = match serde_json::from_str(&payload) {
                            Ok(e) => e,
                            Err(e) => {
                                warn!(error = %e, "Failed to parse presence change event");
                                continue;
                            }
                        };

                        // Filter out our own messages
                        if event.node_id == node_id {
                            continue;
                        }

                        debug!(
                            aor = %event.aor,
                            source_node = %event.node_id,
                            "Received remote presence change"
                        );

                        yield event;
                    }
                    None => {
                        warn!("Redis pubsub stream ended");
                        break;
                    }
                }
            }
        };

        Ok(stream)
    }
}
