//! Backups taken by `fcp-backup` and set up from the admin
//! (`docs/SCHEDULED_BACKUPS.md`): where they go, when, how many are kept,
//! and each run.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Where snapshots go: a restic repository at a destination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupDestination {
    pub id: uuid::Uuid,
    pub name: String,
    pub target: BackupTarget,
    /// The repository's password, sealed at rest: restic encrypts every
    /// snapshot with it.
    #[serde(default)]
    pub password: Option<String>,
    pub schedule: BackupSchedule,
    pub keep: BackupRetention,
    /// Whether each snapshot holds the master key ring too (Q4: off by
    /// default; the key is downloaded once instead).
    #[serde(default)]
    pub include_key: bool,
    pub enabled: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

/// The destination itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupTarget {
    /// An SFTP server: restic over `ssh -s sftp`.
    Sftp {
        host: String,
        #[serde(default = "default_ssh_port")]
        port: u16,
        user: String,
        /// The repository's directory on the server.
        path: String,
        /// An OpenSSH private key, sealed at rest.
        #[serde(default)]
        private_key: Option<String>,
        /// The server's host key, `known_hosts` style, pinned when first
        /// seen (by a test) and required after.
        #[serde(default)]
        host_key: Option<String>,
    },
    /// An S3-compatible bucket.
    S3 {
        /// `https://s3.us-east-1.amazonaws.com`, `https://<account>.r2.cloudflarestorage.com`, …
        endpoint: String,
        bucket: String,
        #[serde(default)]
        region: Option<String>,
        /// A folder in the bucket.
        #[serde(default)]
        prefix: Option<String>,
        access_key_id: String,
        /// Sealed at rest.
        #[serde(default)]
        secret_access_key: Option<String>,
    },
}

fn default_ssh_port() -> u16 {
    22
}

impl BackupTarget {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Sftp { .. } => "sftp",
            Self::S3 { .. } => "s3",
        }
    }

    /// The restic repository string (`sftp:…`, `s3:…`); credentials travel
    /// apart, in the environment and options.
    pub fn repository(&self) -> String {
        match self {
            Self::Sftp {
                host,
                port,
                user,
                path,
                ..
            } => {
                let path = if path.starts_with('/') {
                    path.clone()
                } else {
                    format!("/{path}")
                };
                if *port == 22 {
                    format!("sftp:{user}@{host}:{path}")
                } else {
                    // restic's long form, which takes a port.
                    format!(
                        "sftp://{user}@{host}:{port}/{}",
                        path.trim_start_matches('/')
                    )
                }
            }
            Self::S3 {
                endpoint,
                bucket,
                prefix,
                ..
            } => {
                let endpoint = endpoint.trim_end_matches('/');
                match prefix
                    .as_deref()
                    .map(|p| p.trim_matches('/'))
                    .filter(|p| !p.is_empty())
                {
                    Some(p) => format!("s3:{endpoint}/{bucket}/{p}"),
                    None => format!("s3:{endpoint}/{bucket}"),
                }
            }
        }
    }
}

/// When a destination is backed up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSchedule {
    /// `HH:MM`, 24-hour, in the platform's timezone (UTC unless the
    /// platform says).
    pub daily_at: Option<String>,
    /// A cron expression (`min hour dom month dow`) over `daily_at`.
    #[serde(default)]
    pub cron: Option<String>,
}

/// How many snapshots are kept, restic's `forget --keep-*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRetention {
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
}

impl Default for BackupRetention {
    fn default() -> Self {
        Self {
            daily: 7,
            weekly: 4,
            monthly: 12,
        }
    }
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BackupRunKind {
    /// A backup: the database and the files, then retention.
    Backup,
    /// Reach the destination, pin its host key, write and remove a probe.
    Test,
}

/// Where a run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BackupRunState {
    /// Asked for (Back up now, a test); waiting for the service.
    Requested,
    Running,
    Succeeded,
    Failed,
}

impl BackupRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "requested" => Some(Self::Requested),
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl BackupRunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backup => "backup",
            Self::Test => "test",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "backup" => Some(Self::Backup),
            "test" => Some(Self::Test),
            _ => None,
        }
    }
}

/// One run of a destination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupRun {
    pub id: uuid::Uuid,
    pub destination_id: uuid::Uuid,
    pub kind: BackupRunKind,
    pub state: BackupRunState,
    /// `schedule`, `now` (from the page), or `test`.
    pub trigger: String,
    pub requested_at: SystemTime,
    #[serde(default)]
    pub started_at: Option<SystemTime>,
    #[serde(default)]
    pub finished_at: Option<SystemTime>,
    /// The snapshots this run made (the database's and the files').
    #[serde(default)]
    pub snapshots: Vec<String>,
    /// Bytes restic added to the repository.
    #[serde(default)]
    pub bytes_added: u64,
    /// What happened, for a test or a failure.
    #[serde(default)]
    pub message: Option<String>,
    /// The service that ran it.
    #[serde(default)]
    pub node: Option<String>,
}

/// Where destinations and runs are kept.
#[async_trait]
pub trait BackupStore: Send + Sync {
    async fn list_backup_destinations(&self) -> anyhow::Result<Vec<BackupDestination>>;
    async fn get_backup_destination(
        &self,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<BackupDestination>>;
    /// Keep a destination, its secrets sealed.
    async fn put_backup_destination(&self, d: &BackupDestination) -> anyhow::Result<()>;
    async fn delete_backup_destination(&self, id: &uuid::Uuid) -> anyhow::Result<bool>;

    /// Ask for a run; the service claims it.
    async fn request_backup_run(&self, run: &BackupRun) -> anyhow::Result<()>;
    /// The oldest requested run, marked running by `node`, if any: one
    /// service takes it (`FOR UPDATE SKIP LOCKED`).
    async fn claim_backup_run(&self, node: &str) -> anyhow::Result<Option<BackupRun>>;
    /// A run's outcome.
    async fn finish_backup_run(&self, run: &BackupRun) -> anyhow::Result<()>;
    async fn get_backup_run(&self, id: &uuid::Uuid) -> anyhow::Result<Option<BackupRun>>;
    /// Runs newest first, of one destination or all.
    async fn list_backup_runs(
        &self,
        destination: Option<&uuid::Uuid>,
        limit: usize,
    ) -> anyhow::Result<Vec<BackupRun>>;
    /// A destination's last run of `kind` that was asked for by the
    /// schedule, for deciding when the next is due.
    async fn last_scheduled_run(
        &self,
        destination: &uuid::Uuid,
    ) -> anyhow::Result<Option<BackupRun>>;

    /// The key ring's active KEK id, which a snapshot is tagged with so a
    /// restore can check the key it is given (`docs/SCHEDULED_BACKUPS.md`).
    async fn active_kek_id(&self) -> anyhow::Result<Option<String>> {
        Ok(None)
    }

    /// The master key downloaded from the page for `kek_id`, by `actor`.
    async fn record_key_export(&self, _kek_id: &str, _actor: &str) -> anyhow::Result<()> {
        Ok(())
    }

    /// When the key for `kek_id` was last downloaded, and by whom.
    async fn last_key_export(&self, _kek_id: &str) -> anyhow::Result<Option<(SystemTime, String)>> {
        Ok(None)
    }
}

/// Whether a run is due at `now` for a schedule whose last scheduled run
/// was asked for at `last`: the most recent `daily_at` time (UTC) at or
/// before `now` came after `last` (or there was none).
pub fn daily_due(daily_at: &str, last: Option<SystemTime>, now: SystemTime) -> Option<bool> {
    use chrono::{NaiveTime, TimeZone, Utc};
    let time = NaiveTime::parse_from_str(daily_at, "%H:%M").ok()?;
    let now: chrono::DateTime<Utc> = now.into();
    let today = Utc.from_utc_datetime(&now.date_naive().and_time(time));
    let slot = if today <= now {
        today
    } else {
        today - chrono::Duration::days(1)
    };
    Some(match last {
        None => true,
        Some(last) => chrono::DateTime::<Utc>::from(last) < slot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn a_repository_names_its_destination() {
        let sftp = BackupTarget::Sftp {
            host: "backup.example.com".into(),
            port: 22,
            user: "fcp".into(),
            path: "srv/fcp".into(),
            private_key: None,
            host_key: None,
        };
        assert_eq!(sftp.repository(), "sftp:fcp@backup.example.com:/srv/fcp");
        let other_port = BackupTarget::Sftp {
            host: "h".into(),
            port: 2222,
            user: "u".into(),
            path: "/r".into(),
            private_key: None,
            host_key: None,
        };
        assert_eq!(other_port.repository(), "sftp://u@h:2222/r");
        let s3 = BackupTarget::S3 {
            endpoint: "https://s3.example.com/".into(),
            bucket: "b".into(),
            region: None,
            prefix: Some("/fcp/".into()),
            access_key_id: "k".into(),
            secret_access_key: None,
        };
        assert_eq!(s3.repository(), "s3:https://s3.example.com/b/fcp");
    }

    #[test]
    fn a_daily_run_is_due_once_its_time_has_passed() {
        let at =
            |h, m| -> SystemTime { Utc.with_ymd_and_hms(2026, 11, 27, h, m, 0).unwrap().into() };
        // Never run: due.
        assert_eq!(daily_due("02:00", None, at(9, 0)), Some(true));
        // Run yesterday at 02:00, now 01:59: not yet.
        let yesterday: SystemTime = Utc.with_ymd_and_hms(2026, 11, 26, 2, 0, 5).unwrap().into();
        assert_eq!(daily_due("02:00", Some(yesterday), at(1, 59)), Some(false));
        // Now 02:00: due.
        assert_eq!(daily_due("02:00", Some(yesterday), at(2, 0)), Some(true));
        // Run today at 02:00: done until tomorrow.
        assert_eq!(daily_due("02:00", Some(at(2, 0)), at(23, 0)), Some(false));
        assert_eq!(daily_due("25:00", None, at(2, 0)), None);
    }
}
