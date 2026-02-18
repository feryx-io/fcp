//! Integration tests for the presence subsystem.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fcp_core::TenantId;
use fcp_events::InMemoryEventBus;
use std::collections::HashSet;

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
        sip_call_id: None,
        sip_from_tag: None,
        sip_to_tag: None,
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
        sip_call_id: None,
        sip_from_tag: None,
        sip_to_tag: None,
    };
    mgr.add_watcher(expired_watcher).await.unwrap();
    // get_watchers() now filters expired at read time (Fix 5), so only 1 active watcher visible
    assert_eq!(mgr.get_watchers("alice@example.com").len(), 1);

    // Cleanup expired (still removes from internal storage + sends terminated NOTIFYs)
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
    ).await;
    assert!(!etag1.is_empty());

    // Refresh → new ETag
    let etag2 = mgr.refresh_publish(&etag1, 7200).unwrap();
    assert_ne!(etag1, etag2);

    // Old ETag should fail (412)
    assert!(mgr.refresh_publish(&etag1, 3600).is_err());

    // Modify → new ETag
    let etag3 = mgr
        .modify_publish(&etag2, PresenceState::Busy, Some("DND".into()), 3600)
        .await
        .unwrap();
    assert_ne!(etag2, etag3);

    // Remove
    mgr.remove_publish(&etag3).await.unwrap();

    // Removed ETag should fail (412)
    assert!(mgr.remove_publish(&etag3).await.is_err());
    assert!(mgr.refresh_publish(&etag3, 3600).is_err());

    // Cleanup should handle expired PUBLISHes
    let etag_short = mgr.store_publish(
        "alice@example.com",
        PresenceState::Open,
        None,
        &tenant,
        0, // expires immediately
    ).await;
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
        allow_list: HashSet::new(),
        block_list: HashSet::from(["bob@example.com".to_string()]),
    };
    mgr.set_auth_policy("alice@example.com", policy, &TenantId::nil());
    assert!(!mgr.is_watcher_authorized("bob@example.com", "alice@example.com"));
    assert!(mgr.is_watcher_authorized("charlie@example.com", "alice@example.com"));

    // Change to allow-list-only mode
    let policy2 = PresenceAuthPolicy {
        default_allow: false,
        allow_list: HashSet::from(["charlie@example.com".to_string()]),
        block_list: HashSet::new(),
    };
    mgr.set_auth_policy("alice@example.com", policy2, &TenantId::nil());
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
            allow_list: HashSet::from(["bob@example.com".to_string()]),
            block_list: HashSet::new(),
        },
        &tenant,
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

// ---- Fix 1: NOTIFY retry queue tests ----

/// Test: enqueue, dequeue, and max retries exceeded behavior.
#[tokio::test]
async fn test_notify_retry_queue_lifecycle() {
    let config = PresenceConfig {
        max_notify_retries: 2,
        notify_retry_base_delay_ms: 10,
        notify_retry_max_delay_ms: 100,
        ..test_config()
    };
    let mgr = PresenceManager::new(config, None, None);
    let tenant = TenantId::new();

    let watcher = make_watcher("bob@example.com", "alice@example.com", &tenant, "presence");

    // Enqueue a retry entry
    let entry = fcp_presence::NotifyRetryEntry {
        watcher: watcher.clone(),
        subscription_state: "active;expires=3600".to_string(),
        attempt: 1,
        next_retry_at: Instant::now(), // due immediately
        created_at: Instant::now(),
    };
    mgr.enqueue_retry(entry);
    assert_eq!(mgr.retry_queue_len(), 1);

    // Dequeue should return it (it's due now)
    let due = mgr.dequeue_due_retries();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].watcher.watcher_aor, "bob@example.com");
    assert_eq!(mgr.retry_queue_len(), 0); // removed from queue

    // Enqueue with future next_retry_at — should NOT be dequeued
    let future_entry = fcp_presence::NotifyRetryEntry {
        watcher: watcher.clone(),
        subscription_state: "active;expires=3600".to_string(),
        attempt: 1,
        next_retry_at: Instant::now() + Duration::from_secs(3600),
        created_at: Instant::now(),
    };
    mgr.enqueue_retry(future_entry);
    assert_eq!(mgr.retry_queue_len(), 1);
    let due = mgr.dequeue_due_retries();
    assert_eq!(due.len(), 0);
    assert_eq!(mgr.retry_queue_len(), 1); // still in queue

    // Remove by subscription_id + aor
    mgr.remove_retry(&watcher.subscription_id, &watcher.presentity_aor);
    assert_eq!(mgr.retry_queue_len(), 0);
}

/// Test: entries exceeding max retries are removed on dequeue.
#[tokio::test]
async fn test_notify_retry_max_exceeded() {
    let config = PresenceConfig {
        max_notify_retries: 2,
        ..test_config()
    };
    let mgr = PresenceManager::new(config, None, None);
    let tenant = TenantId::new();

    let watcher = make_watcher("bob@example.com", "alice@example.com", &tenant, "presence");

    // Enqueue with attempt > max_retries
    let entry = fcp_presence::NotifyRetryEntry {
        watcher,
        subscription_state: "active;expires=3600".to_string(),
        attempt: 5, // exceeds max of 2
        next_retry_at: Instant::now(),
        created_at: Instant::now(),
    };
    mgr.enqueue_retry(entry);
    assert_eq!(mgr.retry_queue_len(), 1);

    // Dequeue should remove it (exceeded max retries) but NOT return it
    let due = mgr.dequeue_due_retries();
    assert_eq!(due.len(), 0);
    assert_eq!(mgr.retry_queue_len(), 0);
}

// ---- Fix 3: Auth policy tenant persistence test ----

/// Test: auth policy persists with correct tenant_id (no nil leak).
#[tokio::test]
async fn test_auth_policy_with_tenant_id() {
    let mgr = PresenceManager::new(test_config(), None, None);
    let tenant = TenantId::new();

    let policy = PresenceAuthPolicy {
        default_allow: false,
        allow_list: HashSet::from(["bob@example.com".to_string()]),
        block_list: HashSet::new(),
    };

    // Set with specific tenant (no store means no actual persistence, but API works)
    mgr.set_auth_policy("alice@example.com", policy.clone(), &tenant);
    assert!(mgr.get_auth_policy("alice@example.com").is_some());

    let retrieved = mgr.get_auth_policy("alice@example.com").unwrap();
    assert!(!retrieved.default_allow);
    assert!(retrieved.allow_list.contains("bob@example.com"));
}

// ---- Fix 4: Shutdown tests ----

/// Test: shutdown receiver signals cleanup task to stop.
#[tokio::test]
async fn test_shutdown_receiver_signals() {
    let mgr = PresenceManager::new(test_config(), None, None);
    let mut rx = mgr.shutdown_receiver();

    // Not yet shut down
    assert!(!*rx.borrow());

    // Trigger shutdown
    mgr.shutdown().await;

    // Receiver should see the signal
    assert!(rx.changed().await.is_ok());
    assert!(*rx.borrow());
}

/// Test: get_all_watchers_including_winfo returns both regular and winfo watchers.
#[tokio::test]
async fn test_get_all_watchers_including_winfo() {
    let mgr = PresenceManager::new(test_config(), None, None);
    let tenant = TenantId::new();

    // Add a regular watcher
    let watcher = make_watcher("bob@example.com", "alice@example.com", &tenant, "presence");
    mgr.add_watcher(watcher).await.unwrap();

    // Add a winfo watcher
    let winfo_watcher = make_watcher("admin@example.com", "alice@example.com", &tenant, "presence.winfo");
    mgr.add_winfo_watcher(winfo_watcher);

    let regular = mgr.get_all_watchers();
    assert_eq!(regular.len(), 1);

    let all = mgr.get_all_watchers_including_winfo();
    assert_eq!(all.len(), 2);
}

// ---- Fix 5: Watcher expiry filtering tests ----

/// Test: get_watchers() excludes expired; cleanup_expired_watchers() still removes them.
#[tokio::test]
async fn test_expired_watchers_filtered_at_read_time() {
    let mgr = PresenceManager::new(test_config(), None, None);
    let tenant = TenantId::new();

    // Add a valid watcher
    let valid = make_watcher("bob@example.com", "alice@example.com", &tenant, "presence");
    mgr.add_watcher(valid).await.unwrap();

    // Add an expired watcher
    let expired = Watcher {
        watcher_aor: "charlie@example.com".to_string(),
        presentity_aor: "alice@example.com".to_string(),
        subscription_id: "sub-expired".to_string(),
        expires_at: Instant::now() - Duration::from_secs(1),
        tenant_id: tenant.clone(),
        contact_uri: "sip:charlie@192.168.1.101:5060".to_string(),
        event_package: "presence".to_string(),
        sip_call_id: None,
        sip_from_tag: None,
        sip_to_tag: None,
    };
    mgr.add_watcher(expired).await.unwrap();

    // Read-time filter: only valid watcher visible
    assert_eq!(mgr.get_watchers("alice@example.com").len(), 1);
    assert_eq!(mgr.get_watchers("alice@example.com")[0].watcher_aor, "bob@example.com");

    // get_all_watchers() also filtered
    assert_eq!(mgr.get_all_watchers().len(), 1);

    // total_watcher_count() also filtered
    assert_eq!(mgr.total_watcher_count(), 1);

    // find_existing_watcher() skips expired
    assert!(mgr.find_existing_watcher("charlie@example.com", "alice@example.com", "presence").is_none());
    assert!(mgr.find_existing_watcher("bob@example.com", "alice@example.com", "presence").is_some());

    // get_watchers_by_event() filtered
    assert_eq!(mgr.get_watchers_by_event("alice@example.com", "presence").len(), 1);

    // Cleanup still removes the expired watcher from internal storage
    let removed = mgr.cleanup_expired_watchers();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].subscription_id, "sub-expired");
}

/// Test: expired winfo watchers are filtered from get_winfo_watchers().
#[tokio::test]
async fn test_expired_winfo_watchers_filtered() {
    let mgr = PresenceManager::new(test_config(), None, None);
    let tenant = TenantId::new();

    // Add valid winfo watcher
    let valid = make_watcher("admin@example.com", "alice@example.com", &tenant, "presence.winfo");
    mgr.add_winfo_watcher(valid);

    // Add expired winfo watcher
    let expired = Watcher {
        watcher_aor: "old-admin@example.com".to_string(),
        presentity_aor: "alice@example.com".to_string(),
        subscription_id: "winfo-expired".to_string(),
        expires_at: Instant::now() - Duration::from_secs(1),
        tenant_id: tenant.clone(),
        contact_uri: "sip:old-admin@192.168.1.102:5060".to_string(),
        event_package: "presence.winfo".to_string(),
        sip_call_id: None,
        sip_from_tag: None,
        sip_to_tag: None,
    };
    mgr.add_winfo_watcher(expired);

    // Only valid winfo watcher should be visible
    assert_eq!(mgr.get_winfo_watchers("alice@example.com").len(), 1);
    assert_eq!(
        mgr.get_winfo_watchers("alice@example.com")[0].watcher_aor,
        "admin@example.com"
    );
}
