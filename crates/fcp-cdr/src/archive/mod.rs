//! The archive sink (`docs/CDR_PIPELINE.md` §13, phase 5): each tenant's
//! records for a closed UTC day, as gzipped JSON Lines, written from the
//! system of record to a directory or an S3-compatible bucket and
//! confirmed there, with a `cdr.archived` event per object.
//!
//! One line per record, the pipeline's own record format
//! (`CallDetailRecord`, with its `schema_version`), oldest first. A day is
//! closed once it ended `grace` ago; records stored for a day after it was
//! written (a node's spool draining after an outage) are noted by the
//! store, and the day is written again under the same key. The store's
//! bookkeeping is `fcp_core::cdr_archive`; retention waits for it where a
//! tenant's policy requires the archive.

pub mod directory;
pub mod s3;
pub use fcp_core::sigv4;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use chrono::NaiveDate;
use fcp_core::cdr_archive::{day_is_closed, object_key, CdrArchiveWrite};
use fcp_core::storage::{CdrCursor, CdrStore};
use fcp_core::TenantId;
use fcp_events::{CdrEvent, EventPublisher, PlatformEvent};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

pub use directory::DirectoryTarget;
pub use s3::{S3Config, S3Target};

/// What can go wrong writing the archive.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("archive I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("record encoding: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("store: {0}")]
    Store(#[from] anyhow::Error),
    #[error("archive target: {0}")]
    Target(String),
    #[error("archive configuration: {0}")]
    Config(String),
}

/// An object as written: what the target is asked to confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    /// Size, compressed.
    pub bytes: u64,
    /// Hex SHA-256 of the bytes.
    pub sha256: String,
    pub records: u64,
}

/// Where objects go.
#[async_trait]
pub trait ArchiveTarget: Send + Sync {
    /// The target as the event names it: `s3://bucket`, `file:///path`.
    fn location(&self) -> String;
    /// Store `file` at `key`, replacing what is there.
    async fn put(&self, key: &str, file: &Path, object: &ObjectInfo) -> Result<(), ArchiveError>;
    /// Whether the object at `key` is `object`: its size and SHA-256.
    async fn confirm(&self, key: &str, object: &ObjectInfo) -> Result<bool, ArchiveError>;
    /// Place (`on`) or lift a legal hold on the object at `key`, where the
    /// store can lock an object against deletion. `false` when this target
    /// has no such lock.
    async fn set_legal_hold(&self, key: &str, on: bool) -> Result<bool, ArchiveError> {
        let _ = (key, on);
        Ok(false)
    }
}

/// The hex SHA-256 of a file.
pub async fn sha256_file(path: &Path) -> Result<String, ArchiveError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(|e| ArchiveError::Target(e.to_string()))?
    .map_err(Into::into)
}

/// The lock a run holds, so one host writes the archive at a time.
pub const ARCHIVE_LOCK: &str = "fcp:cdr-archive";

/// How the archiver runs.
#[derive(Debug, Clone)]
pub struct ArchiveConfig {
    /// How often a run starts.
    pub every: Duration,
    /// How long after a day ends it is written: records reaching the store
    /// later are late, and cost a second write of the day.
    pub grace: Duration,
    /// Prepended to every object key (`cdrs/`).
    pub prefix: String,
    /// Records read per query.
    pub page: usize,
    /// Where objects are built before they are sent.
    pub work_dir: PathBuf,
    /// Pending tenant-days (failed or late) retried per run.
    pub pending_per_run: usize,
    /// A run stops after this many writes fail in a row: the target is
    /// likely down, and the next run tries again.
    pub max_consecutive_failures: u32,
    /// Lock the objects of days a legal hold covers, where the target can
    /// (S3 Object Lock), and unlock them when no hold does
    /// (`docs/COMPLIANCE_FOLLOWUPS.md` §5).
    pub object_lock: bool,
}

impl Default for ArchiveConfig {
    fn default() -> Self {
        Self {
            every: Duration::from_secs(3600),
            grace: Duration::from_secs(2 * 3600),
            prefix: String::new(),
            page: 5000,
            work_dir: std::env::temp_dir().join("fcp-cdr-archive"),
            pending_per_run: 1000,
            max_consecutive_failures: 3,
            object_lock: false,
        }
    }
}

/// What one run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ArchiveRun {
    /// Days the walk moved past.
    pub days: u64,
    /// Objects written and confirmed.
    pub objects: u64,
    pub records: u64,
    pub bytes: u64,
    /// Tenant-days that failed; they are retried on the next run.
    pub failures: u64,
    /// Objects locked for a legal hold, and unlocked when it went.
    #[serde(default)]
    pub locked: u64,
    #[serde(default)]
    pub unlocked: u64,
}

/// Counters across runs, for `/status` and `/metrics`.
#[derive(Debug, Default)]
pub struct ArchiveStats {
    runs: AtomicU64,
    days: AtomicU64,
    objects: AtomicU64,
    records: AtomicU64,
    bytes: AtomicU64,
    failures: AtomicU64,
    last_run_unix: AtomicU64,
    last_run_millis: AtomicU64,
}

/// A copy of the counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ArchiveStatsSnapshot {
    pub runs: u64,
    pub days: u64,
    pub objects: u64,
    pub records: u64,
    pub bytes: u64,
    pub failures: u64,
    /// When the last run finished, seconds since the Unix epoch; 0 before one.
    pub last_run_unix: u64,
    pub last_run_millis: u64,
}

impl ArchiveStats {
    fn record(&self, run: &ArchiveRun, took: Duration) {
        self.runs.fetch_add(1, Ordering::Relaxed);
        self.days.fetch_add(run.days, Ordering::Relaxed);
        self.objects.fetch_add(run.objects, Ordering::Relaxed);
        self.records.fetch_add(run.records, Ordering::Relaxed);
        self.bytes.fetch_add(run.bytes, Ordering::Relaxed);
        self.failures.fetch_add(run.failures, Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last_run_unix.store(now, Ordering::Relaxed);
        self.last_run_millis
            .store(took.as_millis() as u64, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> ArchiveStatsSnapshot {
        ArchiveStatsSnapshot {
            runs: self.runs.load(Ordering::Relaxed),
            days: self.days.load(Ordering::Relaxed),
            objects: self.objects.load(Ordering::Relaxed),
            records: self.records.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            last_run_unix: self.last_run_unix.load(Ordering::Relaxed),
            last_run_millis: self.last_run_millis.load(Ordering::Relaxed),
        }
    }
}

impl ArchiveStatsSnapshot {
    /// Prometheus text for the counters, with `labels` on every sample.
    pub fn prometheus(&self, labels: &str) -> String {
        let mut out = String::new();
        for (name, kind, help, value) in [
            (
                "cdr_archive_runs_total",
                "counter",
                "Archive runs finished.",
                self.runs,
            ),
            (
                "cdr_archive_days_total",
                "counter",
                "Days the archive walk moved past.",
                self.days,
            ),
            (
                "cdr_archive_objects_total",
                "counter",
                "Archive objects written and confirmed.",
                self.objects,
            ),
            (
                "cdr_archive_records_total",
                "counter",
                "Records written to the archive.",
                self.records,
            ),
            (
                "cdr_archive_bytes_total",
                "counter",
                "Bytes written to the archive, compressed.",
                self.bytes,
            ),
            (
                "cdr_archive_failures_total",
                "counter",
                "Tenant-days whose write failed.",
                self.failures,
            ),
            (
                "cdr_archive_last_run_timestamp_seconds",
                "gauge",
                "When the last archive run finished.",
                self.last_run_unix,
            ),
        ] {
            crate::stats::family(&mut out, name, kind, help);
            out.push_str(&format!("{name}{{{labels}}} {value}\n"));
        }
        out
    }
}

/// Writes the archive.
pub struct Archiver {
    store: Arc<dyn CdrStore>,
    target: Arc<dyn ArchiveTarget>,
    events: Option<Arc<dyn EventPublisher>>,
    config: ArchiveConfig,
    stats: ArchiveStats,
}

/// Removes the object being built when the write ends, however it ends.
struct BuildFile(PathBuf);

impl Drop for BuildFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Counts and hashes what passes through to the file.
struct Hashing<W> {
    inner: W,
    hasher: Sha256,
    bytes: u64,
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

type Encoder = flate2::write::GzEncoder<Hashing<std::io::BufWriter<std::fs::File>>>;

impl Archiver {
    pub fn new(
        store: Arc<dyn CdrStore>,
        target: Arc<dyn ArchiveTarget>,
        config: ArchiveConfig,
    ) -> Self {
        Self {
            store,
            target,
            events: None,
            config,
            stats: ArchiveStats::default(),
        }
    }

    /// Announce each object as `cdr.archived`.
    pub fn with_events(mut self, events: Arc<dyn EventPublisher>) -> Self {
        self.events = Some(events);
        self
    }

    pub fn stats(&self) -> &ArchiveStats {
        &self.stats
    }

    pub fn target(&self) -> &Arc<dyn ArchiveTarget> {
        &self.target
    }

    /// Run on the configured interval, the first run at once.
    pub fn spawn(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.config.every);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                match self.run_once().await {
                    Ok(Some(run)) if run != ArchiveRun::default() => {
                        info!(
                            days = run.days,
                            objects = run.objects,
                            records = run.records,
                            bytes = run.bytes,
                            failures = run.failures,
                            "CDR archive run finished"
                        )
                    }
                    Ok(Some(_)) => debug!("CDR archive is up to date"),
                    Ok(None) => debug!("Another host is writing the CDR archive"),
                    Err(e) => warn!(error = %e, "CDR archive run failed"),
                }
            }
        })
    }

    /// One run: the pending tenant-days, then every closed day the walk has
    /// not reached. `None` when another host holds the archive lock.
    pub async fn run_once(&self) -> Result<Option<ArchiveRun>, ArchiveError> {
        let Some(lock) = self.store.try_maintenance_lock(ARCHIVE_LOCK).await? else {
            return Ok(None);
        };
        let started = Instant::now();
        let mut run = ArchiveRun::default();
        let mut result = self.run_locked(&mut run).await;
        if self.config.object_lock {
            let locks = self.sync_locks(&mut run).await;
            result = result.and(locks);
        }
        lock.release().await;
        self.stats.record(&run, started.elapsed());
        result.map(|()| Some(run))
    }

    async fn run_locked(&self, run: &mut ArchiveRun) -> Result<(), ArchiveError> {
        tokio::fs::create_dir_all(&self.config.work_dir).await?;
        let Some(mut day) = self.store.cdr_archive_next_day().await? else {
            return Ok(());
        };
        let mut failed_in_a_row = 0u32;

        // Days written before and not confirmed, or given late records.
        for (tenant, pending_day) in self
            .store
            .pending_cdr_archives(day, self.config.pending_per_run)
            .await?
        {
            if !self
                .write_counted(run, &tenant, pending_day, &mut failed_in_a_row)
                .await?
            {
                return Ok(());
            }
        }

        // The walk. A tenant-day whose write fails stays unconfirmed in the
        // store, so the walk moves on and the next run retries it.
        let now = SystemTime::now();
        while day_is_closed(day, self.config.grace, now) {
            for tenant in self.store.cdr_tenants_on(day).await? {
                if !self
                    .write_counted(run, &tenant, day, &mut failed_in_a_row)
                    .await?
                {
                    return Ok(());
                }
            }
            self.store.advance_cdr_archive(day).await?;
            run.days += 1;
            day = match day.succ_opt() {
                Some(next) => next,
                None => break,
            };
        }
        Ok(())
    }

    /// Bring the objects' locks in line with the legal holds: a day a hold
    /// covers is locked, a locked day no hold covers is unlocked. A target
    /// that cannot lock is told once and left alone.
    async fn sync_locks(&self, run: &mut ArchiveRun) -> Result<(), ArchiveError> {
        loop {
            let changes = self.store.cdr_archive_lock_changes(100).await?;
            if changes.is_empty() {
                return Ok(());
            }
            let count = changes.len();
            for (tenant, day, key, lock) in changes {
                if !self.target.set_legal_hold(&key, lock).await? {
                    warn!(
                        target = %self.target.location(),
                        "The archive target cannot lock objects; object_lock has no effect"
                    );
                    return Ok(());
                }
                self.store
                    .set_cdr_archive_locked(&tenant, day, lock)
                    .await?;
                if lock {
                    run.locked += 1;
                } else {
                    run.unlocked += 1;
                }
                info!(tenant_id = %tenant, day = %day, key = %key, lock, "Archive object lock changed for a legal hold");
            }
            if count < 100 {
                return Ok(());
            }
        }
    }

    /// Write one tenant-day into `run`. `false` when the run should stop.
    async fn write_counted(
        &self,
        run: &mut ArchiveRun,
        tenant: &TenantId,
        day: NaiveDate,
        failed_in_a_row: &mut u32,
    ) -> Result<bool, ArchiveError> {
        // Beginning is bookkeeping: if the store cannot do it, stop.
        let generation = self.store.begin_cdr_archive(tenant, day).await?;
        match self.write(tenant, day, generation).await {
            Ok(write) => {
                *failed_in_a_row = 0;
                if write.object_key.is_some() {
                    run.objects += 1;
                    run.records += write.records;
                    run.bytes += write.bytes;
                }
                Ok(true)
            }
            Err(e) => {
                warn!(tenant_id = %tenant, day = %day, error = %e, "Could not archive a day of CDRs");
                run.failures += 1;
                *failed_in_a_row += 1;
                Ok(*failed_in_a_row < self.config.max_consecutive_failures)
            }
        }
    }

    /// Write a tenant's day under `generation`, which the caller has begun:
    /// build the object, send it, confirm it, and record it.
    pub async fn write(
        &self,
        tenant: &TenantId,
        day: NaiveDate,
        generation: u64,
    ) -> Result<CdrArchiveWrite, ArchiveError> {
        let path = self
            .config
            .work_dir
            .join(format!("{tenant}_{day}_{generation}.jsonl.gz.part"));
        let build = BuildFile(path.clone());
        let file = std::fs::File::create(&path)?;
        let mut encoder: Encoder = flate2::write::GzEncoder::new(
            Hashing {
                inner: std::io::BufWriter::with_capacity(1 << 20, file),
                hasher: Sha256::new(),
                bytes: 0,
            },
            flate2::Compression::default(),
        );

        let mut records = 0u64;
        let mut after: Option<CdrCursor> = None;
        loop {
            let page = self
                .store
                .cdr_archive_page(tenant, day, after, self.config.page)
                .await?;
            let full = page.len() == self.config.page;
            after = page.last().map(CdrCursor::after);
            records += page.len() as u64;
            // Encoding is CPU work: off the async threads.
            encoder = tokio::task::spawn_blocking(move || -> Result<Encoder, ArchiveError> {
                for record in &page {
                    serde_json::to_writer(&mut encoder, record)?;
                    encoder.write_all(b"\n")?;
                }
                Ok(encoder)
            })
            .await
            .map_err(|e| ArchiveError::Target(e.to_string()))??;
            if !full {
                break;
            }
        }

        let mut write = CdrArchiveWrite {
            tenant_id: *tenant,
            day,
            generation,
            object_key: None,
            records,
            bytes: 0,
            sha256: None,
        };
        // A day with no records is left undescribed — unless an earlier
        // write stored an object for it: then its records were deleted or
        // erased since, and the object is replaced by an empty one rather
        // than left holding them (docs/COMPLIANCE_FOLLOWUPS.md §6.5).
        if records == 0
            && !self
                .store
                .get_cdr_archive(tenant, day)
                .await?
                .is_some_and(|entry| entry.object_key.is_some())
        {
            drop(build);
            self.complete(&write).await?;
            return Ok(write);
        }

        let hashing = tokio::task::spawn_blocking(move || -> std::io::Result<_> {
            let mut hashing = encoder.finish()?;
            hashing.flush()?;
            Ok(hashing)
        })
        .await
        .map_err(|e| ArchiveError::Target(e.to_string()))??;
        let object = ObjectInfo {
            bytes: hashing.bytes,
            sha256: hex::encode(hashing.hasher.finalize()),
            records,
        };
        drop(hashing.inner);

        let key = object_key(&self.config.prefix, tenant, day);
        self.target.put(&key, &path, &object).await?;
        if !self.target.confirm(&key, &object).await? {
            return Err(ArchiveError::Target(format!(
                "{} did not confirm {key} ({} bytes, sha256 {})",
                self.target.location(),
                object.bytes,
                object.sha256
            )));
        }
        drop(build);

        write.object_key = Some(key.clone());
        write.bytes = object.bytes;
        write.sha256 = Some(object.sha256.clone());
        if self.complete(&write).await? {
            info!(
                tenant_id = %tenant,
                day = %day,
                generation,
                records,
                bytes = object.bytes,
                key = %key,
                "Archived a day of CDRs"
            );
            self.announce(&write).await;
        }
        Ok(write)
    }

    async fn complete(&self, write: &CdrArchiveWrite) -> Result<bool, ArchiveError> {
        let current = self.store.complete_cdr_archive(write).await?;
        if !current {
            debug!(
                tenant_id = %write.tenant_id,
                day = %write.day,
                generation = write.generation,
                "A newer write of this day has begun; this one is not recorded"
            );
        }
        Ok(current)
    }

    async fn announce(&self, write: &CdrArchiveWrite) {
        let (Some(events), Some(key), Some(sha256)) =
            (&self.events, &write.object_key, &write.sha256)
        else {
            return;
        };
        let event = PlatformEvent::Cdr(CdrEvent::Archived {
            tenant_id: write.tenant_id,
            day: write.day.to_string(),
            target: self.target.location(),
            object_key: key.clone(),
            records: write.records,
            bytes: write.bytes,
            sha256: sha256.clone(),
            generation: write.generation,
            timestamp: SystemTime::now(),
        });
        if let Err(e) = events.publish(event).await {
            warn!(tenant_id = %write.tenant_id, day = %write.day, error = %e, "Failed to announce cdr.archived");
        }
    }
}
