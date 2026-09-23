//! A partition is visible (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3, phase 3),
//! against a real Redis. Gated on `FCP_TEST_REDIS_URL`; skipped with a
//! notice when unset.
//!
//! The two shapes a node can see: every other region going quiet at once
//! while its own stays, and the cluster store itself out of reach — the
//! latter through a proxy this test cuts, as a network would.

use fcp_cluster::{ClusterManager, ClusterMember, ClusterReader, ViewState};
use fcp_events::{
    EventFilter, EventStream, EventSubscriber, InMemoryEventBus, PlatformEvent, SystemEvent,
};
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

fn member(id: &str, region: &str) -> ClusterMember {
    ClusterMember {
        node_id: id.to_string(),
        node_type: "call_manager".to_string(),
        http_address: "127.0.0.1:8080".parse().unwrap(),
        sip_address: "127.0.0.1:5060".parse().unwrap(),
        sip_tls_address: None,
        capabilities: Vec::new(),
        active_calls: 0,
        started_at: 0,
        version: "test".to_string(),
        public_url: None,
        capacity: None,
        draining: false,
        region: Some(region.to_string()),
        video_budget: Vec::new(),
        registering_trunks: Vec::new(),
        view: None,
    }
}

fn redis_url() -> Option<String> {
    let url = std::env::var("FCP_TEST_REDIS_URL").ok();
    if url.is_none() {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping partition test");
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
fn node(storage: RedisStorage, id: &str, region: &str) -> ClusterManager {
    ClusterManager::new(storage, member(id, region)).with_intervals(1, 3)
}

/// Wait until `check` holds, or fail saying `what`.
async fn until(what: &str, within: Duration, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + within;
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The next `system.cluster_view_changed` on `bus`.
async fn view_event(events: &mut EventStream) -> SystemEvent {
    let deadline = Duration::from_secs(10);
    loop {
        let event = tokio::time::timeout(deadline, events.next())
            .await
            .expect("an event in time")
            .expect("the bus is open");
        if let PlatformEvent::System(e @ SystemEvent::ClusterViewChanged { .. }) = event {
            return e;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_other_region_going_quiet_is_said_and_its_return_heals_it() {
    let Some(url) = redis_url() else { return };
    let prefix = format!("test:{}:", uuid::Uuid::new_v4().simple());
    let bus = Arc::new(InMemoryEventBus::new());
    let mut events = bus.subscribe(EventFilter::new()).await.unwrap();

    let us = node(storage(&url, &prefix).await, "cm-us-1", "us-east")
        .with_event_publisher(Arc::clone(&bus) as Arc<dyn fcp_events::EventPublisher>);
    let us_peer = node(storage(&url, &prefix).await, "cm-us-2", "us-east");
    let eu = node(storage(&url, &prefix).await, "cm-eu-1", "eu-west");
    let _h1 = us.start();
    let _h2 = us_peer.start();
    let eu_handle = eu.start();
    until("the nodes see each other", Duration::from_secs(10), || {
        us.members().len() == 2
    })
    .await;
    assert_eq!(us.view().state, ViewState::Whole);

    // The EU node goes quiet — no shutdown, its heartbeat just stops, as a
    // cut link or a lost region looks from here.
    eu_handle.abort();
    until(
        "the US node never said it lost the other region",
        Duration::from_secs(15),
        || us.view().state == ViewState::RemoteRegionsLost,
    )
    .await;
    let view = us.view();
    assert_eq!(view.lost_regions, vec!["eu-west"]);
    assert!(view.summary.contains("us-east stays"), "{}", view.summary);

    let SystemEvent::ClusterViewChanged {
        state,
        previous,
        lost_regions,
        ..
    } = view_event(&mut events).await
    else {
        unreachable!()
    };
    assert_eq!(
        (state.as_str(), previous.as_str()),
        ("remote_regions_lost", "whole")
    );
    assert_eq!(lost_regions, vec!["eu-west"]);

    // Its heartbeat record says so, so the admin API can.
    let reader = ClusterReader::new(storage(&url, &prefix).await, 3);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let nodes = reader.list_nodes().await.unwrap();
    let us_record = nodes.iter().find(|n| n.node_id == "cm-us-1").unwrap();
    assert_eq!(
        us_record.view.as_ref().map(|v| v.state),
        Some(ViewState::RemoteRegionsLost),
        "the heartbeat record does not carry the view"
    );
    assert!(reader.get_stats().await.unwrap().nodes_with_partial_view >= 1);

    // The region comes back.
    let eu_again = node(storage(&url, &prefix).await, "cm-eu-2", "eu-west");
    let _h3 = eu_again.start();
    until(
        "the US node never saw the region return",
        Duration::from_secs(10),
        || us.view().state == ViewState::Whole,
    )
    .await;
    let SystemEvent::ClusterViewChanged {
        state, lasted_secs, ..
    } = view_event(&mut events).await
    else {
        unreachable!()
    };
    assert_eq!(state, "whole");
    assert!(
        lasted_secs.is_some(),
        "the healing event says how long it lasted"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_region_that_shuts_down_is_not_a_partition() {
    let Some(url) = redis_url() else { return };
    let prefix = format!("test:{}:", uuid::Uuid::new_v4().simple());
    let bus = Arc::new(InMemoryEventBus::new());
    let mut events = bus.subscribe(EventFilter::new()).await.unwrap();

    let us = node(storage(&url, &prefix).await, "cm-us-1", "us-east")
        .with_event_publisher(Arc::clone(&bus) as Arc<dyn fcp_events::EventPublisher>);
    let eu = node(storage(&url, &prefix).await, "cm-eu-1", "eu-west");
    let _h1 = us.start();
    let _h2 = eu.start();
    until("the nodes see each other", Duration::from_secs(10), || {
        us.members().len() == 1
    })
    .await;

    eu.shutdown().await;
    until(
        "the US node never saw the EU node leave",
        Duration::from_secs(10),
        || us.members().is_empty(),
    )
    .await;
    // A few more discoveries, and still nothing to say.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(us.view().state, ViewState::Whole);

    // And the node that left is said to have shut down, not gone quiet.
    let reason = loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.next())
            .await
            .expect("a NodeLeft event")
            .unwrap();
        match event {
            PlatformEvent::System(SystemEvent::NodeLeft { reason, .. }) => break reason,
            PlatformEvent::System(SystemEvent::ClusterViewChanged { summary, .. }) => {
                panic!("a shutdown was taken for a partition: {summary}")
            }
            _ => {}
        }
    };
    assert_eq!(reason, "shutdown");
}

/// A TCP proxy standing in for the link to Redis: `cut` drops every
/// connection and refuses new ones, `restore` lets them through again.
struct Link {
    port: u16,
    target: SocketAddr,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Link {
    async fn open(target: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let link = Self {
            port,
            target,
            tasks: Arc::new(Mutex::new(Vec::new())),
        };
        link.serve(listener);
        link
    }

    fn serve(&self, listener: TcpListener) {
        let target = self.target;
        let tasks = Arc::clone(&self.tasks);
        let accept = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = listener.accept().await {
                let pipe = tokio::spawn(async move {
                    if let Ok(mut outbound) = TcpStream::connect(target).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
                tasks.lock().unwrap().push(pipe);
            }
        });
        self.tasks.lock().unwrap().push(accept);
    }

    fn cut(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    async fn restore(&self) {
        let listener = TcpListener::bind(("127.0.0.1", self.port)).await.unwrap();
        self.serve(listener);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_node_cut_off_from_the_store_says_so_and_what_it_still_reaches() {
    let Some(url) = redis_url() else { return };
    let parsed = redis::Client::open(url.as_str()).unwrap();
    let info = parsed.get_connection_info();
    let (host, port) = match &info.addr {
        redis::ConnectionAddr::Tcp(host, port) => (host.clone(), *port),
        other => panic!("a TCP Redis is needed here, not {other:?}"),
    };
    let target: SocketAddr = format!("{host}:{port}")
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)));
    let db = info.redis.db;
    let link = Link::open(target).await;
    let through_link = format!("redis://127.0.0.1:{}/{db}", link.port);

    let prefix = format!("test:{}:", uuid::Uuid::new_v4().simple());
    let cut_off = node(storage(&through_link, &prefix).await, "cm-us-1", "us-east")
        // The database is still there: the node is the one cut off.
        .with_database_probe(Arc::new(|| Box::pin(async { true })));
    let other = node(storage(&url, &prefix).await, "cm-eu-1", "eu-west");
    let _h1 = cut_off.start();
    let _h2 = other.start();
    until("the nodes see each other", Duration::from_secs(10), || {
        cut_off.members().len() == 1
    })
    .await;

    link.cut();
    until(
        "the node never said it cannot reach the store",
        Duration::from_secs(15),
        || cut_off.view().state == ViewState::StoreUnreachable,
    )
    .await;
    until(
        "the database probe was never asked",
        Duration::from_secs(10),
        || cut_off.view().database_reachable == Some(true),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let view = cut_off.view();
    assert!(view.view_age_secs.unwrap_or_default() >= 2, "{view:?}");
    assert!(
        view.summary.contains("probably the one cut off"),
        "{}",
        view.summary
    );
    // What §2.3 now says plainly: the member list does not shrink while the
    // store is out of reach. It is as old as `view_age_secs`.
    assert_eq!(
        cut_off.members().len(),
        1,
        "the stale list is kept, not emptied"
    );

    // The link comes back, and so does the node — within seconds, not the
    // minutes the Redis client's default backoff used to take.
    link.restore().await;
    until("the node never came back", Duration::from_secs(10), || {
        cut_off.view().state == ViewState::Whole
    })
    .await;
    assert_eq!(cut_off.view().database_reachable, None);
}
