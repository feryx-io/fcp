//! The CDR archive (`docs/CDR_PIPELINE.md` §13, phase 5), against a real
//! database: every closed day of every tenant's records written once,
//! confirmed, and announced; a day that gains a record written again; a
//! failed write held unconfirmed and retried; the tenant's frontier moving
//! only with what is confirmed; and the role in the running `fcp-jobs`.
//! Skipped without `FCP_TEST_DATABASE_URL` (and `FCP_TEST_REDIS_URL` for the
//! binary).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use chrono::{Duration as Days, NaiveDate, Utc};
use fcp_cdr::archive::{
    sha256_file, ArchiveConfig, ArchiveError, ArchiveRun, ArchiveTarget, Archiver, DirectoryTarget,
    ObjectInfo, ARCHIVE_LOCK,
};
use fcp_cdr::{CdrKind, Record, RecordBuilder};
use fcp_core::cdr_archive::{day_start, object_key};
use fcp_core::storage::CdrStore;
use fcp_core::{CallId, TenantId};
use fcp_events::{CdrEvent, EventPublisher, PlatformEvent};
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{free_port, Service, TestDatabase};

const JOBS: &str = env!("CARGO_BIN_EXE_fcp-jobs");

#[derive(Default)]
struct Events(Mutex<Vec<PlatformEvent>>);

#[async_trait]
impl EventPublisher for Events {
    async fn publish(&self, event: PlatformEvent) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

impl Events {
    fn archived(&self) -> Vec<(TenantId, String, u64, u64, String)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PlatformEvent::Cdr(CdrEvent::Archived {
                    tenant_id,
                    day,
                    records,
                    generation,
                    sha256,
                    ..
                }) => Some((
                    *tenant_id,
                    day.clone(),
                    *records,
                    *generation,
                    sha256.clone(),
                )),
                _ => None,
            })
            .collect()
    }
}

/// A target that refuses every write while `down` is set.
struct Flaky {
    inner: DirectoryTarget,
    down: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl ArchiveTarget for Flaky {
    fn location(&self) -> String {
        self.inner.location()
    }
    async fn put(&self, key: &str, file: &Path, object: &ObjectInfo) -> Result<(), ArchiveError> {
        if self.down.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ArchiveError::Target("the target is down".into()));
        }
        self.inner.put(key, file, object).await
    }
    async fn confirm(&self, key: &str, object: &ObjectInfo) -> Result<bool, ArchiveError> {
        self.inner.confirm(key, object).await
    }
}

fn record(tenant: TenantId, start: SystemTime) -> Record {
    RecordBuilder::new(CdrKind::Call, CallId::new(), tenant)
        .from("sip:alice@archive.example", None)
        .to("sip:+14155550100@carrier.example", None)
        .started_at(start)
        .answered_at(Some(start + Duration::from_secs(2)))
        .ended_at(Some(start + Duration::from_secs(62)))
        .build()
}

/// The records in an object, in order.
fn read_object(path: &Path) -> Vec<Record> {
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn storage(url: &str) -> Arc<PostgresStorage> {
    Arc::new(
        PostgresStorage::new(PostgresConfig::new(url))
            .await
            .unwrap(),
    )
}

#[tokio::test]
async fn every_closed_day_is_written_once_confirmed_and_written_again_when_it_gains_records() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let pg = storage(db.url()).await;
    let store: Arc<dyn CdrStore> = pg.clone();
    let archive = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let (a, b) = (TenantId::new(), TenantId::new());
    let today = Utc::now().date_naive();
    let day = |n: i64| today - Days::days(n);
    let at = |d: NaiveDate, secs: u64| day_start(d) + Duration::from_secs(secs);

    // a on each of the last three days, twice on the oldest; b on the
    // middle one; and a call today, whose day is not closed.
    let mut seeded = vec![
        record(a, at(day(3), 3_600)),
        record(a, at(day(3), 60)),
        record(a, at(day(2), 86_399)),
        record(a, at(day(1), 0)),
        record(b, at(day(2), 7_200)),
        record(a, SystemTime::now() - Duration::from_secs(1)),
    ];
    store.insert_cdrs(seeded.clone()).await.unwrap();

    let target = Arc::new(Flaky {
        inner: DirectoryTarget::new(archive.path()),
        down: false.into(),
    });
    let events = Arc::new(Events::default());
    let config = ArchiveConfig {
        grace: Duration::ZERO,
        prefix: "cdrs/".into(),
        page: 1,
        work_dir: work.path().to_path_buf(),
        ..ArchiveConfig::default()
    };
    let archiver = Archiver::new(Arc::clone(&store), target.clone(), config.clone())
        .with_events(events.clone());
    let path =
        |t: &TenantId, d: NaiveDate| -> PathBuf { archive.path().join(object_key("cdrs/", t, d)) };

    let run = archiver
        .run_once()
        .await
        .unwrap()
        .expect("the lock is free");
    assert_eq!(
        run,
        ArchiveRun {
            days: 3,
            objects: 4,
            records: 5,
            bytes: run.bytes,
            failures: 0,
            ..ArchiveRun::default()
        }
    );

    // Each object holds its tenant's day, oldest first, and matches what
    // the store recorded and what was announced.
    seeded.sort_by_key(|r| r.start_time);
    for (tenant, d, expected) in [
        (a, day(3), 2),
        (a, day(2), 1),
        (a, day(1), 1),
        (b, day(2), 1),
    ] {
        let lines = read_object(&path(&tenant, d));
        assert_eq!(lines.len(), expected, "{tenant} {d}");
        let ids: Vec<_> = seeded
            .iter()
            .filter(|r| r.tenant_id == tenant && fcp_core::cdr_archive::day_of(r.start_time) == d)
            .map(|r| r.cdr_id)
            .collect();
        assert_eq!(lines.iter().map(|r| r.cdr_id).collect::<Vec<_>>(), ids);
        let entry = store.get_cdr_archive(&tenant, d).await.unwrap().unwrap();
        assert!(entry.archived_at.is_some() && !entry.late);
        assert_eq!(entry.records, expected as u64);
        assert_eq!(
            entry.sha256.as_deref(),
            Some(sha256_file(&path(&tenant, d)).await.unwrap().as_str())
        );
    }
    assert!(!path(&a, today).exists(), "today is not closed");
    assert_eq!(events.archived().len(), 4);
    assert!(events
        .archived()
        .iter()
        .all(|(_, _, _, generation, _)| *generation == 1));
    assert_eq!(
        store.cdr_archive_frontier(&a).await.unwrap(),
        Some(day_start(today))
    );
    assert!(
        std::fs::read_dir(work.path()).unwrap().next().is_none(),
        "nothing left in the work dir"
    );

    // Nothing new: nothing written.
    assert_eq!(
        archiver.run_once().await.unwrap(),
        Some(ArchiveRun::default())
    );

    // Another host holds the lock: this one does nothing.
    let lock = store
        .try_maintenance_lock(ARCHIVE_LOCK)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archiver.run_once().await.unwrap(), None);
    lock.release().await;

    // A late record for an archived day holds a's frontier back until the
    // day is written again, with it.
    let late = record(a, at(day(2), 43_200));
    store.insert_cdrs(vec![late.clone()]).await.unwrap();
    assert_eq!(
        store.cdr_archive_frontier(&a).await.unwrap(),
        Some(day_start(day(2)))
    );
    assert_eq!(
        store.cdr_archive_frontier(&b).await.unwrap(),
        Some(day_start(today))
    );
    let run = archiver.run_once().await.unwrap().unwrap();
    assert_eq!((run.objects, run.records, run.days), (1, 2, 0));
    let lines = read_object(&path(&a, day(2)));
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].cdr_id, late.cdr_id, "oldest first");
    let entry = store.get_cdr_archive(&a, day(2)).await.unwrap().unwrap();
    assert_eq!((entry.generation, entry.records), (2, 2));
    assert!(events.archived().contains(&(
        a,
        day(2).to_string(),
        2,
        2,
        entry.sha256.clone().unwrap()
    )));
    assert_eq!(
        store.cdr_archive_frontier(&a).await.unwrap(),
        Some(day_start(today))
    );

    // The target goes down while a day gains a record: the write fails, the
    // day stays unconfirmed and the old object stays in place; when the
    // target is back, the next run writes it.
    target.down.store(true, std::sync::atomic::Ordering::SeqCst);
    store
        .insert_cdrs(vec![record(b, at(day(3), 500))])
        .await
        .unwrap();
    let run = archiver.run_once().await.unwrap().unwrap();
    assert_eq!((run.objects, run.failures), (0, 1));
    let entry = store.get_cdr_archive(&b, day(3)).await.unwrap().unwrap();
    assert!(entry.archived_at.is_none());
    assert_eq!(
        store.cdr_archive_frontier(&b).await.unwrap(),
        Some(day_start(day(3)))
    );
    assert!(
        std::fs::read_dir(work.path()).unwrap().next().is_none(),
        "the failed build is removed"
    );
    target
        .down
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let run = archiver.run_once().await.unwrap().unwrap();
    assert_eq!((run.objects, run.failures), (1, 0));
    assert_eq!(read_object(&path(&b, day(3))).len(), 1);
    assert_eq!(
        store.cdr_archive_frontier(&b).await.unwrap(),
        Some(day_start(today))
    );

    let status = store.cdr_archive_status().await.unwrap();
    assert_eq!(status.next_day, Some(today));
    assert_eq!(
        (status.archived_days, status.pending_days, status.records),
        (5, 0, 7)
    );
    let stats = archiver.stats().snapshot();
    assert_eq!((stats.objects, stats.failures), (6, 1));

    drop(archiver);
    drop(store);
    pg.close().await;
    db.drop_db().await;
}

/// A directory target that records the object locks it is asked for.
struct Locking {
    inner: DirectoryTarget,
    locks: Mutex<Vec<(String, bool)>>,
}

#[async_trait]
impl ArchiveTarget for Locking {
    fn location(&self) -> String {
        self.inner.location()
    }
    async fn put(&self, key: &str, file: &Path, object: &ObjectInfo) -> Result<(), ArchiveError> {
        self.inner.put(key, file, object).await
    }
    async fn confirm(&self, key: &str, object: &ObjectInfo) -> Result<bool, ArchiveError> {
        self.inner.confirm(key, object).await
    }
    async fn set_legal_hold(&self, key: &str, on: bool) -> Result<bool, ArchiveError> {
        self.locks.lock().unwrap().push((key.to_string(), on));
        Ok(true)
    }
}

#[tokio::test]
async fn a_legal_hold_locks_its_archived_days_until_it_is_released() {
    use fcp_core::compliance::ComplianceStore;
    use fcp_core::legal_hold::{HoldRelease, HoldScope, LegalHold};
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let pg = storage(db.url()).await;
    let store: Arc<dyn CdrStore> = pg.clone();
    let compliance =
        fcp_storage_postgres::compliance::PostgresComplianceStore::new(pg.pool().clone());
    let archive = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let tenant = TenantId::new();
    let today = Utc::now().date_naive();
    let day = |n: i64| today - Days::days(n);
    store
        .insert_cdrs(vec![
            record(tenant, day_start(day(3)) + Duration::from_secs(60)),
            record(tenant, day_start(day(2)) + Duration::from_secs(60)),
        ])
        .await
        .unwrap();

    // Held from the start of day 2 onwards.
    let hold = LegalHold {
        id: fcp_core::LegalHoldId::new(),
        tenant_id: tenant,
        matter: "Case 7".into(),
        reason: None,
        scope: HoldScope {
            from: Some(day_start(day(2))),
            ..Default::default()
        },
        subject_keys: Vec::new(),
        created_by: "counsel".into(),
        created_at: SystemTime::now(),
        released: None,
    };
    compliance.place_hold(&hold).await.unwrap();

    let target = Arc::new(Locking {
        inner: DirectoryTarget::new(archive.path()),
        locks: Mutex::new(Vec::new()),
    });
    let archiver = Archiver::new(
        Arc::clone(&store),
        target.clone(),
        ArchiveConfig {
            grace: Duration::ZERO,
            work_dir: work.path().to_path_buf(),
            object_lock: true,
            ..ArchiveConfig::default()
        },
    );
    let key2 = object_key("", &tenant, day(2));
    let run = archiver.run_once().await.unwrap().unwrap();
    assert_eq!(
        (run.objects, run.locked, run.unlocked),
        (2, 1, 0),
        "{run:?}"
    );
    assert_eq!(*target.locks.lock().unwrap(), vec![(key2.clone(), true)]);
    assert_eq!(
        archiver.run_once().await.unwrap().unwrap().locked,
        0,
        "once"
    );

    // A late record rewrites the day: a new object, locked again.
    store
        .insert_cdrs(vec![record(
            tenant,
            day_start(day(2)) + Duration::from_secs(120),
        )])
        .await
        .unwrap();
    let run = archiver.run_once().await.unwrap().unwrap();
    assert_eq!((run.objects, run.locked), (1, 1), "{run:?}");

    // Released: unlocked.
    compliance
        .release_hold(
            &tenant,
            &hold.id,
            &HoldRelease {
                by: "counsel".into(),
                at: SystemTime::now(),
                reason: "closed".into(),
            },
        )
        .await
        .unwrap();
    let run = archiver.run_once().await.unwrap().unwrap();
    assert_eq!((run.locked, run.unlocked), (0, 1), "{run:?}");
    assert_eq!(target.locks.lock().unwrap().last(), Some(&(key2, false)));

    drop(archiver);
    drop(store);
    pg.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn the_role_in_fcp_jobs_writes_the_archive_and_announces_it() {
    let (Some(db), Some(redis)) = (
        TestDatabase::create().await,
        fcp_testkit::cluster::private_redis_url(),
    ) else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    let pg = storage(db.url()).await;
    let tenant = TenantId::new();
    let yesterday = Utc::now().date_naive() - Days::days(1);
    let seeded = record(tenant, day_start(yesterday) + Duration::from_secs(40_000));
    pg.insert_cdrs(vec![seeded.clone()]).await.unwrap();

    let archive = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let http = free_port();
    let config = format!(
        r#"
[service]
node_id = "e2e-archive"

[redis]
url = "{redis}"

[workers]
count = 1
shutdown_timeout = 5

[health]
enabled = true
bind_address = "127.0.0.1:{http}"

[cdr_archive]
enabled = true
database_url = "{db}"
every_secs = 1
grace_secs = 0
prefix = "cdrs/"
target = "directory"
directory = "{archive}"
work_dir = "{work}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        archive = archive.path().display(),
        work = work.path().display(),
    );
    let jobs = Service::start("jobs", JOBS, &config, &loopback(http, "/health")).await;

    let object = archive.path().join(object_key("cdrs/", &tenant, yesterday));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !object.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no object at {}",
            object.display()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let lines = read_object(&object);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].cdr_id, seeded.cdr_id);

    // Announced on the event stream.
    let client = redis::Client::open(redis.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let entries: Vec<(String, Vec<(String, String)>)> = redis::cmd("XRANGE")
            .arg("fcp:event_stream")
            .arg("-")
            .arg("+")
            .query_async(&mut conn)
            .await
            .unwrap_or_default();
        let announced = entries.iter().any(|(_, fields)| {
            fields
                .iter()
                .any(|(_, v)| v.contains("\"archived\"") && v.contains(&tenant.to_string()))
        });
        if announced {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no cdr.archived: {entries:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Status and metrics show the walk reached today.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (code, body) = http_get(&loopback(http, "/status")).await.unwrap();
        assert_eq!(code, 200);
        let status: serde_json::Value = serde_json::from_str(&body).unwrap();
        if status["cdr_archive"]["archive"]["archived_days"] == 1
            && status["cdr_archive"]["archive"]["next_day"]
                == (yesterday + Days::days(1)).to_string()
        {
            assert!(status["cdr_archive"]["target"]
                .as_str()
                .unwrap()
                .starts_with("file://"));
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "{status}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let (_, metrics) = http_get(&loopback(http, "/metrics")).await.unwrap();
    assert!(
        metrics.contains("cdr_archive_objects_total{node=\"e2e-archive\"} 1"),
        "{metrics}"
    );
    assert!(
        metrics.contains("cdr_archive_pending_days{node=\"e2e-archive\"} 0"),
        "{metrics}"
    );

    drop(jobs);
    pg.close().await;
    db.drop_db().await;
}
