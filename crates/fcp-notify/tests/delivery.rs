//! On-site notification end to end (`docs/E911_BLOCK_2.md` §8): an
//! emergency call is placed, everyone the tenant listed is told, every
//! attempt is recorded, and what fails is tried again.
//!
//! Gated on `FCP_TEST_DATABASE_URL` like the other database-backed suites;
//! the mail server, the SMS provider and the call manager are the
//! testkit's sinks.

use fcp_core::e911::notify::{ContactKind, EmergencyNotice, NotificationStatus, NotifyContact};
use fcp_core::e911::{EmergencyCallRecord, EmergencyConfig, EmergencyKind, EmergencyStore};
use fcp_core::TenantId;
use fcp_events::{EventSubscriber, InMemoryEventBus, PlatformEvent};
use fcp_notify::{EmergencyNotifier, RetryPolicy};
use fcp_storage_postgres::emergency::PostgresEmergencyStore;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use fcp_testkit::{HttpSink, SmtpSink, TestDatabase};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const SOON: Duration = Duration::from_secs(10);

macro_rules! require_db {
    () => {
        match TestDatabase::create().await {
            Some(db) => db,
            None => {
                eprintln!("skipping: FCP_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

struct Fixture {
    store: Arc<dyn EmergencyStore>,
    tenant: TenantId,
    events: Arc<InMemoryEventBus>,
}

async fn fixture(url: &str, contacts: Vec<NotifyContact>) -> Fixture {
    let storage = PostgresStorage::new(PostgresConfig::new(url))
        .await
        .expect("connect + migrate test database");
    let store: Arc<dyn EmergencyStore> =
        Arc::new(PostgresEmergencyStore::new(storage.pool().clone()));
    let tenant = TenantId::new();
    let mut config = EmergencyConfig::defaults(tenant);
    config.contacts = contacts;
    store
        .put_emergency_config(&config)
        .await
        .expect("save the tenant's emergency configuration");
    Fixture {
        store,
        tenant,
        events: Arc::new(InMemoryEventBus::new()),
    }
}

fn notice(tenant: TenantId, call_id: &str, kind: EmergencyKind) -> EmergencyNotice {
    EmergencyNotice {
        tenant_id: tenant,
        call_id: call_id.to_string(),
        kind,
        dialed: if kind == EmergencyKind::Test {
            "933".to_string()
        } else {
            "911".to_string()
        },
        caller: "sip:alice@pbx.example.com".to_string(),
        caller_name: Some("alice".to_string()),
        callback_number: Some("+15185550100".to_string()),
        location_summary: Some("HQ: 1 State St, Albany, NY 12207".to_string()),
        location_source: Some("user_location".to_string()),
        client_address: Some("10.0.0.5".to_string()),
        trunk_name: Some("carrier-a".to_string()),
        location_id: None,
        placed_at: SystemTime::now(),
    }
}

/// A record for the call, as a node writes when it places one.
async fn record(store: &Arc<dyn EmergencyStore>, tenant: TenantId, call_id: &str) -> i64 {
    let record = EmergencyCallRecord {
        id: 0,
        tenant_id: tenant,
        call_id: call_id.to_string(),
        sip_call_id: None,
        caller_uri: "sip:alice@pbx.example.com".to_string(),
        caller_username: Some("alice".to_string()),
        dialed_number: "911".to_string(),
        kind: EmergencyKind::Live,
        callback_number: Some("+15185550100".to_string()),
        location: None,
        trunk_name: None,
        trunk_id: None,
        outcome: "placed".to_string(),
        placed_at: SystemTime::now(),
        notified: false,
        sip_status: None,
        answered_at: None,
        ended_at: None,
        delivery: None,
        location_structured: None,
        geolocation_error: None,
    };
    store
        .insert_emergency_call(&record)
        .await
        .expect("write the emergency call record")
}

/// Everyone the tenant listed is told, in the words Kari's Law asks for,
/// and the call's record says someone was.
#[tokio::test]
async fn every_contact_is_told_and_the_record_says_so() {
    let db = require_db!();
    let mail = SmtpSink::start().await;
    let sms = HttpSink::start().await;
    let calls = HttpSink::start_with(200, serde_json::json!({ "outcome": "answered" })).await;

    let f = fixture(
        db.url(),
        vec![
            NotifyContact::new(ContactKind::Email, "security@example.com"),
            NotifyContact::new(ContactKind::Sms, "+15185550143"),
            NotifyContact::new(ContactKind::Call, "2001"),
        ],
    )
    .await;
    let call_id = uuid::Uuid::new_v4().to_string();
    let record_id = record(&f.store, f.tenant, &call_id).await;

    let notifier = EmergencyNotifier::new(Arc::clone(&f.store), RetryPolicy::default())
        .with_events(f.events.clone())
        .with_channel(Arc::new(
            fcp_notify::email::EmailChannel::new(&fcp_notify::email::SmtpSettings {
                host: mail.host(),
                port: mail.port(),
                encryption: "none".to_string(),
                username: None,
                password: None,
                from: "fcp@example.com".to_string(),
                timeout: SOON,
            })
            .expect("email channel"),
        ))
        .with_channel(Arc::new(
            fcp_notify::sms::SmsChannel::new(fcp_notify::sms::SmsSettings {
                url: sms.url(),
                token: Some("s3cret".to_string()),
                from: Some("+15185550100".to_string()),
                timeout: SOON,
            })
            .expect("sms channel"),
        ))
        .with_channel(Arc::new(
            fcp_notify::call::CallChannel::new(fcp_notify::call::CallSettings {
                call_manager_url: calls.url(),
                auth_token: None,
                from: Some("sip:alerts@pbx.example.com".to_string()),
                ring_timeout: Duration::from_secs(20),
                repeat: 2,
                timeout: SOON,
            })
            .expect("call channel"),
        ));

    notifier
        .notify(notice(f.tenant, &call_id, EmergencyKind::Live))
        .await;

    // The email says who, where and when.
    let mail = mail.wait_for(1, SOON).await;
    assert_eq!(mail[0].to, vec!["security@example.com".to_string()]);
    assert!(
        mail[0].subject().contains("alice"),
        "{:?}",
        mail[0].subject()
    );
    let body = mail[0].body();
    assert!(body.contains("911"), "{body}");
    assert!(body.contains("1 State St"), "{body}");
    assert!(body.contains("+15185550100"), "{body}");

    // The text is one line, and the call is asked for in words.
    let texts = sms.wait_for(1, SOON).await;
    assert_eq!(texts[0]["to"], "+15185550143");
    assert!(texts[0]["text"].as_str().unwrap().contains("alice"));
    let rung = calls.wait_for(1, SOON).await;
    assert_eq!(rung[0]["to"], "2001");
    assert!(
        rung[0]["text"].as_str().unwrap().contains("nine one one"),
        "{}",
        rung[0]["text"]
    );

    // Every attempt is a row, and the record says someone was told.
    let rows = f
        .store
        .list_emergency_notifications(&f.tenant, &call_id)
        .await
        .expect("notifications");
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(
        rows.iter()
            .all(|r| r.status == NotificationStatus::Delivered),
        "{rows:?}"
    );
    assert!(rows.iter().all(|r| r.attempt == 1));
    let stored = f
        .store
        .list_emergency_calls(&f.tenant, 10)
        .await
        .expect("calls");
    let stored = stored
        .iter()
        .find(|c| c.id == record_id)
        .expect("the record");
    assert!(stored.notified, "the record says someone was told");

    db.drop_db().await;
}

/// What fails is tried again, and what keeps failing is given up on —
/// loudly, so a tenant can see that nobody was told.
#[tokio::test]
async fn a_failed_contact_is_retried_then_given_up_on() {
    let db = require_db!();
    let sms = HttpSink::start_with(500, serde_json::json!({ "error": "provider down" })).await;
    let f = fixture(
        db.url(),
        vec![NotifyContact::new(ContactKind::Sms, "+15185550143")],
    )
    .await;
    let call_id = uuid::Uuid::new_v4().to_string();
    record(&f.store, f.tenant, &call_id).await;

    let notifier = EmergencyNotifier::new(
        Arc::clone(&f.store),
        RetryPolicy {
            max_attempts: 2,
            // Due at once, so the test does not wait out a backoff.
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            give_up_after: Duration::from_secs(300),
        },
    )
    .with_events(f.events.clone())
    .with_channel(Arc::new(
        fcp_notify::sms::SmsChannel::new(fcp_notify::sms::SmsSettings {
            url: sms.url(),
            token: None,
            from: None,
            timeout: SOON,
        })
        .expect("sms channel"),
    ));

    let mut events = f
        .events
        .subscribe(fcp_events::EventFilter::new())
        .await
        .expect("subscribe");

    notifier
        .notify(notice(f.tenant, &call_id, EmergencyKind::Live))
        .await;
    let rows = f
        .store
        .list_emergency_notifications(&f.tenant, &call_id)
        .await
        .expect("notifications");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, NotificationStatus::Retrying, "{rows:?}");
    assert!(rows[0].next_retry_at.is_some());
    assert!(rows[0].error.as_deref().unwrap_or_default().contains("500"));

    // The second attempt is the last one this contact gets.
    let tried = notifier.run_due_retries(10).await;
    assert_eq!(tried, 1);
    let rows = f
        .store
        .list_emergency_notifications(&f.tenant, &call_id)
        .await
        .expect("notifications");
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[1].attempt, 2);
    assert_eq!(rows[1].status, NotificationStatus::Failed, "{rows:?}");
    assert!(rows[1].next_retry_at.is_none());

    // And it is announced, so something can be done about it.
    let failure = tokio::time::timeout(SOON, async {
        loop {
            match events.next().await {
                Some(PlatformEvent::Call(fcp_events::CallEvent::EmergencyNotificationFailed {
                    target,
                    attempts,
                    ..
                })) => return (target, attempts),
                Some(_) => continue,
                None => panic!("the event stream ended"),
            }
        }
    })
    .await
    .expect("emergency.notification_failed");
    assert_eq!(failure.0, "+15185550143");
    assert_eq!(failure.1, 2);

    // Nothing is due any more.
    assert_eq!(notifier.run_due_retries(10).await, 0);

    db.drop_db().await;
}

/// A 933 test call's notice says it is a test everywhere, and the
/// readiness report can find it afterwards (§8.4).
#[tokio::test]
async fn a_test_calls_notice_says_it_is_a_test() {
    let db = require_db!();
    let mail = SmtpSink::start().await;
    let f = fixture(
        db.url(),
        vec![NotifyContact::new(
            ContactKind::Email,
            "security@example.com",
        )],
    )
    .await;
    let call_id = uuid::Uuid::new_v4().to_string();

    let notifier = EmergencyNotifier::new(Arc::clone(&f.store), RetryPolicy::default())
        .with_channel(Arc::new(
            fcp_notify::email::EmailChannel::new(&fcp_notify::email::SmtpSettings {
                host: mail.host(),
                port: mail.port(),
                encryption: "none".to_string(),
                username: None,
                password: None,
                from: "fcp@example.com".to_string(),
                timeout: SOON,
            })
            .expect("email channel"),
        ));
    notifier
        .notify(notice(f.tenant, &call_id, EmergencyKind::Test))
        .await;

    let mail = mail.wait_for(1, SOON).await;
    assert!(
        mail[0].subject().starts_with("[TEST]"),
        "{}",
        mail[0].subject()
    );
    assert!(mail[0]
        .body()
        .contains("No emergency services were contacted"));

    let last = f
        .store
        .last_emergency_notifications(&f.tenant, true)
        .await
        .expect("last test notifications");
    assert_eq!(last.len(), 1, "{last:?}");
    assert_eq!(last[0].contact.target, "security@example.com");
    assert!(last[0].delivered_at.is_some());

    db.drop_db().await;
}

/// A call from a site tells the site's contacts first, then the tenant's
/// unless the site says its own are enough (`docs/E911.md` "On-site
/// notification").
#[tokio::test]
async fn a_sites_contacts_are_told_and_the_tenants_unless_it_says_not() {
    use fcp_core::storage::SiteLocationStore;
    let db = require_db!();
    let mail = SmtpSink::start().await;
    let f = fixture(
        db.url(),
        vec![NotifyContact::new(ContactKind::Email, "office@example.com")],
    )
    .await;
    let storage = PostgresStorage::new(PostgresConfig::new(db.url()))
        .await
        .expect("connect");
    let mut site = fcp_core::SiteLocation::new("Albany".to_string(), f.tenant);
    site.emergency_contacts = vec![NotifyContact::new(
        ContactKind::Email,
        "albany-desk@example.com",
    )];
    storage
        .put_location(site.clone())
        .await
        .expect("save the site");

    let notifier = EmergencyNotifier::new(Arc::clone(&f.store), RetryPolicy::default())
        .with_channel(Arc::new(
            fcp_notify::email::EmailChannel::new(&fcp_notify::email::SmtpSettings {
                host: mail.host(),
                port: mail.port(),
                encryption: "none".to_string(),
                username: None,
                password: None,
                from: "fcp@example.com".to_string(),
                timeout: SOON,
            })
            .expect("email channel"),
        ));
    let from_site = |call_id: &str| EmergencyNotice {
        location_id: Some(site.id.to_string()),
        ..notice(f.tenant, call_id, EmergencyKind::Live)
    };

    notifier
        .notify(from_site(&uuid::Uuid::new_v4().to_string()))
        .await;
    let seen = mail.wait_for(2, SOON).await;
    let bare = |to: &[String]| -> Vec<String> {
        to.iter()
            .map(|a| a.trim_matches(|c| c == '<' || c == '>').to_string())
            .collect()
    };
    let mut to: Vec<String> = seen.iter().flat_map(|m| bare(&m.to)).collect();
    to.sort();
    assert_eq!(to, vec!["albany-desk@example.com", "office@example.com"]);

    site.notify_tenant_contacts = false;
    storage
        .put_location(site.clone())
        .await
        .expect("save the site");
    notifier
        .notify(from_site(&uuid::Uuid::new_v4().to_string()))
        .await;
    let seen = mail.wait_for(3, SOON).await;
    assert_eq!(bare(&seen[2].to), vec!["albany-desk@example.com"]);

    db.drop_db().await;
}
