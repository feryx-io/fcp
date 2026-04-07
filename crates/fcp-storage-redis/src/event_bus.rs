//! Redis-backed distributed event bus.
//!
//! Two-layer architecture:
//! - **Pub/Sub** for real-time fan-out — every node gets every event
//! - **Streams** for durability — events-server uses a consumer group for replay
//!
//! Each node filters out self-published events via `node_id` embedded in the
//! message envelope. The local `InMemoryEventBus` remains the fan-out mechanism
//! within a single node.

use crate::RedisStorage;
use anyhow::Result;
use async_trait::async_trait;
use fcp_events::{EventPublisher, InMemoryEventBus, PlatformEvent};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Envelope wrapping a PlatformEvent with metadata for cluster routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventEnvelope {
    /// Node that published this event
    node_id: String,
    /// The actual event
    event: PlatformEvent,
}

/// Redis-backed distributed event bus.
///
/// Publishes events to both Redis Pub/Sub (real-time) and Streams (durability).
/// Subscribes to the Pub/Sub channel and re-publishes remote events to the
/// local `InMemoryEventBus`.
pub struct RedisEventBus {
    storage: RedisStorage,
    node_id: String,
    channel: String,
    stream_key: String,
}

impl RedisEventBus {
    /// Create a new Redis event bus.
    pub fn new(storage: RedisStorage, node_id: String) -> Self {
        let channel = storage.key("events");
        let stream_key = storage.key("event_stream");
        Self {
            storage,
            node_id,
            channel,
            stream_key,
        }
    }

    /// Create with custom channel and stream names.
    pub fn with_keys(
        storage: RedisStorage,
        node_id: String,
        channel: String,
        stream_key: String,
    ) -> Self {
        Self {
            storage,
            node_id,
            channel,
            stream_key,
        }
    }

    /// Start the subscriber loop that listens for remote events and republishes
    /// them to the local event bus.
    ///
    /// Returns a `JoinHandle` for the background task.
    pub fn start_subscriber(&self, local_bus: Arc<InMemoryEventBus>) -> JoinHandle<()> {
        let node_id = self.node_id.clone();
        let channel = self.channel.clone();
        let client = self.storage.client().clone();

        tokio::spawn(async move {
            let mut backoff_secs: u64 = 1;
            let max_backoff_secs: u64 = 60;
            let mut attempt: u64 = 0;

            loop {
                let start = std::time::Instant::now();

                if let Err(e) = run_subscriber_loop(&client, &channel, &node_id, &local_bus).await {
                    attempt += 1;
                    // Reset backoff if connection was healthy (lasted >30s)
                    if start.elapsed() > std::time::Duration::from_secs(30) {
                        backoff_secs = 1;
                        attempt = 1;
                    }
                    error!(
                        error = %e,
                        attempt = attempt,
                        reconnect_delay_secs = backoff_secs,
                        "Redis subscriber disconnected, reconnecting..."
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                    backoff_secs = (backoff_secs * 2).min(max_backoff_secs);
                }
            }
        })
    }

    /// Get the stream key for consumer group usage (by events-server).
    pub fn stream_key(&self) -> &str {
        &self.stream_key
    }

    /// Get the Pub/Sub channel name.
    pub fn channel(&self) -> &str {
        &self.channel
    }
}

#[async_trait]
impl EventPublisher for RedisEventBus {
    async fn publish(&self, event: PlatformEvent) -> Result<()> {
        let mut conn = self.storage.connection();

        let envelope = EventEnvelope {
            node_id: self.node_id.clone(),
            event,
        };

        let json = serde_json::to_string(&envelope)?;

        // Pipeline: PUBLISH (real-time) + XADD (durability)
        let _: () = redis::pipe()
            .publish(&self.channel, &json)
            .cmd("XADD")
            .arg(&self.stream_key)
            .arg("MAXLEN")
            .arg("~")
            .arg(10000) // Keep ~10k events in the stream
            .arg("*") // Auto-generate ID
            .arg("data")
            .arg(&json)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("Redis event publish failed: {}", e))?;

        debug!(
            channel = %self.channel,
            node_id = %self.node_id,
            "Published event to Redis"
        );

        Ok(())
    }
}

/// Internal subscriber loop that reconnects on failure.
async fn run_subscriber_loop(
    client: &redis::Client,
    channel: &str,
    self_node_id: &str,
    local_bus: &InMemoryEventBus,
) -> Result<()> {
    // Create a dedicated PubSub connection from the client
    let mut pubsub = client
        .get_async_pubsub()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create pubsub connection: {}", e))?;

    pubsub
        .subscribe(channel)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to subscribe to {}: {}", channel, e))?;

    info!(channel = %channel, "Redis event subscriber started");

    let mut msg_stream = pubsub.into_on_message();

    while let Some(msg) = msg_stream.next().await {
        let payload: String = match msg.get_payload() {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "Failed to get pubsub message payload");
                continue;
            }
        };

        let envelope: EventEnvelope = match serde_json::from_str(&payload) {
            Ok(env) => env,
            Err(e) => {
                warn!(error = %e, "Failed to deserialize event envelope");
                continue;
            }
        };

        // Skip self-published events
        if envelope.node_id == self_node_id {
            continue;
        }

        debug!(
            from_node = %envelope.node_id,
            event_type = %envelope.event.event_type_name(),
            "Received remote event via Redis Pub/Sub"
        );

        // Re-publish to local bus for local subscribers
        if let Err(e) = local_bus.publish(envelope.event).await {
            warn!(error = %e, "Failed to republish remote event to local bus");
        }
    }

    Err(anyhow::anyhow!("Pub/Sub message stream ended"))
}

/// Read events from a Redis Stream using a consumer group.
///
/// This is intended for the events-server to consume durable events
/// with at-least-once delivery guarantees.
pub struct RedisStreamConsumer {
    storage: RedisStorage,
    stream_key: String,
    group_name: String,
    consumer_name: String,
}

impl RedisStreamConsumer {
    /// Create a new stream consumer.
    pub fn new(
        storage: RedisStorage,
        stream_key: String,
        group_name: String,
        consumer_name: String,
    ) -> Self {
        Self {
            storage,
            stream_key,
            group_name,
            consumer_name,
        }
    }

    /// Ensure the consumer group exists (idempotent).
    pub async fn ensure_group(&self) -> Result<()> {
        let mut conn = self.storage.connection();

        // XGROUP CREATE — use MKSTREAM to auto-create the stream if it doesn't exist
        let result: redis::RedisResult<()> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(&self.stream_key)
            .arg(&self.group_name)
            .arg("0") // Read from the beginning
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;

        match result {
            Ok(()) => {
                info!(
                    stream = %self.stream_key,
                    group = %self.group_name,
                    "Created Redis consumer group"
                );
            }
            Err(e) if e.to_string().contains("BUSYGROUP") => {
                // Group already exists, that's fine
                debug!(
                    stream = %self.stream_key,
                    group = %self.group_name,
                    "Consumer group already exists"
                );
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to create consumer group: {}", e));
            }
        }

        Ok(())
    }

    /// Read the next batch of events from the stream.
    ///
    /// Returns events and their stream IDs (for acknowledgement).
    pub async fn read_events(
        &self,
        count: usize,
        block_ms: usize,
    ) -> Result<Vec<(String, PlatformEvent)>> {
        let mut conn = self.storage.connection();

        let results: redis::Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(&self.group_name)
            .arg(&self.consumer_name)
            .arg("COUNT")
            .arg(count)
            .arg("BLOCK")
            .arg(block_ms)
            .arg("STREAMS")
            .arg(&self.stream_key)
            .arg(">") // Only new messages
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XREADGROUP failed: {}", e))?;

        let events = parse_stream_results(results)?;
        Ok(events)
    }

    /// Acknowledge processed events.
    pub async fn ack(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        let mut conn = self.storage.connection();

        let mut cmd = redis::cmd("XACK");
        cmd.arg(&self.stream_key).arg(&self.group_name);
        for id in ids {
            cmd.arg(id);
        }

        let _: usize = cmd
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XACK failed: {}", e))?;

        Ok(())
    }
}

/// Parse XREADGROUP results into (stream_id, PlatformEvent) pairs.
fn parse_stream_results(value: redis::Value) -> Result<Vec<(String, PlatformEvent)>> {
    let mut events = Vec::new();

    // XREADGROUP returns: [[stream_name, [[id, [field, value, ...]], ...]]]
    // or Nil if block timeout expired
    let streams = match value {
        redis::Value::Array(streams) => streams,
        redis::Value::Nil => return Ok(events),
        other => {
            return Err(anyhow::anyhow!(
                "Unexpected XREADGROUP response type: {:?}",
                other
            ));
        }
    };

    for stream in streams {
        let stream_parts = match stream {
            redis::Value::Array(parts) => parts,
            _ => continue,
        };

        // parts[0] = stream name, parts[1] = entries
        if stream_parts.len() < 2 {
            continue;
        }

        let entries = match &stream_parts[1] {
            redis::Value::Array(entries) => entries,
            _ => continue,
        };

        for entry in entries {
            let entry_parts = match entry {
                redis::Value::Array(parts) => parts,
                _ => continue,
            };

            if entry_parts.len() < 2 {
                continue;
            }

            // entry_parts[0] = stream ID
            let stream_id = match &entry_parts[0] {
                redis::Value::BulkString(id) => String::from_utf8_lossy(id).to_string(),
                _ => continue,
            };

            // entry_parts[1] = [field, value, field, value, ...]
            let fields = match &entry_parts[1] {
                redis::Value::Array(fields) => fields,
                _ => continue,
            };

            // Find the "data" field
            let mut i = 0;
            while i + 1 < fields.len() {
                let field_name = match &fields[i] {
                    redis::Value::BulkString(name) => String::from_utf8_lossy(name).to_string(),
                    _ => {
                        i += 2;
                        continue;
                    }
                };

                if field_name == "data" {
                    if let redis::Value::BulkString(data) = &fields[i + 1] {
                        let json = String::from_utf8_lossy(data);
                        match serde_json::from_str::<EventEnvelope>(&json) {
                            Ok(envelope) => {
                                events.push((stream_id.clone(), envelope.event));
                            }
                            Err(e) => {
                                warn!(
                                    stream_id = %stream_id,
                                    error = %e,
                                    "Failed to deserialize event from stream"
                                );
                            }
                        }
                    }
                }

                i += 2;
            }
        }
    }

    Ok(events)
}
