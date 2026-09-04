//! Redis-backed distributed event bus.
//!
//! Two-layer architecture:
//! - **Pub/Sub** for real-time fan-out — every node gets every event
//! - **Streams** for durability — events-server uses a consumer group for replay
//!
//! Each node filters out self-published events via `node_id` embedded in the
//! message envelope. The local `InMemoryEventBus` remains the fan-out mechanism
//! within a single node.
//!
//! **Across regions** ([`EventBridge`], `docs/MULTI_REGION.md`): each region
//! has its own Redis, so its events stay there until a bridge forwards
//! them. The bridge reads the local stream through one consumer group per
//! peer region and re-publishes every entry that originated here into the
//! peer's Redis (Pub/Sub + stream, the same shape a local publisher
//! writes), marking it `relayed` with its `origin_region`. A relayed entry
//! is never relayed again, so regions peered in a full mesh each see every
//! event exactly once.

use crate::{RedisConfig, RedisStorage};
use anyhow::Result;
use async_trait::async_trait;
use fcp_events::{EventPublisher, InMemoryEventBus, PlatformEvent};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Envelope wrapping a PlatformEvent with metadata for cluster routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Node that published this event.
    pub node_id: String,
    /// Region the publishing node advertised, when it had one. Absent on
    /// envelopes written by older versions and by unlabeled nodes; a
    /// bridge fills it in with its own region when it relays such an
    /// envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_region: Option<String>,
    /// Written into this Redis by a bridge from another region, not by a
    /// node here. Never relayed again. Absent (false) on envelopes written
    /// by older versions.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relayed: bool,
    /// The actual event.
    pub event: PlatformEvent,
}

/// One entry of the event stream, as read by a consumer group.
#[derive(Debug, Clone)]
pub struct StreamEntry {
    /// Stream id, for acknowledgement.
    pub id: String,
    pub envelope: EventEnvelope,
}

/// Redis-backed distributed event bus.
///
/// Publishes events to both Redis Pub/Sub (real-time) and Streams (durability).
/// Subscribes to the Pub/Sub channel and re-publishes remote events to the
/// local `InMemoryEventBus`.
pub struct RedisEventBus {
    storage: RedisStorage,
    node_id: String,
    region: Option<String>,
    channel: String,
    stream_key: String,
}

/// Pub/Sub channel and stream key for a keyspace.
fn bus_keys(storage: &RedisStorage) -> (String, String) {
    (storage.key("events"), storage.key("event_stream"))
}

/// Entries kept in the stream (`XADD MAXLEN ~`).
const STREAM_MAXLEN: usize = 10_000;

/// Queue `PUBLISH` + `XADD` for one serialized envelope on a pipeline.
fn queue_publish(pipe: &mut redis::Pipeline, channel: &str, stream_key: &str, json: &str) {
    pipe.publish(channel, json)
        .ignore()
        .cmd("XADD")
        .arg(stream_key)
        .arg("MAXLEN")
        .arg("~")
        .arg(STREAM_MAXLEN)
        .arg("*")
        .arg("data")
        .arg(json)
        .ignore();
}

impl RedisEventBus {
    /// Create a new Redis event bus.
    pub fn new(storage: RedisStorage, node_id: String) -> Self {
        let (channel, stream_key) = bus_keys(&storage);
        Self {
            storage,
            node_id,
            region: None,
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
            region: None,
            channel,
            stream_key,
        }
    }

    /// Stamp published events with the region this node runs in, so a
    /// bridge can tell where they came from.
    pub fn with_region(mut self, region: Option<String>) -> Self {
        self.region = region;
        self
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
            origin_region: self.region.clone(),
            relayed: false,
            event,
        };

        let json = serde_json::to_string(&envelope)?;

        // Pipeline: PUBLISH (real-time) + XADD (durability)
        let mut pipe = redis::pipe();
        queue_publish(&mut pipe, &self.channel, &self.stream_key, &json);
        let _: () = pipe
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
            origin_region = envelope.origin_region.as_deref().unwrap_or("-"),
            relayed = envelope.relayed,
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

    /// Read the next batch of entries from the stream, envelope and all.
    pub async fn read_entries(&self, count: usize, block_ms: usize) -> Result<Vec<StreamEntry>> {
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

        parse_stream_results(results)
    }

    /// Read the next batch of events from the stream.
    ///
    /// Returns events and their stream IDs (for acknowledgement).
    pub async fn read_events(
        &self,
        count: usize,
        block_ms: usize,
    ) -> Result<Vec<(String, PlatformEvent)>> {
        Ok(self
            .read_entries(count, block_ms)
            .await?
            .into_iter()
            .map(|e| (e.id, e.envelope.event))
            .collect())
    }

    /// Take over entries this group delivered but nobody acknowledged
    /// within `min_idle` (a consumer that died mid-batch), so they are
    /// processed after all. Returns them like [`read_entries`](Self::read_entries).
    pub async fn claim_stale(&self, min_idle: Duration, count: usize) -> Result<Vec<StreamEntry>> {
        let mut conn = self.storage.connection();
        let mut claimed = Vec::new();
        let mut start = "0-0".to_string();
        loop {
            // XAUTOCLAIM → [next start, entries, (deleted ids on 7.0+)]
            let value: redis::Value = redis::cmd("XAUTOCLAIM")
                .arg(&self.stream_key)
                .arg(&self.group_name)
                .arg(&self.consumer_name)
                .arg(min_idle.as_millis() as u64)
                .arg(&start)
                .arg("COUNT")
                .arg(count)
                .query_async(&mut conn)
                .await
                .map_err(|e| anyhow::anyhow!("XAUTOCLAIM failed: {}", e))?;
            let parts = match value {
                redis::Value::Array(parts) if parts.len() >= 2 => parts,
                _ => break,
            };
            let next = match &parts[0] {
                redis::Value::BulkString(s) => String::from_utf8_lossy(s).to_string(),
                _ => "0-0".to_string(),
            };
            if let redis::Value::Array(entries) = &parts[1] {
                claimed.extend(parse_entries(entries));
            }
            if next == "0-0" || claimed.len() >= count {
                break;
            }
            start = next;
        }
        Ok(claimed)
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

/// Parse XREADGROUP results into stream entries.
fn parse_stream_results(value: redis::Value) -> Result<Vec<StreamEntry>> {
    // XREADGROUP returns: [[stream_name, [[id, [field, value, ...]], ...]]]
    // or Nil if block timeout expired
    let streams = match value {
        redis::Value::Array(streams) => streams,
        redis::Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(anyhow::anyhow!(
                "Unexpected XREADGROUP response type: {:?}",
                other
            ));
        }
    };

    let mut events = Vec::new();
    for stream in streams {
        let stream_parts = match stream {
            redis::Value::Array(parts) => parts,
            _ => continue,
        };
        // parts[0] = stream name, parts[1] = entries
        if stream_parts.len() < 2 {
            continue;
        }
        if let redis::Value::Array(entries) = &stream_parts[1] {
            events.extend(parse_entries(entries));
        }
    }
    Ok(events)
}

/// Parse `[[id, [field, value, ...]], ...]` into entries, skipping any
/// whose `data` field does not hold an envelope.
fn parse_entries(entries: &[redis::Value]) -> Vec<StreamEntry> {
    let mut out = Vec::new();
    for entry in entries {
        let entry_parts = match entry {
            redis::Value::Array(parts) => parts,
            _ => continue,
        };
        if entry_parts.len() < 2 {
            continue;
        }
        let stream_id = match &entry_parts[0] {
            redis::Value::BulkString(id) => String::from_utf8_lossy(id).to_string(),
            _ => continue,
        };
        let fields = match &entry_parts[1] {
            redis::Value::Array(fields) => fields,
            _ => continue,
        };
        for pair in fields.chunks(2) {
            let (redis::Value::BulkString(name), Some(redis::Value::BulkString(data))) =
                (&pair[0], pair.get(1))
            else {
                continue;
            };
            if name.as_slice() != b"data" {
                continue;
            }
            let json = String::from_utf8_lossy(data);
            match serde_json::from_str::<EventEnvelope>(&json) {
                Ok(envelope) => out.push(StreamEntry {
                    id: stream_id.clone(),
                    envelope,
                }),
                Err(e) => {
                    warn!(stream_id = %stream_id, error = %e, "Failed to deserialize event from stream");
                }
            }
        }
    }
    out
}

// ── Cross-region bridge ──────────────────────────────────────────────────

/// A peer region's Redis, as the bridge forwards to it.
#[derive(Debug, Clone)]
pub struct PeerRegion {
    /// The peer's region label (`[cluster] region` there).
    pub region: String,
    pub redis_url: String,
    /// Key prefix the peer's nodes use (normally the same as here).
    pub key_prefix: String,
}

/// What the bridge knows about one peer, for health endpoints.
#[derive(Debug, Clone, Serialize)]
pub struct PeerBridgeStatus {
    pub region: String,
    /// Connected to the peer's Redis at last try.
    pub connected: bool,
    /// Envelopes forwarded since start.
    pub forwarded: u64,
    /// Entries taken over from consumers that stopped acknowledging.
    pub claimed: u64,
    /// Forwarding attempts that failed since start (each is retried).
    pub failed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

struct PeerCounters {
    region: String,
    connected: AtomicBool,
    forwarded: AtomicU64,
    claimed: AtomicU64,
    failed: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl PeerCounters {
    fn fail(&self, e: &anyhow::Error) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string());
    }
    fn status(&self) -> PeerBridgeStatus {
        PeerBridgeStatus {
            region: self.region.clone(),
            connected: self.connected.load(Ordering::Relaxed),
            forwarded: self.forwarded.load(Ordering::Relaxed),
            claimed: self.claimed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            last_error: self
                .last_error
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
        }
    }
}

/// Forwards this region's events to peer regions. See the module docs.
pub struct EventBridge {
    local: RedisStorage,
    local_region: Option<String>,
    stream_key: String,
    consumer_name: String,
    /// Unacknowledged entries older than this are taken over from dead
    /// consumers (at start and once a minute).
    claim_min_idle: Duration,
    peers: Vec<(PeerRegion, Arc<PeerCounters>)>,
}

/// How often stale pending entries are re-claimed while entries flow.
const CLAIM_INTERVAL: Duration = Duration::from_secs(60);
/// How often they are re-claimed while nothing is flowing: a dead
/// consumer's leftovers are all there is to do then, so look sooner.
const IDLE_CLAIM_INTERVAL: Duration = Duration::from_secs(5);
/// Entries read per batch.
const BATCH: usize = 100;

/// Whether a stream entry should be forwarded to peers: only what was
/// published here, never what a bridge brought in.
pub fn forwardable(envelope: &EventEnvelope) -> bool {
    !envelope.relayed
}

/// The envelope a peer receives for a forwardable entry.
fn relayed_envelope(mut envelope: EventEnvelope, local_region: Option<&str>) -> EventEnvelope {
    envelope.relayed = true;
    if envelope.origin_region.is_none() {
        envelope.origin_region = local_region.map(str::to_owned);
    }
    envelope
}

impl EventBridge {
    /// A bridge reading `local`'s event stream on behalf of `local_region`.
    /// `consumer_name` should be unique per process (every events-server
    /// in the region shares the per-peer consumer groups, so each entry is
    /// forwarded once).
    pub fn new(
        local: RedisStorage,
        local_region: Option<String>,
        consumer_name: String,
        peers: Vec<PeerRegion>,
    ) -> Self {
        let (_, stream_key) = bus_keys(&local);
        let peers = peers
            .into_iter()
            .map(|p| {
                let counters = Arc::new(PeerCounters {
                    region: p.region.clone(),
                    connected: AtomicBool::new(false),
                    forwarded: AtomicU64::new(0),
                    claimed: AtomicU64::new(0),
                    failed: AtomicU64::new(0),
                    last_error: Mutex::new(None),
                });
                (p, counters)
            })
            .collect();
        Self {
            local,
            local_region,
            stream_key,
            consumer_name,
            claim_min_idle: Duration::from_secs(60),
            peers,
        }
    }

    /// How long an unacknowledged entry may sit before another consumer
    /// takes it over (default 60 s).
    pub fn with_claim_min_idle(mut self, min_idle: Duration) -> Self {
        self.claim_min_idle = min_idle;
        self
    }

    /// The consumer group name used for `peer_region`.
    pub fn group_name(peer_region: &str) -> String {
        format!("bridge:{peer_region}")
    }

    /// Per-peer status, in configuration order.
    pub fn status(&self) -> Vec<PeerBridgeStatus> {
        self.peers.iter().map(|(_, c)| c.status()).collect()
    }

    pub fn local_region(&self) -> Option<&str> {
        self.local_region.as_deref()
    }

    /// Start one forwarding task per peer. Each reads the local stream on
    /// a connection of its own (its reads block), connects to its peer
    /// with backoff, forwards batches (retrying until the peer takes
    /// them), and never gives up.
    pub fn spawn(&self) -> Vec<JoinHandle<()>> {
        self.peers
            .iter()
            .map(|(peer, counters)| {
                let local = self.local.clone();
                let stream_key = self.stream_key.clone();
                let group = Self::group_name(&peer.region);
                let consumer_name = self.consumer_name.clone();
                let peer = peer.clone();
                let counters = Arc::clone(counters);
                let local_region = self.local_region.clone();
                let claim_min_idle = self.claim_min_idle;
                tokio::spawn(async move {
                    let local = until_ok("opening a local Redis connection", &counters, || {
                        local.detached()
                    })
                    .await;
                    let consumer =
                        RedisStreamConsumer::new(local, stream_key, group, consumer_name);
                    run_peer(consumer, peer, counters, local_region, claim_min_idle).await
                })
            })
            .collect()
    }
}

/// Retry `op` with exponential backoff until it succeeds.
async fn until_ok<T, F, Fut>(what: &str, counters: &PeerCounters, mut op: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut backoff = Duration::from_secs(1);
    loop {
        match op().await {
            Ok(v) => return v,
            Err(e) => {
                counters.fail(&e);
                warn!(peer = %counters.region, error = %e, retry_in_secs = backoff.as_secs(), "Event bridge: {what} failed");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        }
    }
}

async fn run_peer(
    consumer: RedisStreamConsumer,
    peer: PeerRegion,
    counters: Arc<PeerCounters>,
    local_region: Option<String>,
    claim_min_idle: Duration,
) {
    let peer_storage = until_ok("connecting to peer Redis", &counters, || {
        RedisStorage::new(RedisConfig {
            url: peer.redis_url.clone(),
            key_prefix: peer.key_prefix.clone(),
        })
    })
    .await;
    counters.connected.store(true, Ordering::Relaxed);
    let (peer_channel, peer_stream) = bus_keys(&peer_storage);
    until_ok("creating consumer group", &counters, || {
        consumer.ensure_group()
    })
    .await;
    info!(
        peer = %peer.region,
        local_region = local_region.as_deref().unwrap_or("-"),
        "Event bridge forwarding to peer region"
    );

    let forward = |entries: Vec<StreamEntry>| {
        let peer_storage = peer_storage.clone();
        let peer_channel = peer_channel.clone();
        let peer_stream = peer_stream.clone();
        let local_region = local_region.clone();
        async move {
            let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
            let mut pipe = redis::pipe();
            let mut n = 0usize;
            for entry in entries {
                if !forwardable(&entry.envelope) {
                    continue;
                }
                let json = serde_json::to_string(&relayed_envelope(
                    entry.envelope,
                    local_region.as_deref(),
                ))?;
                queue_publish(&mut pipe, &peer_channel, &peer_stream, &json);
                n += 1;
            }
            if n > 0 {
                let mut conn = peer_storage.connection();
                let _: () = pipe
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| anyhow::anyhow!("forwarding to peer failed: {e}"))?;
            }
            Ok::<_, anyhow::Error>((ids, n))
        }
    };

    let mut last_claim: Option<Instant> = None;
    let mut idle = true;
    loop {
        let interval = if idle {
            IDLE_CLAIM_INTERVAL
        } else {
            CLAIM_INTERVAL
        };
        let due = last_claim.is_none_or(|t| t.elapsed() >= interval);
        let entries = if due {
            match consumer.claim_stale(claim_min_idle, BATCH).await {
                Ok(stale) => {
                    last_claim = Some(Instant::now());
                    if !stale.is_empty() {
                        counters
                            .claimed
                            .fetch_add(stale.len() as u64, Ordering::Relaxed);
                        info!(peer = %peer.region, count = stale.len(), "Event bridge: took over unacknowledged entries");
                    }
                    stale
                }
                Err(e) => {
                    // Try again on the next pass rather than in a minute.
                    counters.fail(&e);
                    warn!(peer = %peer.region, error = %e, "Event bridge: claiming stale entries failed");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        let entries = if entries.is_empty() {
            match consumer.read_entries(BATCH, 2000).await {
                Ok(entries) => entries,
                Err(e) => {
                    counters.fail(&e);
                    warn!(peer = %peer.region, error = %e, "Event bridge: reading local stream failed");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            }
        } else {
            entries
        };
        idle = entries.is_empty();
        if idle {
            continue;
        }

        // Forward, retrying the same batch until the peer takes it, then
        // acknowledge everything in it (relayed entries included: they are
        // done as far as this group is concerned).
        let mut batch = Some(entries);
        let (ids, n) = until_ok("forwarding a batch", &counters, || {
            let entries = batch.take().expect("batch retried after success");
            let clone = entries.clone();
            let fut = forward(entries);
            batch = Some(clone);
            fut
        })
        .await;
        counters.connected.store(true, Ordering::Relaxed);
        counters.forwarded.fetch_add(n as u64, Ordering::Relaxed);
        if n > 0 {
            debug!(peer = %peer.region, forwarded = n, "Event bridge: batch forwarded");
        }
        if let Err(e) = consumer.ack(&ids).await {
            counters.fail(&e);
            warn!(peer = %peer.region, error = %e, "Event bridge: acknowledging forwarded entries failed; they will be re-claimed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_events::SystemEvent;
    use std::time::SystemTime;

    fn event() -> PlatformEvent {
        PlatformEvent::System(SystemEvent::NodeLeft {
            node_id: "cm-1".into(),
            reason: "test".into(),
            timestamp: SystemTime::UNIX_EPOCH,
        })
    }

    #[test]
    fn envelope_is_wire_compatible_with_older_versions() {
        // Old writer: no region, no relayed flag.
        let old = format!(
            r#"{{"node_id":"cm-1","event":{}}}"#,
            serde_json::to_string(&event()).unwrap()
        );
        let env: EventEnvelope = serde_json::from_str(&old).unwrap();
        assert_eq!(env.origin_region, None);
        assert!(!env.relayed);
        assert!(forwardable(&env));

        // New writer without a region and not relayed: same shape as before.
        let json = serde_json::to_string(&EventEnvelope {
            node_id: "cm-1".into(),
            origin_region: None,
            relayed: false,
            event: event(),
        })
        .unwrap();
        assert!(!json.contains("origin_region") && !json.contains("relayed"));
    }

    #[test]
    fn relayed_envelopes_keep_their_origin_and_are_not_forwarded_again() {
        let from_labeled = relayed_envelope(
            EventEnvelope {
                node_id: "cm-us".into(),
                origin_region: Some("us-east".into()),
                relayed: false,
                event: event(),
            },
            Some("bridge-region"),
        );
        assert!(from_labeled.relayed);
        assert_eq!(from_labeled.origin_region.as_deref(), Some("us-east"));
        assert_eq!(from_labeled.node_id, "cm-us");
        assert!(!forwardable(&from_labeled));

        // An unlabeled publisher's event gets the bridge's region.
        let from_unlabeled = relayed_envelope(
            EventEnvelope {
                node_id: "cm-old".into(),
                origin_region: None,
                relayed: false,
                event: event(),
            },
            Some("us-east"),
        );
        assert_eq!(from_unlabeled.origin_region.as_deref(), Some("us-east"));

        let round: EventEnvelope =
            serde_json::from_str(&serde_json::to_string(&from_labeled).unwrap()).unwrap();
        assert!(round.relayed && !forwardable(&round));
    }

    #[test]
    fn stream_entries_parse_and_skip_junk() {
        use redis::Value::{Array, BulkString};
        let good = serde_json::to_string(&EventEnvelope {
            node_id: "cm-1".into(),
            origin_region: Some("eu-west".into()),
            relayed: true,
            event: event(),
        })
        .unwrap();
        let entries = vec![
            Array(vec![
                BulkString(b"1-0".to_vec()),
                Array(vec![
                    BulkString(b"data".to_vec()),
                    BulkString(good.into_bytes()),
                ]),
            ]),
            Array(vec![
                BulkString(b"2-0".to_vec()),
                Array(vec![
                    BulkString(b"data".to_vec()),
                    BulkString(b"{}".to_vec()),
                ]),
            ]),
            Array(vec![
                BulkString(b"3-0".to_vec()),
                Array(vec![
                    BulkString(b"other".to_vec()),
                    BulkString(b"x".to_vec()),
                ]),
            ]),
        ];
        let parsed = parse_entries(&entries);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "1-0");
        assert_eq!(parsed[0].envelope.origin_region.as_deref(), Some("eu-west"));
        assert!(parsed[0].envelope.relayed);

        let wrapped = parse_stream_results(Array(vec![Array(vec![
            BulkString(b"fcp:event_stream".to_vec()),
            Array(entries),
        ])]))
        .unwrap();
        assert_eq!(wrapped.len(), 1);
        assert!(parse_stream_results(redis::Value::Nil).unwrap().is_empty());
    }
}
