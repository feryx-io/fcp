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
    /// A directory this service can write: an NFS or SMB share, or a USB
    /// disk, mounted into it (B4).
    Directory { path: String },
    /// A restic REST server (`rest-server`, which can be append-only).
    Rest {
        /// `https://backup.example.com:8000/fcp/`.
        url: String,
        #[serde(default)]
        username: Option<String>,
        /// Sealed at rest.
        #[serde(default)]
        password: Option<String>,
    },
    /// A remote through rclone, for a provider none of the others covers.
    Rclone {
        /// `<remote>:<path>`, as rclone names it (`b2:bucket/fcp`).
        remote: String,
        /// The rclone configuration that defines the remote (`rclone.conf`
        /// text), sealed at rest.
        #[serde(default)]
        config: Option<String>,
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
            Self::Directory { .. } => "directory",
            Self::Rest { .. } => "rest",
            Self::Rclone { .. } => "rclone",
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
            Self::Directory { path } => path.clone(),
            Self::Rest { url, .. } => format!("rest:{url}"),
            Self::Rclone { remote, .. } => format!("rclone:{remote}"),
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
    /// Read a share of the stored data back (`restic check
    /// --read-data-subset`), weekly (B4).
    Check,
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
            Self::Check => "check",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "backup" => Some(Self::Backup),
            "test" => Some(Self::Test),
            "check" => Some(Self::Check),
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

/// A file one of a tenant's rows names, as the service that wrote it sees
/// the path: what a tenant may put back (`docs/SCHEDULED_BACKUPS.md` B3b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TenantFile {
    /// `call_recording`, `conference_recording`, `voicemail`,
    /// `voicemail_greeting` or `prompt`.
    pub class: String,
    /// The row's id.
    pub item: String,
    pub path: String,
    /// When the row was made (or the recording began), RFC 3339.
    #[serde(default)]
    pub created_at: Option<String>,
}

/// The classes of [`TenantFile`].
pub const TENANT_FILE_CLASSES: [&str; 5] = [
    "call_recording",
    "conference_recording",
    "voicemail",
    "voicemail_greeting",
    "prompt",
];

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

    /// Every file `tenant`'s rows name and this deployment holds (a site's
    /// are the site's), newest first, at most `limit`.
    async fn tenant_files(&self, _tenant: &str, _limit: usize) -> anyhow::Result<Vec<TenantFile>> {
        Ok(Vec::new())
    }

    /// When `destination` was last found overdue, with the last success it
    /// was overdue from: noticed once per lapse (B4).
    async fn overdue_notice(
        &self,
        _destination: &uuid::Uuid,
    ) -> anyhow::Result<Option<Option<SystemTime>>> {
        Ok(None)
    }

    async fn note_overdue(
        &self,
        _destination: &uuid::Uuid,
        _last_success: Option<SystemTime>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Every destination's status, without opening a secret: what a site
    /// reports up its link (B5).
    async fn backup_status(&self) -> anyhow::Result<Vec<BackupStatus>> {
        Ok(Vec::new())
    }

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

/// How long after one was due a destination may go without a good backup
/// before it is overdue (`docs/SCHEDULED_BACKUPS.md` §2).
pub const OVERDUE_AFTER: std::time::Duration = std::time::Duration::from_secs(26 * 3600);

/// How often a destination's data is read back.
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

/// Whether a scheduled destination is overdue at `now`: no good backup in
/// [`OVERDUE_AFTER`], counting from when it was made if it never had one.
pub fn overdue(
    dest: &BackupDestination,
    last_success: Option<SystemTime>,
    now: SystemTime,
) -> bool {
    let scheduled = dest.schedule.daily_at.is_some() || dest.schedule.cron.is_some();
    overdue_from(
        dest.enabled && scheduled,
        dest.created_at,
        last_success,
        now,
    )
}

/// [`overdue`], from what a status holds.
pub fn overdue_from(
    watched: bool,
    created_at: SystemTime,
    last_success: Option<SystemTime>,
    now: SystemTime,
) -> bool {
    watched
        && now
            .duration_since(last_success.unwrap_or(created_at))
            .is_ok_and(|d| d > OVERDUE_AFTER)
}

/// A destination as a site's link reports it up (`docs/SCHEDULED_BACKUPS.md`
/// B5): no secrets, no target beyond its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BackupStatus {
    /// `bkd-<uuid>`, at the site.
    pub destination_id: String,
    pub name: String,
    /// `sftp`, `s3`, `directory`, `rest` or `rclone`.
    pub kind: String,
    pub enabled: bool,
    pub scheduled: bool,
    /// RFC 3339.
    #[serde(default)]
    pub last_success_at: Option<String>,
    /// The last backup run's state and when it was asked for.
    #[serde(default)]
    pub last_run_state: Option<String>,
    #[serde(default)]
    pub last_run_at: Option<String>,
    pub overdue: bool,
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
    fn a_destination_is_overdue_a_day_and_two_hours_on() {
        let made: SystemTime = Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap().into();
        let mut d = BackupDestination {
            id: uuid::Uuid::nil(),
            name: "x".into(),
            target: BackupTarget::Directory {
                path: "/mnt/b".into(),
            },
            password: None,
            schedule: BackupSchedule {
                daily_at: Some("02:00".into()),
                cron: None,
            },
            keep: BackupRetention::default(),
            include_key: false,
            enabled: true,
            created_at: made,
            updated_at: made,
        };
        let at = |d: u32, h: u32| -> SystemTime {
            Utc.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap().into()
        };
        // Never run: counted from when it was made.
        assert!(!overdue(&d, None, at(2, 1)));
        assert!(overdue(&d, None, at(2, 3)));
        // A good backup at 02:00 on the 5th: fine until 04:00 on the 6th.
        assert!(!overdue(&d, Some(at(5, 2)), at(6, 4)));
        assert!(overdue(&d, Some(at(5, 2)), at(6, 5)));
        // Off, or by hand only: never.
        d.schedule.daily_at = None;
        assert!(!overdue(&d, None, at(20, 0)));
        d.schedule.daily_at = Some("02:00".into());
        d.enabled = false;
        assert!(!overdue(&d, None, at(20, 0)));
        assert_eq!(
            BackupTarget::Rest {
                url: "https://b:8000/fcp/".into(),
                username: None,
                password: None
            }
            .repository(),
            "rest:https://b:8000/fcp/"
        );
        assert_eq!(
            BackupTarget::Rclone {
                remote: "b2:bucket/fcp".into(),
                config: None
            }
            .repository(),
            "rclone:b2:bucket/fcp"
        );
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
