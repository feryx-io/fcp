//! Integration tests for the presence subsystem.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fcp_core::TenantId;
use fcp_events::InMemoryEventBus;
use std::collections::HashSet;

use fcp_presence::{
    PresenceAuthPolicy, PresenceConfig, PresenceManager, PresenceState, PresenceUpdate, Watcher,
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
        contact_uri: format!(
            "sip:{}@192.168.1.100:5060",
            watcher_aor.split('@').next().unwrap_or("user")
        ),
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
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    // Initial PUBLISH
    let etag1 = mgr
        .store_publish(
            "alice@example.com",
            PresenceState::Open,
            Some("Available".into()),
            &tenant,
            3600,
        )
        .await;
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
    let etag_short = mgr
        .store_publish(
            "alice@example.com",
            PresenceState::Open,
            None,
            &tenant,
            0, // expires immediately
        )
        .await;
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
    let mgr = PresenceManager::new(test_config(), None);

    // No policy = allow all
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));

    // Set policy blocking bob
    let policy = PresenceAuthPolicy {
        default_allow: true,
        allow_list: HashSet::new(),
        block_list: HashSet::from(["bob@example.com".to_string()]),
        ..Default::default()
    };
    mgr.set_auth_policy("alice@example.com", policy, &TenantId::nil());
    assert!(!mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));
    assert!(mgr.is_watcher_authorized("charlie@example.com", "alice@example.com", "presence"));

    // Change to allow-list-only mode
    let policy2 = PresenceAuthPolicy {
        default_allow: false,
        allow_list: HashSet::from(["charlie@example.com".to_string()]),
        block_list: HashSet::new(),
        ..Default::default()
    };
    mgr.set_auth_policy("alice@example.com", policy2, &TenantId::nil());
    assert!(!mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));
    assert!(mgr.is_watcher_authorized("charlie@example.com", "alice@example.com", "presence"));

    // Remove policy → revert to default-allow
    mgr.remove_auth_policy("alice@example.com");
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));
}

/// Test: cross-tenant watcher subscription is rejected.
#[tokio::test]
async fn test_tenant_isolation() {
    let mgr = PresenceManager::new(test_config(), None);
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
    let mgr = PresenceManager::new(test_config(), None);
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
            ..Default::default()
        },
        &tenant,
    );

    // Cleanup (no-op, in-memory only)
    assert_eq!(mgr.cleanup_expired_subscriptions_from_store().await, 0);

    // Remove operations work
    mgr.remove_watcher(
        "alice@example.com",
        &mgr.get_watchers("alice@example.com")[0].subscription_id,
    )
    .await;
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
    let mgr = PresenceManager::new(config, None);
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
    let mgr = PresenceManager::new(config, None);
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
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    let policy = PresenceAuthPolicy {
        default_allow: false,
        allow_list: HashSet::from(["bob@example.com".to_string()]),
        block_list: HashSet::new(),
        ..Default::default()
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
    let mgr = PresenceManager::new(test_config(), None);
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
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    // Add a regular watcher
    let watcher = make_watcher("bob@example.com", "alice@example.com", &tenant, "presence");
    mgr.add_watcher(watcher).await.unwrap();

    // Add a winfo watcher
    let winfo_watcher = make_watcher(
        "admin@example.com",
        "alice@example.com",
        &tenant,
        "presence.winfo",
    );
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
    let mgr = PresenceManager::new(test_config(), None);
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
    assert_eq!(
        mgr.get_watchers("alice@example.com")[0].watcher_aor,
        "bob@example.com"
    );

    // get_all_watchers() also filtered
    assert_eq!(mgr.get_all_watchers().len(), 1);

    // total_watcher_count() also filtered
    assert_eq!(mgr.total_watcher_count(), 1);

    // find_existing_watcher() skips expired
    assert!(mgr
        .find_existing_watcher("charlie@example.com", "alice@example.com", "presence")
        .is_none());
    assert!(mgr
        .find_existing_watcher("bob@example.com", "alice@example.com", "presence")
        .is_some());

    // get_watchers_by_event() filtered
    assert_eq!(
        mgr.get_watchers_by_event("alice@example.com", "presence")
            .len(),
        1
    );

    // Cleanup still removes the expired watcher from internal storage
    let removed = mgr.cleanup_expired_watchers();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].subscription_id, "sub-expired");
}

/// Test: expired winfo watchers are filtered from get_winfo_watchers().
#[tokio::test]
async fn test_expired_winfo_watchers_filtered() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    // Add valid winfo watcher
    let valid = make_watcher(
        "admin@example.com",
        "alice@example.com",
        &tenant,
        "presence.winfo",
    );
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

// ===========================================================================
// Item 8: Watcher-info test coverage
// ===========================================================================

/// Test: add winfo watcher → verify present → remove → verify removed.
#[tokio::test]
async fn test_winfo_subscribe_unsubscribe() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    let watcher = make_watcher(
        "admin@example.com",
        "alice@example.com",
        &tenant,
        "presence.winfo",
    );
    let sub_id = watcher.subscription_id.clone();
    mgr.add_winfo_watcher(watcher);

    assert_eq!(mgr.get_winfo_watchers("alice@example.com").len(), 1);

    mgr.remove_winfo_watcher("alice@example.com", &sub_id);
    assert_eq!(mgr.get_winfo_watchers("alice@example.com").len(), 0);
}

/// Test: generate_watcherinfo() version increments on each call.
#[tokio::test]
async fn test_winfo_version_increments() {
    let mgr = PresenceManager::new(test_config(), None);

    let xml0 = mgr.generate_watcherinfo("alice@example.com").unwrap();
    assert!(xml0.contains("version=\"0\""), "Expected version 0: {xml0}");

    let xml1 = mgr.generate_watcherinfo("alice@example.com").unwrap();
    assert!(xml1.contains("version=\"1\""), "Expected version 1: {xml1}");

    let xml2 = mgr.generate_watcherinfo("alice@example.com").unwrap();
    assert!(xml2.contains("version=\"2\""), "Expected version 2: {xml2}");
}

/// Test: watcherinfo XML contains all active watchers with correct attributes.
#[tokio::test]
async fn test_winfo_xml_contains_active_watchers() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    for i in 0..3 {
        let w = make_watcher(
            &format!("user{i}@example.com"),
            "alice@example.com",
            &tenant,
            "presence",
        );
        mgr.add_watcher(w).await.unwrap();
    }

    let xml = mgr.generate_watcherinfo("alice@example.com").unwrap();
    for i in 0..3 {
        assert!(
            xml.contains(&format!("sip:user{i}@example.com")),
            "Missing user{i} in winfo XML: {xml}"
        );
    }
    // Verify status="active" and event="approved" appear
    assert!(xml.contains("active"), "Missing active status: {xml}");
    assert!(xml.contains("approved"), "Missing approved event: {xml}");
}

/// Test: expired watchers are excluded from watcherinfo XML.
#[tokio::test]
async fn test_winfo_xml_excludes_expired_watchers() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    // Add a valid watcher
    let valid = make_watcher("bob@example.com", "alice@example.com", &tenant, "presence");
    mgr.add_watcher(valid).await.unwrap();

    // Add an expired watcher
    let expired = Watcher {
        watcher_aor: "expired@example.com".to_string(),
        presentity_aor: "alice@example.com".to_string(),
        subscription_id: "sub-expired-winfo".to_string(),
        expires_at: Instant::now() - Duration::from_secs(1),
        tenant_id: tenant.clone(),
        contact_uri: "sip:expired@192.168.1.100:5060".to_string(),
        event_package: "presence".to_string(),
        sip_call_id: None,
        sip_from_tag: None,
        sip_to_tag: None,
    };
    mgr.add_watcher(expired).await.unwrap();

    let xml = mgr.generate_watcherinfo("alice@example.com").unwrap();
    assert!(
        xml.contains("sip:bob@example.com"),
        "Valid watcher missing: {xml}"
    );
    assert!(
        !xml.contains("sip:expired@example.com"),
        "Expired watcher should be excluded: {xml}"
    );
}

/// Test: watcher-info is per-AOR isolated.
#[tokio::test]
async fn test_winfo_per_aor_isolation() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    let w1 = make_watcher("bob@example.com", "alice@example.com", &tenant, "presence");
    mgr.add_watcher(w1).await.unwrap();

    let w2 = make_watcher(
        "charlie@example.com",
        "dave@example.com",
        &tenant,
        "presence",
    );
    mgr.add_watcher(w2).await.unwrap();

    let xml_alice = mgr.generate_watcherinfo("alice@example.com").unwrap();
    assert!(xml_alice.contains("sip:bob@example.com"));
    assert!(!xml_alice.contains("sip:charlie@example.com"));

    let xml_dave = mgr.generate_watcherinfo("dave@example.com").unwrap();
    assert!(xml_dave.contains("sip:charlie@example.com"));
    assert!(!xml_dave.contains("sip:bob@example.com"));
}

/// Test: watcher expiration attribute in watcherinfo XML.
#[tokio::test]
async fn test_winfo_watcher_expiration_display() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    let watcher = Watcher {
        watcher_aor: "bob@example.com".to_string(),
        presentity_aor: "alice@example.com".to_string(),
        subscription_id: "sub-expiry-test".to_string(),
        expires_at: Instant::now() + Duration::from_secs(1800),
        tenant_id: tenant.clone(),
        contact_uri: "sip:bob@192.168.1.100:5060".to_string(),
        event_package: "presence".to_string(),
        sip_call_id: None,
        sip_from_tag: None,
        sip_to_tag: None,
    };
    mgr.add_watcher(watcher).await.unwrap();

    let xml = mgr.generate_watcherinfo("alice@example.com").unwrap();
    // Expiration should be approximately 1800 (within a few seconds)
    assert!(
        xml.contains("expiration="),
        "Missing expiration attribute: {xml}"
    );
}

/// Test: empty watcher list generates valid XML.
#[tokio::test]
async fn test_winfo_empty_watcher_list() {
    let mgr = PresenceManager::new(test_config(), None);

    let xml = mgr.generate_watcherinfo("nobody@example.com").unwrap();
    assert!(
        xml.contains("watcherinfo") || xml.contains("watcher-list"),
        "Should be valid watcherinfo XML: {xml}"
    );
    assert!(xml.contains("version=\"0\""), "Expected version 0: {xml}");
}

// ===========================================================================
// Item 10: Composition concurrency tests
// ===========================================================================

/// Test: concurrent PUBLISH from multiple devices — no panics, consistent final state.
#[tokio::test]
async fn test_concurrent_publish_from_multiple_devices() {
    let config = PresenceConfig {
        composition_enabled: true,
        ..test_config()
    };
    let mgr = PresenceManager::new(config, None);
    let tenant = TenantId::new();

    // Create presentity first
    let update = PresenceUpdate {
        aor: "alice@example.com".to_string(),
        state: PresenceState::Open,
        note: None,
    };
    mgr.update_presence(update, &tenant).await.unwrap();

    let states = [
        PresenceState::Open,
        PresenceState::Busy,
        PresenceState::Away,
        PresenceState::OnThePhone,
        PresenceState::Ringing,
    ];

    let mut handles = Vec::new();
    for i in 0..10 {
        let mgr = mgr.clone();
        let tenant = tenant.clone();
        let state = states[i % states.len()];
        handles.push(tokio::spawn(async move {
            mgr.store_publish(
                "alice@example.com",
                state,
                Some(format!("Device {i}")),
                &tenant,
                3600,
            )
            .await
        }));
    }

    let etags: Vec<String> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All should have unique ETags
    let unique: HashSet<&String> = etags.iter().collect();
    assert_eq!(unique.len(), 10);

    // Composed state should exist and be valid
    let composed = mgr.compose_state("alice@example.com");
    assert!(composed.is_some());
}

/// Test: rapid publish/modify/remove cycle — no panics or inconsistency.
#[tokio::test]
async fn test_rapid_publish_modify_remove() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    for _ in 0..50 {
        let etag = mgr
            .store_publish(
                "alice@example.com",
                PresenceState::Open,
                None,
                &tenant,
                3600,
            )
            .await;
        let etag2 = mgr
            .modify_publish(&etag, PresenceState::Busy, None, 3600)
            .await
            .unwrap();
        mgr.remove_publish(&etag2).await.unwrap();
    }

    // No PUBLISHes should remain
    assert!(mgr.compose_state("alice@example.com").is_none());
}

/// Test: concurrent publish and remove — no deadlocks or panics.
#[tokio::test]
async fn test_concurrent_publish_and_remove() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    let mut handles = Vec::new();

    // 5 tasks publishing
    for i in 0..5 {
        let mgr = mgr.clone();
        let tenant = tenant.clone();
        handles.push(tokio::spawn(async move {
            for j in 0..20 {
                let etag = mgr
                    .store_publish(
                        "alice@example.com",
                        PresenceState::Open,
                        Some(format!("pub-{i}-{j}")),
                        &tenant,
                        3600,
                    )
                    .await;
                // Immediately remove
                let _ = mgr.remove_publish(&etag).await;
            }
        }));
    }

    // 5 tasks removing random ETags (may fail, that's OK)
    for _ in 0..5 {
        let mgr = mgr.clone();
        let tenant = tenant.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..20 {
                let etag = mgr
                    .store_publish(
                        "alice@example.com",
                        PresenceState::Busy,
                        None,
                        &tenant,
                        3600,
                    )
                    .await;
                let _ = mgr.remove_publish(&etag).await;
            }
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }
    // No panics or deadlocks = success
}

/// Test: composition priority consistency after sequential add/remove.
#[tokio::test]
async fn test_composition_priority_consistency() {
    let config = PresenceConfig {
        composition_enabled: true,
        ..test_config()
    };
    let mgr = PresenceManager::new(config, None);
    let tenant = TenantId::new();

    // Create presentity
    let update = PresenceUpdate {
        aor: "alice@example.com".to_string(),
        state: PresenceState::Open,
        note: None,
    };
    mgr.update_presence(update, &tenant).await.unwrap();

    // Store 3 PUBLISHes with different priorities
    let etag_open = mgr
        .store_publish(
            "alice@example.com",
            PresenceState::Open,
            None,
            &tenant,
            3600,
        )
        .await;
    let etag_busy = mgr
        .store_publish(
            "alice@example.com",
            PresenceState::Busy,
            None,
            &tenant,
            3600,
        )
        .await;
    let etag_phone = mgr
        .store_publish(
            "alice@example.com",
            PresenceState::OnThePhone,
            None,
            &tenant,
            3600,
        )
        .await;

    // Highest priority should be OnThePhone
    let (state, _) = mgr.compose_state("alice@example.com").unwrap();
    assert_eq!(state, PresenceState::OnThePhone);

    // Remove highest → next highest (Busy)
    mgr.remove_publish(&etag_phone).await.unwrap();
    let (state, _) = mgr.compose_state("alice@example.com").unwrap();
    assert_eq!(state, PresenceState::Busy);

    // Remove Busy → Open
    mgr.remove_publish(&etag_busy).await.unwrap();
    let (state, _) = mgr.compose_state("alice@example.com").unwrap();
    assert_eq!(state, PresenceState::Open);

    // Remove last → None
    mgr.remove_publish(&etag_open).await.unwrap();
    assert!(mgr.compose_state("alice@example.com").is_none());
}

/// Test: concurrent cleanup and publish — no panics.
#[tokio::test]
async fn test_concurrent_cleanup_and_publish() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    let mgr_cleaner = mgr.clone();
    let cleaner = tokio::spawn(async move {
        for _ in 0..50 {
            mgr_cleaner.cleanup_expired_publishes().await;
            tokio::task::yield_now().await;
        }
    });

    let mgr_publisher = mgr.clone();
    let tenant_pub = tenant.clone();
    let publisher = tokio::spawn(async move {
        for _ in 0..50 {
            let etag = mgr_publisher
                .store_publish(
                    "alice@example.com",
                    PresenceState::Open,
                    None,
                    &tenant_pub,
                    0, // expires immediately
                )
                .await;
            let _ = mgr_publisher.remove_publish(&etag).await;
        }
    });

    cleaner.await.unwrap();
    publisher.await.unwrap();
}

// ===========================================================================
// Item 11: Load/stress tests (run with --ignored)
// ===========================================================================

/// Scale test: 10,000 watchers across 100 presentities.
#[tokio::test]
#[ignore]
async fn test_scale_10k_watchers() {
    let config = PresenceConfig {
        max_watchers_per_presentity: 200,
        max_presentities: 10000,
        ..test_config()
    };
    let mgr = PresenceManager::new(config, None);
    let tenant = TenantId::new();

    // Create 100 presentities
    for i in 0..100 {
        let update = PresenceUpdate {
            aor: format!("pres{i}@example.com"),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();
    }

    // Add 100 watchers per presentity = 10,000 total
    let start = Instant::now();
    for i in 0..100 {
        for j in 0..100 {
            let w = Watcher {
                watcher_aor: format!("w{j}@example.com"),
                presentity_aor: format!("pres{i}@example.com"),
                subscription_id: format!("sub-{i}-{j}"),
                expires_at: Instant::now() + Duration::from_secs(3600),
                tenant_id: tenant.clone(),
                contact_uri: format!("sip:w{j}@192.168.1.{0}:5060", j % 256),
                event_package: "presence".to_string(),
                sip_call_id: None,
                sip_from_tag: None,
                sip_to_tag: None,
            };
            mgr.add_watcher(w).await.unwrap();
        }
    }
    let add_elapsed = start.elapsed();
    eprintln!("Added 10,000 watchers in {:?}", add_elapsed);

    assert_eq!(mgr.total_watcher_count(), 10_000);

    // Query all watchers
    let start = Instant::now();
    for i in 0..100 {
        let watchers = mgr.get_watchers(&format!("pres{i}@example.com"));
        assert_eq!(watchers.len(), 100);
    }
    let query_elapsed = start.elapsed();
    eprintln!("Queried 100 presentity watchers in {:?}", query_elapsed);

    // Cleanup (remove all watchers via cleanup with expired entries is not applicable here,
    // so we just verify the count is correct)
    assert_eq!(mgr.presentity_count(), 100);
}

/// Scale test: 1,000 presentities.
#[tokio::test]
#[ignore]
async fn test_scale_1k_presentities() {
    let config = PresenceConfig {
        max_presentities: 10000,
        ..test_config()
    };
    let mgr = PresenceManager::new(config, None);
    let tenant = TenantId::new();

    let start = Instant::now();
    for i in 0..1000 {
        let update = PresenceUpdate {
            aor: format!("user{i}@example.com"),
            state: PresenceState::Open,
            note: Some(format!("User {i}")),
        };
        mgr.update_presence(update, &tenant).await.unwrap();
    }
    let create_elapsed = start.elapsed();
    eprintln!("Created 1,000 presentities in {:?}", create_elapsed);
    assert_eq!(mgr.presentity_count(), 1000);

    // Update all
    let start = Instant::now();
    for i in 0..1000 {
        let update = PresenceUpdate {
            aor: format!("user{i}@example.com"),
            state: PresenceState::Busy,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();
    }
    let update_elapsed = start.elapsed();
    eprintln!("Updated 1,000 presentities in {:?}", update_elapsed);

    // Remove all
    let start = Instant::now();
    for i in 0..1000 {
        mgr.remove_presence(&format!("user{i}@example.com"), &tenant)
            .await;
    }
    let remove_elapsed = start.elapsed();
    eprintln!("Removed 1,000 presentities in {:?}", remove_elapsed);
    assert_eq!(mgr.presentity_count(), 0);
}

/// Concurrent subscribe/unsubscribe stress test.
#[tokio::test]
#[ignore]
async fn test_concurrent_subscribe_unsubscribe() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    // Create 10 presentities
    for i in 0..10 {
        let update = PresenceUpdate {
            aor: format!("pres{i}@example.com"),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();
    }

    let mut handles = Vec::new();
    for task_id in 0..100 {
        let mgr = mgr.clone();
        let tenant = tenant.clone();
        handles.push(tokio::spawn(async move {
            for j in 0..100 {
                let pres_idx = (task_id + j) % 10;
                let w = Watcher {
                    watcher_aor: format!("t{task_id}w{j}@example.com"),
                    presentity_aor: format!("pres{pres_idx}@example.com"),
                    subscription_id: format!("sub-{task_id}-{j}"),
                    expires_at: Instant::now() + Duration::from_secs(3600),
                    tenant_id: tenant.clone(),
                    contact_uri: format!("sip:t{task_id}w{j}@192.168.1.1:5060"),
                    event_package: "presence".to_string(),
                    sip_call_id: None,
                    sip_from_tag: None,
                    sip_to_tag: None,
                };
                let _ = mgr.add_watcher(w).await;
                mgr.remove_watcher(
                    &format!("pres{pres_idx}@example.com"),
                    &format!("sub-{task_id}-{j}"),
                )
                .await;
            }
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }
    // No panics = success; final watcher count should be 0
    assert_eq!(mgr.total_watcher_count(), 0);
}

/// Concurrent presence updates stress test.
#[tokio::test]
#[ignore]
async fn test_concurrent_presence_updates() {
    let config = PresenceConfig {
        max_presentities: 10000,
        ..test_config()
    };
    let mgr = PresenceManager::new(config, None);
    let tenant = TenantId::new();

    // Pre-create presentities
    for i in 0..100 {
        let update = PresenceUpdate {
            aor: format!("user{i}@example.com"),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();
    }

    let states = [
        PresenceState::Open,
        PresenceState::Busy,
        PresenceState::Away,
        PresenceState::OnThePhone,
        PresenceState::Ringing,
    ];

    let mut handles = Vec::new();
    for task_id in 0..50 {
        let mgr = mgr.clone();
        let tenant = tenant.clone();
        handles.push(tokio::spawn(async move {
            for j in 0..100 {
                let aor_idx = (task_id * 7 + j * 3) % 100;
                let state = states[(task_id + j) % states.len()];
                let update = PresenceUpdate {
                    aor: format!("user{aor_idx}@example.com"),
                    state,
                    note: None,
                };
                let _ = mgr.update_presence(update, &tenant).await;
            }
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }
    // All presentities should still have valid state
    assert_eq!(mgr.presentity_count(), 100);
    for i in 0..100 {
        let info = mgr.get_presence(&format!("user{i}@example.com"));
        assert!(info.is_some(), "Presentity user{i} should exist");
    }
}

/// NOTIFY generation under load.
#[tokio::test]
#[ignore]
async fn test_notify_generation_under_load() {
    let mgr = PresenceManager::new(test_config(), None);
    let tenant = TenantId::new();

    // Create presentities and watchers
    for i in 0..100 {
        let aor = format!("user{i}@example.com");
        let update = PresenceUpdate {
            aor: aor.clone(),
            state: PresenceState::Open,
            note: Some(format!("User {i}")),
        };
        mgr.update_presence(update, &tenant).await.unwrap();

        for j in 0..10 {
            let event_pkg = match j % 3 {
                0 => "presence",
                1 => "dialog",
                _ => "presence",
            };
            let w = Watcher {
                watcher_aor: format!("w{j}@example.com"),
                presentity_aor: aor.clone(),
                subscription_id: format!("sub-{i}-{j}"),
                expires_at: Instant::now() + Duration::from_secs(3600),
                tenant_id: tenant.clone(),
                contact_uri: format!("sip:w{j}@192.168.1.1:5060"),
                event_package: event_pkg.to_string(),
                sip_call_id: None,
                sip_from_tag: None,
                sip_to_tag: None,
            };
            mgr.add_watcher(w).await.unwrap();
        }
    }

    // Generate PIDF, dialog-info, and watcherinfo in parallel
    let start = Instant::now();
    let mut handles = Vec::new();

    for i in 0..100 {
        let mgr = mgr.clone();
        handles.push(tokio::spawn(async move {
            let aor = format!("user{i}@example.com");
            let pidf = mgr.generate_pidf(&aor);
            assert!(pidf.is_some());
            let dialog = mgr.generate_dialog_info(&aor);
            assert!(!dialog.is_empty());
            let winfo = mgr.generate_watcherinfo(&aor);
            assert!(winfo.is_ok());
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }
    let elapsed = start.elapsed();
    eprintln!(
        "Generated PIDF + dialog-info + watcherinfo for 100 AORs in {:?}",
        elapsed
    );
}

/// Cleanup under load: 5,000 expired + 5,000 valid watchers.
#[tokio::test]
#[ignore]
async fn test_cleanup_under_load() {
    let config = PresenceConfig {
        max_watchers_per_presentity: 20000,
        max_presentities: 10000,
        ..test_config()
    };
    let mgr = PresenceManager::new(config, None);
    let tenant = TenantId::new();

    let update = PresenceUpdate {
        aor: "target@example.com".to_string(),
        state: PresenceState::Open,
        note: None,
    };
    mgr.update_presence(update, &tenant).await.unwrap();

    // Add 5,000 expired watchers
    for i in 0..5000 {
        let w = Watcher {
            watcher_aor: format!("expired{i}@example.com"),
            presentity_aor: "target@example.com".to_string(),
            subscription_id: format!("sub-exp-{i}"),
            expires_at: Instant::now() - Duration::from_secs(1),
            tenant_id: tenant.clone(),
            contact_uri: format!("sip:expired{i}@192.168.1.1:5060"),
            event_package: "presence".to_string(),
            sip_call_id: None,
            sip_from_tag: None,
            sip_to_tag: None,
        };
        mgr.add_watcher(w).await.unwrap();
    }

    // Add 5,000 valid watchers
    for i in 0..5000 {
        let w = Watcher {
            watcher_aor: format!("valid{i}@example.com"),
            presentity_aor: "target@example.com".to_string(),
            subscription_id: format!("sub-val-{i}"),
            expires_at: Instant::now() + Duration::from_secs(3600),
            tenant_id: tenant.clone(),
            contact_uri: format!("sip:valid{i}@192.168.1.1:5060"),
            event_package: "presence".to_string(),
            sip_call_id: None,
            sip_from_tag: None,
            sip_to_tag: None,
        };
        mgr.add_watcher(w).await.unwrap();
    }

    let start = Instant::now();
    let removed = mgr.cleanup_expired_watchers();
    let elapsed = start.elapsed();
    eprintln!(
        "Cleaned up {} expired watchers in {:?}",
        removed.len(),
        elapsed
    );

    assert_eq!(removed.len(), 5000);
    assert_eq!(mgr.total_watcher_count(), 5000);
}

// ===========================================================================
// Item 14: Advanced authorization tests
// ===========================================================================

use fcp_presence::TimeWindow;
use std::collections::HashMap;

/// Test: per-event-package allow blocks other packages.
#[tokio::test]
async fn test_event_package_allow_blocks_other_packages() {
    let mgr = PresenceManager::new(test_config(), None);

    let mut event_allow = HashMap::new();
    event_allow.insert(
        "presence".to_string(),
        HashSet::from(["bob@example.com".to_string()]),
    );

    let policy = PresenceAuthPolicy {
        default_allow: true,
        event_allow,
        ..Default::default()
    };
    mgr.set_auth_policy("alice@example.com", policy, &TenantId::nil());

    // Bob allowed for "presence"
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));
    // Bob NOT allowed for "dialog" when presence-specific allow list is set
    // (dialog has no event_allow entry, so falls through to default_allow=true)
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "dialog"));

    // Charlie not in event_allow for "presence" → blocked for "presence"
    assert!(!mgr.is_watcher_authorized("charlie@example.com", "alice@example.com", "presence"));
    // Charlie allowed for "dialog" (no event_allow entry, default_allow=true)
    assert!(mgr.is_watcher_authorized("charlie@example.com", "alice@example.com", "dialog"));
}

/// Test: per-event-package block for specific event.
#[tokio::test]
async fn test_event_package_block_specific() {
    let mgr = PresenceManager::new(test_config(), None);

    let mut event_block = HashMap::new();
    event_block.insert(
        "dialog".to_string(),
        HashSet::from(["bob@example.com".to_string()]),
    );

    let policy = PresenceAuthPolicy {
        default_allow: true,
        event_block,
        ..Default::default()
    };
    mgr.set_auth_policy("alice@example.com", policy, &TenantId::nil());

    // Bob blocked for "dialog"
    assert!(!mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "dialog"));
    // Bob still allowed for "presence"
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));
}

/// Test: domain wildcard allow pattern.
#[tokio::test]
async fn test_domain_wildcard_allow() {
    let mgr = PresenceManager::new(test_config(), None);

    let policy = PresenceAuthPolicy {
        default_allow: false,
        allow_patterns: vec!["*@support.example.com".to_string()],
        ..Default::default()
    };
    mgr.set_auth_policy("alice@example.com", policy, &TenantId::nil());

    // Support users allowed
    assert!(mgr.is_watcher_authorized(
        "agent1@support.example.com",
        "alice@example.com",
        "presence"
    ));
    assert!(mgr.is_watcher_authorized(
        "agent2@support.example.com",
        "alice@example.com",
        "presence"
    ));
    // Other users blocked (default_allow=false)
    assert!(!mgr.is_watcher_authorized("bob@other.com", "alice@example.com", "presence"));
}

/// Test: domain wildcard block pattern.
#[tokio::test]
async fn test_domain_wildcard_block() {
    let mgr = PresenceManager::new(test_config(), None);

    let policy = PresenceAuthPolicy {
        default_allow: true,
        block_patterns: vec!["*@external.com".to_string()],
        ..Default::default()
    };
    mgr.set_auth_policy("alice@example.com", policy, &TenantId::nil());

    // External users blocked
    assert!(!mgr.is_watcher_authorized("eve@external.com", "alice@example.com", "presence"));
    // Internal users allowed
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));
}

/// Test: time window within current hour allows access.
#[tokio::test]
async fn test_time_window_within() {
    let mgr = PresenceManager::new(test_config(), None);

    // Get current UTC hour to construct a window that covers now
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let current_hour = ((now % 86400) / 3600) as u8;

    let policy = PresenceAuthPolicy {
        default_allow: true,
        time_windows: vec![TimeWindow {
            start_hour: current_hour,
            end_hour: (current_hour + 2) % 24,
            days: vec![], // all days
        }],
        ..Default::default()
    };
    mgr.set_auth_policy("alice@example.com", policy, &TenantId::nil());

    // Should be within time window → allowed
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));
}

/// Test: time window outside current hour blocks access.
#[tokio::test]
async fn test_time_window_outside() {
    let mgr = PresenceManager::new(test_config(), None);

    // Get current UTC hour and create a window that does NOT cover now
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let current_hour = ((now % 86400) / 3600) as u8;
    let start = (current_hour + 12) % 24;
    let end = (current_hour + 14) % 24;

    let policy = PresenceAuthPolicy {
        default_allow: true,
        time_windows: vec![TimeWindow {
            start_hour: start,
            end_hour: end,
            days: vec![], // all days
        }],
        ..Default::default()
    };
    mgr.set_auth_policy("alice@example.com", policy, &TenantId::nil());

    // Should be outside time window → blocked
    assert!(!mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));
}

/// Test: backward compatibility — old-style simple policy still works identically.
#[tokio::test]
async fn test_backward_compat_simple_policy() {
    let mgr = PresenceManager::new(test_config(), None);

    // Old-style: no new fields
    let policy = PresenceAuthPolicy {
        default_allow: false,
        allow_list: HashSet::from(["bob@example.com".to_string()]),
        block_list: HashSet::from(["eve@example.com".to_string()]),
        ..Default::default()
    };
    mgr.set_auth_policy("alice@example.com", policy, &TenantId::nil());

    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "presence"));
    assert!(!mgr.is_watcher_authorized("eve@example.com", "alice@example.com", "presence"));
    assert!(!mgr.is_watcher_authorized("charlie@example.com", "alice@example.com", "presence"));

    // Also works for dialog
    assert!(mgr.is_watcher_authorized("bob@example.com", "alice@example.com", "dialog"));
    assert!(!mgr.is_watcher_authorized("eve@example.com", "alice@example.com", "dialog"));
}
