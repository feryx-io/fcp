//! Integration tests for multi-node cluster scenarios.
//!
//! These tests require a running Redis instance and are marked `#[ignore]`.
//! Run with:
//!
//! ```bash
//! TEST_REDIS_URL=redis://localhost:6379 cargo test -p fcp-cluster -- --ignored
//! ```
//!
//! Each test uses a UUID-based key prefix for isolation, so multiple tests
//! can run concurrently against the same Redis instance.

use fcp_cluster::{CallRegistry, ClusterManager, ClusterMember};
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Get Redis URL from environment or skip.
fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string())
}

/// Generate a unique key prefix for test isolation.
fn test_key_prefix() -> String {
    format!("test:{}:", uuid::Uuid::new_v4())
}

/// Create a ClusterMember with a unique node ID.
fn test_member(name: &str, http_port: u16) -> ClusterMember {
    ClusterMember {
        node_id: format!("{}-{}", name, uuid::Uuid::new_v4()),
        node_type: "call_manager".to_string(),
        http_address: SocketAddr::from(([127, 0, 0, 1], http_port)),
        sip_address: SocketAddr::from(([127, 0, 0, 1], 5060)),
        sip_tls_address: None,
        capabilities: vec!["registrar".to_string(), "b2bua".to_string()],
        active_calls: 0,
        started_at: 1000,
        version: "0.1.0".to_string(),
        draining: false,
    }
}

/// Clean up Redis keys matching a prefix.
async fn cleanup_keys(storage: &RedisStorage) {
    let mut conn = storage.connection();
    let pattern = storage.key("*");
    let mut cursor = 0u64;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(&pattern)
            .arg("COUNT")
            .arg(100)
            .query_async(&mut conn)
            .await
            .unwrap();

        if !keys.is_empty() {
            let mut pipe = redis::pipe();
            for key in &keys {
                pipe.del(key);
            }
            let _: Vec<()> = pipe.query_async(&mut conn).await.unwrap_or_default();
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_two_nodes_discover_each_other() {
    // Create shared storage with same prefix (simulates shared Redis)
    let config = RedisConfig {
        url: redis_url(),
        key_prefix: test_key_prefix(),
    };
    let storage = RedisStorage::new(config.clone())
        .await
        .expect("Redis connection failed");

    let node_a = test_member("node-a", 8084);
    let node_b = test_member("node-b", 8085);
    let node_a_id = node_a.node_id.clone();
    let node_b_id = node_b.node_id.clone();

    // Create cluster managers with short intervals
    let storage_b = RedisStorage::new(config).await.unwrap();

    let cm_a = ClusterManager::new(storage.clone(), node_a).with_intervals(1, 3);
    let cm_b = ClusterManager::new(storage_b.clone(), node_b).with_intervals(1, 3);

    let _handle_a = cm_a.start();
    let _handle_b = cm_b.start();

    // Wait for discovery cycles
    sleep(Duration::from_secs(3)).await;

    // Node A should see Node B
    let members_a = cm_a.members();
    assert!(
        members_a.iter().any(|m| m.node_id == node_b_id),
        "Node A should discover Node B. Members: {:?}",
        members_a.iter().map(|m| &m.node_id).collect::<Vec<_>>()
    );

    // Node B should see Node A
    let members_b = cm_b.members();
    assert!(
        members_b.iter().any(|m| m.node_id == node_a_id),
        "Node B should discover Node A. Members: {:?}",
        members_b.iter().map(|m| &m.node_id).collect::<Vec<_>>()
    );

    // Cleanup
    cm_a.shutdown().await;
    cm_b.shutdown().await;
    cleanup_keys(&storage).await;
}

#[tokio::test]
#[ignore]
async fn test_node_departure_detected() {
    let config = RedisConfig {
        url: redis_url(),
        key_prefix: test_key_prefix(),
    };
    let storage = RedisStorage::new(config.clone()).await.unwrap();
    let storage_b = RedisStorage::new(config).await.unwrap();

    let node_a = test_member("node-a", 8084);
    let node_b = test_member("node-b", 8085);
    let node_b_id = node_b.node_id.clone();

    let cm_a = ClusterManager::new(storage.clone(), node_a).with_intervals(1, 3);
    let cm_b = ClusterManager::new(storage_b, node_b).with_intervals(1, 3);

    let _handle_a = cm_a.start();
    let _handle_b = cm_b.start();

    // Wait for mutual discovery
    sleep(Duration::from_secs(3)).await;
    assert!(!cm_a.members().is_empty(), "Node A should see Node B");

    // Shut down Node B
    cm_b.shutdown().await;

    // Wait for TTL expiry (3s) + discovery cycle (1s)
    sleep(Duration::from_secs(5)).await;

    // Node A should no longer see Node B
    let members = cm_a.members();
    assert!(
        !members.iter().any(|m| m.node_id == node_b_id),
        "Node B should be gone after TTL expiry. Members: {:?}",
        members.iter().map(|m| &m.node_id).collect::<Vec<_>>()
    );

    cm_a.shutdown().await;
    cleanup_keys(&storage).await;
}

#[tokio::test]
#[ignore]
async fn test_call_registry_cross_node_lookup() {
    let config = RedisConfig {
        url: redis_url(),
        key_prefix: test_key_prefix(),
    };
    let storage = RedisStorage::new(config.clone()).await.unwrap();
    let storage_b = RedisStorage::new(config).await.unwrap();

    let reg_a = CallRegistry::new(storage.clone(), "node-a".to_string());
    let reg_b = CallRegistry::new(storage_b, "node-b".to_string());

    // Node A registers a call
    reg_a
        .register_call("call-001")
        .await
        .expect("Failed to register call");

    // Node B should find the call owner
    let owner = reg_b
        .get_call_owner("call-001")
        .await
        .expect("Failed to get call owner");
    assert_eq!(owner.as_deref(), Some("node-a"));

    // Node B should not think it owns the call
    assert!(!reg_b
        .is_local_call("call-001")
        .await
        .expect("is_local_call failed"));

    // Node A should think it owns the call
    assert!(reg_a
        .is_local_call("call-001")
        .await
        .expect("is_local_call failed"));

    // Cleanup
    reg_a.unregister_call("call-001").await.unwrap();
    cleanup_keys(&storage).await;
}

#[tokio::test]
#[ignore]
async fn test_call_registry_ttl_expiry() {
    let config = RedisConfig {
        url: redis_url(),
        key_prefix: test_key_prefix(),
    };
    let storage = RedisStorage::new(config).await.unwrap();

    // Create registry with very short TTL (2 seconds)
    let reg = CallRegistry::with_ttl(storage.clone(), "node-a".to_string(), 2);

    reg.register_call("call-expiry-001")
        .await
        .expect("Failed to register call");

    // Should exist immediately
    let owner = reg.get_call_owner("call-expiry-001").await.unwrap();
    assert_eq!(owner.as_deref(), Some("node-a"));

    // Wait for TTL to expire
    sleep(Duration::from_secs(3)).await;

    // Should be gone now
    let owner = reg.get_call_owner("call-expiry-001").await.unwrap();
    assert!(
        owner.is_none(),
        "Call ownership should have expired, got: {:?}",
        owner
    );

    cleanup_keys(&storage).await;
}

#[tokio::test]
#[ignore]
async fn test_membership_events_emitted() {
    use fcp_events::{EventPublisher, EventSubscriber, InMemoryEventBus, PlatformEvent};

    let config = RedisConfig {
        url: redis_url(),
        key_prefix: test_key_prefix(),
    };
    let storage = RedisStorage::new(config.clone()).await.unwrap();
    let storage_b = RedisStorage::new(config).await.unwrap();

    let event_bus = Arc::new(InMemoryEventBus::new());

    let node_a = test_member("node-a", 8084);
    let node_b = test_member("node-b", 8085);
    let node_b_id = node_b.node_id.clone();

    // Node A watches for events
    let cm_a = ClusterManager::new(storage.clone(), node_a)
        .with_intervals(1, 3)
        .with_event_publisher(Arc::clone(&event_bus) as Arc<dyn EventPublisher>);

    let _handle_a = cm_a.start();

    // Subscribe to events
    let filter = fcp_events::EventFilter::new();
    let mut stream = event_bus.subscribe(filter).await.unwrap();

    // Start Node B
    let cm_b = ClusterManager::new(storage_b, node_b).with_intervals(1, 3);
    let _handle_b = cm_b.start();

    // Wait for NodeJoined event
    let mut joined = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            event = stream.next() => {
                if let Some(PlatformEvent::System(fcp_events::SystemEvent::NodeJoined { node_id, .. })) = event {
                    if node_id == node_b_id {
                        joined = true;
                        break;
                    }
                }
            }
            _ = sleep(Duration::from_millis(100)) => {}
        }
    }
    assert!(joined, "Should have received NodeJoined event for Node B");

    // Shut down Node B and wait for NodeLeft
    cm_b.shutdown().await;

    let mut left = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            event = stream.next() => {
                if let Some(PlatformEvent::System(fcp_events::SystemEvent::NodeLeft { node_id, .. })) = event {
                    if node_id == node_b_id {
                        left = true;
                        break;
                    }
                }
            }
            _ = sleep(Duration::from_millis(100)) => {}
        }
    }
    assert!(left, "Should have received NodeLeft event for Node B");

    cm_a.shutdown().await;
    cleanup_keys(&storage).await;
}

#[tokio::test]
#[ignore]
async fn test_call_refresh_extends_ttl() {
    let config = RedisConfig {
        url: redis_url(),
        key_prefix: test_key_prefix(),
    };
    let storage = RedisStorage::new(config).await.unwrap();

    // Create registry with short TTL (3 seconds)
    let reg = CallRegistry::with_ttl(storage.clone(), "node-a".to_string(), 3);

    reg.register_call("call-refresh-001")
        .await
        .expect("Failed to register call");

    // Wait 2 seconds (within TTL)
    sleep(Duration::from_secs(2)).await;

    // Refresh extends the TTL
    reg.refresh_call("call-refresh-001")
        .await
        .expect("Failed to refresh call");

    // Wait another 2 seconds (would have expired without refresh)
    sleep(Duration::from_secs(2)).await;

    // Should still exist because refresh extended TTL
    let owner = reg.get_call_owner("call-refresh-001").await.unwrap();
    assert_eq!(
        owner.as_deref(),
        Some("node-a"),
        "Call should still exist after refresh"
    );

    // Wait for full TTL expiry
    sleep(Duration::from_secs(2)).await;

    let owner = reg.get_call_owner("call-refresh-001").await.unwrap();
    assert!(
        owner.is_none(),
        "Call should have expired after TTL. Got: {:?}",
        owner
    );

    cleanup_keys(&storage).await;
}
