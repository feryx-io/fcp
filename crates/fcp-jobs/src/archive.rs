//! The CDR archive role (`docs/CDR_PIPELINE.md` §13, phase 5): each
//! tenant's closed days of records, gzipped JSON Lines, to a directory or an
//! S3-compatible bucket, confirmed there and announced as `cdr.archived`.
//! Enabled with `[cdr_archive] enabled = true`; any number of `fcp-jobs`
//! replicas may enable it, one writes at a time under the store's lock.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use fcp_cdr::archive::{
    ArchiveConfig, ArchiveTarget, Archiver, DirectoryTarget, S3Config, S3Target,
};
use fcp_core::cdr_archive::CdrArchiveStatus;
use fcp_core::storage::CdrStore;
use fcp_events::EventPublisher;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tracing::info;

use crate::config::JobsConfig;

/// Where the objects go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// `[cdr_archive.s3]`.
    #[default]
    S3,
    /// `[cdr_archive] directory`.
    Directory,
}

/// The `[cdr_archive]` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CdrArchiveConfig {
    #[serde(default)]
    pub enabled: bool,

    /// The system of record the archive is written from; it must be the
    /// primary, not a replica.
    #[serde(default = "default_database_url")]
    pub database_url: String,

    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// How often a run starts.
    #[serde(default = "default_every_secs")]
    pub every_secs: u64,

    /// How long after a UTC day ends it is written. Records reaching the
    /// store later cost a second write of the day.
    #[serde(default = "default_grace_secs")]
    pub grace_secs: u64,

    /// Prepended to every object key.
    #[serde(default)]
    pub prefix: String,

    /// Where objects are built before they are sent; the system's temporary
    /// directory when unset. Needs room for a tenant's largest day,
    /// compressed.
    #[serde(default)]
    pub work_dir: Option<PathBuf>,

    #[serde(default)]
    pub target: TargetKind,

    /// The archive directory when `target = "directory"`.
    #[serde(default)]
    pub directory: Option<PathBuf>,

    /// The bucket when `target = "s3"`.
    #[serde(default)]
    pub s3: Option<S3Config>,

    /// Lock the objects of days a legal hold covers (S3 Object Lock's legal
    /// hold; the bucket must have Object Lock enabled), and unlock them when
    /// no hold does.
    #[serde(default)]
    pub object_lock: bool,

    /// The Redis key prefix `cdr.archived` is published under.
    #[serde(default = "default_key_prefix")]
    pub key_prefix: String,

    /// Region stamped on the events; falls back to `FCP_REGION`.
    #[serde(default)]
    pub region: Option<String>,
}

fn default_database_url() -> String {
    "postgresql://localhost/fcp".to_string()
}
fn default_max_connections() -> u32 {
    4
}
fn default_every_secs() -> u64 {
    3600
}
fn default_grace_secs() -> u64 {
    2 * 3600
}
fn default_key_prefix() -> String {
    "fcp:".to_string()
}

impl Default for CdrArchiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            database_url: default_database_url(),
            max_connections: default_max_connections(),
            every_secs: default_every_secs(),
            grace_secs: default_grace_secs(),
            prefix: String::new(),
            work_dir: None,
            target: TargetKind::default(),
            directory: None,
            s3: None,
            object_lock: false,
            key_prefix: default_key_prefix(),
            region: None,
        }
    }
}

impl CdrArchiveConfig {
    fn target(&self) -> Result<Arc<dyn ArchiveTarget>> {
        Ok(match self.target {
            TargetKind::Directory => {
                let Some(dir) = &self.directory else {
                    bail!("[cdr_archive] target = \"directory\" needs a directory");
                };
                Arc::new(DirectoryTarget::new(dir))
            }
            TargetKind::S3 => {
                let Some(s3) = &self.s3 else {
                    bail!("[cdr_archive] target = \"s3\" needs a [cdr_archive.s3] section");
                };
                Arc::new(S3Target::new(s3.clone()).context("[cdr_archive.s3]")?)
            }
        })
    }
}

/// How long the archive's totals are reused: they sum a table that grows
/// by a row per tenant per day, and `/metrics` is scraped often.
const STATUS_TTL: Duration = Duration::from_secs(60);

/// The running role.
pub struct CdrArchiveRole {
    pub archiver: Arc<Archiver>,
    store: Arc<dyn CdrStore>,
    task: JoinHandle<()>,
    status: tokio::sync::Mutex<Option<(std::time::Instant, CdrArchiveStatus)>>,
}

impl CdrArchiveRole {
    pub async fn start(config: &JobsConfig) -> Result<Self> {
        let cfg = &config.cdr_archive;
        let target = cfg.target()?;
        let pg = fcp_storage_postgres::PostgresStorage::new(fcp_storage_postgres::PostgresConfig {
            connection_string: cfg.database_url.clone(),
            max_connections: cfg.max_connections.max(2),
            connection_timeout: Duration::from_secs(30),
            encryption: None,
            read_replica_url: None,
            max_replica_lag: Duration::from_secs(5),
        })
        .await
        .context("Failed to connect the CDR archive to its database")?;
        let store: Arc<dyn CdrStore> = Arc::new(pg);

        let redis = fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: config.redis.url.clone(),
            key_prefix: cfg.key_prefix.clone(),
        })
        .await
        .context("Failed to connect the CDR archive to Redis")?;
        let region = cfg
            .region
            .clone()
            .or_else(|| std::env::var("FCP_REGION").ok());
        let events: Arc<dyn EventPublisher> = Arc::new(
            fcp_storage_redis::event_bus::RedisEventBus::new(redis, config.service.node_id.clone())
                .with_region(region),
        );

        let archive_config = ArchiveConfig {
            every: Duration::from_secs(cfg.every_secs.max(1)),
            grace: Duration::from_secs(cfg.grace_secs),
            prefix: cfg.prefix.clone(),
            object_lock: cfg.object_lock,
            work_dir: cfg
                .work_dir
                .clone()
                .unwrap_or_else(|| ArchiveConfig::default().work_dir),
            ..ArchiveConfig::default()
        };
        info!(
            target = %target.location(),
            prefix = %archive_config.prefix,
            every_secs = cfg.every_secs,
            grace_secs = cfg.grace_secs,
            "CDR archive started"
        );
        let archiver =
            Arc::new(Archiver::new(Arc::clone(&store), target, archive_config).with_events(events));
        let task = Arc::clone(&archiver).spawn();
        Ok(Self {
            archiver,
            store,
            task,
            status: tokio::sync::Mutex::new(None),
        })
    }

    /// Stop starting runs. A run under way is dropped at its next await; the
    /// store's bookkeeping leaves its tenant-day unconfirmed, and the next
    /// run on any host writes it again.
    pub fn stop(&self) {
        self.task.abort();
    }

    /// The store's view of the archive for `/metrics`, at most a minute old.
    async fn store_status(&self) -> anyhow::Result<CdrArchiveStatus> {
        let mut cached = self.status.lock().await;
        if let Some((at, status)) = cached.as_ref() {
            if at.elapsed() < STATUS_TTL {
                return Ok(status.clone());
            }
        }
        let status = self.store.cdr_archive_status().await?;
        *cached = Some((std::time::Instant::now(), status.clone()));
        Ok(status)
    }

    /// The role's part of `/status`, read fresh.
    pub async fn status(&self) -> serde_json::Value {
        let store = match self.store.cdr_archive_status().await {
            Ok(s) => serde_json::to_value(s).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        };
        serde_json::json!({
            "target": self.archiver.target().location(),
            "archive": store,
            "runs": self.archiver.stats().snapshot(),
        })
    }

    pub async fn metrics(&self, node_id: &str) -> String {
        let labels = format!("node=\"{}\"", node_id.replace('"', ""));
        let mut out = self.archiver.stats().snapshot().prometheus(&labels);
        if let Ok(status) = self.store_status().await {
            for (name, help, value) in [
                (
                    "cdr_archive_pending_days",
                    "Tenant-days written and not confirmed, or given late records.",
                    status.pending_days,
                ),
                (
                    "cdr_archive_next_day_timestamp_seconds",
                    "Start of the first day the archive walk has not reached.",
                    status
                        .next_day
                        .map(|d| {
                            fcp_core::cdr_archive::day_start(d)
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                        })
                        .unwrap_or(0),
                ),
            ] {
                fcp_cdr::stats::family(&mut out, name, "gauge", help);
                out.push_str(&format!("{name}{{{labels}}} {value}\n"));
            }
        }
        out
    }
}
