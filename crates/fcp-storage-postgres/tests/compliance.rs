//! The compliance foundations against a real database
//! (`docs/COMPLIANCE_FOLLOWUPS.md` §4, §7): recordings deleted by their rows
//! and their files reaped, one tenant's window never reaching another's
//! files, voicemail retention, the deletion log's attribution (retention,
//! a person, a cascade, nobody), greetings kept while a row names them,
//! trace tenancy, orphan files. Gated on `FCP_TEST_DATABASE_URL`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fcp_core::compliance::{
    deleting_as, run_retention, run_upkeep, CompliancePolicy, ComplianceStore, Deleter,
    DeletionCause, DeletionQuery, UpkeepConfig,
};
use fcp_core::flow_trace::{FlowTraceEntry, FlowTraceEventType, FlowTraceStore};
use fcp_core::recording::{
    CallRecording, CallRecordingSource, CallRecordingStore, Recording, RecordingFormat,
    RecordingKind, RecordingParticipant, RecordingStore, RecordingTrigger,
};
use fcp_core::sip_trace::{SipTraceMessage, SipTraceStore};
use fcp_core::storage::{CdrStore, LineBindingStore, PhoneNumberStore, VoicemailStore};
use fcp_core::{
    BindingType, CallId, DynStorage, GreetingType, LineBinding, PhoneNumber, TenantId, UserId,
    VoicemailGreeting, VoicemailMailbox, VoicemailMessage,
};
use fcp_storage_postgres::compliance::PostgresComplianceStore;
use fcp_storage_postgres::flow_trace::PostgresFlowTraceStore;
use fcp_storage_postgres::recording::PostgresRecordingStore;
use fcp_storage_postgres::sip_trace::PostgresSipTraceStore;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use sqlx::{Connection, PgConnection};

struct Db {
    admin_url: String,
    name: String,
    pg: Arc<PostgresStorage>,
    storage: DynStorage,
    compliance: Arc<PostgresComplianceStore>,
}

impl Db {
    async fn new() -> Option<Self> {
        let admin_url = std::env::var("FCP_TEST_DATABASE_URL").ok()?;
        let name = format!("fcp_test_{}", uuid::Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&admin_url).await.unwrap();
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&mut admin)
            .await
            .unwrap();
        let (base, query) = match admin_url.split_once('?') {
            Some((b, q)) => (b.to_string(), format!("?{q}")),
            None => (admin_url.clone(), String::new()),
        };
        let url = format!("{}/{name}{query}", &base[..base.rfind('/').unwrap()]);
        let pg = Arc::new(
            PostgresStorage::new(PostgresConfig::new(url))
                .await
                .unwrap(),
        );
        let pool = pg.pool().clone();
        let compliance = Arc::new(PostgresComplianceStore::new(pool.clone()));
        let recordings = Arc::new(PostgresRecordingStore::new(pool.clone()));
        let storage = DynStorage::from_all_in_one(pg.clone())
            .with_compliance_store(compliance.clone() as Arc<dyn ComplianceStore>)
            .with_recording_store(recordings.clone() as Arc<dyn RecordingStore>)
            .with_call_recording_store(recordings as Arc<dyn CallRecordingStore>)
            .with_sip_trace_store(
                Arc::new(PostgresSipTraceStore::new(pool.clone())) as Arc<dyn SipTraceStore>
            )
            .with_flow_trace_store(
                Arc::new(PostgresFlowTraceStore::new(pool)) as Arc<dyn FlowTraceStore>
            );
        Some(Self {
            admin_url,
            name,
            pg,
            storage,
            compliance,
        })
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(self.pg.pool())
            .await
            .unwrap()
    }

    async fn finish(self) {
        drop(self.storage);
        self.pg.close().await;
        let mut admin = PgConnection::connect(&self.admin_url).await.unwrap();
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.name))
            .execute(&mut admin)
            .await
            .unwrap();
    }
}

macro_rules! require_db {
    () => {
        match Db::new().await {
            Some(db) => db,
            None => {
                eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
                return;
            }
        }
    };
}

fn days_ago(days: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(days * 86_400)
}

/// A file at `path` last modified `days` ago.
fn file(path: &Path, days: u64) -> String {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"audio").unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(days_ago(days))
        .unwrap();
    path.to_string_lossy().into_owned()
}

fn record(
    tenant: TenantId,
    call_id: CallId,
    recording: &str,
    start: SystemTime,
) -> fcp_core::storage::CallDetailRecord {
    fcp_cdr::RecordBuilder::new(fcp_cdr::CdrKind::Call, call_id, tenant)
        .from("sip:alice@a.example", None)
        .to("sip:bob@a.example", None)
        .started_at(start)
        .ended_at(Some(start + Duration::from_secs(30)))
        .recording(Some(recording.to_string()))
        .build()
}

async fn mailbox(db: &Db, tenant: TenantId, e164: &str) -> (PhoneNumber, VoicemailMailbox) {
    let number = PhoneNumber::new(e164.to_string(), tenant).unwrap();
    db.pg.put_phone_number(number.clone()).await.unwrap();
    let binding = LineBinding::new(number.id, None, BindingType::User, tenant);
    db.pg.put_line_binding(binding.clone()).await.unwrap();
    let mb = VoicemailMailbox::new(
        tenant,
        UserId::new(),
        binding.id,
        "1001".into(),
        "1234".into(),
    );
    db.pg.put_mailbox(mb.clone()).await.unwrap();
    (number, mb)
}

async fn message(db: &Db, mb: &VoicemailMailbox, path: String, days: u64) -> VoicemailMessage {
    let mut m = VoicemailMessage::new(mb.id.clone(), "+14155550199".into(), 5, PathBuf::from(path));
    m.created_at = days_ago(days);
    db.pg.put_message(m.clone()).await.unwrap();
    m
}

#[tokio::test]
async fn deletions_are_logged_attributed_and_their_files_reaped_within_their_tenant() {
    let db = require_db!();
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let (a, b) = (TenantId::new(), TenantId::new());
    let upkeep = UpkeepConfig {
        file_roots: vec![root.path().to_path_buf()],
        ..UpkeepConfig::default()
    };

    // ── Call recordings: rows, the record's reference, one tenant's window.
    let a_file = file(&root.path().join(a.to_string()).join("old.wav"), 40);
    let b_file = file(&root.path().join(b.to_string()).join("old.wav"), 40);
    let a_call = CallId::new();
    db.pg
        .insert_cdrs(vec![record(a, a_call, &a_file, days_ago(40))])
        .await
        .unwrap();
    for (tenant, call, path) in [
        (a, a_call.to_string(), &a_file),
        (b, CallId::new().to_string(), &b_file),
    ] {
        let mut r = CallRecording::starting(tenant, call, CallRecordingSource::Flow, path.clone());
        r.started_at = days_ago(40);
        db.storage
            .call_recording_store()
            .unwrap()
            .create_call_recording(&r)
            .await
            .unwrap();
    }
    let mut policy = CompliancePolicy::defaults(a);
    policy.recording_retention_days = Some(30);
    let report = run_retention(&db.storage, &policy, "retention-test").await;
    assert_eq!(report.recordings_deleted, 1, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let logged = db
        .compliance
        .list_deletions(&DeletionQuery {
            before: None,
            tenant_id: Some(a),
            class: Some("call_recording".into()),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(logged.len(), 1);
    assert_eq!(
        (logged[0].cause.as_str(), logged[0].actor.as_str()),
        ("retention", "retention-test")
    );
    assert_eq!(logged[0].detail.as_ref().unwrap()["path"], a_file);
    let stored = db.pg.get_cdr(&a_call).await.unwrap().unwrap();
    assert_eq!(stored.recording_url, None, "the record no longer names it");
    // The file goes when the reaper runs, and only a's.
    assert!(Path::new(&a_file).exists());
    let done = run_upkeep(&db.storage, &upkeep, "upkeep-test").await;
    assert_eq!(done.files_removed, 1, "{:?}", done.errors);
    assert!(!Path::new(&a_file).exists());
    assert!(
        Path::new(&b_file).exists(),
        "tenant b's recording is b's to keep"
    );

    // ── Voicemail: a person's delete, a cascade, nobody's, retention.
    let (number, mb) = mailbox(&db, a, "+14155550100").await;
    let heard = message(&db, &mb, file(&root.path().join("vm/heard.wav"), 1), 1).await;
    deleting_as(
        Deleter::new(DeletionCause::Manual, "alice")
            .in_api_request(Some("req-heard".into()))
            .via("phone sip:alice@a.example"),
        db.pg.delete_message(&heard.id),
    )
    .await
    .unwrap();
    let old = message(&db, &mb, file(&root.path().join("vm/old.wav"), 100), 100).await;
    let recent = message(&db, &mb, file(&root.path().join("vm/recent.wav"), 1), 1).await;
    let mut policy = CompliancePolicy::defaults(a);
    policy.voicemail_retention_days = Some(30);
    let report = run_retention(&db.storage, &policy, "retention-test").await;
    assert_eq!(report.voicemail_messages_deleted, 1);
    assert!(db.pg.get_message(&old.id).await.unwrap().is_none());
    assert!(db.pg.get_message(&recent.id).await.unwrap().is_some());
    let unattributed = message(&db, &mb, file(&root.path().join("vm/nobody.wav"), 1), 1).await;
    db.pg.delete_message(&unattributed.id).await.unwrap();
    // The number goes, and with it the binding; the mailbox outlives its
    // line (migration 097), its message with it.
    deleting_as(
        Deleter::new(DeletionCause::Manual, "bob"),
        db.pg.delete_phone_number(&number.id),
    )
    .await
    .unwrap();
    let kept = db
        .pg
        .get_mailbox(&mb.id)
        .await
        .unwrap()
        .expect("the mailbox");
    assert_eq!(kept.line_binding_id, None);
    assert!(db.pg.get_message(&recent.id).await.unwrap().is_some());
    // The mailbox goes, and with it the message.
    deleting_as(
        Deleter::new(DeletionCause::Manual, "bob").in_api_request(Some("req-mailbox".into())),
        db.pg.delete_mailbox(&mb.id),
    )
    .await
    .unwrap();
    let voicemail = db
        .compliance
        .list_deletions(&DeletionQuery {
            before: None,
            tenant_id: Some(a),
            class: Some("voicemail".into()),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    let by_item = |id: &str| voicemail.iter().find(|r| r.item == id).cloned().unwrap();
    assert_eq!(
        (by_item(&heard.id).cause, by_item(&heard.id).actor),
        ("manual".into(), "alice".into())
    );
    // The request that deleted it and how (#342), a cascade's included.
    assert_eq!(
        by_item(&heard.id).api_request_id.as_deref(),
        Some("req-heard")
    );
    assert_eq!(
        by_item(&heard.id).via.as_deref(),
        Some("phone sip:alice@a.example")
    );
    assert_eq!(
        by_item(&recent.id).api_request_id.as_deref(),
        Some("req-mailbox")
    );
    assert_eq!(by_item(&recent.id).via, None);
    assert_eq!(by_item(&old.id).api_request_id, None);
    let of_request = |api_request_id: &str, until: Option<SystemTime>| {
        let compliance = db.compliance.clone();
        let api_request_id = api_request_id.to_string();
        async move {
            compliance
                .list_deletions(&DeletionQuery {
                    tenant_id: Some(a),
                    api_request_id: Some(api_request_id),
                    until,
                    limit: 10,
                    ..Default::default()
                })
                .await
                .unwrap()
                .into_iter()
                .map(|r| r.item)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(of_request("req-heard", None).await, vec![heard.id.clone()]);
    assert!(
        of_request(
            "req-heard",
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1))
        )
        .await
        .is_empty(),
        "until filters"
    );
    assert_eq!(by_item(&old.id).cause, "retention");
    assert_eq!(by_item(&unattributed.id).cause, "unattributed");
    assert_eq!(
        (by_item(&recent.id).cause, by_item(&recent.id).actor),
        ("cascade".into(), "bob".into())
    );
    let done = run_upkeep(&db.storage, &upkeep, "upkeep-test").await;
    assert_eq!(done.files_removed, 4, "{:?}", done.errors);
    assert_eq!(
        db.scalar("SELECT count(*) FROM pending_file_deletions")
            .await,
        0
    );

    // ── A greeting rewritten in place keeps its file.
    let (_, mut mb2) = mailbox(&db, a, "+14155550101").await;
    let greeting = file(&root.path().join("greetings/busy.wav"), 40);
    mb2.greetings = vec![VoicemailGreeting::new(
        GreetingType::Busy,
        PathBuf::from(&greeting),
    )];
    db.pg.put_mailbox(mb2.clone()).await.unwrap();
    db.pg.put_mailbox(mb2.clone()).await.unwrap();
    let done = run_upkeep(&db.storage, &upkeep, "upkeep-test").await;
    assert!(done.files_kept >= 1, "{done:?}");
    assert!(Path::new(&greeting).exists());

    // ── Mailbox retention, and messages a user deleted.
    mb2.retention_days = Some(10);
    db.pg.put_mailbox(mb2.clone()).await.unwrap();
    let expired = message(&db, &mb2, file(&root.path().join("vm/expired.wav"), 20), 20).await;
    let mut binned = message(&db, &mb2, file(&root.path().join("vm/binned.wav"), 1), 1).await;
    binned.status = fcp_core::MessageStatus::Deleted;
    db.pg.put_message(binned.clone()).await.unwrap();
    sqlx::query(
        "UPDATE voicemail_messages SET deleted_at = now() - interval '8 days' WHERE id = $1",
    )
    .bind(&binned.id)
    .execute(db.pg.pool())
    .await
    .unwrap();
    let done = run_upkeep(&db.storage, &upkeep, "upkeep-test").await;
    assert_eq!(
        (done.voicemail_expired, done.voicemail_purged),
        (1, 1),
        "{done:?}"
    );
    assert!(db.pg.get_message(&expired.id).await.unwrap().is_none());

    // ── A queued path outside every root is never touched.
    let stray = file(&outside.path().join("stray.wav"), 1);
    sqlx::query("INSERT INTO pending_file_deletions (tenant_id, class, path) VALUES ($1, 'call_recording', $2)")
        .bind(a.to_string())
        .bind(&stray)
        .execute(db.pg.pool())
        .await
        .unwrap();
    let done = run_upkeep(&db.storage, &upkeep, "upkeep-test").await;
    assert!(
        done.errors
            .iter()
            .any(|e| e.contains("not under a configured file root")),
        "{done:?}"
    );
    assert!(Path::new(&stray).exists());
    assert_eq!(
        db.scalar("SELECT max(attempts)::bigint FROM pending_file_deletions")
            .await,
        1
    );
    sqlx::query("DELETE FROM pending_file_deletions")
        .execute(db.pg.pool())
        .await
        .unwrap();

    // ── Orphans: an old file no row names goes; a named one stays.
    let orphan = file(&root.path().join("lost/orphan.wav"), 40);
    let orphans = UpkeepConfig {
        orphan_files_after: Some(Duration::from_secs(30 * 86_400)),
        ..upkeep.clone()
    };
    let done = run_upkeep(&db.storage, &orphans, "upkeep-test").await;
    assert!(!Path::new(&orphan).exists());
    assert!(Path::new(&b_file).exists(), "named by b's recording");
    assert!(Path::new(&greeting).exists(), "named by a greeting");
    assert_eq!(done.orphan_files_removed, 1, "{done:?}");
    let platform = db
        .compliance
        .list_deletions(&DeletionQuery {
            before: None,
            class: Some("orphan_file".into()),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(platform.len(), 1);
    assert_eq!(
        (platform[0].tenant_id, platform[0].cause.as_str()),
        (None, "upkeep")
    );

    db.finish().await;
}

#[tokio::test]
async fn traces_find_their_tenant_and_rooms_name_who_was_recorded() {
    let db = require_db!();
    let tenant = TenantId::new();
    let call = CallId::new();
    db.pg
        .insert_cdrs(vec![record(tenant, call, "", SystemTime::now())])
        .await
        .unwrap();
    let traces = db.storage.sip_trace_store().unwrap();
    for (call_id, tenant_id) in [
        (call.to_string(), "_system"),
        ("call-register".to_string(), "_system"),
    ] {
        traces
            .insert_sip_trace(&SipTraceMessage {
                id: 0,
                call_id,
                sip_call_id: "x@y".into(),
                tenant_id: tenant_id.into(),
                timestamp: days_ago(2),
                direction: "inbound".into(),
                method_or_status: "INVITE".into(),
                cseq: "1 INVITE".into(),
                from_uri: "sip:alice@a.example".into(),
                to_uri: "sip:bob@a.example".into(),
                source_addr: String::new(),
                dest_addr: String::new(),
                transport: "udp".into(),
                raw_message: String::new(),
                node_id: None,
            })
            .await
            .unwrap();
    }
    db.storage
        .flow_trace_store()
        .unwrap()
        .insert_flow_trace(&FlowTraceEntry {
            id: 0,
            call_id: call.to_string(),
            tenant_id: None,
            flow_id: "flow-1".into(),
            flow_name: None,
            node_id: None,
            event_type: FlowTraceEventType::Started,
            timestamp: SystemTime::now(),
            detail: None,
            flow_version: None,
            elapsed_ms: None,
        })
        .await
        .unwrap();
    let done = run_upkeep(
        &db.storage,
        &UpkeepConfig {
            system_traces_after: Some(Duration::from_secs(86_400)),
            ..UpkeepConfig::default()
        },
        "upkeep-test",
    )
    .await;
    assert_eq!(
        (done.sip_traces_adopted, done.flow_traces_adopted),
        (1, 1),
        "{done:?}"
    );
    assert_eq!(
        done.system_traces_deleted, 1,
        "the registration's trace belongs to nobody"
    );
    assert_eq!(
        traces.get_sip_traces(&call.to_string()).await.unwrap()[0].tenant_id,
        tenant.to_string()
    );

    // A room recording names who was in it, and who joined later.
    let store = db.storage.recording_store().unwrap();
    let mut room = Recording::starting(
        tenant,
        "8001",
        "node-a",
        "/r/room.wav",
        RecordingFormat::Wav,
        RecordingTrigger::Auto,
    );
    let alice = RecordingParticipant {
        participant_id: "p1".into(),
        uri: Some("sip:alice@a.example".into()),
        display_name: Some("Alice".into()),
        joined_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000),
    };
    room.participant_list = vec![alice.clone()];
    store.create_recording(&room).await.unwrap();
    let bob = RecordingParticipant {
        participant_id: "p2".into(),
        uri: Some("sip:bob@a.example".into()),
        display_name: None,
        joined_at: SystemTime::UNIX_EPOCH + Duration::from_secs(2_000),
    };
    store
        .add_recording_participants(&room.id, std::slice::from_ref(&bob))
        .await
        .unwrap();
    let mut own = Recording::starting(
        tenant,
        "8001",
        "node-a",
        "/r/p1.wav",
        RecordingFormat::Wav,
        RecordingTrigger::Api,
    );
    own.kind = RecordingKind::Participant;
    own.participant_id = Some("p1".into());
    store.create_recording(&own).await.unwrap();
    let read = store.get_recording(&room.id).await.unwrap().unwrap();
    assert_eq!(read.participant_list, vec![alice, bob]);
    let read = store.get_recording(&own.id).await.unwrap().unwrap();
    assert_eq!(
        (read.kind, read.participant_id.as_deref()),
        (RecordingKind::Participant, Some("p1"))
    );

    db.finish().await;
}

fn policy_all(tenant: TenantId, days: u32) -> CompliancePolicy {
    let mut p = CompliancePolicy::defaults(tenant);
    p.cdr_retention_days = Some(days);
    p.sip_trace_retention_days = Some(days);
    p.flow_trace_retention_days = Some(days);
    p.recording_retention_days = Some(days);
    p.voicemail_retention_days = Some(days);
    p.audit_retention_days = Some(days);
    p
}

#[tokio::test]
async fn subject_keys_are_the_same_in_sql() {
    let db = require_db!();
    for (spelling, expected) in fcp_core::subject::KEY_VECTORS {
        let sql: Vec<String> = sqlx::query_scalar("SELECT fcp_subject_keys($1)")
            .bind(spelling)
            .fetch_one(db.pg.pool())
            .await
            .unwrap();
        assert_eq!(sql, *expected, "{spelling:?}");
        assert_eq!(fcp_core::subject::keys(spelling), sql, "{spelling:?}");
    }
    db.finish().await;
}

#[tokio::test]
async fn a_hold_keeps_what_it_covers_from_every_deletion_until_released() {
    use fcp_core::legal_hold::{
        DataClass, HoldRelease, HoldScope, LegalHold, LegalHoldViolation, SubjectIdentifier,
    };
    use fcp_core::storage::TenantStore;
    let db = require_db!();
    let root = tempfile::tempdir().unwrap();
    let tenant = fcp_core::Tenant::new("held".to_string());
    let t = tenant.id;
    db.pg.put_tenant(tenant.clone()).await.unwrap();

    // Two old calls: one with the subject, one without; a recording each.
    let (held_call, free_call) = (CallId::new(), CallId::new());
    let mut with_subject = record(t, held_call, "", days_ago(40));
    with_subject.to_uri = "sip:+1 (415) 555-0100@carrier.example".into();
    let without = record(t, free_call, "", days_ago(40));
    db.pg
        .insert_cdrs(vec![with_subject, without])
        .await
        .unwrap();
    let recordings = db.storage.call_recording_store().unwrap();
    let mut rec_ids = Vec::new();
    for call in [held_call, free_call] {
        let mut r = CallRecording::starting(
            t,
            call.to_string(),
            CallRecordingSource::Api,
            file(&root.path().join(format!("{call}.wav")), 40),
        );
        r.started_at = days_ago(40);
        recordings.create_call_recording(&r).await.unwrap();
        rec_ids.push(r.id);
    }
    // A mailbox whose old message is from the subject, and one that is not.
    let (number, mb) = mailbox(&db, t, "+14155550999").await;
    let mut from_subject = VoicemailMessage::new(
        mb.id.clone(),
        "4155550100".into(),
        3,
        root.path().join("m1.wav"),
    );
    from_subject.created_at = days_ago(40);
    db.pg.put_message(from_subject.clone()).await.unwrap();
    let mut other = VoicemailMessage::new(
        mb.id.clone(),
        "+442071234567".into(),
        3,
        root.path().join("m2.wav"),
    );
    other.created_at = days_ago(40);
    db.pg.put_message(other.clone()).await.unwrap();

    // The subject, by a national spelling of their number.
    let keys = fcp_core::legal_hold::expand_subjects(
        &db.storage,
        &t,
        &[SubjectIdentifier::Number {
            number: "+14155550100".into(),
        }],
    )
    .await
    .unwrap();
    let hold = LegalHold {
        id: fcp_core::LegalHoldId::new(),
        tenant_id: t,
        matter: "Case 42".into(),
        reason: None,
        scope: HoldScope {
            to: Some(days_ago(1)),
            subjects: vec![SubjectIdentifier::Number {
                number: "+14155550100".into(),
            }],
            ..Default::default()
        },
        subject_keys: keys,
        created_by: "counsel".into(),
        created_at: SystemTime::now(),
        released: None,
    };
    db.compliance.place_hold(&hold).await.unwrap();
    let listed = db.compliance.list_holds(&t, false).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].scope.subjects, hold.scope.subjects);

    // Retention keeps what the hold covers and deletes the rest.
    let report = run_retention(&db.storage, &policy_all(t, 30), "retention-test").await;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        (
            report.cdrs_deleted,
            report.recordings_deleted,
            report.voicemail_messages_deleted
        ),
        (1, 1, 1)
    );
    assert_eq!(report.held.get("cdr"), Some(&1), "{:?}", report.held);
    assert_eq!(report.held.get("call_recording"), Some(&1));
    assert_eq!(report.held.get("voicemail"), Some(&1));
    assert!(db.pg.get_cdr(&held_call).await.unwrap().is_some());
    assert!(db.pg.get_cdr(&free_call).await.unwrap().is_none());
    assert!(db.pg.get_message(&from_subject.id).await.unwrap().is_some());
    assert!(db.pg.get_message(&other.id).await.unwrap().is_none());
    let runs = db.compliance.list_retention_runs(&t, 1).await;
    assert!(runs.is_ok());

    // The counts and the single-item check.
    let counts = db.compliance.held_counts(&t, &hold.id).await.unwrap();
    let count = |class: DataClass| {
        counts
            .iter()
            .find(|(c, _)| *c == class)
            .and_then(|(_, n)| *n)
    };
    assert_eq!(
        (
            count(DataClass::Cdr),
            count(DataClass::CallRecording),
            count(DataClass::Voicemail)
        ),
        (Some(1), Some(1), Some(1))
    );
    assert_eq!(
        db.compliance
            .hold_on(&t, DataClass::Voicemail, &from_subject.id)
            .await
            .unwrap(),
        Some(hold.id)
    );

    // Every other way in is refused, cascades and the tenant included.
    let violation = |e: anyhow::Error| e.downcast::<LegalHoldViolation>().map(|v| v.hold_id).ok();
    assert_eq!(
        violation(db.pg.delete_message(&from_subject.id).await.unwrap_err()),
        Some(hold.id.to_string())
    );
    assert_eq!(
        violation(db.pg.delete_mailbox(&mb.id).await.unwrap_err()),
        Some(hold.id.to_string()),
        "the mailbox's cascade reaches a held message"
    );
    // The number's no longer does: the mailbox outlives its line.
    db.pg.delete_phone_number(&number.id).await.unwrap();
    assert!(db.pg.get_message(&from_subject.id).await.unwrap().is_some());
    let raw = sqlx::query("DELETE FROM cdrs WHERE call_id = $1")
        .bind(held_call.to_string())
        .execute(db.pg.pool())
        .await
        .unwrap_err();
    assert_eq!(
        raw.as_database_error().and_then(|d| d.code()).as_deref(),
        Some("HLD01")
    );
    assert!(violation(db.pg.delete_tenant(&t).await.unwrap_err()).is_some());

    // Released, the next run deletes it all.
    let release = HoldRelease {
        by: "counsel".into(),
        at: SystemTime::now(),
        reason: "settled".into(),
    };
    assert!(db
        .compliance
        .release_hold(&t, &hold.id, &release)
        .await
        .unwrap());
    assert!(
        !db.compliance
            .release_hold(&t, &hold.id, &release)
            .await
            .unwrap(),
        "once"
    );
    let report = run_retention(&db.storage, &policy_all(t, 30), "retention-test").await;
    assert_eq!(
        (
            report.cdrs_deleted,
            report.recordings_deleted,
            report.voicemail_messages_deleted
        ),
        (1, 1, 1),
        "{:?}",
        report.errors
    );
    assert!(report.held.is_empty());
    let kept = db.compliance.list_holds(&t, true).await.unwrap();
    assert_eq!(
        kept[0].released.as_ref().map(|r| r.reason.as_str()),
        Some("settled"),
        "never deleted"
    );
    let _ = rec_ids;

    db.finish().await;
}

#[tokio::test]
async fn a_tenant_wide_hold_covers_traces_audit_and_rooms_and_only_its_window() {
    use fcp_core::legal_hold::{DataClass, HoldScope, LegalHold};
    let db = require_db!();
    let t = TenantId::new();
    let call = CallId::new();
    db.pg
        .insert_cdrs(vec![record(t, call, "", days_ago(40))])
        .await
        .unwrap();
    // A trace captured before its tenant was known.
    db.storage
        .sip_trace_store()
        .unwrap()
        .insert_sip_trace(&SipTraceMessage {
            id: 0,
            call_id: call.to_string(),
            sip_call_id: "x@y".into(),
            tenant_id: "_system".into(),
            timestamp: days_ago(40),
            direction: "inbound".into(),
            method_or_status: "INVITE".into(),
            cseq: "1 INVITE".into(),
            from_uri: "sip:alice@a.example".into(),
            to_uri: "sip:bob@a.example".into(),
            source_addr: String::new(),
            dest_addr: String::new(),
            transport: "udp".into(),
            raw_message: String::new(),
            node_id: None,
        })
        .await
        .unwrap();
    let store = db.storage.recording_store().unwrap();
    let mut room = Recording::starting(
        t,
        "8001",
        "node-a",
        "/r/room.wav",
        RecordingFormat::Wav,
        RecordingTrigger::Auto,
    );
    room.started_at = days_ago(40);
    room.ended_at = Some(days_ago(40));
    store.create_recording(&room).await.unwrap();
    let mut recent = Recording::starting(
        t,
        "8001",
        "node-a",
        "/r/recent.wav",
        RecordingFormat::Wav,
        RecordingTrigger::Auto,
    );
    recent.started_at = days_ago(20);
    recent.ended_at = Some(days_ago(20));
    store.create_recording(&recent).await.unwrap();

    // Everything from 50 to 30 days ago, of three classes.
    let hold = LegalHold {
        id: fcp_core::LegalHoldId::new(),
        tenant_id: t,
        matter: "Audit".into(),
        reason: Some("regulator".into()),
        scope: HoldScope {
            from: Some(days_ago(50)),
            to: Some(days_ago(30)),
            classes: vec![
                DataClass::SipTrace,
                DataClass::ConferenceRecording,
                DataClass::Cdr,
            ],
            ..Default::default()
        },
        subject_keys: Vec::new(),
        created_by: "counsel".into(),
        created_at: SystemTime::now(),
        released: None,
    };
    db.compliance.place_hold(&hold).await.unwrap();

    // The node's sweep does not see the held recording; it does the other.
    let expired = store.expired_recordings(&t, days_ago(10)).await.unwrap();
    assert_eq!(
        expired.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![recent.id]
    );
    let report = run_retention(&db.storage, &policy_all(t, 10), "retention-test").await;
    assert_eq!(
        (report.cdrs_deleted, report.sip_traces_deleted),
        (0, 0),
        "{:?}",
        report.errors
    );
    assert_eq!(report.held.get("sip_trace"), Some(&1));
    // A trace still under `_system` is refused through its call's tenant.
    let raw = sqlx::query("DELETE FROM sip_trace_messages")
        .execute(db.pg.pool())
        .await
        .unwrap_err();
    assert_eq!(
        raw.as_database_error().and_then(|d| d.code()).as_deref(),
        Some("HLD01")
    );

    db.finish().await;
}

// ── Recording consent (docs/COMPLIANCE_FOLLOWUPS.md §8) ────────────────

#[tokio::test]
async fn the_consent_policy_resolves_to_a_plan_and_the_outcome_reaches_the_record() {
    use fcp_core::compliance::{ConsentFailureAction, ConsentPlan, RecordingConsentMode};
    use fcp_core::prompt::{Prompt, PromptCategory, PromptStore};
    use fcp_core::RecordingConsent;
    use fcp_storage_postgres::prompt::PostgresPromptStore;

    let db = require_db!();
    let prompts = PostgresPromptStore::new(db.pg.pool().clone());
    let dir = tempfile::tempdir().unwrap();
    let tenant = TenantId::new();
    let other = TenantId::new();

    // Nothing configured: nothing to announce.
    let plan = ConsentPlan::resolve(Some(db.compliance.as_ref()), Some(&prompts), &tenant).await;
    assert_eq!(plan, ConsentPlan::not_required());

    let audio = dir.path().join("consent.wav");
    std::fs::write(&audio, b"RIFF").unwrap();
    let prompt = Prompt::new_file(
        tenant,
        "consent",
        PromptCategory::Custom,
        "en-US",
        audio.clone(),
    );
    prompts.put_prompt(prompt.clone()).await.unwrap();
    let foreign = Prompt::new_file(
        other,
        "consent",
        PromptCategory::Custom,
        "en-US",
        audio.clone(),
    );
    prompts.put_prompt(foreign.clone()).await.unwrap();

    let mut policy = CompliancePolicy::defaults(tenant);
    policy.recording_consent = RecordingConsentMode::Announce;
    policy.recording_consent_prompt_id = Some(prompt.id.to_string());
    policy.recording_consent_on_failure = ConsentFailureAction::Skip;
    policy.recording_notice_interval_secs = Some(20);
    db.compliance.put_compliance_policy(&policy).await.unwrap();
    let stored = db
        .compliance
        .get_compliance_policy(&tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.recording_consent_on_failure,
        ConsentFailureAction::Skip
    );
    assert_eq!(stored.recording_notice_interval_secs, Some(20));

    let plan = ConsentPlan::resolve(Some(db.compliance.as_ref()), Some(&prompts), &tenant).await;
    assert!(plan.announce);
    assert_eq!(plan.prompt_audio.as_deref(), Some(audio.as_path()));
    assert_eq!(plan.on_failure, ConsentFailureAction::Skip);
    assert_eq!(plan.notice_interval, Some(Duration::from_secs(20)));
    assert_eq!(plan.outcome(false), RecordingConsent::Refused);

    // A prompt of another tenant's plays nothing, nor does one whose file
    // is gone: the announcement is still owed, so it is made some other
    // way or fails.
    policy.recording_consent_prompt_id = Some(foreign.id.to_string());
    db.compliance.put_compliance_policy(&policy).await.unwrap();
    let plan = ConsentPlan::resolve(Some(db.compliance.as_ref()), Some(&prompts), &tenant).await;
    assert!(plan.announce && plan.prompt_audio.is_none());

    policy.recording_consent_prompt_id = Some(prompt.id.to_string());
    policy.recording_consent_on_failure = ConsentFailureAction::Record;
    policy.recording_notice_interval_secs = None;
    db.compliance.put_compliance_policy(&policy).await.unwrap();
    std::fs::remove_file(&audio).unwrap();
    let plan = ConsentPlan::resolve(Some(db.compliance.as_ref()), Some(&prompts), &tenant).await;
    assert!(plan.announce && plan.prompt_audio.is_none());
    assert_eq!(plan.notice_interval, None);
    assert_eq!(plan.outcome(false), RecordingConsent::Failed);

    // The record carries what the parties were told, and nothing when
    // nothing was to be recorded.
    let told = CallId::new();
    let untold = CallId::new();
    let mut with = record(tenant, told, "/rec/a.wav", days_ago(1));
    with.recording_consent = Some(RecordingConsent::Announced);
    let mut without = record(tenant, untold, "/rec/b.wav", days_ago(1));
    without.recording_url = None;
    db.storage.insert_cdrs(vec![with, without]).await.unwrap();
    let got = db.storage.get_cdr(&told).await.unwrap().unwrap();
    assert_eq!(got.recording_consent, Some(RecordingConsent::Announced));
    let got = db.storage.get_cdr(&untold).await.unwrap().unwrap();
    assert_eq!(got.recording_consent, None);

    db.finish().await;
}

// ── Subject export (docs/COMPLIANCE_FOLLOWUPS.md §6) ───────────────────

/// The bundle's files, by name.
fn unpack(bundle: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(bundle).unwrap();
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut out = std::collections::BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let name = entry.path().unwrap().to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        out.insert(name, bytes);
    }
    out
}

fn jsonl(
    files: &std::collections::BTreeMap<String, Vec<u8>>,
    class: &str,
) -> Vec<serde_json::Value> {
    files
        .get(&format!("data/{class}.jsonl"))
        .map(|b| {
            String::from_utf8_lossy(b)
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn an_export_gathers_the_subject_from_every_store_and_nobody_else() {
    use fcp_core::legal_hold::{HoldScope, LegalHold, SubjectIdentifier};
    use fcp_core::storage::{CredentialStore, TenantStore};
    use fcp_core::subject_request::{
        run_subject_requests, ExportSettings, LocalMedia, SubjectRequest, SubjectRequestKind,
        SubjectRequestStatus,
    };

    let db = require_db!();
    let root = tempfile::tempdir().unwrap();
    let pool = db.pg.pool().clone();
    let storage = db.storage.clone().with_subject_data_store(
        db.pg.clone() as Arc<dyn fcp_core::subject_request::SubjectDataStore>
    );

    let mut tenant = fcp_core::Tenant::new("export".into());
    tenant.sip_domains = vec!["pbx.example".into()];
    db.pg.put_tenant(tenant.clone()).await.unwrap();
    let t = tenant.id;
    let ts = t.to_string();
    let user = |name: &str, ext: &str, phone: Option<&str>| -> fcp_core::storage::User {
        let mut v = serde_json::json!({
            "username": name, "password": "x", "tenant_id": t, "display_name": name,
            "extension": ext, "enabled": true, "created_at": 1_789_000_000u64,
            "updated_at": 1_789_000_000u64, "email": format!("{name}@mail.example"),
            "sip_ha1_hash": "secret-ha1",
        });
        if let Some(p) = phone {
            v["phone_number"] = p.into();
        }
        serde_json::from_value(v).unwrap()
    };
    let alice = user("alice", "1001", Some("+14155550100"));
    db.pg.put_user(alice.clone()).await.unwrap();
    db.pg.put_user(user("bob", "1002", None)).await.unwrap();

    // Alice's mailbox: a greeting, a message Bob left her; one she left Bob.
    let (_, mut hers) = mailbox(&db, t, "+14155550111").await;
    hers.user_id = alice.id;
    let greeting = file(&root.path().join("vm/greetings/alice-name.wav"), 1);
    hers.greetings = vec![VoicemailGreeting::new(
        GreetingType::Name,
        PathBuf::from(&greeting),
    )];
    db.pg.put_mailbox(hers.clone()).await.unwrap();
    let from_bob = file(&root.path().join("vm/msgs/from-bob.wav"), 1);
    let m = VoicemailMessage::new(
        hers.id.clone(),
        "+14155550199".into(),
        5,
        PathBuf::from(&from_bob),
    );
    db.pg.put_message(m).await.unwrap();
    let (_, bobs) = mailbox(&db, t, "+14155550122").await;
    let to_bob = file(&root.path().join("vm/msgs/from-alice.wav"), 1);
    let left = VoicemailMessage::new(
        bobs.id.clone(),
        "4155550100".into(),
        7,
        PathBuf::from(&to_bob),
    );
    db.pg.put_message(left).await.unwrap();
    let not_hers = VoicemailMessage::new(
        bobs.id.clone(),
        "+444155550100".into(),
        7,
        PathBuf::from(&to_bob),
    );
    db.pg.put_message(not_hers).await.unwrap();

    // Calls: hers with Bob (recorded, traced), Bob's with Carol, and a
    // stranger who shares her name elsewhere.
    let hers_call = CallId::new();
    let bob_carol = CallId::new();
    let stranger = CallId::new();
    let rec = |from: &str, to: &str, call: CallId| {
        fcp_cdr::RecordBuilder::new(fcp_cdr::CdrKind::Call, call, t)
            .from(from, Some("Someone".into()))
            .to(to, None)
            .started_at(days_ago(3))
            .ended_at(Some(days_ago(3) + Duration::from_secs(60)))
            .build()
    };
    db.storage
        .insert_cdrs(vec![
            rec("sip:alice@pbx.example", "sip:bob@pbx.example", hers_call),
            rec("sip:bob@pbx.example", "sip:carol@pbx.example", bob_carol),
            rec(
                "sip:alice@elsewhere.example",
                "sip:bob@pbx.example",
                stranger,
            ),
        ])
        .await
        .unwrap();
    let recordings = db.storage.call_recording_store().unwrap();
    let her_audio = file(&root.path().join("calls/hers.wav"), 1);
    recordings
        .create_call_recording(&CallRecording::starting(
            t,
            hers_call.to_string(),
            CallRecordingSource::Api,
            her_audio,
        ))
        .await
        .unwrap();
    let their_audio = file(&root.path().join("calls/theirs.wav"), 1);
    recordings
        .create_call_recording(&CallRecording::starting(
            t,
            bob_carol.to_string(),
            CallRecordingSource::Api,
            their_audio,
        ))
        .await
        .unwrap();
    // Her call's recording file is on a host this one does not mount.
    recordings
        .create_call_recording(&CallRecording::starting(
            t,
            hers_call.to_string(),
            CallRecordingSource::Api,
            "/elsewhere/hers-2.wav",
        ))
        .await
        .unwrap();
    for (call, from) in [
        (hers_call, "sip:alice@pbx.example"),
        (bob_carol, "sip:bob@pbx.example"),
    ] {
        db.storage
            .sip_trace_store()
            .unwrap()
            .insert_sip_trace(&SipTraceMessage {
                id: 0,
                call_id: call.to_string(),
                sip_call_id: "x@y".into(),
                tenant_id: ts.clone(),
                timestamp: days_ago(3),
                direction: "inbound".into(),
                method_or_status: "INVITE".into(),
                cseq: "1 INVITE".into(),
                from_uri: from.into(),
                to_uri: "sip:bob@pbx.example".into(),
                source_addr: String::new(),
                dest_addr: String::new(),
                transport: "udp".into(),
                raw_message: "INVITE sip:bob@pbx.example SIP/2.0".into(),
                node_id: None,
            })
            .await
            .unwrap();
    }

    // A room she was recorded in.
    let mut room = Recording::starting(
        t,
        "8001",
        "node-1",
        "/conf/8001.wav",
        RecordingFormat::Wav,
        RecordingTrigger::Api,
    );
    room.participant_list = vec![
        RecordingParticipant {
            participant_id: "p1".into(),
            uri: Some("sip:1001@pbx.example".into()),
            display_name: None,
            joined_at: days_ago(2),
        },
        RecordingParticipant {
            participant_id: "p2".into(),
            uri: Some("sip:bob@pbx.example".into()),
            display_name: None,
            joined_at: days_ago(2),
        },
    ];
    db.storage
        .recording_store()
        .unwrap()
        .create_recording(&room)
        .await
        .unwrap();

    // What else names her.
    for sql in [
        format!("INSERT INTO registrations (tenant_id, aor, contact, expires_at, call_id, cseq, registered_at) VALUES ('{ts}', 'alice@pbx.example', 'sip:alice@10.0.0.5:5060', now() + interval '1 hour', 'c1', 1, now())"),
        format!("INSERT INTO registrations (tenant_id, aor, contact, expires_at, call_id, cseq, registered_at) VALUES ('{ts}', 'bob@pbx.example', 'sip:bob@10.0.0.6:5060', now() + interval '1 hour', 'c2', 1, now())"),
        format!("INSERT INTO emergency_calls (tenant_id, call_id, sip_call_id, caller_uri, caller_username, dialed_number, kind, outcome, placed_at) VALUES ('{ts}', 'call-e', 'e@x', 'sip:alice@pbx.example', 'alice', '911', 'test', 'answered', now())"),
        format!("INSERT INTO admin_audit_log (tenant_id, actor, actor_kind, method, path, status, action, occurred_at) VALUES ('{ts}', 'alice', 'session', 'PUT', '/api/v1/users/alice', 200, 'PUT /api/v1/users/:username', now())"),
        format!("INSERT INTO admin_audit_log (tenant_id, actor, actor_kind, method, path, status, action, occurred_at) VALUES ('{ts}', 'bob', 'session', 'PUT', '/api/v1/users/bob', 200, 'PUT /api/v1/users/:username', now())"),
        format!("INSERT INTO webhook_endpoints (id, tenant_id, endpoint_json, created_at, updated_at) VALUES ('wh-1', '{ts}', '{{}}', now(), now())"),
        "INSERT INTO webhook_delivery_logs (id, endpoint_id, delivery_json, delivered, created_at) VALUES ('d-1', 'wh-1', '{\"event_type\":\"call.started\",\"payload\":\"{\\\"from\\\":\\\"sip:alice@pbx.example\\\"}\"}', true, now())".to_string(),
        "INSERT INTO webhook_delivery_logs (id, endpoint_id, delivery_json, delivered, created_at) VALUES ('d-2', 'wh-1', '{\"event_type\":\"call.started\",\"payload\":\"{\\\"from\\\":\\\"sip:alice@elsewhere.example\\\"}\"}', true, now())".to_string(),
    ] {
        sqlx::query(&sql).execute(&pool).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    let mut blocked = PhoneNumber::new("+14155550177".into(), t).unwrap();
    blocked.block_list = vec!["+14155550100".into(), "+14155550999".into()];
    db.pg.put_phone_number(blocked).await.unwrap();

    // A hold on her call: exported all the same, and counted.
    db.compliance
        .place_hold(&LegalHold {
            id: fcp_core::LegalHoldId::new(),
            tenant_id: t,
            matter: "Matter 7".into(),
            reason: None,
            scope: HoldScope {
                call_ids: vec![hers_call.to_string()],
                ..Default::default()
            },
            subject_keys: Vec::new(),
            created_by: "legal".into(),
            created_at: SystemTime::now(),
            released: None,
        })
        .await
        .unwrap();

    let request = SubjectRequest::received(
        t,
        SubjectRequestKind::Export,
        vec![SubjectIdentifier::User {
            username: "alice".into(),
        }],
        "dpo",
        30,
    );
    db.compliance
        .create_subject_request(&request)
        .await
        .unwrap();
    let exports = tempfile::tempdir().unwrap();
    let settings = ExportSettings {
        exports_dir: exports.path().to_path_buf(),
        bundle_ttl: Duration::from_secs(7 * 86_400),
    };
    let media = LocalMedia {
        roots: vec![root.path().to_path_buf()],
        profile_pictures_dir: None,
    };
    let access = fcp_core::subject_request::WorkerAccess {
        media: &media,
        remover: &media,
        dead_letters: None,
    };
    let finished = run_subject_requests(&storage, access, &settings)
        .await
        .unwrap();
    assert_eq!(finished.len(), 1);
    let done = db
        .compliance
        .get_subject_request(&t, &request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        done.status,
        SubjectRequestStatus::Completed,
        "{:?}",
        done.error
    );
    let expansion = done.expansion.clone().unwrap();
    assert!(
        expansion
            .identifiers
            .contains(&"1001@pbx.example".to_string()),
        "{expansion:?}"
    );
    let bundle = done.bundle.clone().unwrap();
    let (size, sha) = fcp_core::subject_request::sha256_file(Path::new(&bundle.path))
        .await
        .unwrap();
    assert_eq!((size, sha), (bundle.size_bytes, bundle.sha256.clone()));
    assert!(!exports
        .path()
        .join(format!(".staging-{}", request.id))
        .exists());

    let files = unpack(Path::new(&bundle.path));
    let manifest: serde_json::Value = serde_json::from_slice(&files["manifest.json"]).unwrap();
    let report = done.report.clone().unwrap();
    let count = |class: &str| report.classes.get(class).map(|c| c.items).unwrap_or(0);

    let profile = jsonl(&files, "profile");
    assert_eq!(profile.len(), 1);
    assert_eq!(profile[0]["username"], "alice");
    assert!(
        !files["data/profile.jsonl"]
            .windows(10)
            .any(|w| w == b"secret-ha1"),
        "no secrets"
    );

    let cdrs = jsonl(&files, "cdrs");
    assert_eq!(cdrs.len(), 1, "only her call: {cdrs:#?}");
    assert_eq!(cdrs[0]["call_id"], serde_json::to_value(hers_call).unwrap());
    assert_eq!(report.classes["cdrs"].held, 1);
    assert_eq!(
        report.held_by.values().collect::<Vec<_>>(),
        vec!["Matter 7"]
    );

    assert_eq!(count("call_recordings"), 2);
    assert_eq!(report.classes["call_recordings"].files, 1);
    assert_eq!(report.classes["call_recordings"].missing_files.len(), 1);
    assert_eq!(count("sip_traces"), 1);
    assert_eq!(
        count("voicemail_messages"),
        2,
        "the one in her box and the one she left"
    );
    assert_eq!(count("voicemail_greetings"), 1);
    assert_eq!(count("mailboxes"), 1);
    assert!(!files["data/mailboxes.jsonl"]
        .windows(4)
        .any(|w| w == b"\"pin"));
    assert_eq!(count("conference_recordings"), 1);
    assert_eq!(
        report.classes["conference_recordings"].missing_files.len(),
        1,
        "on its node"
    );
    assert_eq!(count("registrations"), 1);
    assert_eq!(count("emergency_calls"), 1);
    assert_eq!(count("audit_events"), 1);
    assert_eq!(count("webhook_deliveries"), 1);
    assert_eq!(count("number_lists"), 1);
    assert_eq!(jsonl(&files, "number_lists")[0]["entry"], "+14155550100");

    // Every file the manifest names is in the bundle with its checksum, and
    // the recordings with others in them are marked for review.
    for f in manifest["files"].as_array().unwrap() {
        let name = f["path"].as_str().unwrap();
        let bytes = files
            .get(name)
            .unwrap_or_else(|| panic!("{name} in the bundle"));
        assert_eq!(f["bytes"], bytes.len());
    }
    assert!(manifest["not_exported"]["backups"].is_string());
    assert!(report
        .review
        .iter()
        .any(|r| r.starts_with("media/call_recordings/")));
    assert!(report
        .review
        .iter()
        .any(|r| r.starts_with("media/voicemail_messages/")));
    assert_eq!(report.review.len(), 2, "{:?}", report.review);

    // Removed once it expires.
    assert_eq!(
        fcp_core::subject_request::remove_expired_bundles(
            &storage,
            SystemTime::now() + Duration::from_secs(8 * 86_400)
        )
        .await
        .unwrap(),
        1
    );
    assert!(!Path::new(&bundle.path).exists());
    let removed = db
        .compliance
        .get_subject_request(&t, &request.id)
        .await
        .unwrap()
        .unwrap();
    assert!(removed.bundle.unwrap().removed);
    db.finish().await;
}

// ── Subject erasure (docs/COMPLIANCE_FOLLOWUPS.md §6.3-6.6) ────────────

/// Dead letters in memory.
#[derive(Default)]
struct Dead(std::sync::Mutex<Vec<(String, String)>>);

#[async_trait::async_trait]
impl fcp_core::subject_erasure::DeadLetterStore for Dead {
    async fn dead_letter_page(
        &self,
        after: Option<&str>,
        _limit: usize,
    ) -> anyhow::Result<(Vec<(String, String)>, Option<String>)> {
        let all = self.0.lock().unwrap().clone();
        Ok((
            all.into_iter()
                .filter(|(id, _)| after.is_none_or(|a| id.as_str() > a))
                .collect(),
            None,
        ))
    }

    async fn delete_dead_letters(&self, ids: &[String]) -> anyhow::Result<u64> {
        let mut all = self.0.lock().unwrap();
        let before = all.len();
        all.retain(|(id, _)| !ids.contains(id));
        Ok((before - all.len()) as u64)
    }
}

/// Removes conference recordings' files as a node would, noting which.
#[derive(Default)]
struct Nodes(std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl fcp_core::subject_erasure::MediaRemover for Nodes {
    async fn remove(
        &self,
        _tenant: &TenantId,
        _class: fcp_core::subject_request::ExportClass,
        row: &fcp_core::subject_request::SubjectRow,
    ) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .push(row.file.clone().unwrap_or_default());
        Ok(())
    }
}

#[tokio::test]
async fn an_erasure_previews_then_deletes_pseudonymises_and_keeps_what_it_must() {
    use fcp_core::legal_hold::{HoldScope, LegalHold, SubjectIdentifier};
    use fcp_core::storage::{CredentialStore, TenantStore};
    use fcp_core::subject_request::{
        run_subject_requests, ExportSettings, LocalMedia, SubjectRequest, SubjectRequestKind,
        SubjectRequestStatus, WorkerAccess,
    };

    let db = require_db!();
    let root = tempfile::tempdir().unwrap();
    let pool = db.pg.pool().clone();
    let storage = db
        .storage
        .clone()
        .with_subject_data_store(
            db.pg.clone() as Arc<dyn fcp_core::subject_request::SubjectDataStore>
        )
        .with_subject_erase_store(
            db.pg.clone() as Arc<dyn fcp_core::subject_erasure::SubjectEraseStore>
        );
    let scalar = |sql: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(&sql)
                .fetch_one(&pool)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"))
        }
    };
    let text = |sql: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>(&sql)
                .fetch_one(&pool)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"))
        }
    };
    let exec = |sql: String| {
        let pool = pool.clone();
        async move {
            sqlx::query(&sql)
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
    };

    let mut tenant = fcp_core::Tenant::new("erasure".into());
    tenant.sip_domains = vec!["pbx.example".into()];
    db.pg.put_tenant(tenant.clone()).await.unwrap();
    let t = tenant.id;
    let ts = t.to_string();
    let user = |name: &str, ext: &str| -> fcp_core::storage::User {
        serde_json::from_value(serde_json::json!({
            "username": name, "password": "x", "tenant_id": t, "display_name": name,
            "extension": ext, "enabled": true, "created_at": 1_789_000_000u64,
            "updated_at": 1_789_000_000u64, "email": format!("{name}@mail.example"),
            "phone_number": if name == "alice" { "+14155550100" } else { "+14155550200" },
        }))
        .unwrap()
    };
    let alice = user("alice", "1001");
    db.pg.put_user(alice.clone()).await.unwrap();
    db.pg.put_user(user("bob", "1002")).await.unwrap();
    exec("UPDATE cdr_archive_state SET next_day = current_date + 1".into()).await;

    // Her mailbox with a greeting and a message; one she left Bob.
    let (_, mut hers) = mailbox(&db, t, "+14155550111").await;
    hers.user_id = alice.id;
    let greeting = file(&root.path().join("vm/alice-name.wav"), 1);
    hers.greetings = vec![VoicemailGreeting::new(
        GreetingType::Name,
        PathBuf::from(&greeting),
    )];
    db.pg.put_mailbox(hers.clone()).await.unwrap();
    db.pg
        .put_message(VoicemailMessage::new(
            hers.id.clone(),
            "+14155550199".into(),
            5,
            file(&root.path().join("vm/in-hers.wav"), 1).into(),
        ))
        .await
        .unwrap();
    let (_, bobs) = mailbox(&db, t, "+14155550122").await;
    let left = VoicemailMessage::new(
        bobs.id.clone(),
        "4155550100".into(),
        7,
        file(&root.path().join("vm/left.wav"), 1).into(),
    );
    db.pg.put_message(left.clone()).await.unwrap();
    let bobs_own = VoicemailMessage::new(
        bobs.id.clone(),
        "+14155550188".into(),
        7,
        file(&root.path().join("vm/other.wav"), 1).into(),
    );
    db.pg.put_message(bobs_own.clone()).await.unwrap();

    // Calls: two of hers (one held), Bob's with Carol.
    let hers_call = CallId::new();
    let held_call = CallId::new();
    let theirs = CallId::new();
    let rec = |from: &str, to: &str, call: CallId| {
        let mut r = fcp_cdr::RecordBuilder::new(fcp_cdr::CdrKind::Call, call, t)
            .from(from, Some("Alice Smith".into()))
            .to(to, Some("Bob".into()))
            .started_at(SystemTime::now() - Duration::from_secs(3600))
            .build();
        r.custom_data = Some(serde_json::json!({ "account": "A-1", "callback": "+14155550100" }));
        r
    };
    db.storage
        .insert_cdrs(vec![
            rec(
                "\"Alice\" <sip:alice@pbx.example>",
                "sip:bob@pbx.example",
                hers_call,
            ),
            rec("sip:alice@pbx.example", "sip:bob@pbx.example", held_call),
            rec("sip:bob@pbx.example", "sip:carol@pbx.example", theirs),
        ])
        .await
        .unwrap();
    exec("DELETE FROM cdr_archive_late".into()).await;
    let audio = file(&root.path().join("calls/hers.wav"), 1);
    db.storage
        .call_recording_store()
        .unwrap()
        .create_call_recording(&CallRecording::starting(
            t,
            hers_call.to_string(),
            CallRecordingSource::Api,
            audio.clone(),
        ))
        .await
        .unwrap();
    let mut room = Recording::starting(
        t,
        "8001",
        "node-1",
        "/conf/8001.wav",
        RecordingFormat::Wav,
        RecordingTrigger::Api,
    );
    room.participant_list = vec![RecordingParticipant {
        participant_id: "p1".into(),
        uri: Some("sip:alice@pbx.example".into()),
        display_name: None,
        joined_at: SystemTime::now(),
    }];
    db.storage
        .recording_store()
        .unwrap()
        .create_recording(&room)
        .await
        .unwrap();
    for sql in [
        format!("INSERT INTO api_tokens (id, name, token_hash, prefix, user_id, tenant_id, permissions, created_at) VALUES ('token-a', 'cli', 'h1', 'fcp_abcd', '{}', '{ts}', '[]', now())", alice.id),
        format!("INSERT INTO registrations (tenant_id, aor, contact, expires_at, call_id, cseq, registered_at) VALUES ('{ts}', 'alice@pbx.example', 'sip:alice@10.0.0.5', now() + interval '1 hour', 'c1', 1, now())"),
        format!("INSERT INTO registrations (tenant_id, aor, contact, expires_at, call_id, cseq, registered_at) VALUES ('{ts}', 'bob@pbx.example', 'sip:bob@10.0.0.6', now() + interval '1 hour', 'c2', 1, now())"),
        format!("INSERT INTO emergency_calls (tenant_id, call_id, sip_call_id, caller_uri, caller_username, dialed_number, kind, outcome, placed_at) VALUES ('{ts}', 'call-e', 'e@x', 'sip:alice@pbx.example', 'alice', '911', 'test', 'answered', now())"),
        format!("INSERT INTO admin_audit_log (tenant_id, actor, actor_kind, method, path, status, action, occurred_at) VALUES ('{ts}', 'alice', 'session', 'PUT', '/api/v1/users/alice', 200, 'PUT /api/v1/users/:username', now())"),
        format!("INSERT INTO webhook_endpoints (id, tenant_id, endpoint_json, created_at, updated_at) VALUES ('wh-e', '{ts}', '{{}}', now(), now())"),
        "INSERT INTO webhook_delivery_logs (id, endpoint_id, delivery_json, delivered, created_at) VALUES ('d-a', 'wh-e', '{\"payload\":\"{\\\"from\\\":\\\"sip:alice@pbx.example\\\"}\"}', true, now())".to_string(),
        format!("INSERT INTO port_requests (id, tenant_id, status, created_at, updated_at, request_json) VALUES ('port-1', '{ts}', 'submitted', now(), now(), '{{\"authorized_contact_email\": \"alice@mail.example\", \"pin\": \"9999\"}}')"),
    ] {
        exec(sql).await;
    }
    let mut blocked = PhoneNumber::new("+14155550177".into(), t).unwrap();
    blocked.block_list = vec!["+14155550100".into(), "+14155550999".into()];
    db.pg.put_phone_number(blocked.clone()).await.unwrap();

    db.compliance
        .place_hold(&LegalHold {
            id: fcp_core::LegalHoldId::new(),
            tenant_id: t,
            matter: "Matter 9".into(),
            reason: None,
            scope: HoldScope {
                call_ids: vec![held_call.to_string()],
                ..Default::default()
            },
            subject_keys: Vec::new(),
            created_by: "legal".into(),
            created_at: SystemTime::now(),
            released: None,
        })
        .await
        .unwrap();

    let dead = Dead::default();
    let tenant_json = serde_json::to_string(&t).unwrap();
    dead.0.lock().unwrap().extend([
        (
            "1-0".to_string(),
            format!("{{\"tenant_id\":{tenant_json},\"from_uri\":\"sip:alice@pbx.example\"}}"),
        ),
        (
            "2-0".to_string(),
            format!("{{\"tenant_id\":{tenant_json},\"from_uri\":\"sip:bob@pbx.example\"}}"),
        ),
    ]);
    let nodes = Nodes::default();
    let media = LocalMedia {
        roots: vec![root.path().to_path_buf()],
        profile_pictures_dir: None,
    };
    let access = WorkerAccess {
        media: &media,
        remover: &nodes,
        dead_letters: Some(&dead),
    };
    let settings = ExportSettings {
        exports_dir: root.path().join("exports"),
        bundle_ttl: Duration::from_secs(86_400),
    };

    let request = SubjectRequest::received(
        t,
        SubjectRequestKind::Erasure,
        vec![SubjectIdentifier::User {
            username: "alice".into(),
        }],
        "dpo",
        30,
    );
    db.compliance
        .create_subject_request(&request)
        .await
        .unwrap();

    // ── The preview: a report, and nothing changed.
    run_subject_requests(&storage, access, &settings)
        .await
        .unwrap();
    let previewed = db
        .compliance
        .get_subject_request(&t, &request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        previewed.status,
        SubjectRequestStatus::Preview,
        "{:?}",
        previewed.error
    );
    let preview = previewed.preview.clone().unwrap();
    assert!(preview.preview);
    assert_eq!(preview.classes["cdrs"].items, 1, "{preview:#?}");
    assert_eq!(preview.classes["cdrs"].held, 1);
    assert_eq!(preview.classes["cdrs"].treatment, "pseudonymise");
    assert_eq!(preview.classes["voicemail_messages"].items, 2);
    assert_eq!(
        preview.kept["port_requests"],
        "a carrier's record of a number port"
    );
    assert_eq!(preview.dead_letters.as_ref().unwrap().items, 1);
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM users WHERE tenant_id = '{ts}'"
        ))
        .await,
        2
    );
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM cdrs WHERE tenant_id = '{ts}' AND from_uri LIKE '%alice%'"
        ))
        .await,
        2
    );
    assert_eq!(dead.0.lock().unwrap().len(), 2);
    assert!(nodes.0.lock().unwrap().is_empty());

    // ── Confirmed, it runs against the data as it is.
    let mut confirmed = previewed;
    confirmed.status = SubjectRequestStatus::Confirmed;
    confirmed.confirmed_at = Some(SystemTime::now());
    confirmed.confirmed_by = Some("dpo-2".into());
    db.compliance
        .update_subject_request(&confirmed)
        .await
        .unwrap();
    run_subject_requests(&storage, access, &settings)
        .await
        .unwrap();
    let done = db
        .compliance
        .get_subject_request(&t, &request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        done.status,
        SubjectRequestStatus::Completed,
        "{:?}",
        done.error
    );
    let report = done.erasure.clone().unwrap();
    assert!(!report.preview);
    assert!(
        report.classes.values().all(|c| c.errors.is_empty()),
        "{report:#?}"
    );

    // Deleted.
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM users WHERE tenant_id = '{ts}'"
        ))
        .await,
        1,
        "Bob stays"
    );
    assert_eq!(
        scalar("SELECT count(*) FROM api_tokens WHERE id = 'token-a'".into()).await,
        0
    );
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM voicemail_mailboxes WHERE id = '{}'",
            hers.id
        ))
        .await,
        0
    );
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM voicemail_messages WHERE id = '{}'",
            left.id
        ))
        .await,
        0
    );
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM voicemail_messages WHERE id = '{}'",
            bobs_own.id
        ))
        .await,
        1
    );
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM call_recordings WHERE tenant_id = '{ts}'"
        ))
        .await,
        0
    );
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM pending_file_deletions WHERE path = '{audio}'"
        ))
        .await,
        1,
        "its file is queued"
    );
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM conference_recordings WHERE tenant_id = '{ts}'"
        ))
        .await,
        0
    );
    assert_eq!(nodes.0.lock().unwrap().as_slice(), ["/conf/8001.wav"]);
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM registrations WHERE tenant_id = '{ts}'"
        ))
        .await,
        1,
        "Bob's stays"
    );
    assert_eq!(
        scalar("SELECT count(*) FROM webhook_delivery_logs WHERE id = 'd-a'".into()).await,
        0
    );
    let list = text(format!(
        "SELECT block_list::text FROM phone_numbers WHERE id = '{}'",
        blocked.id
    ))
    .await
    .unwrap();
    assert_eq!(list, "[\"+14155550999\"]");
    assert_eq!(dead.0.lock().unwrap().len(), 1);

    // Pseudonymised: her side only, one token everywhere, the form kept.
    let from = text(format!(
        "SELECT from_uri FROM cdrs WHERE call_id = '{hers_call}'"
    ))
    .await
    .unwrap();
    assert!(
        from.starts_with("<sip:erased-") && from.ends_with("@pbx.example>"),
        "{from}"
    );
    let token = from
        .trim_start_matches("<sip:")
        .split('@')
        .next()
        .unwrap()
        .to_string();
    assert_eq!(
        text(format!(
            "SELECT to_uri FROM cdrs WHERE call_id = '{hers_call}'"
        ))
        .await
        .unwrap(),
        "sip:bob@pbx.example"
    );
    assert!(text(format!(
        "SELECT from_display_name FROM cdrs WHERE call_id = '{hers_call}'"
    ))
    .await
    .is_none());
    assert_eq!(
        text(format!(
            "SELECT to_display_name FROM cdrs WHERE call_id = '{hers_call}'"
        ))
        .await
        .unwrap(),
        "Bob"
    );
    assert_eq!(
        text(format!(
            "SELECT sip_call_id FROM cdrs WHERE call_id = '{hers_call}'"
        ))
        .await
        .unwrap(),
        "erased"
    );
    assert_eq!(
        text(format!(
            "SELECT custom_data::text FROM cdrs WHERE call_id = '{hers_call}'"
        ))
        .await
        .unwrap(),
        "{\"account\": \"A-1\"}"
    );
    assert_eq!(
        text(format!(
            "SELECT caller_uri FROM emergency_calls WHERE tenant_id = '{ts}'"
        ))
        .await
        .unwrap(),
        format!("sip:{token}@pbx.example")
    );
    assert_eq!(
        text(format!(
            "SELECT caller_username FROM emergency_calls WHERE tenant_id = '{ts}'"
        ))
        .await
        .unwrap(),
        token
    );
    assert_eq!(
        text(format!(
            "SELECT actor FROM admin_audit_log WHERE tenant_id = '{ts}' AND method = 'PUT'"
        ))
        .await
        .unwrap(),
        token
    );
    assert_eq!(
        text(format!(
            "SELECT path FROM admin_audit_log WHERE tenant_id = '{ts}' AND method = 'PUT'"
        ))
        .await
        .unwrap(),
        format!("/api/v1/users/{token}")
    );
    // Its day is written again by the archive.
    assert_eq!(
        scalar(format!(
            "SELECT count(*) FROM cdr_archive_late WHERE tenant_id = '{ts}'"
        ))
        .await,
        1
    );

    // Kept: the held call as it was, the port request, Bob's call.
    assert_eq!(
        text(format!(
            "SELECT from_uri FROM cdrs WHERE call_id = '{held_call}'"
        ))
        .await
        .unwrap(),
        "sip:alice@pbx.example"
    );
    assert_eq!(report.classes["cdrs"].held, 1);
    assert_eq!(
        report.held_by.values().collect::<Vec<_>>(),
        vec!["Matter 9"]
    );
    assert_eq!(
        scalar("SELECT count(*) FROM port_requests WHERE id = 'port-1'".into()).await,
        1
    );
    assert_eq!(
        text(format!(
            "SELECT from_uri FROM cdrs WHERE call_id = '{theirs}'"
        ))
        .await
        .unwrap(),
        "sip:bob@pbx.example"
    );
    assert!(report.not_reached.contains_key("backups"));

    // Every deletion is logged under the request, as an erasure by the person
    // who confirmed it.
    let logged = db
        .compliance
        .list_deletions(&DeletionQuery {
            before: None,
            request_id: Some(request.id.to_string()),
            limit: 200,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        logged
            .iter()
            .all(|r| r.cause == "erasure" && r.actor == "dpo-2"),
        "{logged:#?}"
    );
    for class in [
        "call_recording",
        "voicemail",
        "profile",
        "registrations",
        "cdrs",
    ] {
        assert!(
            logged.iter().any(|r| r.class == class),
            "{class} logged: {logged:#?}"
        );
    }

    // A later request for her gets the same pseudonym.
    let again = fcp_core::subject_erasure::SubjectEraseStore::pseudonym_secret(db.pg.as_ref(), &t)
        .await
        .unwrap();
    assert_eq!(
        fcp_core::subject_erasure::pseudonym(&again, "user:alice"),
        token
    );
    db.finish().await;
}

#[tokio::test]
async fn a_tenants_deletion_removes_every_row_and_then_the_tenant() {
    use fcp_core::storage::{CredentialStore, TenantStore};
    use fcp_core::subject_request::{
        run_subject_requests, ExportSettings, LocalMedia, SubjectRequest, SubjectRequestKind,
        SubjectRequestStatus, WorkerAccess,
    };

    let db = require_db!();
    let root = tempfile::tempdir().unwrap();
    let pool = db.pg.pool().clone();
    let storage = db
        .storage
        .clone()
        .with_subject_data_store(
            db.pg.clone() as Arc<dyn fcp_core::subject_request::SubjectDataStore>
        )
        .with_subject_erase_store(
            db.pg.clone() as Arc<dyn fcp_core::subject_erasure::SubjectEraseStore>
        );
    let count = |sql: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(&sql)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };

    let gone = fcp_core::Tenant::new("leaving".into());
    let stays = fcp_core::Tenant::new("staying".into());
    db.pg.put_tenant(gone.clone()).await.unwrap();
    db.pg.put_tenant(stays.clone()).await.unwrap();
    for tenant in [gone.id, stays.id] {
        let user: fcp_core::storage::User = serde_json::from_value(serde_json::json!({
            "username": "u", "password": "x", "tenant_id": tenant, "enabled": true,
            "created_at": 1_789_000_000u64, "updated_at": 1_789_000_000u64,
        }))
        .unwrap();
        db.pg.put_user(user).await.unwrap();
        let (_, mb) = mailbox(
            &db,
            tenant,
            if tenant == gone.id {
                "+14155551001"
            } else {
                "+14155552001"
            },
        )
        .await;
        db.pg
            .put_message(VoicemailMessage::new(
                mb.id.clone(),
                "+1".into(),
                1,
                file(&root.path().join(format!("{tenant}.wav")), 1).into(),
            ))
            .await
            .unwrap();
        let call = CallId::new();
        db.storage
            .insert_cdrs(vec![record(tenant, call, "", days_ago(1))])
            .await
            .unwrap();
        let ts = tenant.to_string();
        sqlx::query(&format!(
            "INSERT INTO sip_trace_messages (call_id, sip_call_id, tenant_id, timestamp, direction, method_or_status, cseq, from_uri, to_uri, source_addr, dest_addr, transport, raw_message)
             VALUES ('{call}', 'x', '_system', now(), 'in', 'INVITE', '1', 'a', 'b', '', '', 'udp', '')"
        ))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(&format!("INSERT INTO admin_audit_log (tenant_id, actor, actor_kind, method, path, status, action, occurred_at) VALUES ('{ts}', 'u', 'session', 'PUT', '/x', 200, 'PUT /x', now())"))
            .execute(&pool)
            .await
            .unwrap();
        let mut room = Recording::starting(
            tenant,
            "8001",
            "node-1",
            format!("/conf/{ts}.wav"),
            RecordingFormat::Wav,
            RecordingTrigger::Api,
        );
        room.participants = 1;
        db.storage
            .recording_store()
            .unwrap()
            .create_recording(&room)
            .await
            .unwrap();
    }

    let mut request = SubjectRequest::received(
        gone.id,
        SubjectRequestKind::TenantDeletion,
        Vec::new(),
        "platform",
        30,
    );
    request.status = SubjectRequestStatus::Confirmed;
    request.confirmed_at = Some(SystemTime::now());
    db.compliance
        .create_subject_request(&request)
        .await
        .unwrap();
    let nodes = Nodes::default();
    let media = LocalMedia::default();
    let access = WorkerAccess {
        media: &media,
        remover: &nodes,
        dead_letters: None,
    };
    let settings = ExportSettings {
        exports_dir: root.path().join("exports"),
        bundle_ttl: Duration::from_secs(60),
    };
    run_subject_requests(&storage, access, &settings)
        .await
        .unwrap();

    let done = db
        .compliance
        .get_subject_request(&gone.id, &request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        done.status,
        SubjectRequestStatus::Completed,
        "{:?}",
        done.error
    );
    let report = done.erasure.unwrap();
    assert!(db.pg.get_tenant(&gone.id).await.unwrap().is_none());
    assert!(db.pg.get_tenant(&stays.id).await.unwrap().is_some());
    assert_eq!(report.tables["tenants"], 1);
    assert_eq!(
        nodes.0.lock().unwrap().as_slice(),
        [format!("/conf/{}.wav", gone.id)]
    );
    for (table, column) in [
        ("users", "tenant_id"),
        ("cdrs", "tenant_id"),
        ("voicemail_mailboxes", "tenant_id"),
        ("voicemail_messages", "tenant_id"),
        ("phone_numbers", "tenant_id"),
        ("admin_audit_log", "tenant_id"),
        ("conference_recordings", "tenant_id"),
    ] {
        assert_eq!(
            count(format!(
                "SELECT count(*) FROM {table} WHERE {column} = '{}'",
                gone.id
            ))
            .await,
            0,
            "{table} emptied"
        );
        assert_eq!(
            count(format!(
                "SELECT count(*) FROM {table} WHERE {column} = '{}'",
                stays.id
            ))
            .await,
            1,
            "{table} of another tenant kept"
        );
    }
    assert_eq!(
        count("SELECT count(*) FROM sip_trace_messages".into()).await,
        1,
        "the other tenant's call's trace stays"
    );
    assert!(count(format!("SELECT count(*) FROM deletion_log WHERE cause = 'tenant_deletion' AND request_id = '{}'", request.id)).await > 0);
    assert_eq!(
        count(format!(
            "SELECT count(*) FROM subject_requests WHERE id = '{}'",
            request.id
        ))
        .await,
        1,
        "the request itself stays"
    );
    db.finish().await;
}

/// A mailbox written while its greeting is stored: both replace the
/// mailbox's greetings, and a greeting stored between the mailbox's delete
/// and its insert made the insert a duplicate and the write a 500 (the
/// admin API's mailbox PUT racing a renamed owner's personalized greeting,
/// #343). Both lock the mailbox's row first now.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mailbox_written_while_its_greeting_is_stored_is_written() {
    let db = require_db!();
    let tenant = TenantId::new();
    let (_, mut mb) = mailbox(&db, tenant, "+14155550142").await;
    let greeting = VoicemailGreeting::new(GreetingType::Personalized, PathBuf::from("/g/p.wav"));
    mb.add_greeting(greeting.clone());
    db.pg.put_mailbox(mb.clone()).await.unwrap();

    let writer = {
        let (pg, mb) = (Arc::clone(&db.pg), mb.clone());
        tokio::spawn(async move {
            let mut failed = Vec::new();
            for _ in 0..200 {
                if let Err(e) = pg.put_mailbox(mb.clone()).await {
                    failed.push(e.to_string());
                }
            }
            failed
        })
    };
    let greeter = {
        let (pg, id) = (Arc::clone(&db.pg), mb.id.clone());
        tokio::spawn(async move {
            let mut failed = Vec::new();
            for _ in 0..200 {
                if let Err(e) = pg
                    .set_greeting(&id, GreetingType::Personalized, Some(greeting.clone()))
                    .await
                {
                    failed.push(e.to_string());
                }
            }
            failed
        })
    };
    let (written, greeted) = (writer.await.unwrap(), greeter.await.unwrap());
    assert!(written.is_empty(), "put_mailbox failed: {written:?}");
    assert!(greeted.is_empty(), "set_greeting failed: {greeted:?}");
    db.finish().await;
}
