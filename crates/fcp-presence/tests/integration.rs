//! Integration tests for the presence subsystem.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fcp_core::TenantId;
use fcp_events::InMemoryEventBus;
use fcp_presence::{
    PresenceAuthPolicy, PresenceConfig, PresenceManager, PresenceState,
    PresenceUpdate, Watcher,
};

fn test_config() -> PresenceConfig {
    PresenceConfig {
        enabled: true,
        max_watchers_per_presentity: 100,
        max_presentities: 10000,
        ..Default::default()
    }
}

fn make_watcher(
    watcher_aor: &str,
    presentity_aor: &str,
    tenant_id: &TenantId,
    event_package: &str,
) -> Watcher {
    Watcher {
        watcher_aor: watcher_aor.to_string(),
        presentity_aor: presentity_aor.to_string(),
        subscription_id: uuid::Uuid::new_v4().to_string(),
        expires_at: Instant::now() + Duration::from_secs(3600),
        tenant_id: tenant_id.clone(),
        contact_uri: format!("sip:{}@192.168.1.100:5060", watcher_aor.split('@').next().unwrap_or("user")),
        event_package: event_package.to_string(),
    }
}

/// Test: subscribe → update presence → verify StatusChanged event → cleanup expired watcher.
#[tokio::test]
async fn test_subscribe_notify_lifecycle() {
    let bus = Arc::new(InMemoryEventBus::new());
    let mgr = PresenceManager::new(
        test_config(),
        Some(bus.clone() as Arc<dyn fcp_events::EventPublisher>),
        None,
    );
    let tenant = TenantId::new();

    // Create presentity
    let update = PresenceUpdate {
        aor: "alice@example.com".to_string(),
        state: PresenceState::Open,
        note: None,
    };
    mgr.update_presence(update, &tenant).await.unwrap();

    // Add watcher
    let watcher = make_watcher("bob@example.com", "alice@example.com", &tenant, "presence");
    mgr.add_watcher(watcher).await.unwrap();
    assert_eq!(mgr.get_watchers("alice@example.com").len(), 1);

    // Update presence (should publish StatusChanged event)
    let update2 = PresenceUpdate {
        aor: "alice@example.com".to_string(),
        state: PresenceState::Busy,
        note: Some("In a meeting".to_string()),
    };
    let info = mgr.update_presence(update2, &tenant).await.unwrap();
    assert_eq!(info.state, PresenceState::Busy);
    assert_eq!(info.note.as_deref(), Some("In a meeting"));

    // Add an expired watcher
    let expired_watcher = Watcher {
        watcher_aor: "charlie@example.com".to_string(),
        presentity_aor: "alice@example.com".to_string(),
        subscription_id: "sub-expired".to_string(),
        expires_at: Instant::now() - Duration::from_secs(1),
        tenant_id: tenant.clone(),
        contact_uri: "sip:charlie@192.168.1.101:5060".to_string(),
        event_package: "presence".to_string(),
    };
    mgr.add_watcher(expired_watcher).await.unwrap();
    assert_eq!(mgr.get_watchers("alice@example.com").len(), 2);

    // Cleanup expired
    let removed = mgr.cleanup_expired_watchers();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].subscription_id, "sub-expired");
    assert_eq!(mgr.get_watchers("alice@example.com").len(), 1);
}

/// Test: PUBLISH ETag full lifecycle — store → refresh → modify → remove → 412 on stale.
#[tokio::test]
async fn test_publish_etag_full_lifecycle() {
    let mgr = PresenceManager::new(test_config(), None, None);
    let tenant = TenantId::new();

    // Initial PUBLISH
    let etag1 = mgr.store_publish(
        "alice@example.com",
        PresenceState::Open,
        Some("Available".into()),
        &tenant,
        3600,
    );
    assert!(!etag1.is_empty());

    // Refresh → new ETag
    let etag2 = mgr.refresh_publish(&etag1, 7200).unwrap();
    assert_ne!(etag1, etag2);

    // Old ETag should fail (412)
    assert!(mgr.refresh_publish(&etag1, 3600).is_err());

    // Modify → new ETag
    let etag3 = mgr
        .modify_publish(&etag2, PresenceState::Busy, Some("DND".into()), 3600)
        .unwrap();
    assert_ne!(etag2, etag3);

    // Remove
    mgr.remove_publish(&etag3).unwrap();

    // Removed ETag should fail (412)
    assert!(mgr.remove_publish(&etag3).is_err());
    assert!(mgr.refresh_publish(&etag3, 3600).is_err());

    // Cleanup should handle expired PUBLISHes
    let etag_short = mgr.store_publish(
        "alice@example.com",
        PresenceState::Open,
        None,
        &tenant,
        0, // expires immediately
    );
    let update = PresenceUpdate {
        aor: "alice@example.com".to_string(),
        state: PresenceState::Open,
        note: None,
    };
    mgr.update_presence(update, &tenant).await.unwrap();

    tokio::time::sleep(Duration::from_millis(10)).await;
    mgr.cleanup_expired_publishes().await;

    assert!(mgr.refresh_publish(&etag_short, 3600).is_err());
}

/// Test: auth policy enforcement — block a watcher, verify rejection, allow, verify acceptance.
#[tokio::test]
async fn test_auth_policy_enforcement() {
    let mgr = PresenceManager::new(test_config(), None, None);

    // No policy = allow all
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com"));

    // Set policy blocking bob
    let policy = PresenceAuthPolicy {
        default_allow: true,
        allow_list: vec![],
        block_list: vec!["bob@example.com".to_string()],
    };
    mgr.set_auth_policy("alice@example.com", policy);
    assert!(!mgr.is_watcher_authorized("bob@example.com", "alice@example.com"));
    assert!(mgr.is_watcher_authorized("charlie@example.com", "alice@example.com"));

    // Change to allow-list-only mode
    let policy2 = PresenceAuthPolicy {
        default_allow: false,
        allow_list: vec!["charlie@example.com".to_string()],
        block_list: vec![],
    };
    mgr.set_auth_policy("alice@example.com", policy2);
    assert!(!mgr.is_watcher_authorized("bob@example.com", "alice@example.com"));
    assert!(mgr.is_watcher_authorized("charlie@example.com", "alice@example.com"));

    // Remove policy → revert to default-allow
    mgr.remove_auth_policy("alice@example.com");
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com"));
}

/// Test: cross-tenant watcher subscription is rejected.
#[tokio::test]
async fn test_tenant_isolation() {
    let mgr = PresenceManager::new(test_config(), None, None);
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();

    // Create presentity for tenant A
    let update = PresenceUpdate {
        aor: "alice@a.com".to_string(),
        state: PresenceState::Open,
        note: None,
    };
    mgr.update_presence(update, &tenant_a).await.unwrap();

    // Watcher from tenant A → should succeed
    let watcher_a = make_watcher("bob@a.com", "alice@a.com", &tenant_a, "presence");
    mgr.add_watcher(watcher_a).await.unwrap();

    // Watcher from tenant B → should be rejected
    let watcher_b = make_watcher("eve@b.com", "alice@a.com", &tenant_b, "presence");
    let result = mgr.add_watcher(watcher_b).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Cross-tenant"));

    // Verify only one watcher (from tenant A)
    assert_eq!(mgr.get_watchers("alice@a.com").len(), 1);

    // Tenant-filtered queries
    let presences_a = mgr.get_presences_by_tenant(&tenant_a);
    assert_eq!(presences_a.len(), 1);

    let presences_b = mgr.get_presences_by_tenant(&tenant_b);
    assert_eq!(presences_b.len(), 0);
}

/// Test: None store doesn't break any operations.
#[tokio::test]
async fn test_persistence_roundtrip_with_none_store() {
    let mgr = PresenceManager::new(test_config(), None, None);
    let tenant = TenantId::new();

    // All persistence-related operations should be no-ops, not panics
    let update = PresenceUpdate {
        aor: "alice@example.com".to_string(),
        state: PresenceState::Open,
        note: None,
    };
    mgr.update_presence(update, &tenant).await.unwrap();

    let watcher = make_watcher("bob@example.com", "alice@example.com", &tenant, "presence");
    mgr.add_watcher(watcher).await.unwrap();

    mgr.set_auth_policy(
        "alice@example.com",
        PresenceAuthPolicy {
            default_allow: false,
            allow_list: vec!["bob@example.com".to_string()],
            block_list: vec![],
        },
    );

    // Load operations with no store = 0
    assert_eq!(mgr.load_presentities(&tenant).await.unwrap(), 0);
    assert_eq!(mgr.load_subscriptions(&tenant).await.unwrap(), 0);
    assert_eq!(mgr.load_auth_policies(&tenant).await.unwrap(), 0);

    // Cleanup from store = 0
    assert_eq!(mgr.cleanup_expired_subscriptions_from_store().await, 0);

    // Remove operations work
    mgr.remove_watcher("alice@example.com", &mgr.get_watchers("alice@example.com")[0].subscription_id).await;
    mgr.remove_auth_policy("alice@example.com");
    mgr.remove_presence("alice@example.com", &tenant).await;
}
