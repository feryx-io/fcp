//! Cross-region event bridge against a real Redis. Gated on
//! `FCP_TEST_REDIS_URL` (a `redis:7` service in CI); skipped with a notice
//! when unset. Two key prefixes on one server stand in for two regions'
//! Redis instances. See `docs/MULTI_REGION.md`.

use fcp_events::{EventPublisher, PlatformEvent, SystemEvent};
use fcp_storage_redis::event_bus::{
    EventBridge, PeerRegion, RedisEventBus, RedisStreamConsumer, StreamEntry,
};
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::time::{Duration, SystemTime};

fn redis_url() -> Option<String> {
    match std::env::var("FCP_TEST_REDIS_URL") {
        Ok(u) => Some(u),
        Err(_) => {
            eprintln!("FCP_TEST_REDIS_URL not set; skipping event bridge test");
            None
        }
    }
}

/// A "region": its own keyspace on the shared test Redis.
struct Region {
    label: &'static str,
    prefix: String,
    storage: RedisStorage,
}

impl Region {
    async fn new(url: &str, label: &'static str) -> Self {
        let prefix = format!("test:{}:{}:", uuid::Uuid::new_v4().simple(), label);
        let storage = RedisStorage::new(RedisConfig {
            url: url.to_string(),
            key_prefix: prefix.clone(),
        })
        .await
        .expect("connect to redis");
        Self {
            label,
            prefix,
            storage,
        }
    }

    fn peer(&self, url: &str) -> PeerRegion {
        PeerRegion {
            region: self.label.to_string(),
            redis_url: url.to_string(),
            key_prefix: self.prefix.clone(),
        }
    }

    /// A publisher for a node of this region.
    fn publisher(&self, node_id: &str) -> RedisEventBus {
        RedisEventBus::new(self.storage.clone(), node_id.to_string())
            .with_region(Some(self.label.to_string()))
    }

    /// A consumer group reading this region's stream, like its
    /// events-server does.
    async fn consumer(&self, group: &str) -> RedisStreamConsumer {
        let c = RedisStreamConsumer::new(
            self.storage.clone(),
            self.storage.key("event_stream"),
            group.to_string(),
            format!("{group}-consumer"),
        );
        c.ensure_group().await.unwrap();
        c
    }

    async fn stream_len(&self) -> usize {
        let mut conn = self.storage.connection();
        redis::cmd("XLEN")
            .arg(self.storage.key("event_stream"))
            .query_async(&mut conn)
            .await
            .unwrap()
    }
}

fn node_left(node_id: &str) -> PlatformEvent {
    PlatformEvent::System(SystemEvent::NodeLeft {
        node_id: node_id.to_string(),
        reason: "test".to_string(),
        timestamp: SystemTime::now(),
    })
}

fn left_node_id(event: &PlatformEvent) -> &str {
    match event {
        PlatformEvent::System(SystemEvent::NodeLeft { node_id, .. }) => node_id,
        other => panic!("unexpected event {other:?}"),
    }
}

/// Read until `n` entries have arrived or `timeout` passes.
async fn collect(consumer: &RedisStreamConsumer, n: usize, timeout: Duration) -> Vec<StreamEntry> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut got = Vec::new();
    while got.len() < n && tokio::time::Instant::now() < deadline {
        got.extend(consumer.read_entries(10, 500).await.unwrap());
    }
    got
}

#[tokio::test]
async fn events_cross_regions_once_and_never_loop_back() {
    let Some(url) = redis_url() else { return };
    let us = Region::new(&url, "us-east").await;
    let eu = Region::new(&url, "eu-west").await;

    // Bridges both ways, as a full mesh of two regions would run.
    let us_bridge = EventBridge::new(
        us.storage.clone(),
        Some("us-east".into()),
        "us-events-1".into(),
        vec![eu.peer(&url)],
    );
    let eu_bridge = EventBridge::new(
        eu.storage.clone(),
        Some("eu-west".into()),
        "eu-events-1".into(),
        vec![us.peer(&url)],
    );
    // Consumer groups are created by the bridge tasks; create the
    // events-server-like readers first so they see everything from the start.
    let eu_reader = eu.consumer("fcp-events-server").await;
    let us_reader = us.consumer("fcp-events-server").await;
    let handles: Vec<_> = us_bridge
        .spawn()
        .into_iter()
        .chain(eu_bridge.spawn())
        .collect();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // A US node publishes two events; an EU node one.
    let us_pub = us.publisher("cm-us-1");
    let eu_pub = eu.publisher("cm-eu-1");
    us_pub.publish(node_left("a")).await.unwrap();
    us_pub.publish(node_left("b")).await.unwrap();
    eu_pub.publish(node_left("c")).await.unwrap();

    // The EU stream ends up with its own event plus the two relayed ones.
    let eu_entries = collect(&eu_reader, 3, Duration::from_secs(10)).await;
    let mut seen: Vec<(String, bool, Option<String>, String)> = eu_entries
        .iter()
        .map(|e| {
            (
                left_node_id(&e.envelope.event).to_string(),
                e.envelope.relayed,
                e.envelope.origin_region.clone(),
                e.envelope.node_id.clone(),
            )
        })
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            ("a".into(), true, Some("us-east".into()), "cm-us-1".into()),
            ("b".into(), true, Some("us-east".into()), "cm-us-1".into()),
            ("c".into(), false, Some("eu-west".into()), "cm-eu-1".into()),
        ]
    );

    // And the US stream with its own two plus the relayed EU one.
    let us_entries = collect(&us_reader, 3, Duration::from_secs(10)).await;
    let mut relayed: Vec<&str> = us_entries
        .iter()
        .filter(|e| e.envelope.relayed)
        .map(|e| left_node_id(&e.envelope.event))
        .collect();
    relayed.sort();
    assert_eq!(relayed, ["c"]);

    // Nothing loops: give the bridges time to (wrongly) relay the relayed
    // entries back, then check the streams did not grow.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        us.stream_len().await,
        3,
        "US stream grew: a relayed entry was relayed back"
    );
    assert_eq!(
        eu.stream_len().await,
        3,
        "EU stream grew: a relayed entry was relayed back"
    );

    // The bridges report what they did.
    let us_status = &us_bridge.status()[0];
    assert_eq!(us_status.region, "eu-west");
    assert!(us_status.connected);
    assert_eq!(us_status.forwarded, 2, "{us_status:?}");
    assert_eq!(eu_bridge.status()[0].forwarded, 1);

    for h in handles {
        h.abort();
    }
}

#[tokio::test]
async fn a_bridge_takes_over_entries_a_dead_consumer_left_pending() {
    let Some(url) = redis_url() else { return };
    let us = Region::new(&url, "us-east").await;
    let eu = Region::new(&url, "eu-west").await;

    // A previous bridge instance in the US read one entry for the EU
    // group and died before forwarding or acknowledging it.
    us.publisher("cm-us-1")
        .publish(node_left("orphan"))
        .await
        .unwrap();
    let dead = RedisStreamConsumer::new(
        us.storage.clone(),
        us.storage.key("event_stream"),
        EventBridge::group_name("eu-west"),
        "us-events-dead".into(),
    );
    dead.ensure_group().await.unwrap();
    let pending = dead.read_entries(10, 500).await.unwrap();
    assert_eq!(pending.len(), 1);

    // A fresh bridge instance claims it (idle threshold 0 for the test)
    // and forwards it.
    let bridge = EventBridge::new(
        us.storage.clone(),
        Some("us-east".into()),
        "us-events-2".into(),
        vec![eu.peer(&url)],
    )
    .with_claim_min_idle(Duration::ZERO);
    let eu_reader = eu.consumer("fcp-events-server").await;
    let handles = bridge.spawn();

    let got = collect(&eu_reader, 1, Duration::from_secs(10)).await;
    assert_eq!(got.len(), 1);
    assert_eq!(left_node_id(&got[0].envelope.event), "orphan");
    assert!(got[0].envelope.relayed);
    assert_eq!(bridge.status()[0].forwarded, 1);

    for h in handles {
        h.abort();
    }
}
