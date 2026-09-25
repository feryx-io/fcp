//! A node running another release is flagged (`docs/BACKUP_RESTORE_UPGRADE.md`
//! phase 5, `docs/BACKUP_AND_RESTORE.md` §5), against a real Redis. Gated
//! on `FCP_TEST_REDIS_URL`; skipped with a notice when unset.
//!
//! Two nodes on one release see nothing to say. A third on another
//! release starts — the first node of an upgrade — and every node groups
//! it apart, says `system.version_skew`, and the admin's reader counts
//! the skew; it shuts down, and the cluster is said to run one release
//! again.

use fcp_cluster::{ClusterManager, ClusterMember, ClusterReader};
use fcp_events::{
    EventFilter, EventStream, EventSubscriber, InMemoryEventBus, PlatformEvent, SystemEvent,
};
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::sync::Arc;
use std::time::Duration;

fn member(id: &str, version: &str) -> ClusterMember {
    ClusterMember {
        node_id: id.to_string(),
        node_type: "call_manager".to_string(),
        http_address: "127.0.0.1:8080".parse().unwrap(),
        sip_address: "127.0.0.1:5060".parse().unwrap(),
        sip_tls_address: None,
        capabilities: Vec::new(),
        active_calls: 0,
        started_at: 0,
        version: version.to_string(),
        public_url: None,
        capacity: None,
        draining: false,
        region: None,
        video_budget: Vec::new(),
        registering_trunks: Vec::new(),
        view: None,
    }
}

fn redis_url() -> Option<String> {
    let url = std::env::var("FCP_TEST_REDIS_URL").ok();
    if url.is_none() {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the version skew test");
    }
    url
}

async fn storage(url: &str, prefix: &str) -> RedisStorage {
    RedisStorage::new(RedisConfig {
        url: url.to_string(),
        key_prefix: prefix.to_string(),
    })
    .await
    .expect("connect to redis")
}

/// A node heartbeating every second with a three-second TTL.
fn node(storage: RedisStorage, id: &str, version: &str) -> ClusterManager {
    ClusterManager::new(storage, member(id, version)).with_intervals(1, 3)
}

async fn until(what: &str, within: Duration, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + within;
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The next `system.version_skew` on `bus`.
async fn skew_event(events: &mut EventStream) -> SystemEvent {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .expect("an event in time")
            .expect("the bus is open");
        if let PlatformEvent::System(e @ SystemEvent::VersionSkew { .. }) = event {
            return e;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_node_on_another_release_is_flagged_until_it_goes() {
    let Some(url) = redis_url() else { return };
    let prefix = format!("test:{}:", uuid::Uuid::new_v4().simple());
    let bus = Arc::new(InMemoryEventBus::new());
    let mut events = bus.subscribe(EventFilter::new()).await.unwrap();

    let first = node(storage(&url, &prefix).await, "cm-1", "1.0.0")
        .with_event_publisher(Arc::clone(&bus) as Arc<dyn fcp_events::EventPublisher>);
    let second = node(storage(&url, &prefix).await, "cm-2", "1.0.0");
    let _h1 = first.start();
    let _h2 = second.start();
    until("the nodes see each other", Duration::from_secs(10), || {
        first.members().len() == 1
    })
    .await;
    let versions = first.versions();
    assert!(!versions.skew, "{versions:?}");
    assert_eq!(versions.summary, "every node on 1.0.0 (2 nodes)");

    // The first node of an upgrade: another release joins.
    let third = node(storage(&url, &prefix).await, "cm-3", "2.0.0");
    let _h3 = third.start();
    until("the skew was never seen", Duration::from_secs(10), || {
        first.versions().skew
    })
    .await;
    let versions = first.versions();
    assert_eq!(versions.versions["1.0.0"], vec!["cm-1", "cm-2"]);
    assert_eq!(versions.versions["2.0.0"], vec!["cm-3"]);
    assert_eq!(versions.summary, "2 nodes on 1.0.0, 1 on 2.0.0 (cm-3)");

    let SystemEvent::VersionSkew {
        node_id,
        versions: said,
        skew,
        ..
    } = skew_event(&mut events).await
    else {
        unreachable!()
    };
    assert_eq!(node_id, "cm-1");
    assert!(skew);
    assert_eq!(said["2.0.0"], vec!["cm-3"]);

    // The admin's reader counts it from the heartbeat records.
    let reader = ClusterReader::new(storage(&url, &prefix).await, 3);
    let stats = reader.get_stats().await.unwrap();
    assert!(stats.version_skew, "{:?}", stats.versions);
    assert_eq!(stats.versions["2.0.0"], vec!["cm-3"]);
    assert_eq!(stats.versions.len(), 2);

    // The odd node goes: one release again, said once.
    third.shutdown().await;
    until("the skew never resolved", Duration::from_secs(10), || {
        !first.versions().skew
    })
    .await;
    let SystemEvent::VersionSkew { skew, summary, .. } = skew_event(&mut events).await else {
        unreachable!()
    };
    assert!(!skew);
    assert_eq!(summary, "every node on 1.0.0 (2 nodes)");
    assert!(!reader.get_stats().await.unwrap().version_skew);
}
