//! Region-aware membership against a real Redis. Gated on
//! `FCP_TEST_REDIS_URL` (a `redis:7` service in CI); skipped with a notice
//! when unset. See `docs/MULTI_REGION.md`.

use fcp_cluster::{ClusterManager, ClusterMember};
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::time::Duration;

fn member(id: &str, node_type: &str, region: Option<&str>, calls: usize) -> ClusterMember {
    ClusterMember {
        node_id: id.to_string(),
        node_type: node_type.to_string(),
        http_address: "127.0.0.1:8080".parse().unwrap(),
        sip_address: "127.0.0.1:5060".parse().unwrap(),
        sip_tls_address: None,
        capabilities: Vec::new(),
        active_calls: calls,
        started_at: 0,
        version: "test".to_string(),
        public_url: None,
        capacity: None,
        draining: false,
        region: region.map(str::to_owned),
        video_budget: Vec::new(),
        registering_trunks: Vec::new(),
        view: None,
    }
}

/// One manager per member, all on one throwaway keyspace, heartbeating
/// every second.
async fn cluster(members: Vec<ClusterMember>) -> Option<Vec<ClusterManager>> {
    let url = match std::env::var("FCP_TEST_REDIS_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("FCP_TEST_REDIS_URL not set; skipping region membership test");
            return None;
        }
    };
    let prefix = format!("test:{}:", uuid::Uuid::new_v4().simple());
    let mut managers = Vec::new();
    for m in members {
        let storage = RedisStorage::new(RedisConfig {
            url: url.clone(),
            key_prefix: prefix.clone(),
        })
        .await
        .expect("connect to redis");
        managers.push(ClusterManager::new(storage, m).with_intervals(1, 5));
    }
    Some(managers)
}

#[tokio::test]
async fn regions_travel_with_heartbeats_and_drive_local_lookups() {
    let Some(managers) = cluster(vec![
        member("cm-us", "call_manager", Some("us-east"), 0),
        member("conf-us-busy", "conference", Some("us-east"), 20),
        member("conf-us-idle", "conference", Some("us-east"), 2),
        member("conf-eu", "conference", Some("eu-west"), 0),
        member("conf-legacy", "conference", None, 0),
    ])
    .await
    else {
        return;
    };
    // Every node advertises its own active-call count.
    for (cm, calls) in managers.iter().zip([0usize, 20, 2, 0, 0]) {
        cm.update_active_calls(calls);
    }
    let handles: Vec<_> = managers.iter().map(|cm| cm.start()).collect();
    tokio::time::sleep(Duration::from_secs(3)).await;

    let us = &managers[0];
    assert_eq!(us.region(), Some("us-east"));

    // Regions arrive intact through Redis, and an unlabeled node stays so.
    let all = us.members();
    assert_eq!(all.len(), 4, "{all:?}");
    let region_of = |id: &str| {
        all.iter()
            .find(|m| m.node_id == id)
            .and_then(|m| m.region.clone())
    };
    assert_eq!(region_of("conf-eu").as_deref(), Some("eu-west"));
    assert_eq!(region_of("conf-us-idle").as_deref(), Some("us-east"));
    assert_eq!(region_of("conf-legacy"), None);

    // Local views.
    let mut local: Vec<String> = us.local_members().into_iter().map(|m| m.node_id).collect();
    local.sort();
    assert_eq!(local, ["conf-us-busy", "conf-us-idle"]);
    let eu: Vec<String> = us
        .members_in_region("eu-west")
        .into_iter()
        .map(|m| m.node_id)
        .collect();
    assert_eq!(eu, ["conf-eu"]);
    assert!(us.members_in_region("ap-south").is_empty());

    // Nearest least-loaded: the idle US conference node, not the idler EU
    // or unlabeled ones; cluster-wide the idle ones win.
    assert_eq!(
        us.least_loaded_near("conference").unwrap().node_id,
        "conf-us-idle"
    );
    let global = us.least_loaded("conference").unwrap();
    assert!(
        global.node_id == "conf-eu" || global.node_id == "conf-legacy",
        "{global:?}"
    );
    // No call managers nearby at all: falls back cluster-wide, and there
    // are none of those either (self is excluded).
    assert!(us.least_loaded_near("call_manager").is_none());

    // A node without a region has no local members, but region queries
    // and the nearest fallback still work for it.
    let legacy = &managers[4];
    assert_eq!(legacy.region(), None);
    assert!(legacy.local_members().is_empty());
    assert_eq!(legacy.members_in_region("eu-west").len(), 1);
    assert_eq!(
        legacy.least_loaded_near("conference").unwrap().node_id,
        "conf-eu"
    );

    for cm in &managers {
        cm.shutdown().await;
    }
    for h in handles {
        h.abort();
    }
}
