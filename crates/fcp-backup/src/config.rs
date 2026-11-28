//! `fcp-backup`'s configuration (`docs/SCHEDULED_BACKUPS.md` §2).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    #[serde(default)]
    pub service: ServiceSection,
    #[serde(default)]
    pub http: HttpSection,
    pub storage: StorageSection,
    #[serde(default)]
    pub backup: BackupSection,
    #[serde(default)]
    pub events: EventsSection,
    #[serde(default)]
    pub cluster: ClusterSection,
    #[serde(default)]
    pub logging: LoggingSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSection {
    #[serde(default = "default_node_id")]
    pub node_id: String,
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            node_id: default_node_id(),
        }
    }
}

fn default_node_id() -> String {
    "backup-1".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    /// `/health` and `/ready`.
    #[serde(default = "default_bind")]
    pub bind_address: String,
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_bind(),
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1:8096".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageSection {
    /// The platform's database: what is backed up, and where the
    /// destinations and runs are kept. `FCP_BACKUP_DATABASE_URL` wins.
    pub database_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupSection {
    /// The restic binary.
    #[serde(default = "default_restic")]
    pub restic: PathBuf,
    /// `pg_dump`, of the server's major version or newer.
    #[serde(default = "default_pg_dump")]
    pub pg_dump: PathBuf,
    /// The `ssh` client restic's SFTP backend runs.
    #[serde(default = "default_ssh")]
    pub ssh: PathBuf,
    /// `ssh-keyscan`, which a destination's test pins its host key with.
    #[serde(default = "default_ssh_keyscan")]
    pub ssh_keyscan: PathBuf,
    /// Where keys, known hosts and restic's cache are kept.
    #[serde(default = "default_work_dir")]
    pub work_dir: PathBuf,
    /// How often requested and scheduled runs are looked for, in seconds.
    #[serde(default = "default_poll")]
    pub poll_secs: u64,
    /// The files backed up, by name: each a directory this service has
    /// mounted (read-only), stored under `/files/<name>` in a snapshot.
    #[serde(default)]
    pub paths: BTreeMap<String, PathBuf>,
    /// The master key ring, put in a snapshot only for a destination that
    /// asks (`include_key`).
    #[serde(default)]
    pub key_ring: Option<PathBuf>,
}

impl Default for BackupSection {
    fn default() -> Self {
        Self {
            restic: default_restic(),
            pg_dump: default_pg_dump(),
            ssh: default_ssh(),
            ssh_keyscan: default_ssh_keyscan(),
            work_dir: default_work_dir(),
            poll_secs: default_poll(),
            paths: BTreeMap::new(),
            key_ring: None,
        }
    }
}

fn default_restic() -> PathBuf {
    "restic".into()
}
fn default_pg_dump() -> PathBuf {
    "pg_dump".into()
}
fn default_ssh() -> PathBuf {
    "ssh".into()
}
fn default_ssh_keyscan() -> PathBuf {
    "ssh-keyscan".into()
}
fn default_work_dir() -> PathBuf {
    "/var/lib/fcp-backup".into()
}
fn default_poll() -> u64 {
    30
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsSection {
    /// The events server, for `system.backup_completed` and
    /// `system.backup_failed`; unset, none are sent.
    #[serde(default)]
    pub server_url: Option<String>,
    /// `FCP_BACKUP_EVENTS_AUTH_TOKEN` wins.
    #[serde(default)]
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterSection {
    /// The cluster's Redis, for this service's beacon and log lines; unset,
    /// neither.
    #[serde(default)]
    pub redis_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingSection {
    #[serde(default = "default_level")]
    pub level: String,
    #[serde(default = "default_format")]
    pub format: String,
}

impl Default for LoggingSection {
    fn default() -> Self {
        Self {
            level: default_level(),
            format: default_format(),
        }
    }
}

fn default_level() -> String {
    "info".into()
}
fn default_format() -> String {
    "compact".into()
}

pub fn load_config(path: &str) -> anyhow::Result<BackupConfig> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read config '{path}': {e}"))?;
    fcp_core::config::parse_toml(&text)
        .map_err(|e| e.context(format!("Failed to parse config '{path}'")))
}

/// What must hold before the service starts.
pub fn validate(config: &BackupConfig) -> anyhow::Result<()> {
    if config.storage.database_url.trim().is_empty() {
        anyhow::bail!("[storage] database_url names the platform's database");
    }
    for (name, path) in &config.backup.paths {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            anyhow::bail!("[backup.paths] {name:?}: a name is letters, digits, '-' or '_'");
        }
        if !path.is_absolute() {
            anyhow::bail!(
                "[backup.paths] {name}: {} is not an absolute path",
                path.display()
            );
        }
    }
    if config.backup.poll_secs == 0 {
        anyhow::bail!("[backup] poll_secs is at least 1");
    }
    Ok(())
}
