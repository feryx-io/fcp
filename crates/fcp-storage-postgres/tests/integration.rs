//! Integration tests against a real PostgreSQL instance.
//!
//! Gated on `FCP_TEST_DATABASE_URL` (an admin connection string, e.g.
//! `postgresql://postgres:postgres@localhost:5432/postgres`). When it is
//! unset every test returns early with a notice, so `cargo test` stays green
//! on machines without a database. CI provides a Postgres service and sets
//! the variable, so these run on every push.
//!
//! Isolation: each test creates its own database (`fcp_test_<hex>`), runs the
//! full migration set through `PostgresStorage::new`, and drops the database
//! at the end. Tests are therefore parallel-safe and never see each other's
//! rows. A test that panics leaves its database behind; the prefix makes
//! stale ones easy to find and drop.

use fcp_core::billing::{
    BalanceTransactionKind, BalanceTransactionRequest, BillingStore, Decimal, Rate, RatePlan,
    TenantBalance,
};
use fcp_core::encryption::EncryptionStore;
use fcp_core::recording::RecordingStore;
use fcp_core::storage::{
    CallDetailRecord, CallDisposition, CdrQuery, CdrStore, CredentialStore, HangupCause,
    PhoneNumberStore, Registration, RegistrationStore, TenantStore, User,
};
use fcp_core::storage::{ConferenceStore, DeviceStore, TrunkStore, WebhookStore};
use fcp_core::{CallId, PhoneNumber, StorageError, Tenant, TenantId};
use fcp_crypto::EncryptionSettings;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use rust_decimal_macros::dec;
use sqlx::{Connection, PgConnection};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

// ─── Harness ─────────────────────────────────────────────────────────────

/// Skip the test (returning early) unless a database is configured.
macro_rules! require_db {
    () => {
        match TestDb::new().await {
            Some(db) => db,
            None => {
                eprintln!("FCP_TEST_DATABASE_URL not set; skipping PostgreSQL integration test");
                return;
            }
        }
    };
}

struct TestDb {
    admin_url: String,
    name: String,
    storage: PostgresStorage,
}

impl TestDb {
    async fn new() -> Option<Self> {
        Self::with_encryption(EncryptionSettings::Disabled).await
    }

    async fn with_encryption(settings: EncryptionSettings) -> Option<Self> {
        let admin_url = std::env::var("FCP_TEST_DATABASE_URL").ok()?;
        let name = format!("fcp_test_{}", Uuid::new_v4().simple());

        let mut admin = PgConnection::connect(&admin_url)
            .await
            .expect("connect to admin database");
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&mut admin)
            .await
            .expect("create test database");
        admin.close().await.ok();

        let storage = PostgresStorage::new(
            PostgresConfig::new(Self::url(&admin_url, &name)).with_encryption(settings),
        )
        .await
        .expect("connect + migrate test database");
        Some(Self {
            admin_url,
            name,
            storage,
        })
    }

    /// Swap the database name in an admin connection string, keeping any
    /// query parameters (`?sslmode=…`).
    fn url(admin_url: &str, database: &str) -> String {
        let (base, query) = match admin_url.split_once('?') {
            Some((b, q)) => (b, format!("?{q}")),
            None => (admin_url, String::new()),
        };
        let (head, _) = base
            .rsplit_once('/')
            .expect("FCP_TEST_DATABASE_URL must contain a database path");
        format!("{head}/{database}{query}")
    }

    fn storage(&self) -> &PostgresStorage {
        &self.storage
    }

    /// A second backend over the same database with its own settings.
    async fn connect_with(&self, settings: EncryptionSettings) -> anyhow::Result<PostgresStorage> {
        PostgresStorage::new(
            PostgresConfig::new(TestDb::url(&self.admin_url, &self.name)).with_encryption(settings),
        )
        .await
    }

    /// Raw column value, bypassing the backend's open step.
    async fn raw(&self, sql: &str, id: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(sql)
            .bind(id)
            .fetch_one(self.storage.pool())
            .await
            .expect("raw query")
    }

    /// Drop the database. Call at the end of a passing test.
    async fn finish(self) {
        self.storage.close().await;
        let mut admin = PgConnection::connect(&self.admin_url)
            .await
            .expect("connect to admin database");
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.name))
            .execute(&mut admin)
            .await
            .expect("drop test database");
        admin.close().await.ok();
    }
}

fn tenant(name: &str) -> Tenant {
    let mut t = Tenant::new(name.to_string());
    t.sip_domains = vec![format!("{name}.example.com")];
    t
}

/// Build a user through serde so the fixture does not have to track every
/// enterprise field on the struct; only the non-defaulted ones are given.
fn user(tenant_id: TenantId, username: &str) -> User {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    serde_json::from_value(serde_json::json!({
        "username": username,
        "password": "secret",
        "tenant_id": tenant_id,
        "display_name": format!("User {username}"),
        "enabled": true,
        "created_at": now,
        "updated_at": now,
    }))
    .expect("user fixture deserializes")
}

fn registration(tenant_id: TenantId, aor: &str, contact: &str, ttl: Duration) -> Registration {
    let now = SystemTime::now();
    Registration {
        tenant_id,
        aor: aor.to_string(),
        contact: contact.to_string(),
        expires_at: now + ttl,
        call_id: format!("{}@test", Uuid::new_v4()),
        cseq: 1,
        user_agent: Some("fcp-test/1.0".to_string()),
        q: None,
        registered_at: now,
        path: None,
        flow_token: None,
        source_address: None,
        instance_id: None,
        reg_id: None,
        push: None,
        flow_node: None,
    }
}

fn cdr(
    tenant_id: TenantId,
    to_uri: &str,
    answered_ms: Option<u64>,
    cost: Option<Decimal>,
) -> CallDetailRecord {
    let start = SystemTime::now() - Duration::from_secs(120);
    let (disposition, answer_time) = match answered_ms {
        Some(_) => (
            CallDisposition::Answered,
            Some(start + Duration::from_secs(2)),
        ),
        None => (CallDisposition::NoAnswer, None),
    };
    CallDetailRecord {
        cdr_id: uuid::Uuid::new_v4(),
        kind: fcp_core::storage::CdrKind::Call,
        schema_version: fcp_core::storage::CDR_SCHEMA_VERSION,
        call_id: CallId::new(),
        tenant_id,
        sip_call_id: format!("{}@test", Uuid::new_v4()),
        from_uri: "sip:alice@test.example.com".to_string(),
        from_display_name: Some("Alice".to_string()),
        to_uri: to_uri.to_string(),
        to_display_name: None,
        start_time: start,
        answer_time,
        end_time: Some(start + Duration::from_secs(60)),
        setup_duration_ms: Some(2000),
        call_duration_ms: answered_ms,
        total_duration_ms: Some(60_000),
        disposition,
        hangup_cause: HangupCause::NormalClearing,
        hangup_party: None,
        sip_status_code: Some(200),
        sip_reason: None,
        route_id: None,
        gateway_id: Some("trunk-a".to_string()),
        media_anchored: true,
        codec_a_leg: Some("PCMU".to_string()),
        codec_b_leg: Some("PCMU".to_string()),
        rtp_packets_sent: None,
        rtp_packets_received: None,
        rtp_bytes_sent: None,
        rtp_bytes_received: None,
        recording_url: None,
        recording_consent: None,
        cost,
        currency: cost.map(|_| "USD".to_string()),
        direction: None,
        node_id: None,
        site_id: None,
        region: None,
        correlation_id: None,
        trunk_name: None,
        trunk_id: None,
        rtp_packets_lost: None,
        rtp_jitter_ms: None,
        rate_plan_id: None,
        matched_prefix: None,
        billable_seconds: None,
        rated_at: None,
        created_at: None,
        custom_data: None,
    }
}

fn rate_plan(tenant_id: TenantId, name: &str, is_default: bool) -> RatePlan {
    let now = SystemTime::now();
    RatePlan {
        id: Uuid::new_v4(),
        tenant_id,
        name: name.to_string(),
        currency: "USD".to_string(),
        default_rate_per_minute: dec!(0.02),
        connect_fee: Decimal::ZERO,
        billing_increment_secs: 60,
        is_default,
        enabled: true,
        created_at: now,
        updated_at: now,
    }
}

fn rate(plan_id: Uuid, prefix: &str, per_minute: Decimal) -> Rate {
    Rate {
        id: Uuid::new_v4(),
        rate_plan_id: plan_id,
        prefix: prefix.to_string(),
        rate_per_minute: per_minute,
        connect_fee: None,
        description: None,
    }
}

fn balance(tenant_id: TenantId, enforce: bool) -> TenantBalance {
    TenantBalance {
        tenant_id,
        balance: Decimal::ZERO,
        currency: "USD".to_string(),
        credit_limit: Decimal::ZERO,
        enforce,
        low_balance_threshold: None,
        updated_at: SystemTime::now(),
    }
}

fn charge(tenant_id: TenantId, call_id: CallId, amount: Decimal) -> BalanceTransactionRequest {
    BalanceTransactionRequest {
        tenant_id,
        kind: BalanceTransactionKind::CallCharge,
        amount: -amount,
        currency: "USD".to_string(),
        call_id: Some(call_id),
        description: None,
        external_ref: None,
    }
}

fn is_conflict(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<fcp_core::Error>(),
        Some(fcp_core::Error::Storage(StorageError::Conflict(_)))
    )
}

fn is_not_found(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<fcp_core::Error>(),
        Some(fcp_core::Error::Storage(StorageError::NotFound(_)))
    )
}

fn approx(a: Decimal, b: Decimal) -> bool {
    a == b
}

// ─── Migrations ──────────────────────────────────────────────────────────

#[tokio::test]
async fn migrations_apply_and_are_idempotent() {
    let db = require_db!();
    // A second `new()` against the same database re-runs the migrator, which
    // must find everything applied and succeed.
    let again = PostgresStorage::new(PostgresConfig::new(TestDb::url(&db.admin_url, &db.name)))
        .await
        .expect("second migration pass is a no-op");
    again.close().await;
    db.finish().await;
}

// ─── Tenants ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn tenant_round_trip_and_lookups() {
    let db = require_db!();
    let s = db.storage();

    let t = tenant("acme");
    s.put_tenant(t.clone()).await.unwrap();

    let got = s.get_tenant(&t.id).await.unwrap().expect("tenant stored");
    assert_eq!(got.name, "acme");
    assert_eq!(got.sip_domains, vec!["acme.example.com"]);

    assert_eq!(
        s.get_tenant_by_name("acme").await.unwrap().map(|t| t.id),
        Some(t.id)
    );
    assert_eq!(
        s.get_tenant_by_sip_domain("ACME.example.com")
            .await
            .unwrap()
            .map(|t| t.id),
        Some(t.id),
        "sip domain lookup is case-insensitive"
    );
    assert!(s
        .get_tenant_by_sip_domain("nobody.example.com")
        .await
        .unwrap()
        .is_none());

    // Update through put (upsert) — name change is visible.
    let mut updated = got.clone();
    updated.name = "acme-renamed".to_string();
    s.put_tenant(updated).await.unwrap();
    assert_eq!(
        s.get_tenant(&t.id).await.unwrap().unwrap().name,
        "acme-renamed"
    );
    assert!(s.get_tenant_by_name("acme").await.unwrap().is_none());

    assert!(s.list_tenants().await.unwrap().iter().any(|x| x.id == t.id));

    s.delete_tenant(&t.id).await.unwrap();
    assert!(s.get_tenant(&t.id).await.unwrap().is_none());

    db.finish().await;
}

// ─── Users ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn users_are_scoped_by_tenant() {
    let db = require_db!();
    let s = db.storage();

    let a = tenant("a");
    let b = tenant("b");
    s.put_tenant(a.clone()).await.unwrap();
    s.put_tenant(b.clone()).await.unwrap();

    // Same username in two tenants must coexist.
    s.put_user(user(a.id, "alice")).await.unwrap();
    s.put_user(user(b.id, "alice")).await.unwrap();

    let in_a = s
        .get_user_in_tenant("alice", &a.id)
        .await
        .unwrap()
        .expect("alice in a");
    let in_b = s
        .get_user_in_tenant("alice", &b.id)
        .await
        .unwrap()
        .expect("alice in b");
    assert_ne!(in_a.id, in_b.id);
    assert_eq!(in_a.tenant_id, a.id);
    assert_eq!(in_b.tenant_id, b.id);
    assert_eq!(s.list_users_by_username("alice").await.unwrap().len(), 2);

    let by_id = s
        .get_user_by_id(&in_a.id)
        .await
        .unwrap()
        .expect("lookup by id");
    assert_eq!(by_id.username, "alice");
    assert_eq!(by_id.display_name.as_deref(), Some("User alice"));

    // Deleting in one tenant leaves the other intact.
    s.delete_user_in_tenant("alice", &a.id).await.unwrap();
    assert!(s
        .get_user_in_tenant("alice", &a.id)
        .await
        .unwrap()
        .is_none());
    assert!(s
        .get_user_in_tenant("alice", &b.id)
        .await
        .unwrap()
        .is_some());

    db.finish().await;
}

// ─── Registrations ───────────────────────────────────────────────────────

/// A push binding keeps its phone's instance, where to wake it and the node
/// holding its flow (`docs/PUSH.md` §2.1, migration 111).
#[tokio::test]
async fn a_push_binding_keeps_its_instance_and_where_to_wake_it() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("push");
    s.put_tenant(a.clone()).await.unwrap();
    let aor = "sip:app@push.example.com";
    let mut row = registration(
        a.id,
        aor,
        "<sip:app-1@10.0.0.7:52311;transport=tls>",
        Duration::from_secs(604_800),
    );
    row.instance_id = Some("<urn:uuid:00000000-0000-0000-0000-0000000000a1>".into());
    row.reg_id = Some(1);
    row.flow_token = Some("flow-1".into());
    row.flow_node = Some("node-a".into());
    row.push = Some(fcp_core::push::PushAddress {
        provider: "fcm".into(),
        param: "anvil-12345".into(),
        prid: "dA1b2C3d4E5f6G7h8I9j".into(),
    });
    s.put_registration(row.clone()).await.unwrap();
    let back = s.get_registrations_in_tenant(&a.id, aor).await.unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].instance_id, row.instance_id);
    assert_eq!(back[0].reg_id, Some(1));
    assert_eq!(back[0].flow_node.as_deref(), Some("node-a"));
    assert_eq!(back[0].push, row.push);
    let left = back[0]
        .expires_at
        .duration_since(SystemTime::now())
        .unwrap()
        .as_secs();
    assert!(left > 600_000, "a week is kept: {left}");

    // An ordinary binding has none of it.
    s.put_registration(registration(
        a.id,
        aor,
        "sip:desk@10.0.0.9:5060",
        Duration::from_secs(3600),
    ))
    .await
    .unwrap();
    let desk = s
        .get_registrations_in_tenant(&a.id, aor)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.contact == "sip:desk@10.0.0.9:5060")
        .unwrap();
    assert_eq!(desk.push, None);
    assert_eq!(desk.instance_id, None);

    // A device shows the provider that wakes it, gone once it unregisters.
    let mut device =
        fcp_core::Device::new("App".into(), fcp_core::device::DeviceType::Softphone, a.id);
    device.sip_username = Some("app-1".into());
    s.put_device(device.clone()).await.unwrap();
    let mut update = fcp_core::device::DeviceRegistrationUpdate {
        state: fcp_core::device::RegistrationState::Registered,
        at: SystemTime::now(),
        address: None,
        user_agent: None,
        vendor: None,
        model: None,
        firmware: None,
        push_provider: Some("fcm".into()),
    };
    s.record_device_registration(&device.id, &update)
        .await
        .unwrap();
    let got = s.get_device(&device.id).await.unwrap().unwrap();
    assert_eq!(got.push_provider.as_deref(), Some("fcm"));
    update.state = fcp_core::device::RegistrationState::Unregistered;
    update.push_provider = None;
    s.record_device_registration(&device.id, &update)
        .await
        .unwrap();
    let got = s.get_device(&device.id).await.unwrap().unwrap();
    assert_eq!(got.push_provider, None);
}

#[tokio::test]
async fn registrations_bind_expire_and_scope() {
    let db = require_db!();
    let s = db.storage();

    let a = tenant("a");
    let b = tenant("b");
    s.put_tenant(a.clone()).await.unwrap();
    s.put_tenant(b.clone()).await.unwrap();

    let aor = "sip:2001@a.example.com";
    let hour = Duration::from_secs(3600);
    s.put_registration(registration(a.id, aor, "sip:2001@10.0.0.1:5060", hour))
        .await
        .unwrap();
    s.put_registration(registration(a.id, aor, "sip:2001@10.0.0.2:5060", hour))
        .await
        .unwrap();
    // Same AOR string in another tenant is a separate binding set.
    s.put_registration(registration(b.id, aor, "sip:2001@10.0.1.1:5060", hour))
        .await
        .unwrap();

    assert_eq!(
        s.get_registrations_in_tenant(&a.id, aor)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        s.get_registrations_in_tenant(&b.id, aor)
            .await
            .unwrap()
            .len(),
        1
    );

    // Re-registering the same contact refreshes rather than duplicates.
    let mut refreshed = registration(a.id, aor, "sip:2001@10.0.0.1:5060", hour * 2);
    refreshed.cseq = 2;
    s.put_registration(refreshed).await.unwrap();
    let regs = s.get_registrations_in_tenant(&a.id, aor).await.unwrap();
    assert_eq!(regs.len(), 2);
    assert_eq!(
        regs.iter()
            .find(|r| r.contact == "sip:2001@10.0.0.1:5060")
            .map(|r| r.cseq),
        Some(2)
    );

    s.delete_registration_in_tenant(&a.id, aor, "sip:2001@10.0.0.2:5060")
        .await
        .unwrap();
    assert_eq!(
        s.get_registrations_in_tenant(&a.id, aor)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.get_registrations_in_tenant(&b.id, aor)
            .await
            .unwrap()
            .len(),
        1
    );

    // An already-expired binding is swept by cleanup_expired.
    let expired_aor = "sip:2002@a.example.com";
    let mut expired = registration(a.id, expired_aor, "sip:2002@10.0.0.9:5060", Duration::ZERO);
    expired.expires_at = SystemTime::now() - Duration::from_secs(10);
    s.put_registration(expired).await.unwrap();
    let swept = s.cleanup_expired().await.unwrap();
    assert!(
        swept >= 1,
        "expected at least one expired binding swept, got {swept}"
    );
    assert!(s
        .get_registrations_in_tenant(&a.id, expired_aor)
        .await
        .unwrap()
        .is_empty());

    db.finish().await;
}

// ─── CDRs ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cdrs_insert_query_count_and_aggregate() {
    let db = require_db!();
    let s = db.storage();

    let a = tenant("a");
    let b = tenant("b");
    s.put_tenant(a.clone()).await.unwrap();
    s.put_tenant(b.clone()).await.unwrap();

    let answered = cdr(
        a.id,
        "sip:+14155550100@a.example.com",
        Some(90_000),
        Some(dec!(0.04)),
    );
    let answered_id = answered.call_id;
    s.insert_cdr(answered).await.unwrap();
    s.insert_cdr(cdr(
        a.id,
        "sip:+442079460000@a.example.com",
        Some(30_000),
        Some(dec!(0.03)),
    ))
    .await
    .unwrap();
    s.insert_cdr(cdr(a.id, "sip:2002@a.example.com", None, None))
        .await
        .unwrap();
    s.insert_cdr(cdr(
        b.id,
        "sip:+14155550100@b.example.com",
        Some(10_000),
        Some(dec!(1)),
    ))
    .await
    .unwrap();

    let got = s.get_cdr(&answered_id).await.unwrap().expect("cdr by id");
    assert_eq!(got.tenant_id, a.id);
    assert_eq!(got.disposition, CallDisposition::Answered);
    assert_eq!(got.call_duration_ms, Some(90_000));
    assert_eq!(got.cost, Some(dec!(0.04)));
    assert_eq!(got.currency.as_deref(), Some("USD"));
    assert_eq!(got.gateway_id.as_deref(), Some("trunk-a"));

    let tenant_query = CdrQuery {
        tenant_id: Some(a.id),
        ..Default::default()
    };
    assert_eq!(s.query_cdrs(tenant_query.clone()).await.unwrap().len(), 3);
    assert_eq!(s.count_cdrs(tenant_query.clone()).await.unwrap(), 3);

    let answered_only = CdrQuery {
        tenant_id: Some(a.id),
        disposition: Some(CallDisposition::Answered),
        ..Default::default()
    };
    assert_eq!(s.count_cdrs(answered_only).await.unwrap(), 2);

    let paged = CdrQuery {
        tenant_id: Some(a.id),
        limit: Some(2),
        offset: Some(2),
        ..Default::default()
    };
    assert_eq!(s.query_cdrs(paged).await.unwrap().len(), 1);

    // URI substring filters, and count must agree with query for every
    // filter combination (regression: count used to ignore these).
    let by_to = CdrQuery {
        tenant_id: Some(a.id),
        to_uri: Some("+4420".to_string()),
        ..Default::default()
    };
    assert_eq!(s.query_cdrs(by_to.clone()).await.unwrap().len(), 1);
    assert_eq!(s.count_cdrs(by_to).await.unwrap(), 1);
    let by_from = CdrQuery {
        from_uri: Some("alice@".to_string()),
        ..Default::default()
    };
    assert_eq!(s.count_cdrs(by_from).await.unwrap(), 4);
    let by_call = CdrQuery {
        call_id: Some(answered_id),
        ..Default::default()
    };
    assert_eq!(s.query_cdrs(by_call.clone()).await.unwrap().len(), 1);
    assert_eq!(s.count_cdrs(by_call).await.unwrap(), 1);

    // Time-window filters bind real timestamps.
    let in_window = CdrQuery {
        tenant_id: Some(a.id),
        start_time_from: Some(SystemTime::now() - Duration::from_secs(600)),
        start_time_to: Some(SystemTime::now()),
        ..Default::default()
    };
    assert_eq!(s.query_cdrs(in_window.clone()).await.unwrap().len(), 3);
    assert_eq!(s.count_cdrs(in_window).await.unwrap(), 3);
    let future = CdrQuery {
        tenant_id: Some(a.id),
        start_time_from: Some(SystemTime::now() + Duration::from_secs(60)),
        ..Default::default()
    };
    assert!(s.query_cdrs(future.clone()).await.unwrap().is_empty());
    assert_eq!(s.count_cdrs(future).await.unwrap(), 0);

    // Tenant a owns the number the answered call was placed to, so that call
    // counts as inbound in both aggregates.
    s.put_phone_number(PhoneNumber::new("+14155550100".to_string(), a.id).unwrap())
        .await
        .unwrap();

    let from = SystemTime::now() - Duration::from_secs(3600);
    let to = SystemTime::now() + Duration::from_secs(60);
    let agg = s.aggregate_usage(&a.id, from, to).await.unwrap();
    assert_eq!(agg.total_calls, 3);
    assert_eq!(agg.answered_calls, 2);
    assert_eq!(agg.total_duration_ms, 120_000);
    assert_eq!(agg.inbound_calls, 1);
    assert_eq!(agg.outbound_calls, 0);
    assert!(
        approx(agg.total_cost, dec!(0.07)),
        "total_cost = {}",
        agg.total_cost
    );

    let by_number = s.aggregate_usage_by_number(&a.id, from, to).await.unwrap();
    assert_eq!(
        by_number.len(),
        1,
        "one owned number with traffic: {by_number:?}"
    );
    let n = &by_number[0];
    assert_eq!(n.number, "+14155550100");
    assert_eq!(n.total_calls, 1);
    assert_eq!(n.answered_calls, 1);
    assert_eq!(n.total_duration_ms, 90_000);
    assert_eq!(n.inbound_calls, 1);
    assert_eq!(n.outbound_calls, 0);

    // Tenant b's expensive call must not leak into a's aggregate.
    let agg_b = s.aggregate_usage(&b.id, from, to).await.unwrap();
    assert_eq!(agg_b.total_calls, 1);
    assert!(approx(agg_b.total_cost, dec!(1.0)));

    db.finish().await;
}

#[tokio::test]
async fn cdr_records_are_idempotent_by_cdr_id_and_keep_their_kind_and_pipeline_fields() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    // A call record with everything the node and the processor stamp.
    let mut call = cdr(
        a.id,
        "sip:+14155550100@a.example.com",
        Some(45_000),
        Some(dec!(0.02)),
    );
    call.hangup_party = Some(fcp_core::storage::HangupParty::Callee);
    call.node_id = Some("node-1".into());
    call.region = Some("us-east".into());
    call.trunk_name = Some("carrier-a".into());
    call.correlation_id = Some(CallId::new());
    call.direction = Some(fcp_core::CallDirection::Outbound);
    call.rtp_packets_sent = Some(2_250);
    call.rtp_packets_received = Some(2_240);
    call.rtp_packets_lost = Some(3);
    call.rtp_jitter_ms = Some(4.5);
    call.rate_plan_id = Some(Uuid::new_v4().to_string());
    call.matched_prefix = Some("+1".into());
    call.billable_seconds = Some(60);
    call.rated_at = Some(SystemTime::now());
    call.created_at = Some(SystemTime::now());
    s.insert_cdr(call.clone()).await.unwrap();

    // A replay of the same record stores nothing and is not an error.
    let mut replay = call.clone();
    replay.cost = Some(dec!(99));
    s.insert_cdr(replay).await.unwrap();

    // A queue segment of the same call sits beside the call's own record.
    let mut queue = cdr(a.id, "queue:support", Some(30_000), None);
    queue.call_id = call.call_id;
    queue.cdr_id = Uuid::new_v4();
    queue.kind = fcp_core::storage::CdrKind::Queue;
    s.insert_cdr(queue.clone()).await.unwrap();

    // A second call record for the same call is refused: one per call (at
    // one start time, the partition key being part of every unique rule).
    // The refusal is a CdrRejected, and the batch it was in stores nothing.
    let mut duplicate = cdr(a.id, "sip:+14155550100@a.example.com", Some(1_000), None);
    duplicate.call_id = call.call_id;
    duplicate.cdr_id = Uuid::new_v4();
    duplicate.start_time = call.start_time;
    let innocent = cdr(a.id, "sip:bob@a.example.com", Some(1_000), None);
    let err = s
        .insert_cdrs(vec![innocent.clone(), duplicate])
        .await
        .expect_err("a second call record");
    assert!(
        err.downcast_ref::<fcp_core::storage::CdrRejected>()
            .is_some(),
        "{err:#}"
    );
    assert!(
        s.get_cdr(&innocent.call_id).await.unwrap().is_none(),
        "rolled back"
    );

    // A batch reports which of its records are new.
    let fresh = cdr(a.id, "sip:carol@a.example.com", Some(2_000), None);
    let new = s
        .insert_cdrs(vec![call.clone(), fresh.clone(), fresh.clone()])
        .await
        .unwrap();
    assert_eq!(new, vec![fresh.cdr_id]);
    s.insert_cdrs(vec![innocent.clone()]).await.unwrap();

    let all = s
        .query_cdrs(CdrQuery {
            tenant_id: Some(a.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all.len(), 4, "{all:#?}");
    let calls_only = CdrQuery {
        tenant_id: Some(a.id),
        kind: Some(fcp_core::storage::CdrKind::Call),
        ..Default::default()
    };
    assert_eq!(s.count_cdrs(calls_only.clone()).await.unwrap(), 3);
    let queues = s
        .query_cdrs(CdrQuery {
            tenant_id: Some(a.id),
            kind: Some(fcp_core::storage::CdrKind::Queue),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(queues.len(), 1);
    assert_eq!(queues[0].cdr_id, queue.cdr_id);

    // get_cdr answers with the call's own record, every field intact.
    let got = s.get_cdr(&call.call_id).await.unwrap().expect("stored");
    assert_eq!(got.cdr_id, call.cdr_id);
    assert_eq!(got.kind, fcp_core::storage::CdrKind::Call);
    assert_eq!(got.schema_version, fcp_core::storage::CDR_SCHEMA_VERSION);
    assert_eq!(got.cost, Some(dec!(0.02)), "the replay changed nothing");
    assert_eq!(
        got.hangup_party,
        Some(fcp_core::storage::HangupParty::Callee)
    );
    assert_eq!(got.node_id.as_deref(), Some("node-1"));
    assert_eq!(got.region.as_deref(), Some("us-east"));
    assert_eq!(got.trunk_name.as_deref(), Some("carrier-a"));
    assert_eq!(got.correlation_id, call.correlation_id);
    assert_eq!(got.direction, Some(fcp_core::CallDirection::Outbound));
    assert_eq!(got.rtp_packets_sent, Some(2_250));
    assert_eq!(got.rtp_packets_received, Some(2_240));
    assert_eq!(got.rtp_packets_lost, Some(3));
    assert_eq!(got.rtp_jitter_ms, Some(4.5));
    assert_eq!(got.rate_plan_id, call.rate_plan_id);
    assert_eq!(got.matched_prefix.as_deref(), Some("+1"));
    assert_eq!(got.billable_seconds, Some(60));
    assert!(got.rated_at.is_some());
    assert!(got.created_at.is_some());
}

#[tokio::test]
async fn usage_sums_money_exactly_and_counts_the_stored_direction() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    // Ten charges of 0.1: a float column sums these to 0.9999999999999999.
    let mut batch = Vec::new();
    for i in 0..10 {
        let mut r = cdr(
            a.id,
            "sip:+14155550100@a.example.com",
            Some(60_000),
            Some(dec!(0.1)),
        );
        // The enricher's direction; nothing in phone_numbers says so.
        r.direction = Some(if i < 3 {
            fcp_core::CallDirection::Inbound
        } else {
            fcp_core::CallDirection::Outbound
        });
        batch.push(r);
    }
    s.insert_cdrs(batch).await.unwrap();

    let now = SystemTime::now();
    let agg = s
        .aggregate_usage(&a.id, now - Duration::from_secs(3600), now)
        .await
        .unwrap();
    assert_eq!(agg.total_calls, 10);
    assert_eq!(agg.total_cost, dec!(1));
    assert_eq!((agg.inbound_calls, agg.outbound_calls), (3, 7));
}

#[tokio::test]
async fn records_live_in_monthly_partitions_and_a_new_month_takes_its_rows_from_the_default_one() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();
    let pool = s.pool();

    // Last month through three ahead exist after the migrations.
    let parts: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_inherits i
         JOIN pg_class c ON c.oid = i.inhrelid JOIN pg_class p ON p.oid = i.inhparent
         WHERE p.relname = 'cdrs' ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert!(parts.contains(&"cdrs_default".to_string()), "{parts:?}");
    assert!(parts.len() >= 6, "{parts:?}");

    // A record a year ahead has no month of its own: the default partition.
    let mut far = cdr(a.id, "sip:bob@a.example.com", Some(1_000), None);
    far.start_time = SystemTime::now() + Duration::from_secs(365 * 86_400);
    far.answer_time = Some(far.start_time);
    far.end_time = Some(far.start_time + Duration::from_secs(60));
    s.insert_cdr(far.clone()).await.unwrap();
    let in_default: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cdrs_default")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(in_default, 1);

    // Take away a coming month and put a record in it: it lands in the
    // default partition, and the next ensure moves it into the month.
    let three_ahead = SystemTime::now() + Duration::from_secs(80 * 86_400);
    let month: String = sqlx::query_scalar(
        "SELECT 'cdrs_' || to_char(date_trunc('month', $1::timestamptz), 'YYYY_MM')",
    )
    .bind(chrono::DateTime::<chrono::Utc>::from(three_ahead))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(&format!("DROP TABLE {month}"))
        .execute(pool)
        .await
        .unwrap();
    let mut soon = cdr(a.id, "sip:carol@a.example.com", Some(1_000), None);
    soon.start_time = three_ahead;
    s.insert_cdr(soon.clone()).await.unwrap();
    let created = s.ensure_cdr_partitions().await.unwrap();
    assert_eq!(created, vec![month.clone()]);
    let in_month: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {month}"))
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(in_month, 1, "moved out of the default partition");
    assert!(s.get_cdr(&soon.call_id).await.unwrap().is_some());
    assert!(
        s.ensure_cdr_partitions().await.unwrap().is_empty(),
        "idempotent"
    );
}

#[tokio::test]
async fn a_keyset_walk_crosses_months_and_ties_and_sees_every_record_once() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    // Twelve records over two months, four of them at one instant.
    let now = SystemTime::now();
    let tie = now - Duration::from_secs(3_600);
    let mut ids = Vec::new();
    for i in 0..12u64 {
        let mut r = cdr(a.id, "sip:bob@a.example.com", Some(1_000), None);
        r.start_time = if i < 4 {
            tie
        } else {
            now - Duration::from_secs(i * 5 * 86_400)
        };
        r.answer_time = Some(r.start_time);
        r.end_time = Some(r.start_time + Duration::from_secs(10));
        ids.push(r.cdr_id);
        s.insert_cdr(r).await.unwrap();
    }

    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = s
            .query_cdrs(CdrQuery {
                tenant_id: Some(a.id),
                limit: Some(5),
                cursor,
                ..Default::default()
            })
            .await
            .unwrap();
        if page.is_empty() {
            break;
        }
        cursor = page.last().map(fcp_core::storage::CdrCursor::after);
        seen.extend(page);
    }
    assert_eq!(seen.len(), 12, "every record once");
    let mut unique: Vec<_> = seen.iter().map(|r| r.cdr_id).collect();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 12);
    for pair in seen.windows(2) {
        assert!(
            (pair[0].start_time, pair[0].cdr_id) > (pair[1].start_time, pair[1].cdr_id),
            "newest first, the id breaking ties"
        );
    }
    let encoded = fcp_core::storage::CdrCursor::after(&seen[3]).encode();
    assert_eq!(
        fcp_core::storage::CdrCursor::decode(&encoded),
        Some(fcp_core::storage::CdrCursor::after(&seen[3]))
    );
    let count = s
        .count_cdrs_within(
            CdrQuery {
                tenant_id: Some(a.id),
                ..Default::default()
            },
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(
        count,
        fcp_core::storage::CdrCount {
            count: 12,
            exact: true
        }
    );
}

/// The scheduled rebuild (`fcp_cdr::maintenance`, run by `fcp-admin` and
/// `fcp-jobs`) holds the maintenance lock while the rebuild takes its own:
/// were they one lock, the rebuild would wait on its own scheduler for as
/// long as the process lived, holding a transaction open (found by the
/// soak, `docs/DEEP_TESTING.md` D6).
#[tokio::test]
async fn the_scheduled_rollup_rebuild_finishes() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();
    let mut r = cdr(a.id, "sip:bob@a.example.com", Some(30_000), None);
    r.start_time = SystemTime::now() - Duration::from_secs(2 * 3_600);
    s.insert_cdr(r).await.unwrap();
    let from = SystemTime::now() - Duration::from_secs(3 * 3_600);
    let window = Duration::from_secs(48 * 3_600);
    for round in 0..2 {
        tokio::time::timeout(
            Duration::from_secs(20),
            fcp_cdr::maintenance::rebuild_rollups(s, window),
        )
        .await
        .unwrap_or_else(|_| panic!("scheduled rebuild {round} did not finish"));
    }
    let rolled = s
        .aggregate_usage(&a.id, from, SystemTime::now())
        .await
        .unwrap();
    assert_eq!(rolled.total_calls, 1, "the record was rolled up");
}

#[tokio::test]
async fn usage_from_rollups_and_the_open_edges_matches_usage_from_the_records() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    // Records over the last five hours, the current one included, one of
    // them to a number the tenant owns.
    let now = SystemTime::now();
    let number = PhoneNumber::new("+14155550100".to_string(), a.id).unwrap();
    s.put_phone_number(number).await.unwrap();
    for i in 0..10u64 {
        let to = if i % 3 == 0 {
            "sip:+14155550100@a.example.com"
        } else {
            "sip:bob@a.example.com"
        };
        let mut r = cdr(a.id, to, Some(30_000), Some(dec!(0.25)));
        r.start_time = now - Duration::from_secs(i * 1_800);
        r.answer_time = Some(r.start_time + Duration::from_secs(2));
        r.end_time = Some(r.start_time + Duration::from_secs(32));
        s.insert_cdr(r).await.unwrap();
    }
    let from = now - Duration::from_secs(4 * 3_600 + 900);
    let to = now + Duration::from_secs(60);
    let live = s.aggregate_usage(&a.id, from, to).await.unwrap();
    let live_by_number = s.aggregate_usage_by_number(&a.id, from, to).await.unwrap();

    let run = s
        .rebuild_usage_rollups(Duration::from_secs(48 * 3_600))
        .await
        .unwrap();
    assert!(run.hours >= 4 && run.rows > 0, "{run:?}");
    let rolled = s.aggregate_usage(&a.id, from, to).await.unwrap();
    assert_eq!(rolled.total_calls, live.total_calls);
    assert_eq!(rolled.answered_calls, live.answered_calls);
    assert_eq!(rolled.total_duration_ms, live.total_duration_ms);
    assert_eq!(rolled.inbound_calls, live.inbound_calls);
    assert_eq!(rolled.total_cost, live.total_cost);
    let rolled_by_number = s.aggregate_usage_by_number(&a.id, from, to).await.unwrap();
    assert_eq!(rolled_by_number.len(), live_by_number.len());
    assert_eq!(
        rolled_by_number[0].total_calls,
        live_by_number[0].total_calls
    );

    // A rebuild recomputes: running it again changes nothing.
    s.rebuild_usage_rollups(Duration::from_secs(48 * 3_600))
        .await
        .unwrap();
    let again = s.aggregate_usage(&a.id, from, to).await.unwrap();
    assert_eq!(again.total_calls, live.total_calls);
    assert_eq!(again.total_cost, live.total_cost);

    // Rollups other than call records do not count as calls.
    let mut leg = cdr(a.id, "conference:8000", Some(30_000), None);
    leg.kind = fcp_core::storage::CdrKind::Conference;
    leg.start_time = now - Duration::from_secs(2 * 3_600);
    s.insert_cdr(leg).await.unwrap();
    s.rebuild_usage_rollups(Duration::from_secs(48 * 3_600))
        .await
        .unwrap();
    assert_eq!(
        s.aggregate_usage(&a.id, from, to)
            .await
            .unwrap()
            .total_calls,
        live.total_calls
    );
}

#[tokio::test]
async fn retention_drops_a_month_only_once_its_rows_are_gone_and_locks_keep_to_one_host() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    let this_month_start: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT date_trunc('month', now())")
            .fetch_one(s.pool())
            .await
            .unwrap();
    let cutoff = SystemTime::from(this_month_start);
    // A day before the cutoff, so the record is in last month's partition
    // whatever today is (35 days back skipped September on 1 October and
    // left it empty, and droppable).
    let last_month = cutoff - Duration::from_secs(86_400);
    let mut old = cdr(a.id, "sip:bob@a.example.com", Some(1_000), None);
    old.start_time = last_month;
    s.insert_cdr(old).await.unwrap();

    assert!(
        s.drop_cdr_partitions_before(cutoff)
            .await
            .unwrap()
            .is_empty(),
        "rows still there"
    );
    assert_eq!(s.delete_cdrs_before(&a.id, cutoff).await.unwrap(), 1);
    let dropped = s.drop_cdr_partitions_before(cutoff).await.unwrap();
    assert!(!dropped.is_empty(), "last month's partition goes");

    let first = s
        .try_maintenance_lock("fcp:test")
        .await
        .unwrap()
        .expect("free");
    assert!(
        s.try_maintenance_lock("fcp:test").await.unwrap().is_none(),
        "held"
    );
    first.release().await;
    assert!(
        s.try_maintenance_lock("fcp:test").await.unwrap().is_some(),
        "released"
    );
}

/// A call's quality as a phone reported it (docs/CALL_DIAGNOSTICS.md §2):
/// one report per phone, the second replacing the first, and gone at
/// retention with the call's record, not before it.
#[tokio::test]
async fn a_calls_quality_is_one_report_per_phone_and_goes_with_its_record() {
    use fcp_core::call_quality::{CallQuality, CallQualityStore};
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();
    let carol = user(a.id, "carol");
    s.put_user(carol.clone()).await.unwrap();

    let old = SystemTime::now() - Duration::from_secs(40 * 86_400);
    let report = |call: CallId, device: &str, mos: f32| CallQuality {
        tenant_id: a.id,
        call_id: call,
        user_id: carol.id,
        device_id: Some(device.to_string()),
        direction: "outgoing".into(),
        codec: "pcmu".into(),
        srtp: true,
        duration_seconds: Some(30),
        packets_sent: 1500,
        packets_received: 1490,
        packets_expected: 1500,
        packets_lost: 15,
        loss_pct: 1.0,
        jitter_ms: 4.5,
        rtt_ms: Some(42.0),
        far_loss_pct: None,
        far_jitter_ms: None,
        far_lost: None,
        mos,
        far_mos: None,
        local_rtp: None,
        remote_rtp: Some("203.0.113.5:30000".into()),
        reported_at: old,
    };
    // A call whose record is kept, and one whose record has gone.
    let mut kept = cdr(a.id, "sip:bob@a.example.com", Some(1_000), None);
    kept.start_time = SystemTime::now();
    let kept_call = kept.call_id;
    s.insert_cdr(kept).await.unwrap();
    let gone_call = CallId::new();

    s.put_call_quality(&report(kept_call, "phone", 4.3))
        .await
        .unwrap();
    s.put_call_quality(&report(kept_call, "phone", 2.9))
        .await
        .unwrap();
    s.put_call_quality(&report(kept_call, "laptop", 4.1))
        .await
        .unwrap();
    s.put_call_quality(&report(gone_call, "phone", 4.4))
        .await
        .unwrap();
    let reports = s.call_quality(&a.id, &kept_call).await.unwrap();
    assert_eq!(reports.len(), 2, "one per phone: {reports:?}");
    let phone = reports
        .iter()
        .find(|r| r.device_id.as_deref() == Some("phone"))
        .unwrap();
    assert_eq!(phone.mos, 2.9, "the second replaced the first");
    assert!(phone.is_poor());
    assert_eq!(
        s.call_qualities_of_user(&carol.id, &[kept_call, gone_call])
            .await
            .unwrap()
            .len(),
        3
    );

    s.delete_cdrs_before(&a.id, SystemTime::now() - Duration::from_secs(86_400))
        .await
        .unwrap();
    assert_eq!(s.call_quality(&a.id, &kept_call).await.unwrap().len(), 2);
    assert!(s.call_quality(&a.id, &gone_call).await.unwrap().is_empty());
}

/// Captures from apps (docs/CALL_DIAGNOSTICS.md §3): an allow by user or
/// device, expired captures listed for the sweep, and every removed
/// capture's file queued — a purged user's too.
#[tokio::test]
async fn a_captures_file_is_queued_however_its_row_goes() {
    use fcp_core::device_capture::{
        allowed_until, AllowSubject, CaptureAllow, DeviceCapture, DeviceCaptureStore,
    };
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();
    let carol = user(a.id, "carol");
    s.put_user(carol.clone()).await.unwrap();
    let device = fcp_core::Device::new("App".into(), fcp_core::device::DeviceType::Softphone, a.id);
    s.put_device(device.clone()).await.unwrap();

    let now = SystemTime::now();
    let day = Duration::from_secs(86_400);
    for (subject, until) in [
        (AllowSubject::User { user_id: carol.id }, now + day),
        (
            AllowSubject::Device {
                device_id: device.id,
            },
            now + 3 * day,
        ),
    ] {
        s.put_capture_allow(&CaptureAllow {
            id: uuid::Uuid::new_v4(),
            tenant_id: a.id,
            subject,
            until,
            reason: None,
            created_by: "admin".into(),
            created_at: now,
        })
        .await
        .unwrap();
    }
    let allows = s
        .capture_allows_for(&a.id, &carol.id, &device.id)
        .await
        .unwrap();
    assert_eq!(allows.len(), 2);
    let until = allowed_until(&allows, &carol.id, &device.id, now).unwrap();
    assert!(until > now + 2 * day, "the device's, the later");

    let capture = |uploaded_at: SystemTime, expires_at: SystemTime| {
        let id = uuid::Uuid::new_v4();
        DeviceCapture {
            id,
            tenant_id: a.id,
            device_id: device.id,
            user_id: carol.id,
            call_id: Some(CallId::new()),
            size: 12,
            sha256: "ab".repeat(32),
            file_key: DeviceCapture::key_for(&a.id, &id),
            subject_keys: vec!["user:carol".into()],
            uploaded_at,
            expires_at,
        }
    };
    let (old, new) = (capture(now - 2 * day, now - day), capture(now, now + day));
    s.put_device_capture(&old).await.unwrap();
    s.put_device_capture(&new).await.unwrap();
    let ids = |list: Vec<DeviceCapture>| list.into_iter().map(|c| c.id).collect::<Vec<_>>();
    let listed = s
        .list_device_captures(&a.id, Some(&device.id), None)
        .await
        .unwrap();
    assert_eq!(listed[0].file_key, new.file_key);
    assert_eq!(listed[0].subject_keys, new.subject_keys);
    assert_eq!(ids(listed), vec![new.id, old.id], "newest first");
    let expired = s.expired_device_captures(now, 10).await.unwrap();
    assert_eq!(ids(expired), vec![old.id]);

    // Removed by hand: the file is queued.
    assert!(s
        .delete_device_capture(&a.id, &old.id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        s.due_capture_files(10).await.unwrap(),
        vec![old.file_key.clone()]
    );
    s.capture_file_failed(&old.file_key, "unreachable", day)
        .await
        .unwrap();
    assert!(s.due_capture_files(10).await.unwrap().is_empty(), "later");

    // The user purged: their capture and allow go, the file is queued.
    s.delete_user_in_tenant(&carol.username, &a.id)
        .await
        .unwrap();
    assert!(s
        .list_device_captures(&a.id, None, None)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        s.capture_allows_for(&a.id, &carol.id, &device.id)
            .await
            .unwrap()
            .len(),
        1,
        "the device's allow is the device's"
    );
    let queued: Vec<String> =
        sqlx::query_scalar("SELECT file_key FROM device_capture_file_deletions ORDER BY file_key")
            .fetch_all(s.pool())
            .await
            .unwrap();
    let mut expected = vec![old.file_key.clone(), new.file_key.clone()];
    expected.sort();
    assert_eq!(queued, expected);
    s.capture_file_removed(&new.file_key).await.unwrap();
}

#[tokio::test]
async fn the_archive_walks_days_notes_late_records_and_holds_each_tenants_frontier() {
    use chrono::{Duration as Days, NaiveDate, Utc};
    use fcp_core::cdr_archive::{day_start, CdrArchiveWrite};
    use fcp_core::compliance::{cdr_retention_cutoff, CompliancePolicy, ComplianceStore};
    use fcp_core::storage::CdrCursor;

    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    let b = tenant("b");
    s.put_tenant(a.clone()).await.unwrap();
    s.put_tenant(b.clone()).await.unwrap();
    let today = Utc::now().date_naive();
    let (d0, d1, d2) = (
        today - Days::days(5),
        today - Days::days(4),
        today - Days::days(3),
    );
    let at = |day: NaiveDate, secs: u64| day_start(day) + Duration::from_secs(secs);
    let record = |tenant: TenantId, start: SystemTime| {
        let mut r = cdr(tenant, "sip:bob@example.com", Some(1_000), None);
        r.start_time = start;
        r.answer_time = Some(start);
        r.end_time = Some(start + Duration::from_secs(1));
        r
    };
    let frontier = |t: TenantId| async move { s.cdr_archive_frontier(&t).await.unwrap() };
    let write =
        |tenant: TenantId, day: NaiveDate, generation: u64, key: Option<&str>| CdrArchiveWrite {
            tenant_id: tenant,
            day,
            generation,
            object_key: key.map(str::to_string),
            records: 2,
            bytes: 100,
            sha256: key.map(|_| "ab".repeat(32)),
        };

    // Nothing to walk and nothing confirmed before there are records.
    assert_eq!(s.cdr_archive_next_day().await.unwrap(), None);
    assert_eq!(frontier(a.id).await, None);

    // a: midnight and mid-morning on d0, the last second of d1; b: d1.
    s.insert_cdrs(vec![
        record(a.id, at(d0, 36_000)),
        record(a.id, at(d0, 0)),
        record(a.id, at(d1, 86_399)),
        record(b.id, at(d1, 3_600)),
    ])
    .await
    .unwrap();
    assert_eq!(
        s.cdr_archive_next_day().await.unwrap(),
        Some(d0),
        "the oldest record's day"
    );
    assert_eq!(s.cdr_tenants_on(d0).await.unwrap(), vec![a.id]);
    let mut on_d1 = s.cdr_tenants_on(d1).await.unwrap();
    on_d1.sort_by_key(|t| t.to_string());
    let mut both = vec![a.id, b.id];
    both.sort_by_key(|t| t.to_string());
    assert_eq!(on_d1, both);
    assert!(s.cdr_tenants_on(d2).await.unwrap().is_empty());

    // A day's page is oldest first, and the day is [midnight, midnight).
    let first = s.cdr_archive_page(&a.id, d0, None, 1).await.unwrap();
    assert_eq!(first[0].start_time, at(d0, 0));
    let second = s
        .cdr_archive_page(&a.id, d0, first.last().map(CdrCursor::after), 10)
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].start_time, at(d0, 36_000));
    assert!(s
        .cdr_archive_page(&a.id, d0, second.last().map(CdrCursor::after), 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        s.cdr_archive_page(&a.id, d1, None, 10).await.unwrap().len(),
        1
    );
    assert!(s
        .cdr_archive_page(&a.id, d2, None, 10)
        .await
        .unwrap()
        .is_empty());

    // Walk d0 and d1.
    let g = s.begin_cdr_archive(&a.id, d0).await.unwrap();
    assert_eq!(g, 1);
    assert_eq!(
        frontier(a.id).await,
        Some(day_start(d0)),
        "not walked past d0 yet"
    );
    assert!(s
        .complete_cdr_archive(&write(a.id, d0, g, Some("a/d0")))
        .await
        .unwrap());
    s.advance_cdr_archive(d0).await.unwrap();
    assert!(
        s.advance_cdr_archive(d0).await.is_err(),
        "the walk is past d0"
    );
    for t in [a.id, b.id] {
        let g = s.begin_cdr_archive(&t, d1).await.unwrap();
        assert!(s
            .complete_cdr_archive(&write(t, d1, g, Some("d1")))
            .await
            .unwrap());
    }
    s.advance_cdr_archive(d1).await.unwrap();
    assert_eq!(frontier(a.id).await, Some(day_start(d2)));
    assert_eq!(frontier(b.id).await, Some(day_start(d2)));
    let entry = s.get_cdr_archive(&a.id, d0).await.unwrap().unwrap();
    assert!(entry.archived_at.is_some() && !entry.late);
    assert_eq!(entry.object_key.as_deref(), Some("a/d0"));

    // A late record for an archived day, and one for a day a tenant had
    // no records on: both are pending and hold only their own tenant back.
    s.insert_cdr(record(a.id, at(d0, 50_000))).await.unwrap();
    assert!(s.get_cdr_archive(&a.id, d0).await.unwrap().unwrap().late);
    assert_eq!(frontier(a.id).await, Some(day_start(d0)));
    assert_eq!(frontier(b.id).await, Some(day_start(d2)));
    s.insert_cdr(record(b.id, at(d0, 60_000))).await.unwrap();
    assert_eq!(frontier(b.id).await, Some(day_start(d0)));
    let mut pending = s.pending_cdr_archives(d2, 10).await.unwrap();
    pending.sort_by_key(|(t, d)| (*d, t.to_string()));
    let mut expected = vec![(a.id, d0), (b.id, d0)];
    expected.sort_by_key(|(t, d)| (*d, t.to_string()));
    assert_eq!(pending, expected);
    // A record for a day the walk has not reached is not late.
    s.insert_cdr(record(a.id, at(today, 0))).await.unwrap();
    assert_eq!(s.pending_cdr_archives(today, 10).await.unwrap().len(), 2);

    // Writing a again: unconfirmed while under way; a stale generation's
    // confirmation is ignored.
    let g2 = s.begin_cdr_archive(&a.id, d0).await.unwrap();
    assert_eq!(g2, 2);
    let entry = s.get_cdr_archive(&a.id, d0).await.unwrap().unwrap();
    assert!(entry.archived_at.is_none() && !entry.late);
    assert_eq!(frontier(a.id).await, Some(day_start(d0)));
    assert!(!s
        .complete_cdr_archive(&write(a.id, d0, 1, Some("old")))
        .await
        .unwrap());
    assert!(s
        .complete_cdr_archive(&write(a.id, d0, g2, Some("a/d0")))
        .await
        .unwrap());
    assert_eq!(frontier(a.id).await, Some(day_start(d2)));

    // A record stored while b's write is under way keeps the day pending.
    let g = s.begin_cdr_archive(&b.id, d0).await.unwrap();
    s.insert_cdr(record(b.id, at(d0, 70_000))).await.unwrap();
    assert!(s
        .complete_cdr_archive(&write(b.id, d0, g, Some("b/d0")))
        .await
        .unwrap());
    let entry = s.get_cdr_archive(&b.id, d0).await.unwrap().unwrap();
    assert!(entry.archived_at.is_some() && entry.late);
    assert_eq!(frontier(b.id).await, Some(day_start(d0)));
    assert_eq!(
        s.pending_cdr_archives(d2, 10).await.unwrap(),
        vec![(b.id, d0)]
    );

    // A write that finds no records keeps the object an earlier one stored.
    let g3 = s.begin_cdr_archive(&a.id, d0).await.unwrap();
    assert!(s
        .complete_cdr_archive(&write(a.id, d0, g3, None))
        .await
        .unwrap());
    let entry = s.get_cdr_archive(&a.id, d0).await.unwrap().unwrap();
    assert_eq!(entry.object_key.as_deref(), Some("a/d0"));
    assert!(entry.archived_at.is_some());

    let status = s.cdr_archive_status().await.unwrap();
    assert_eq!(status.next_day, Some(d2));
    assert_eq!(status.archived_days, 4);
    assert_eq!(status.pending_days, 1);
    assert!(status.last_archived_at.is_some());

    // Retention for a tenant that requires the archive stops at its
    // frontier; the policy flag round-trips.
    let compliance =
        fcp_storage_postgres::compliance::PostgresComplianceStore::new(s.pool().clone());
    let mut policy = CompliancePolicy::defaults(b.id);
    policy.cdr_retention_days = Some(1);
    policy.cdr_archive_required = true;
    compliance.put_compliance_policy(&policy).await.unwrap();
    let policy = compliance
        .get_compliance_policy(&b.id)
        .await
        .unwrap()
        .unwrap();
    assert!(policy.cdr_archive_required);
    let (cutoff, held) = cdr_retention_cutoff(s, &policy, SystemTime::now())
        .await
        .unwrap();
    assert_eq!(cutoff, Some(day_start(d0)));
    assert!(held.is_some());

    // The walk does not pass a day while a record for it is being
    // committed: moving past d2 waits for the insert, which notes itself.
    let mut tx = s.pool().begin().await.unwrap();
    sqlx::query(
        "CREATE TEMP TABLE held ON COMMIT DROP AS SELECT * FROM cdrs WHERE tenant_id = $1 LIMIT 1",
    )
    .bind(b.id.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("UPDATE held SET cdr_id = gen_random_uuid(), start_time = $1")
        .bind(chrono::DateTime::<Utc>::from(at(d2, 10)))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cdrs SELECT * FROM held")
        .execute(&mut *tx)
        .await
        .unwrap();
    let advance = s.advance_cdr_archive(d2);
    tokio::pin!(advance);
    assert!(
        tokio::time::timeout(Duration::from_millis(500), &mut advance)
            .await
            .is_err(),
        "waits for the insert"
    );
    tx.commit().await.unwrap();
    advance.await.unwrap();
    assert!(s
        .pending_cdr_archives(d2 + Days::days(1), 10)
        .await
        .unwrap()
        .contains(&(b.id, d2)));
}

#[tokio::test]
async fn the_pipeline_migration_gives_existing_records_an_id_a_kind_and_a_trunk() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    // Rows as the MVP wrote them: no pipeline columns filled.
    let pool = s.pool();
    for (call_id, to_uri, gateway) in [
        (CallId::new(), "sip:bob@a.example.com", Some("trunk-x")),
        (CallId::new(), "queue:support", None),
    ] {
        sqlx::query(
            "INSERT INTO cdrs (call_id, tenant_id, sip_call_id, from_uri, to_uri, start_time,
                               disposition, hangup_cause, media_anchored, gateway_id)
             VALUES ($1, $2, 'x@test', 'sip:alice@a.example.com', $3, now(), 'answered',
                     'normal_clearing', true, $4)",
        )
        .bind(call_id.to_string())
        .bind(a.id.to_string())
        .bind(to_uri)
        .bind(gateway)
        .execute(pool)
        .await
        .unwrap();
    }
    // Replay the backfill statements of 057 on those rows.
    sqlx::query("UPDATE cdrs SET cdr_id = md5(call_id)::uuid")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE cdrs SET kind = 'queue' WHERE kind = 'call' AND to_uri LIKE 'queue:%'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE cdrs SET trunk_name = gateway_id WHERE trunk_name IS NULL AND gateway_id IS NOT NULL")
        .execute(pool)
        .await
        .unwrap();

    let rows = s
        .query_cdrs(CdrQuery {
            tenant_id: Some(a.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "both rows still decode: {rows:#?}");
    let queue = rows.iter().find(|r| r.to_uri == "queue:support").unwrap();
    assert_eq!(queue.kind, fcp_core::storage::CdrKind::Queue);
    let call = rows.iter().find(|r| r.to_uri != "queue:support").unwrap();
    assert_eq!(call.kind, fcp_core::storage::CdrKind::Call);
    assert_eq!(call.trunk_name.as_deref(), Some("trunk-x"));
    assert!(!call.cdr_id.is_nil());
    assert_ne!(call.cdr_id, queue.cdr_id);
}

// ─── Rate plans ──────────────────────────────────────────────────────────

#[tokio::test]
async fn rate_plans_default_flag_uniqueness_and_rates() {
    let db = require_db!();
    let s = db.storage();

    let a = tenant("a");
    let b = tenant("b");
    s.put_tenant(a.clone()).await.unwrap();
    s.put_tenant(b.clone()).await.unwrap();

    let first = rate_plan(a.id, "standard", true);
    s.create_rate_plan(first.clone()).await.unwrap();

    // Duplicate name within the tenant is a Conflict.
    let dup = rate_plan(a.id, "standard", false);
    let err = s.create_rate_plan(dup).await.unwrap_err();
    assert!(is_conflict(&err), "expected Conflict, got {err:#}");

    // Same name in another tenant is fine, and invisible to tenant a.
    s.create_rate_plan(rate_plan(b.id, "standard", true))
        .await
        .unwrap();
    assert_eq!(s.list_rate_plans(&a.id).await.unwrap().len(), 1);
    assert!(s.get_rate_plan(&a.id, first.id).await.unwrap().is_some());
    assert!(s.get_rate_plan(&b.id, first.id).await.unwrap().is_none());

    // Creating a second default demotes the first.
    let second = rate_plan(a.id, "premium", true);
    s.create_rate_plan(second.clone()).await.unwrap();
    let plans = s.list_rate_plans(&a.id).await.unwrap();
    let defaults: Vec<_> = plans
        .iter()
        .filter(|p| p.is_default)
        .map(|p| p.id)
        .collect();
    assert_eq!(defaults, vec![second.id]);

    // Rates: upsert on prefix replaces, longest-prefix data round-trips.
    s.upsert_rate(&a.id, rate(second.id, "44", dec!(0.03)))
        .await
        .unwrap();
    s.upsert_rate(&a.id, rate(second.id, "4420", dec!(0.01)))
        .await
        .unwrap();
    let mut replaced = rate(second.id, "44", dec!(0.05));
    replaced.description = Some("UK".to_string());
    s.upsert_rate(&a.id, replaced).await.unwrap();
    let rates = s.list_rates(&a.id, second.id).await.unwrap();
    assert_eq!(rates.len(), 2);
    let uk = rates.iter().find(|r| r.prefix == "44").unwrap();
    assert!(approx(uk.rate_per_minute, dec!(0.05)));
    assert_eq!(uk.description.as_deref(), Some("UK"));

    // Rates on a plan the tenant does not own are rejected.
    let err = s
        .upsert_rate(&b.id, rate(second.id, "1", dec!(0.01)))
        .await
        .unwrap_err();
    assert!(is_not_found(&err), "expected NotFound, got {err:#}");

    // The rating path lookup returns the enabled default with its rates.
    let (plan, plan_rates) = s
        .get_default_rate_plan_with_rates(&a.id)
        .await
        .unwrap()
        .expect("default plan");
    assert_eq!(plan.id, second.id);
    assert_eq!(plan_rates.len(), 2);

    // Disabling the default hides it from the rating path.
    let mut disabled = plan.clone();
    disabled.enabled = false;
    s.update_rate_plan(disabled).await.unwrap();
    assert!(s
        .get_default_rate_plan_with_rates(&a.id)
        .await
        .unwrap()
        .is_none());

    // Deleting one rate, then the plan cascades the rest.
    let rate_id = rates.iter().find(|r| r.prefix == "4420").unwrap().id;
    s.delete_rate(&a.id, second.id, rate_id).await.unwrap();
    assert_eq!(s.list_rates(&a.id, second.id).await.unwrap().len(), 1);
    s.delete_rate_plan(&a.id, second.id).await.unwrap();
    assert!(s.get_rate_plan(&a.id, second.id).await.unwrap().is_none());
    assert!(s.list_rates(&a.id, second.id).await.unwrap().is_empty());

    // Deleting a plan that is not the tenant's is NotFound.
    let err = s.delete_rate_plan(&a.id, Uuid::new_v4()).await.unwrap_err();
    assert!(is_not_found(&err), "expected NotFound, got {err:#}");

    db.finish().await;
}

// ─── Balances ────────────────────────────────────────────────────────────

#[tokio::test]
async fn balances_settings_ledger_and_single_settlement() {
    let db = require_db!();
    let s = db.storage();

    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    assert!(s.get_balance(&a.id).await.unwrap().is_none());

    // First transaction creates the balance row (enforcement off).
    let topup = s
        .apply_balance_transaction(BalanceTransactionRequest {
            tenant_id: a.id,
            kind: BalanceTransactionKind::TopUp,
            amount: dec!(10),
            currency: "USD".to_string(),
            call_id: None,
            description: Some("card".to_string()),
            external_ref: Some("pay_1".to_string()),
        })
        .await
        .unwrap();
    assert!(approx(topup.balance_after, dec!(10.0)));
    let row = s
        .get_balance(&a.id)
        .await
        .unwrap()
        .expect("balance row created");
    assert!(approx(row.balance, dec!(10.0)));
    assert!(!row.enforce);
    assert_eq!(row.currency, "USD");

    // Upserting settings keeps the amount.
    let mut settings = balance(a.id, true);
    settings.credit_limit = dec!(2.5);
    settings.low_balance_threshold = Some(dec!(1));
    s.upsert_balance(settings).await.unwrap();
    let row = s.get_balance(&a.id).await.unwrap().unwrap();
    assert!(
        approx(row.balance, dec!(10.0)),
        "amount preserved: {}",
        row.balance
    );
    assert!(row.enforce);
    assert!(approx(row.credit_limit, dec!(2.5)));
    assert_eq!(row.low_balance_threshold, Some(dec!(1)));
    assert!(approx(row.available(), dec!(12.5)));

    // A call charge debits; a second charge for the same call is a Conflict
    // and must roll back so the balance is untouched.
    let call_id = CallId::new();
    let debit = s
        .apply_balance_transaction(charge(a.id, call_id, dec!(0.06)))
        .await
        .unwrap();
    assert!(approx(debit.balance_after, dec!(9.94)));
    assert_eq!(debit.call_id, Some(call_id));
    assert_eq!(debit.kind, BalanceTransactionKind::CallCharge);

    let err = s
        .apply_balance_transaction(charge(a.id, call_id, dec!(0.06)))
        .await
        .unwrap_err();
    assert!(is_conflict(&err), "expected Conflict, got {err:#}");
    assert!(approx(
        s.get_balance(&a.id).await.unwrap().unwrap().balance,
        dec!(9.94)
    ));

    // A different call charges normally; adjustments can go either way.
    s.apply_balance_transaction(charge(a.id, CallId::new(), dec!(0.04)))
        .await
        .unwrap();
    let adj = s
        .apply_balance_transaction(BalanceTransactionRequest {
            tenant_id: a.id,
            kind: BalanceTransactionKind::Adjustment,
            amount: dec!(-5),
            currency: "USD".to_string(),
            call_id: None,
            description: Some("write-off".to_string()),
            external_ref: None,
        })
        .await
        .unwrap();
    assert!(approx(adj.balance_after, dec!(4.90)));

    // Ledger: newest first, paginated, tenant-scoped.
    let ledger = s.list_balance_transactions(&a.id, 10, 0).await.unwrap();
    assert_eq!(ledger.len(), 4);
    assert_eq!(ledger[0].kind, BalanceTransactionKind::Adjustment);
    assert_eq!(ledger[3].kind, BalanceTransactionKind::TopUp);
    assert_eq!(ledger[3].external_ref.as_deref(), Some("pay_1"));
    let page = s.list_balance_transactions(&a.id, 2, 1).await.unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].id, ledger[1].id);

    let other = TenantId::new();
    assert!(s
        .list_balance_transactions(&other, 10, 0)
        .await
        .unwrap()
        .is_empty());
    assert!(s.get_balance(&other).await.unwrap().is_none());

    db.finish().await;
}

#[tokio::test]
async fn balance_debits_serialize_under_concurrency() {
    let db = require_db!();
    let s = db.storage().clone();

    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();
    s.upsert_balance(balance(a.id, true)).await.unwrap();

    // 25 concurrent settlements of 0.01 each must land exactly once each.
    let mut handles = Vec::new();
    for _ in 0..25 {
        let s = s.clone();
        let tenant_id = a.id;
        handles.push(tokio::spawn(async move {
            s.apply_balance_transaction(charge(tenant_id, CallId::new(), dec!(0.01)))
                .await
                .unwrap()
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let row = s.get_balance(&a.id).await.unwrap().unwrap();
    assert!(
        approx(row.balance, dec!(-0.25)),
        "balance = {}",
        row.balance
    );
    assert_eq!(
        s.list_balance_transactions(&a.id, 100, 0)
            .await
            .unwrap()
            .len(),
        25
    );

    db.finish().await;
}

// ─── Encryption at rest ──────────────────────────────────────────────────

fn master_key(byte: u8) -> EncryptionSettings {
    use base64::Engine;
    EncryptionSettings::local_from_base64(
        &base64::engine::general_purpose::STANDARD.encode([byte; 32]),
    )
    .unwrap()
}

fn trunk(tenant_id: TenantId, name: &str, password: &str) -> fcp_core::Trunk {
    let mut t = fcp_core::Trunk::new(name.to_string(), tenant_id);
    t.hosts = vec!["10.0.0.1".to_string()];
    t.credentials = Some(fcp_core::trunk::TrunkCredentials {
        username: "trunkuser".to_string(),
        password: password.to_string(),
        realm: None,
    });
    t
}

/// Built through serde: the struct has many defaulted presentation fields.
fn meeting(tenant_id: TenantId, room: &str, pin: &str) -> fcp_core::Meeting {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let ts = serde_json::json!({ "secs_since_epoch": now, "nanos_since_epoch": 0 });
    serde_json::from_value(serde_json::json!({
        "id": fcp_core::MeetingId::new(),
        "tenant_id": tenant_id,
        "room_number": room,
        "name": format!("Room {room}"),
        "guest_pin": pin,
        "host_pin": format!("{pin}9"),
        "require_guest_pin": true,
        "max_participants": 10,
        "wait_for_moderator": false,
        "enabled": true,
        "auto_record": false,
        "default_locked": false,
        "permanent": true,
        "play_entry_tone": true,
        "play_exit_tone": true,
        "created_at": ts,
        "updated_at": ts,
    }))
    .expect("meeting fixture deserializes")
}

fn webhook(tenant_id: TenantId, secret: &str) -> fcp_core::storage::WebhookEndpoint {
    let now = SystemTime::now();
    fcp_core::storage::WebhookEndpoint {
        id: fcp_core::WebhookId::new(),
        tenant_id,
        url: "https://hooks.example.com/fcp".to_string(),
        secret: secret.to_string(),
        events: vec!["call.*".to_string()],
        enabled: true,
        description: None,
        created_at: now,
        updated_at: now,
        previous_secret: None,
        previous_secret_expires_at: None,
    }
}

/// Where a trunk's carrier sends from reads back through every trunk
/// SELECT (`docs/TENANT_TRUNKS.md` §7) — a column one of them forgets would
/// read as "the trunk's hosts" and admit the wrong carrier.
#[tokio::test]
async fn a_trunks_inbound_sources_round_trip() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    let mut t = trunk(a.id, "carrier", "pw");
    t.inbound_sources = vec!["203.0.113.0/24".to_string(), "198.51.100.7".to_string()];
    let id = t.id.clone();
    s.put_trunk(t.clone()).await.unwrap();

    let by_id = s.get_trunk(&id).await.unwrap().unwrap();
    assert_eq!(by_id.inbound_sources, t.inbound_sources);
    assert_eq!(by_id.direction, fcp_core::TrunkDirection::Bidirectional);
    let listed = s.list_trunks(&a.id).await.unwrap();
    assert_eq!(listed[0].inbound_sources, t.inbound_sources);
    let all = s.list_all_trunks().await.unwrap();
    let mine = all.iter().find(|x| x.id == id).unwrap();
    assert_eq!(mine.inbound_sources, t.inbound_sources);
}

#[tokio::test]
async fn meetings_round_trip_video_settings() {
    let db = require_db!();
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    // Defaults: a meeting built without video fields reads back audio-only.
    let m = meeting(a.id, "2000", "1111");
    let plain_id = m.id;
    s.put_meeting(m).await.unwrap();
    let back = s.get_meeting(&plain_id).await.unwrap().unwrap();
    assert!(!back.video_enabled);
    assert_eq!(back.video_layout, "grid");
    assert_eq!(back.video_max_participants, 16);
    assert_eq!(back.video_max_resolution, "720p");
    assert_eq!(back.video_fps, 15);
    assert!(!back.video_exclude_self);
    assert_eq!(
        back.video_codecs,
        vec![fcp_core::VideoCodec::H264, fcp_core::VideoCodec::Vp8]
    );
    assert_eq!(back.video_mode, "mix");
    assert_eq!(back.kind, "meeting");
    assert!(back.panelist_pin.is_none() && back.panelists.is_empty());

    // Non-default values survive the upsert and every read path.
    let mut m = meeting(a.id, "2001", "2222");
    m.video_enabled = true;
    m.video_layout = "spotlight".to_string();
    m.video_max_participants = 4;
    m.video_max_resolution = "1080p".to_string();
    m.video_fps = 30;
    m.video_exclude_self = true;
    m.video_mode = "auto".to_string();
    m.kind = "webinar".to_string();
    m.panelist_pin = Some("5555".to_string());
    m.panelists = vec!["user-1".to_string(), "user-2".to_string()];
    m.video_codecs = vec![
        fcp_core::VideoCodec::Av1,
        fcp_core::VideoCodec::Vp9,
        fcp_core::VideoCodec::H264,
    ];
    let video_id = m.id;
    s.put_meeting(m.clone()).await.unwrap();
    // Second put exercises the ON CONFLICT branch.
    m.video_fps = 24;
    s.put_meeting(m).await.unwrap();

    let check = |back: &fcp_core::Meeting| {
        assert!(back.video_enabled);
        assert_eq!(back.video_layout, "spotlight");
        assert_eq!(back.video_max_participants, 4);
        assert_eq!(back.video_max_resolution, "1080p");
        assert_eq!(back.video_fps, 24);
        assert!(back.video_exclude_self);
        assert_eq!(back.video_mode, "auto");
        assert_eq!(back.kind, "webinar");
        assert_eq!(back.panelist_pin.as_deref(), Some("5555"));
        assert_eq!(back.panelists, vec!["user-1", "user-2"]);
        assert_eq!(
            back.video_codecs,
            vec![
                fcp_core::VideoCodec::Av1,
                fcp_core::VideoCodec::Vp9,
                fcp_core::VideoCodec::H264,
            ]
        );
    };
    check(&s.get_meeting(&video_id).await.unwrap().unwrap());
    check(
        &s.get_meeting_by_room_number(&a.id, "2001")
            .await
            .unwrap()
            .unwrap(),
    );
    let listed = s.list_meetings(&a.id).await.unwrap();
    assert_eq!(listed.len(), 2);
    check(listed.iter().find(|m| m.id == video_id).unwrap());

    // The column holds SDP names in preference order.
    let raw = db
        .raw(
            "SELECT video_codecs FROM conference_meetings WHERE id = $1",
            &video_id.to_string(),
        )
        .await
        .unwrap();
    assert_eq!(raw, "AV1,VP9,H264");

    db.finish().await;
}

#[tokio::test]
async fn encryption_seals_secret_columns_and_reads_them_back() {
    let Some(db) = TestDb::with_encryption(master_key(1)).await else {
        return;
    };
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    let status = s.encryption_status().await.unwrap();
    assert!(status.enabled);
    assert_eq!(status.provider, "local");
    assert!(status.kek_id.as_deref().unwrap().starts_with("local:"));
    assert_eq!(status.key_ids.len(), 1);
    assert_eq!(status.key_ids[0], status.active_key_id.clone().unwrap());
    assert!(status
        .sealed_columns
        .iter()
        .any(|c| c == "users.digest_password"));

    // User secrets.
    let mut u = user(a.id, "alice");
    u.digest_password = Some("sip-pass".to_string());
    u.sip_ha1_hash = Some("d41d8cd98f00b204e9800998ecf8427e".to_string());
    u.sip_provisioning_password = Some("prov-pass".to_string());
    s.put_user(u).await.unwrap();
    let back = s.get_user_in_tenant("alice", &a.id).await.unwrap().unwrap();
    assert_eq!(back.digest_password.as_deref(), Some("sip-pass"));
    assert_eq!(
        back.sip_ha1_hash.as_deref(),
        Some("d41d8cd98f00b204e9800998ecf8427e")
    );
    assert_eq!(back.sip_provisioning_password.as_deref(), Some("prov-pass"));
    let raw = db
        .raw(
            "SELECT digest_password FROM users WHERE id = $1",
            &back.id.to_string(),
        )
        .await
        .unwrap();
    assert!(raw.starts_with("enc:v1:"), "stored value is sealed: {raw}");
    assert!(!raw.contains("sip-pass"));

    // Trunk password.
    let t = trunk(a.id, "carrier", "trunk-pass");
    let trunk_id = t.id.clone();
    s.put_trunk(t).await.unwrap();
    let back = s.get_trunk(&trunk_id).await.unwrap().unwrap();
    assert_eq!(
        back.credentials.as_ref().map(|c| c.password.as_str()),
        Some("trunk-pass")
    );
    let raw = db
        .raw(
            "SELECT auth_password FROM trunks WHERE id = $1",
            &trunk_id.to_string(),
        )
        .await
        .unwrap();
    assert!(raw.starts_with("enc:v1:"));
    assert_eq!(
        s.list_trunks(&a.id).await.unwrap()[0]
            .credentials
            .as_ref()
            .map(|c| c.password.as_str()),
        Some("trunk-pass")
    );

    // Conference PINs, a webinar's panelist PIN among them.
    let mut m = meeting(a.id, "1000", "1234");
    m.kind = "webinar".to_string();
    m.panelist_pin = Some("5555".to_string());
    let meeting_id = m.id;
    s.put_meeting(m).await.unwrap();
    let back = s.get_meeting(&meeting_id).await.unwrap().unwrap();
    assert_eq!(back.guest_pin.as_deref(), Some("1234"));
    assert_eq!(back.host_pin.as_deref(), Some("12349"));
    assert_eq!(back.panelist_pin.as_deref(), Some("5555"));
    for column in ["guest_pin", "panelist_pin"] {
        let raw = db
            .raw(
                &format!("SELECT {column} FROM conference_meetings WHERE id = $1"),
                &meeting_id.to_string(),
            )
            .await
            .unwrap();
        assert!(raw.starts_with("enc:v1:"), "{column} sealed at rest");
    }

    // Webhook secret inside JSONB.
    let w = webhook(a.id, "whsec_123");
    let webhook_id = w.id;
    s.create_endpoint(w).await.unwrap();
    let back = s.get_endpoint(&webhook_id).await.unwrap().unwrap();
    assert_eq!(back.secret, "whsec_123");
    let raw = db
        .raw(
            "SELECT endpoint_json->>'secret' FROM webhook_endpoints WHERE id = $1",
            &webhook_id.to_string(),
        )
        .await
        .unwrap();
    assert!(raw.starts_with("enc:v1:"));
    assert_eq!(
        s.list_endpoints(&a.id).await.unwrap()[0].secret,
        "whsec_123"
    );

    // A rotation's previous secret is sealed beside the current one.
    let mut rotated = back.clone();
    rotated.secret = "whsec_456".to_string();
    rotated.previous_secret = Some("whsec_123".to_string());
    rotated.previous_secret_expires_at = Some(SystemTime::now() + Duration::from_secs(3600));
    s.update_endpoint(rotated).await.unwrap();
    let back = s.get_endpoint(&webhook_id).await.unwrap().unwrap();
    assert_eq!(back.secret, "whsec_456");
    assert_eq!(back.previous_secret.as_deref(), Some("whsec_123"));
    assert_eq!(
        back.signing_secrets(SystemTime::now()),
        vec!["whsec_456", "whsec_123"]
    );
    let raw = db
        .raw(
            "SELECT endpoint_json->>'previous_secret' FROM webhook_endpoints WHERE id = $1",
            &webhook_id.to_string(),
        )
        .await
        .unwrap();
    assert!(raw.starts_with("enc:v1:"), "{raw}");

    db.finish().await;
}

#[tokio::test]
async fn encryption_reads_legacy_plaintext_and_reencrypt_seals_it() {
    let Some(db) = TestDb::with_encryption(master_key(1)).await else {
        return;
    };
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    // A row written before encryption was enabled (plaintext on disk).
    let mut u = user(a.id, "legacy");
    u.digest_password = Some("old-pass".to_string());
    s.put_user(u).await.unwrap();
    let uid = s
        .get_user_in_tenant("legacy", &a.id)
        .await
        .unwrap()
        .unwrap()
        .id
        .to_string();
    sqlx::query("UPDATE users SET digest_password = 'old-pass', sip_ha1_hash = NULL WHERE id = $1")
        .bind(&uid)
        .execute(s.pool())
        .await
        .unwrap();
    let raw = db
        .raw("SELECT digest_password FROM users WHERE id = $1", &uid)
        .await
        .unwrap();
    assert_eq!(raw, "old-pass");

    // Reads pass plaintext through.
    let back = s
        .get_user_in_tenant("legacy", &a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back.digest_password.as_deref(), Some("old-pass"));

    // Bulk re-encrypt seals it and leaves already-sealed rows alone.
    let report = s.reencrypt_secrets().await.unwrap();
    assert_eq!(report.rewritten, 1, "{report:?}");
    let raw = db
        .raw("SELECT digest_password FROM users WHERE id = $1", &uid)
        .await
        .unwrap();
    assert!(raw.starts_with("enc:v1:"));
    assert_eq!(
        s.get_user_in_tenant("legacy", &a.id)
            .await
            .unwrap()
            .unwrap()
            .digest_password
            .as_deref(),
        Some("old-pass")
    );
    let again = s.reencrypt_secrets().await.unwrap();
    assert_eq!(again.rewritten, 0);
    assert_eq!(again.already_current, 1);

    db.finish().await;
}

#[tokio::test]
async fn encryption_rotation_keeps_old_values_readable_and_other_nodes_refresh() {
    let Some(db) = TestDb::with_encryption(master_key(1)).await else {
        return;
    };
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();

    let mut u = user(a.id, "alice");
    u.digest_password = Some("before-rotate".to_string());
    s.put_user(u).await.unwrap();
    let uid = s
        .get_user_in_tenant("alice", &a.id)
        .await
        .unwrap()
        .unwrap()
        .id;
    let old_key = s.encryption_status().await.unwrap().active_key_id.unwrap();

    // A second node connected before the rotation.
    let other = db.connect_with(master_key(1)).await.unwrap();

    let status = s.rotate_encryption_key().await.unwrap();
    let new_key = status.active_key_id.clone().unwrap();
    assert_ne!(new_key, old_key);
    assert_eq!(status.key_ids.len(), 2);

    // Old value still opens; new writes use the new key.
    assert_eq!(
        s.get_user_by_id(&uid)
            .await
            .unwrap()
            .unwrap()
            .digest_password
            .as_deref(),
        Some("before-rotate")
    );
    let mut u2 = user(a.id, "bob");
    u2.digest_password = Some("after-rotate".to_string());
    s.put_user(u2).await.unwrap();
    let bob = s.get_user_in_tenant("bob", &a.id).await.unwrap().unwrap();
    let raw = db
        .raw(
            "SELECT digest_password FROM users WHERE id = $1",
            &bob.id.to_string(),
        )
        .await
        .unwrap();
    assert!(raw.starts_with(&format!("enc:v1:{new_key}:")));

    // The stale node refreshes its key ring on the unknown DEK and reads it.
    let seen = other
        .get_user_in_tenant("bob", &a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(seen.digest_password.as_deref(), Some("after-rotate"));
    assert_eq!(other.encryption_status().await.unwrap().key_ids.len(), 2);

    // Re-encrypt moves alice onto the new key.
    let report = s.reencrypt_secrets().await.unwrap();
    assert_eq!(report.rewritten, 1, "{report:?}");
    let raw = db
        .raw(
            "SELECT digest_password FROM users WHERE id = $1",
            &uid.to_string(),
        )
        .await
        .unwrap();
    assert!(raw.starts_with(&format!("enc:v1:{new_key}:")));

    other.close().await;
    db.finish().await;
}

#[tokio::test]
async fn encryption_fails_closed_on_wrong_master_key_or_no_cipher() {
    let Some(db) = TestDb::with_encryption(master_key(1)).await else {
        return;
    };
    let s = db.storage();
    let a = tenant("a");
    s.put_tenant(a.clone()).await.unwrap();
    let mut u = user(a.id, "alice");
    u.digest_password = Some("secret".to_string());
    s.put_user(u).await.unwrap();

    // A different master key cannot unwrap the active DEK: startup fails.
    let err = db
        .connect_with(master_key(2))
        .await
        .err()
        .expect("wrong KEK rejected");
    assert!(
        format!("{err:#}").contains("KEK"),
        "error names the KEK mismatch: {err:#}"
    );

    // Encryption disabled: sealed values are an error, not garbage.
    let plain = db.connect_with(EncryptionSettings::Disabled).await.unwrap();
    assert!(!plain.encryption_status().await.unwrap().enabled);
    let err = plain.get_user_in_tenant("alice", &a.id).await.unwrap_err();
    assert!(
        format!("{err:#}").contains("not configured"),
        "sealed value without cipher: {err:#}"
    );
    assert!(plain.rotate_encryption_key().await.is_err());
    plain.close().await;

    // Disabled backends still work for tenants with no sealed secrets.
    let db2 = TestDb::new().await.unwrap();
    let b = tenant("b");
    db2.storage().put_tenant(b.clone()).await.unwrap();
    let mut u = user(b.id, "carol");
    u.digest_password = Some("plain".to_string());
    db2.storage().put_user(u).await.unwrap();
    let raw = db2
        .raw(
            "SELECT digest_password FROM users WHERE username = $1",
            "carol",
        )
        .await
        .unwrap();
    assert_eq!(raw, "plain");
    db2.finish().await;

    db.finish().await;
}

// ─── Read replica ────────────────────────────────────────────────────────

/// Routing is proved with two databases on the same server: reads served
/// by the "replica" cannot see what was written to the primary.
#[tokio::test]
async fn replica_reads_follow_health_and_the_read_after_write_window() {
    let primary = require_db!();
    let replica = require_db!();
    let s = PostgresStorage::new(
        PostgresConfig::new(TestDb::url(&primary.admin_url, &primary.name))
            .with_read_replica(TestDb::url(&replica.admin_url, &replica.name))
            .with_max_replica_lag(Duration::from_millis(500))
            .with_encryption(EncryptionSettings::Disabled),
    )
    .await
    .expect("connect with a read replica");

    // Probed at connect: same server, not a standby, so lag is zero.
    let status = s.replica_status().expect("replica configured");
    assert!(status.reachable, "{status:?}");
    assert!(status.healthy, "{status:?}");
    assert_eq!(status.lag_ms, 0);
    assert_eq!(status.max_lag_ms, 500);
    assert!(status.probe_age_secs <= 5, "just probed: {status:?}");
    assert!(s.reads_from_replica());
    // What a binary's /status and /metrics say about it.
    let health = fcp_storage_postgres::ReplicaHealth::of(std::sync::Arc::new(s.clone()));
    let json = health.json();
    assert_eq!(json["healthy"], true, "{json}");
    assert!(json["probe_age_secs"].is_u64(), "{json}");
    let prom = health.prometheus("node=\"t\"");
    assert!(
        prom.contains("fcp_replica_healthy{node=\"t\",replica=\""),
        "{prom}"
    );
    assert!(prom.contains("fcp_replica_probe_age_seconds{"), "{prom}");
    assert_eq!(
        fcp_storage_postgres::ReplicaHealth::none().json(),
        serde_json::Value::Null
    );
    assert!(fcp_storage_postgres::ReplicaHealth::none()
        .prometheus("")
        .is_empty());

    // Written through another backend, so `s` has no write of its own.
    let t = tenant("replica-routing");
    primary.storage().put_tenant(t.clone()).await.unwrap();
    assert!(
        s.get_tenant(&t.id).await.unwrap().is_none(),
        "configuration read served by the replica, which has no such tenant"
    );

    // A write through `s` pins its reads to the primary for the window.
    s.put_tenant(tenant("replica-writer")).await.unwrap();
    assert!(!s.reads_from_replica());
    assert!(
        s.get_tenant(&t.id).await.unwrap().is_some(),
        "read-after-write goes to the primary"
    );
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(s.reads_from_replica(), "window elapsed");
    assert!(s.get_tenant(&t.id).await.unwrap().is_none());

    // Reads that must be current never use the replica: registrations.
    assert!(s.get_registrations("nobody@example.com").await.is_ok());

    // An unhealthy replica sends every read to the primary.
    s.set_replica_healthy(false);
    assert!(!s.reads_from_replica());
    assert!(s.get_tenant(&t.id).await.unwrap().is_some());

    // A fresh probe sees it healthy again.
    s.probe_replica().await;
    assert!(s.reads_from_replica());

    s.close().await;
    primary.finish().await;
    replica.finish().await;
}

#[tokio::test]
async fn unreachable_replica_is_tolerated_and_reads_use_the_primary() {
    let primary = require_db!();
    let s = PostgresStorage::new(
        PostgresConfig::new(TestDb::url(&primary.admin_url, &primary.name))
            .with_read_replica("postgresql://nobody@127.0.0.1:1/nowhere")
            .with_encryption(EncryptionSettings::Disabled),
    )
    .await
    .expect("a replica that is down must not stop the node");
    let status = s.replica_status().expect("replica configured");
    assert!(!status.reachable);
    assert!(!status.healthy);
    assert_eq!(status.host, "127.0.0.1:1");
    assert!(!s.reads_from_replica());

    let t = tenant("no-replica");
    s.put_tenant(t.clone()).await.unwrap();
    assert!(s.get_tenant(&t.id).await.unwrap().is_some());

    s.close().await;
    primary.finish().await;
}

// ─── Conference recordings (migration 053) ───────────────────────────────

fn recording(
    tenant_id: TenantId,
    room: &str,
    node: &str,
    started_secs_ago: u64,
) -> fcp_core::Recording {
    let mut r = fcp_core::Recording::starting(
        tenant_id,
        room,
        node,
        format!("/var/fcp/recordings/{room}-{node}.webm"),
        fcp_core::RecordingFormat::Webm,
        fcp_core::RecordingTrigger::Api,
    );
    // Timestamps come back at whole-second resolution, as they do from
    // every store here, so the fixture starts on one.
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - started_secs_ago;
    r.started_at = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
    r.participants = 3;
    r
}

#[tokio::test]
async fn conference_recordings_round_trip_and_filter() {
    let db = require_db!();
    let store =
        fcp_storage_postgres::recording::PostgresRecordingStore::new(db.storage.pool().clone());
    let t = tenant("recordings");
    db.storage.put_tenant(t.clone()).await.unwrap();
    let other = tenant("recordings-other");
    db.storage.put_tenant(other.clone()).await.unwrap();

    let mut running = recording(t.id, "8001", "conf-a", 60);
    running.consent = fcp_core::RecordingConsent::Announced;
    let finished = recording(t.id, "8002", "conf-b", 7 * 86_400);
    let elsewhere = recording(other.id, "9001", "conf-a", 60);
    for r in [&running, &finished, &elsewhere] {
        store.create_recording(r).await.unwrap();
    }

    // Read one back whole.
    let got = store.get_recording(&running.id).await.unwrap().unwrap();
    assert_eq!(got, running);
    assert!(got.is_running());
    assert_eq!(got.consent, fcp_core::RecordingConsent::Announced);
    assert_eq!(got.participants, 3);

    // Completing it fills in what only the end knows.
    let ended = SystemTime::now();
    store
        .finish_recording(&finished.id, ended, 42, 1_234_567)
        .await
        .unwrap();
    let done = store.get_recording(&finished.id).await.unwrap().unwrap();
    assert!(!done.is_running());
    assert_eq!(done.duration_secs, Some(42));
    assert_eq!(done.size_bytes, Some(1_234_567));

    // A tenant sees its own, newest first, and never the other tenant's.
    let all = store
        .list_recordings(&t.id, &fcp_core::RecordingFilter::recent(50))
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, running.id, "newest first");

    let by_room = store
        .list_recordings(
            &t.id,
            &fcp_core::RecordingFilter {
                room_number: Some("8002".to_string()),
                ..fcp_core::RecordingFilter::recent(50)
            },
        )
        .await
        .unwrap();
    assert_eq!(by_room.len(), 1);
    assert_eq!(by_room[0].room_number, "8002");

    let still_running = store
        .list_recordings(
            &t.id,
            &fcp_core::RecordingFilter {
                running: Some(true),
                ..fcp_core::RecordingFilter::recent(50)
            },
        )
        .await
        .unwrap();
    assert_eq!(still_running.len(), 1);
    assert_eq!(still_running[0].id, running.id);

    let since = store
        .list_recordings(
            &t.id,
            &fcp_core::RecordingFilter {
                since: Some(SystemTime::now() - Duration::from_secs(600)),
                ..fcp_core::RecordingFilter::recent(50)
            },
        )
        .await
        .unwrap();
    assert_eq!(since.len(), 1, "the week-old one is outside the window");

    db.finish().await;
}

#[tokio::test]
async fn a_node_finds_what_it_holds_and_what_has_expired() {
    let db = require_db!();
    let store =
        fcp_storage_postgres::recording::PostgresRecordingStore::new(db.storage.pool().clone());
    let t = tenant("sweep");
    db.storage.put_tenant(t.clone()).await.unwrap();
    let other = tenant("sweep-other");
    db.storage.put_tenant(other.clone()).await.unwrap();

    let old = recording(t.id, "8001", "conf-a", 30 * 86_400);
    let recent = recording(t.id, "8002", "conf-a", 60);
    let elsewhere = recording(other.id, "9001", "conf-b", 30 * 86_400);
    for r in [&old, &recent, &elsewhere] {
        store.create_recording(r).await.unwrap();
    }
    // Only a finished recording can expire.
    store
        .finish_recording(&old.id, SystemTime::now(), 10, 100)
        .await
        .unwrap();

    // The node's own view: which tenants, and which file is a row's.
    let tenants = store.node_tenants("conf-a").await.unwrap();
    assert_eq!(tenants, vec![t.id]);
    assert_eq!(store.node_tenants("conf-b").await.unwrap(), vec![other.id]);
    assert_eq!(
        store
            .recording_at("conf-a", &old.path)
            .await
            .unwrap()
            .map(|r| r.id),
        Some(old.id)
    );
    assert!(
        store
            .recording_at("conf-b", &old.path)
            .await
            .unwrap()
            .is_none(),
        "the path belongs to another node"
    );
    assert!(store
        .recording_at("conf-a", "/var/fcp/recordings/nothing.wav")
        .await
        .unwrap()
        .is_none());

    // Expiry: finished, this tenant's, and older than the cutoff.
    let cutoff = SystemTime::now() - Duration::from_secs(7 * 86_400);
    let expired = store.expired_recordings(&t.id, cutoff).await.unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].id, old.id);

    // A recording still being written is what a restart closes off.
    let running = store.running_recordings("conf-a").await.unwrap();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, recent.id);

    assert!(store.delete_recording(&old.id).await.unwrap());
    assert!(!store.delete_recording(&old.id).await.unwrap());
    assert!(store.get_recording(&old.id).await.unwrap().is_none());

    db.finish().await;
}

/// A tenant whose notification contacts are the free text of block 1 reads
/// back as typed contacts, whether migration 071 converted the row or the
/// store reads it by shape (`docs/E911_BLOCK_2.md` §8.1).
#[tokio::test]
async fn emergency_contacts_saved_as_free_text_read_back_typed() {
    use fcp_core::e911::notify::ContactKind;
    use fcp_core::e911::EmergencyStore;

    let db = require_db!();
    let t = tenant("notify");
    db.storage.put_tenant(t.clone()).await.unwrap();
    let tenant = t.id;
    let store =
        fcp_storage_postgres::emergency::PostgresEmergencyStore::new(db.storage.pool().clone());

    // A row as block 1 wrote it: free text, and no typed contacts.
    sqlx::query(
        "INSERT INTO emergency_configs
            (tenant_id, emergency_numbers, test_numbers, dial_prefixes, notify_contacts,
             require_dispatchable_location, updated_at)
         VALUES ($1, ARRAY['911'], ARRAY['933'], ARRAY['9'], $2, FALSE, NOW())",
    )
    .bind(tenant.to_string())
    .bind(vec![
        "security@example.com".to_string(),
        "sip:frontdesk@pbx.example.com".to_string(),
        "+1 518 555 0143".to_string(),
        "2001".to_string(),
        "  ".to_string(),
    ])
    .execute(db.storage.pool())
    .await
    .expect("insert a block 1 configuration");

    let config = store
        .get_emergency_config(&tenant)
        .await
        .expect("read the configuration")
        .expect("a configuration");
    let kinds: Vec<(ContactKind, String)> = config
        .contacts
        .iter()
        .map(|c| (c.kind, c.target.clone()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (ContactKind::Email, "security@example.com".to_string()),
            (
                ContactKind::Call,
                "sip:frontdesk@pbx.example.com".to_string()
            ),
            (ContactKind::Sms, "+1 518 555 0143".to_string()),
            (ContactKind::Call, "2001".to_string()),
        ],
        "each contact read by its shape, the empty one dropped"
    );

    // Saving it again writes both: the typed contacts, and the plain
    // targets the event has always carried.
    store
        .put_emergency_config(&config)
        .await
        .expect("save the configuration");
    let stored: (serde_json::Value, Vec<String>) = sqlx::query_as(
        "SELECT contacts, notify_contacts FROM emergency_configs WHERE tenant_id = $1",
    )
    .bind(tenant.to_string())
    .fetch_one(db.storage.pool())
    .await
    .expect("read the row");
    assert_eq!(stored.0.as_array().map(|a| a.len()), Some(4));
    assert_eq!(stored.1.len(), 4);
    assert_eq!(stored.1[0], "security@example.com");

    db.finish().await;
}

/// A user's calling settings (docs/ENTERPRISE_CALLING.md §3.1): absent
/// until put, read back as written, listed by tenant, gone when deleted.
#[tokio::test]
async fn calling_settings_round_trip() {
    use fcp_core::{CallTarget, CallingSettings, CallingStore, TenantId, UserId};
    let db = require_db!();
    let storage = db.storage();
    let tenant = TenantId::new();
    let user = UserId::new();
    assert!(storage
        .get_calling_settings(&tenant, &user)
        .await
        .unwrap()
        .is_none());

    let mut settings = CallingSettings::defaults(tenant, user);
    settings.dnd = true;
    settings.no_answer_secs = Some(12);
    settings.forward_busy = Some(CallTarget::Extension("2002".into()));
    settings.forward_unreachable = Some(CallTarget::Voicemail);
    storage
        .put_calling_settings(settings.clone())
        .await
        .unwrap();
    let got = storage
        .get_calling_settings(&tenant, &user)
        .await
        .unwrap()
        .expect("stored");
    assert!(got.dnd);
    assert_eq!(got.no_answer_secs, Some(12));
    assert_eq!(got.forward_busy, Some(CallTarget::Extension("2002".into())));
    assert_eq!(got.forward_unreachable, Some(CallTarget::Voicemail));
    assert!(got.forward_all.is_none());

    // Put again is an update, listed once.
    settings.dnd = false;
    storage.put_calling_settings(settings).await.unwrap();
    let listed = storage.list_calling_settings(&tenant).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].dnd);
    assert!(storage
        .list_calling_settings(&TenantId::new())
        .await
        .unwrap()
        .is_empty());

    storage
        .delete_calling_settings(&tenant, &user)
        .await
        .unwrap();
    assert!(storage
        .get_calling_settings(&tenant, &user)
        .await
        .unwrap()
        .is_none());
    db.finish().await;
}
