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
        Deleter::new(DeletionCause::Manual, "alice"),
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
    // The number goes, and with it the binding, the mailbox, the message.
    deleting_as(
        Deleter::new(DeletionCause::Manual, "bob"),
        db.pg.delete_phone_number(&number.id),
    )
    .await
    .unwrap();
    let voicemail = db
        .compliance
        .list_deletions(&DeletionQuery {
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
        violation(db.pg.delete_phone_number(&number.id).await.unwrap_err()),
        Some(hold.id.to_string()),
        "the number's cascade reaches a held message"
    );
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
