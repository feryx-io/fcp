//! Parking slots claimed in the cluster (`docs/PARKING.md` §2, D8 finding
//! 35): keyed by orbit alone, two tenants shared every slot and two nodes
//! parking at once both wrote the same key, the second replacing the
//! first. A slot is now the tenant's and one call's. Gated on
//! `FCP_TEST_REDIS_URL`.

use fcp_core::storage::{ClusterParkedCall, ClusterParkingStore};
use fcp_core::TenantId;
use fcp_storage_redis::parking::RedisParkingStore;
use fcp_storage_redis::{RedisConfig, RedisStorage};

fn parked(call: &str, node: &str, tenant: &TenantId) -> ClusterParkedCall {
    ClusterParkedCall {
        call_id: call.to_string(),
        node_id: node.to_string(),
        parker_extension: "1001".to_string(),
        tenant_id: tenant.to_string(),
        slot: "701".to_string(),
        lot_id: "default".to_string(),
        parked_at_unix: 0,
    }
}

#[tokio::test]
async fn a_slot_is_one_calls_and_each_tenant_has_its_own() {
    let Ok(url) = std::env::var("FCP_TEST_REDIS_URL") else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping");
        return;
    };
    let config = RedisConfig {
        url,
        key_prefix: format!("test:{}:", uuid::Uuid::new_v4().simple()),
    };
    // Two nodes, each with its own connection.
    let node_a = RedisParkingStore::new(RedisStorage::new(config.clone()).await.unwrap());
    let node_b = RedisParkingStore::new(RedisStorage::new(config).await.unwrap());
    let (one, two) = (TenantId::new(), TenantId::new());

    assert!(node_a
        .cluster_park(&one, "701", parked("call-a", "a", &one), 60)
        .await
        .unwrap());
    assert!(
        !node_b
            .cluster_park(&one, "701", parked("call-b", "b", &one), 60)
            .await
            .unwrap(),
        "a second call does not take a held slot"
    );
    let held = node_b.cluster_lookup(&one, "701").await.unwrap().unwrap();
    assert_eq!(held.call_id, "call-a", "the first call keeps it");

    assert!(
        node_b
            .cluster_park(&two, "701", parked("call-c", "b", &two), 60)
            .await
            .unwrap(),
        "another tenant's 701 is its own"
    );

    let taken = node_b.cluster_unpark(&one, "701").await.unwrap().unwrap();
    assert_eq!(taken.call_id, "call-a");
    assert!(node_a.cluster_unpark(&one, "701").await.unwrap().is_none());
    assert!(node_a
        .cluster_park(&one, "701", parked("call-d", "a", &one), 60)
        .await
        .unwrap());
    assert_eq!(
        node_a
            .cluster_lookup(&two, "701")
            .await
            .unwrap()
            .unwrap()
            .call_id,
        "call-c"
    );
}
