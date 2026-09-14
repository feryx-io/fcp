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
