//! Conference registry against a real Redis. Gated on `FCP_TEST_REDIS_URL`
//! (a `redis:7` service in CI); skipped with a notice when unset.

use fcp_cluster::conference_registry::{ConferenceHost, ConferenceRegistry};
use fcp_storage_redis::{RedisConfig, RedisStorage};

/// Two registries (two nodes) sharing one throwaway keyspace.
async fn two_nodes() -> Option<(ConferenceRegistry, ConferenceRegistry)> {
    let url = match std::env::var("FCP_TEST_REDIS_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("FCP_TEST_REDIS_URL not set; skipping conference registry test");
            return None;
        }
    };
    let prefix = format!("test:{}:", uuid::Uuid::new_v4().simple());
    let mut regs = Vec::new();
    for node in ["conf-a", "conf-b"] {
        let storage = RedisStorage::new(RedisConfig {
            url: url.clone(),
            key_prefix: prefix.clone(),
        })
        .await
        .expect("connect to redis");
        regs.push(ConferenceRegistry::with_ttl(storage, node.to_string(), 30));
    }
    let b = regs.pop().unwrap();
    let a = regs.pop().unwrap();
    Some((a, b))
}

#[tokio::test]
async fn hosts_are_tracked_per_node_and_removed_at_zero() {
    let Some((a, b)) = two_nodes().await else {
        return;
    };
    assert!(a.hosts("t1", "8000").await.unwrap().is_empty());

    a.set_participants("t1", "8000", 3).await.unwrap();
    b.set_participants("t1", "8000", 1).await.unwrap();
    b.set_participants("t1", "9000", 2).await.unwrap();

    let hosts = a.hosts("t1", "8000").await.unwrap();
    assert_eq!(
        hosts,
        vec![
            ConferenceHost {
                node_id: "conf-a".into(),
                participants: 3
            },
            ConferenceHost {
                node_id: "conf-b".into(),
                participants: 1
            },
        ]
    );
    let peers = a.peers("t1", "8000").await.unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].node_id, "conf-b");

    // Rooms are per tenant.
    assert!(a.hosts("t2", "8000").await.unwrap().is_empty());
    assert_eq!(a.hosts("t1", "9000").await.unwrap().len(), 1);

    // Zero removes only that node; leaving the last node empties the room.
    a.set_participants("t1", "8000", 0).await.unwrap();
    let hosts = a.hosts("t1", "8000").await.unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].node_id, "conf-b");
    b.leave_room("t1", "8000").await.unwrap();
    assert!(a.hosts("t1", "8000").await.unwrap().is_empty());
}

#[tokio::test]
async fn lock_flag_is_room_wide_and_separate_from_hosts() {
    let Some((a, b)) = two_nodes().await else {
        return;
    };
    a.set_participants("t1", "8000", 2).await.unwrap();
    assert!(!b.room("t1", "8000").await.unwrap().locked);

    // Locked by one node, seen by the other; hosts unaffected.
    a.set_locked("t1", "8000", true).await.unwrap();
    let record = b.room("t1", "8000").await.unwrap();
    assert!(record.locked);
    assert_eq!(record.hosts.len(), 1);
    assert_eq!(record.hosts[0].node_id, "conf-a");
    assert_eq!(a.hosts("t1", "8000").await.unwrap().len(), 1);

    // A node that starts hosting the room later inherits the flag.
    b.set_participants("t1", "8000", 1).await.unwrap();
    assert!(b.room("t1", "8000").await.unwrap().locked);
    assert_eq!(b.peers("t1", "8000").await.unwrap().len(), 1);

    b.set_locked("t1", "8000", false).await.unwrap();
    let record = a.room("t1", "8000").await.unwrap();
    assert!(!record.locked);
    assert_eq!(record.hosts.len(), 2);

    // Other rooms are untouched.
    a.set_locked("t1", "9000", true).await.unwrap();
    assert!(!a.room("t1", "8000").await.unwrap().locked);
    assert!(a.room("t1", "9000").await.unwrap().locked);
}

/// A node that stops publishing vanishes from a room after one TTL even
/// while another node keeps the room's key alive.
#[tokio::test]
async fn dead_node_entries_go_stale_while_others_keep_the_room() {
    let url = match std::env::var("FCP_TEST_REDIS_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("FCP_TEST_REDIS_URL not set; skipping conference registry test");
            return;
        }
    };
    let prefix = format!("test:{}:", uuid::Uuid::new_v4().simple());
    let storage = |_: &str| {
        let url = url.clone();
        let prefix = prefix.clone();
        async move {
            RedisStorage::new(RedisConfig {
                url,
                key_prefix: prefix,
            })
            .await
            .expect("connect to redis")
        }
    };
    // The reader's TTL is what decides staleness: 1 s here.
    let a = ConferenceRegistry::with_ttl(storage("a").await, "conf-a".into(), 1);
    let b = ConferenceRegistry::with_ttl(storage("b").await, "conf-b".into(), 60);

    a.set_participants("t1", "8000", 4).await.unwrap();
    b.set_participants("t1", "8000", 2).await.unwrap();
    assert_eq!(a.hosts("t1", "8000").await.unwrap().len(), 2);

    // conf-a dies. conf-b keeps refreshing, which also keeps the key alive.
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    b.set_participants("t1", "8000", 2).await.unwrap();

    let hosts = a.hosts("t1", "8000").await.unwrap();
    assert_eq!(hosts.len(), 1, "stale conf-a entry dropped: {hosts:?}");
    assert_eq!(hosts[0].node_id, "conf-b");
    assert_eq!(hosts[0].participants, 2);
    // conf-b (60 s tolerance) still sees the raw entry as fresh enough.
    assert_eq!(b.hosts("t1", "8000").await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_webinar_s_stage_is_claimed_once_taken_from_a_gone_holder_and_released() {
    let Some((a, b)) = two_nodes().await else {
        return;
    };
    a.set_participants("t1", "7000", 1).await.unwrap();
    assert_eq!(a.claim_stage("t1", "7000", None).await.unwrap(), "conf-a");
    // B finds it held and is told by whom.
    assert_eq!(b.claim_stage("t1", "7000", None).await.unwrap(), "conf-a");
    let room = b.room("t1", "7000").await.unwrap();
    assert_eq!(room.stage.as_deref(), Some("conf-a"));
    assert_eq!(
        room.live_stage().map(|h| h.node_id.as_str()),
        Some("conf-a")
    );
    assert_eq!(room.hosts.len(), 1, "the stage is not a host");
    // B may release only its own stage.
    b.release_stage("t1", "7000").await.unwrap();
    assert_eq!(
        b.room("t1", "7000").await.unwrap().stage.as_deref(),
        Some("conf-a")
    );
    // B judged A gone: the claim takes it from A, and only from A.
    assert_eq!(
        b.claim_stage("t1", "7000", Some("conf-x")).await.unwrap(),
        "conf-a"
    );
    assert_eq!(
        b.claim_stage("t1", "7000", Some("conf-a")).await.unwrap(),
        "conf-b"
    );
    b.release_stage("t1", "7000").await.unwrap();
    assert!(a.room("t1", "7000").await.unwrap().stage.is_none());
}
