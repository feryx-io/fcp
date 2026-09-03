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
