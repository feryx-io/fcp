//! PostgreSQL storage backend for FCP.
//!
//! This module provides a production-ready PostgreSQL implementation of the FCP storage traits.
//! It uses sqlx for async database access with connection pooling and migrations.
//!
//! # Features
//! - Connection pooling for high performance
//! - Automatic migrations on startup
//! - Full implementation of CredentialStore, RegistrationStore, and CdrStore traits
//! - JSONB for efficient JSON storage
//! - Native timestamp support with TIMESTAMPTZ
//! - GIN indexes for fast JSON queries
//!
//! # Example
//! ```no_run
//! use fcp_storage_postgres::{PostgresStorage, PostgresConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = PostgresConfig::new("postgresql://user:pass@localhost/fcp");
//!     let storage = PostgresStorage::new(config).await?;
//!
//!     // Use storage via traits
//!     // storage.get_user("alice").await?;
//!
//!     Ok(())
//! }
//! ```

pub mod api_token;
mod cdr_archive;
mod cdr_volume;
pub mod compliance;
pub mod config;
mod deletion;
pub mod emergency;
pub mod encryption;
pub mod flow_trace;
mod holds;
pub mod prompt;
pub mod recording;
pub mod sip_trace;
mod subject_erase;
mod subject_export;
mod subject_requests;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::porting::PortingStore;
use fcp_core::storage::{CdrCount, RollupRun};
use fcp_core::{
    storage::{
        AvailableNumberQuery, CdrStore, ConferenceStore, CredentialStore, DeviceStore,
        ExtensionStore, GeoRoutingStore, LineBindingStore, PhoneNumberStore, PresenceHistoryStore,
        PresenceStore, PresenceSummary, QueueConfigStore, QueueStore, RegistrationStore,
        ResourceListStore, RoutingRuleStore, SiteLocationStore, StoredAgent, StoredAuthPolicy,
        StoredPresenceChange, StoredPresentity, StoredResourceList, StoredSubscription,
        TenantStore, TrunkStore, VoicemailStore, WebhookDeliveryLog, WebhookEndpoint, WebhookStore,
    },
    AccountStatus, AudioCodec, BindingType, CallDetailRecord, CallDisposition, CdrQuery, Device,
    DeviceBinding, DeviceId, DeviceType, EmergencyContact, Extension, ExtensionId, ExtensionTarget,
    HangupCause, HangupParty, LineBinding, LineBindingId, LocationId, MediaPreferences,
    NumberState, NumberUsage, PhoneNumber, PhoneNumberId, QueueConfigId, QueueConfigRecord,
    Registration, RegistrationState, RingStrategy, RoutingAction, RoutingRule, RoutingRuleId,
    SiteLocation, Tenant, TenantId, Trunk, TrunkDirection, TrunkId, UsageAggregation, User, UserId,
    WebhookId,
};
use fcp_flow::schema::{FlowDefinition, FlowNode};
use fcp_flow::storage::{FlowQuery, FlowQueryResult, FlowStore, SortField, SortOrder};
use fcp_flow::FlowError;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Pool, Postgres, Row};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

use crate::encryption::EncryptionState;
use fcp_core::encryption::{EncryptionStatus, EncryptionStore, ReencryptReport};
use fcp_crypto::{EncryptionSettings, FieldCipher};

// Helper functions for SystemTime <-> chrono::DateTime conversion
fn system_time_to_datetime(st: SystemTime) -> DateTime<Utc> {
    let duration = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    DateTime::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos()).unwrap_or_default()
}

/// A timestamp to the database's precision (microseconds): what a record's
/// times are read with, so a keyset cursor names a row's position exactly.
fn datetime_to_system_time_micros(dt: DateTime<Utc>) -> SystemTime {
    match u64::try_from(dt.timestamp_micros()) {
        Ok(micros) => UNIX_EPOCH + Duration::from_micros(micros),
        Err(_) => UNIX_EPOCH,
    }
}

fn datetime_to_system_time(dt: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(dt.timestamp() as u64)
}

// Match a CDR URI column against a tenant-owned E.164 number across the URI
// shapes we persist today (bare E.164, tel:, sip:, sips:, and bracketed forms).
fn phone_number_uri_match_sql(column: &str) -> String {
    format!(
        "{col} = pn.e164
         OR {col} = 'tel:' || pn.e164
         OR {col} LIKE 'tel:' || pn.e164 || ';%'
         OR {col} LIKE '%<tel:' || pn.e164 || '>%'
         OR {col} LIKE '%<tel:' || pn.e164 || ';%'
         OR {col} = 'sip:' || pn.e164
         OR {col} = 'sips:' || pn.e164
         OR {col} LIKE 'sip:' || pn.e164 || '@%'
         OR {col} LIKE 'sips:' || pn.e164 || '@%'
         OR {col} LIKE 'sip:' || pn.e164 || ';%'
         OR {col} LIKE 'sips:' || pn.e164 || ';%'
         OR {col} LIKE '%<sip:' || pn.e164 || '>%'
         OR {col} LIKE '%<sips:' || pn.e164 || '>%'
         OR {col} LIKE '%<sip:' || pn.e164 || '@%'
         OR {col} LIKE '%<sips:' || pn.e164 || '@%'
         OR {col} LIKE '%<sip:' || pn.e164 || ';%'
         OR {col} LIKE '%<sips:' || pn.e164 || ';%'",
        col = column
    )
}

fn webhook_endpoint_from_value(value: serde_json::Value) -> Result<WebhookEndpoint> {
    serde_json::from_value(value).context("Failed to deserialize webhook endpoint")
}

fn webhook_delivery_log_from_value(value: serde_json::Value) -> Result<WebhookDeliveryLog> {
    serde_json::from_value(value).context("Failed to deserialize webhook delivery log")
}

fn geo_rule_from_value(value: serde_json::Value) -> Result<fcp_core::GeoRoutingRule> {
    serde_json::from_value(value).context("Failed to deserialize geo routing rule")
}

fn phone_number_location_from_value(
    value: serde_json::Value,
) -> Result<fcp_core::PhoneNumberLocation> {
    serde_json::from_value(value).context("Failed to deserialize phone number location")
}

fn port_request_from_value(value: serde_json::Value) -> Result<fcp_core::PortRequest> {
    serde_json::from_value(value).context("Failed to deserialize port request")
}

fn port_status_to_db(status: fcp_core::PortStatus) -> &'static str {
    match status {
        fcp_core::PortStatus::Draft => "draft",
        fcp_core::PortStatus::Submitted => "submitted",
        fcp_core::PortStatus::Pending => "pending",
        fcp_core::PortStatus::Approved => "approved",
        fcp_core::PortStatus::Scheduled => "scheduled",
        fcp_core::PortStatus::InProgress => "in_progress",
        fcp_core::PortStatus::Completed => "completed",
        fcp_core::PortStatus::Rejected => "rejected",
        fcp_core::PortStatus::Cancelled => "cancelled",
        fcp_core::PortStatus::Failed => "failed",
    }
}

/// Configuration for PostgreSQL storage backend.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// PostgreSQL connection string (e.g., "postgresql://user:pass@localhost/fcp")
    pub connection_string: String,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Connection timeout
    pub connection_timeout: Duration,
    /// Encryption at rest. `None` reads [`EncryptionSettings::from_env`] at
    /// connect time (`FCP_ENCRYPTION_PROVIDER`, `FCP_MASTER_KEY`, …), so
    /// every binary configures it the same way; tests pass explicit settings.
    pub encryption: Option<EncryptionSettings>,
    /// A streaming read replica (hot standby) to serve configuration reads
    /// from — typically the one in this node's region. Writes, and reads
    /// that must be current, always go to `connection_string`. See
    /// `docs/MULTI_REGION.md`.
    pub read_replica_url: Option<String>,
    /// Replication lag beyond which the replica is left alone and reads go
    /// to the primary until it catches up. Also the read-after-write window:
    /// for this long after a write through this backend, its reads go to
    /// the primary so the writer sees its own change.
    pub max_replica_lag: Duration,
}

impl PostgresConfig {
    /// Create a new PostgreSQL configuration with default settings.
    pub fn new<S: Into<String>>(connection_string: S) -> Self {
        Self {
            connection_string: connection_string.into(),
            max_connections: 10,
            connection_timeout: Duration::from_secs(30),
            encryption: None,
            read_replica_url: None,
            max_replica_lag: Duration::from_secs(5),
        }
    }

    /// Serve configuration reads from a streaming replica.
    pub fn with_read_replica<S: Into<String>>(mut self, url: S) -> Self {
        self.read_replica_url = Some(url.into());
        self
    }

    /// Replication lag tolerated before reads return to the primary.
    pub fn with_max_replica_lag(mut self, lag: Duration) -> Self {
        self.max_replica_lag = lag;
        self
    }

    /// Use explicit encryption settings instead of the environment.
    pub fn with_encryption(mut self, settings: EncryptionSettings) -> Self {
        self.encryption = Some(settings);
        self
    }

    /// Set the maximum number of connections in the pool.
    pub fn with_max_connections(mut self, max: u32) -> Self {
        self.max_connections = max;
        self
    }

    /// Set the connection timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = timeout;
        self
    }
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self::new("postgresql://fcp:fcp@localhost/fcp")
    }
}

/// PostgreSQL storage backend implementing all FCP storage traits.
#[derive(Clone)]
pub struct PostgresStorage {
    pool: Pool<Postgres>,
    /// Present when encryption at rest is enabled for this deployment.
    encryption: Option<Arc<EncryptionState>>,
    /// Present when a read replica is configured.
    replica: Option<Arc<ReplicaState>>,
    /// When this backend last wrote, as milliseconds since the Unix epoch
    /// (0 = never). Reads within `max_replica_lag` of it go to the primary.
    last_write_unix_ms: Arc<AtomicU64>,
}

/// A read replica and what the last probe found out about it.
struct ReplicaState {
    pool: Pool<Postgres>,
    /// Reachable and within `max_lag` at the last probe.
    healthy: AtomicBool,
    /// Replication lag at the last probe, in milliseconds.
    lag_ms: AtomicU64,
    /// Whether the last probe reached the replica at all.
    reachable: AtomicBool,
    max_lag: Duration,
    /// The URL's host (for logs and health), never its credentials.
    host: String,
    /// The replica turned out to be a primary (not in recovery); said once.
    warned_not_standby: AtomicBool,
    /// When the last probe finished (Unix ms; 0 = never), so a probe that
    /// stalled reads as stale rather than healthy for ever.
    last_probe_unix_ms: AtomicU64,
}

/// What a health endpoint can say about the read replica.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReplicaStatus {
    /// Host part of the replica URL.
    pub host: String,
    /// Reachable at the last probe.
    pub reachable: bool,
    /// Reachable and within the lag limit: configuration reads go here.
    pub healthy: bool,
    /// Replication lag at the last probe, in milliseconds.
    pub lag_ms: u64,
    /// The lag limit, in milliseconds.
    pub max_lag_ms: u64,
    /// How long ago the last probe finished, in seconds. The probe runs
    /// every five seconds; much more than that means it is stuck.
    pub probe_age_secs: u64,
}

impl ReplicaStatus {
    /// Prometheus exposition of this status, with `labels` (for example
    /// `node="cm-1"`) on every series.
    pub fn prometheus(&self, labels: &str) -> String {
        let host = self.host.replace('"', "");
        let labels = if labels.is_empty() {
            format!("replica=\"{host}\"")
        } else {
            format!("{labels},replica=\"{host}\"")
        };
        format!(
            "# HELP fcp_replica_reachable Whether the read replica answered the last probe\n\
             # TYPE fcp_replica_reachable gauge\n\
             fcp_replica_reachable{{{labels}}} {}\n\
             # HELP fcp_replica_healthy Whether configuration reads go to the read replica (reachable and within max lag)\n\
             # TYPE fcp_replica_healthy gauge\n\
             fcp_replica_healthy{{{labels}}} {}\n\
             # HELP fcp_replica_lag_seconds Replication lag at the last probe\n\
             # TYPE fcp_replica_lag_seconds gauge\n\
             fcp_replica_lag_seconds{{{labels}}} {}\n\
             # HELP fcp_replica_probe_age_seconds Seconds since the last replica probe finished\n\
             # TYPE fcp_replica_probe_age_seconds gauge\n\
             fcp_replica_probe_age_seconds{{{labels}}} {}\n",
            u8::from(self.reachable),
            u8::from(self.healthy),
            self.lag_ms as f64 / 1000.0,
            self.probe_age_secs,
        )
    }
}

/// A binary's way to ask about its replica after the backend has been
/// dropped into `DynStorage` (`docs/DEPLOYMENT_GAPS.md` §1.7). Cheap to
/// clone; `none()` for a binary without a database.
#[derive(Clone, Default)]
pub struct ReplicaHealth(Option<Arc<PostgresStorage>>);

impl std::fmt::Debug for ReplicaHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ReplicaHealth")
            .field(&self.status())
            .finish()
    }
}

impl ReplicaHealth {
    /// The health of `backend`'s replica.
    pub fn of(backend: Arc<PostgresStorage>) -> Self {
        Self(Some(backend))
    }

    /// No database, so no replica.
    pub fn none() -> Self {
        Self(None)
    }

    /// The replica's state as of the last probe; `None` without one.
    pub fn status(&self) -> Option<ReplicaStatus> {
        self.0.as_ref().and_then(|b| b.replica_status())
    }

    /// What `/status` or `/ready` says under `replica`: the status, or
    /// `null` when no replica is configured.
    pub fn json(&self) -> serde_json::Value {
        self.status()
            .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
            .unwrap_or(serde_json::Value::Null)
    }

    /// The replica's series for `/metrics`; empty without a replica.
    pub fn prometheus(&self, labels: &str) -> String {
        self.status()
            .map(|s| s.prometheus(labels))
            .unwrap_or_default()
    }
}

/// How often the replica is probed for reachability and lag.
const REPLICA_PROBE_INTERVAL: Duration = Duration::from_secs(5);
/// How long one probe may take before the replica counts as unreachable
/// (the pool would otherwise keep retrying for its whole acquire timeout).
const REPLICA_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Host part of a connection URL, for logs.
fn url_host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("")
        .to_string()
}

/// Parse a Postgres LSN (`X/Y`, hex) into a comparable number.
fn parse_lsn(lsn: &str) -> Option<u64> {
    let (hi, lo) = lsn.trim().split_once('/')?;
    Some((u64::from_str_radix(hi, 16).ok()? << 32) | u64::from_str_radix(lo, 16).ok()?)
}

/// Replication lag from what the primary and the replica report.
///
/// A replica that has replayed everything the primary has written lags by
/// nothing, whatever `pg_last_xact_replay_timestamp()` says — on an idle
/// primary that timestamp only grows older. Otherwise the age of the last
/// replayed transaction is the lag. A "replica" that is not in recovery is
/// a primary; it lags by nothing.
fn replication_lag(
    in_recovery: bool,
    primary_lsn: Option<&str>,
    replay_lsn: Option<&str>,
    replay_age: Option<f64>,
) -> Duration {
    if !in_recovery {
        return Duration::ZERO;
    }
    if let (Some(p), Some(r)) = (
        primary_lsn.and_then(parse_lsn),
        replay_lsn.and_then(parse_lsn),
    ) {
        if r >= p {
            return Duration::ZERO;
        }
    }
    Duration::from_secs_f64(replay_age.unwrap_or(0.0).max(0.0))
}

impl PostgresStorage {
    /// Create a new PostgreSQL storage backend with the given configuration.
    ///
    /// This will:
    /// 1. Create a connection pool
    /// 2. Test the connection
    /// 3. Run any pending migrations
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Connection to PostgreSQL fails
    /// - Migrations fail to run
    pub async fn new(config: PostgresConfig) -> Result<Self> {
        info!("Connecting to PostgreSQL at {}", config.connection_string);

        // Parse connection options
        let options = PgConnectOptions::from_str(&config.connection_string)
            .context("Invalid PostgreSQL connection string")?;

        // Create connection pool
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(config.connection_timeout)
            .connect_with(options)
            .await
            .context("Failed to connect to PostgreSQL")?;

        info!("PostgreSQL connection pool created");

        // Run migrations
        info!("Running PostgreSQL migrations...");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("Failed to run PostgreSQL migrations")?;

        info!("PostgreSQL migrations complete");

        // The record table's coming months (docs/CDR_PIPELINE.md §8); the
        // maintenance job keeps them coming. Not fatal: rows past the last
        // partition land in the default one.
        match cdr_volume::ensure_partitions(&pool).await {
            Ok(created) if !created.is_empty() => {
                info!(partitions = ?created, "Created CDR partitions")
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "Could not create CDR partitions"),
        }

        // Encryption at rest: resolve the provider and load the key ring.
        let settings = match config.encryption {
            Some(settings) => settings,
            None => EncryptionSettings::from_env().context("Invalid encryption configuration")?,
        };
        let encryption = match settings
            .kek()
            .await
            .context("Failed to initialize encryption key provider")?
        {
            Some(kek) => Some(Arc::new(
                EncryptionState::load(&pool, kek, settings.provider_name())
                    .await
                    .context("Failed to load data encryption keys")?,
            )),
            None => {
                info!("Encryption at rest disabled (FCP_ENCRYPTION_PROVIDER not set)");
                None
            }
        };

        // Read replica: connect lazily so a replica that is down at start
        // does not stop the node; the probe decides whether reads go there.
        let replica = match config.read_replica_url.as_deref() {
            Some(url) if !url.is_empty() => {
                let options = PgConnectOptions::from_str(url)
                    .context("Invalid read replica connection string")?;
                let pool = PgPoolOptions::new()
                    .max_connections(config.max_connections)
                    .acquire_timeout(config.connection_timeout)
                    .connect_lazy_with(options);
                let state = Arc::new(ReplicaState {
                    pool,
                    healthy: AtomicBool::new(false),
                    lag_ms: AtomicU64::new(0),
                    reachable: AtomicBool::new(false),
                    max_lag: config.max_replica_lag,
                    host: url_host(url),
                    warned_not_standby: AtomicBool::new(false),
                    last_probe_unix_ms: AtomicU64::new(0),
                });
                info!(
                    replica = %state.host,
                    max_lag_ms = config.max_replica_lag.as_millis() as u64,
                    "Read replica configured; configuration reads go there while it is healthy"
                );
                Some(state)
            }
            _ => None,
        };

        let storage = Self {
            pool,
            encryption,
            replica,
            last_write_unix_ms: Arc::new(AtomicU64::new(0)),
        };
        if storage.replica.is_some() {
            // First probe before serving, then in the background.
            storage.probe_replica().await;
            let probe = storage.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(REPLICA_PROBE_INTERVAL);
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tick.tick().await;
                    probe.probe_replica().await;
                }
            });
        }
        Ok(storage)
    }

    // ── Read replica routing ───────────────────────────────────────────

    /// The pool for a read that may be served by the replica: the replica
    /// while it is healthy and this backend has not written within the
    /// lag window, else the primary. Configuration-type reads use this;
    /// anything that must be current, and every write, uses
    /// [`writes`](Self::writes).
    pub(crate) fn reads(&self) -> &Pool<Postgres> {
        let Some(replica) = &self.replica else {
            return &self.pool;
        };
        if !replica.healthy.load(Ordering::Relaxed) {
            return &self.pool;
        }
        let last_write = self.last_write_unix_ms.load(Ordering::Relaxed);
        if last_write != 0
            && unix_ms().saturating_sub(last_write) < replica.max_lag.as_millis() as u64
        {
            return &self.pool;
        }
        &replica.pool
    }

    /// The primary. Records the write so the next reads from this backend
    /// see it even with a lagging replica.
    pub(crate) fn writes(&self) -> &Pool<Postgres> {
        if self.replica.is_some() {
            self.last_write_unix_ms.store(unix_ms(), Ordering::Relaxed);
        }
        &self.pool
    }

    /// Whether the next replica-eligible read would go to the replica.
    pub fn reads_from_replica(&self) -> bool {
        match &self.replica {
            Some(replica) => std::ptr::eq(self.reads(), &replica.pool),
            None => false,
        }
    }

    /// The replica's state as of the last probe; `None` without a replica.
    pub fn replica_status(&self) -> Option<ReplicaStatus> {
        self.replica.as_ref().map(|r| ReplicaStatus {
            host: r.host.clone(),
            reachable: r.reachable.load(Ordering::Relaxed),
            healthy: r.healthy.load(Ordering::Relaxed),
            lag_ms: r.lag_ms.load(Ordering::Relaxed),
            max_lag_ms: r.max_lag.as_millis() as u64,
            probe_age_secs: match r.last_probe_unix_ms.load(Ordering::Relaxed) {
                0 => u64::MAX / 1000,
                at => unix_ms().saturating_sub(at) / 1000,
            },
        })
    }

    /// Probe the replica once: reachable, in recovery, and how far behind
    /// the primary. Updates what [`reads`](Self::reads) decides on. Runs
    /// every few seconds in the background; public so tests and health
    /// checks can force one.
    pub async fn probe_replica(&self) {
        let Some(replica) = &self.replica else {
            return;
        };
        let was_healthy = replica.healthy.load(Ordering::Relaxed);
        let probe = async {
            let primary_lsn: Option<String> =
                sqlx::query_scalar("SELECT pg_current_wal_lsn()::text")
                    .fetch_one(&self.pool)
                    .await
                    .ok();
            let row = sqlx::query(
                "SELECT pg_is_in_recovery() AS in_recovery,                         pg_last_wal_replay_lsn()::text AS replay_lsn,                         EXTRACT(EPOCH FROM (now() - pg_last_xact_replay_timestamp()))::float8 AS replay_age",
            )
            .fetch_one(&replica.pool)
            .await?;
            let in_recovery: bool = row.try_get("in_recovery")?;
            let replay_lsn: Option<String> = row.try_get("replay_lsn")?;
            let replay_age: Option<f64> = row.try_get("replay_age")?;
            Ok::<_, sqlx::Error>((in_recovery, primary_lsn, replay_lsn, replay_age))
        };
        let outcome = match tokio::time::timeout(REPLICA_PROBE_TIMEOUT, probe).await {
            Ok(outcome) => outcome,
            Err(_) => Err(sqlx::Error::PoolTimedOut),
        };
        replica
            .last_probe_unix_ms
            .store(unix_ms(), Ordering::Relaxed);
        match outcome {
            Ok((in_recovery, primary_lsn, replay_lsn, replay_age)) => {
                if !in_recovery && !replica.warned_not_standby.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        replica = %replica.host,
                        "Read replica is not in recovery (it is a primary, not a standby); using it as-is"
                    );
                }
                let lag = replication_lag(
                    in_recovery,
                    primary_lsn.as_deref(),
                    replay_lsn.as_deref(),
                    replay_age,
                );
                let healthy = lag <= replica.max_lag;
                replica.reachable.store(true, Ordering::Relaxed);
                replica
                    .lag_ms
                    .store(lag.as_millis() as u64, Ordering::Relaxed);
                replica.healthy.store(healthy, Ordering::Relaxed);
                if healthy != was_healthy {
                    if healthy {
                        info!(replica = %replica.host, lag_ms = lag.as_millis() as u64, "Read replica healthy; configuration reads go to it");
                    } else {
                        tracing::warn!(replica = %replica.host, lag_ms = lag.as_millis() as u64, max_lag_ms = replica.max_lag.as_millis() as u64, "Read replica lagging; configuration reads back on the primary");
                    }
                }
            }
            Err(e) => {
                replica.reachable.store(false, Ordering::Relaxed);
                replica.healthy.store(false, Ordering::Relaxed);
                if was_healthy {
                    tracing::warn!(replica = %replica.host, error = %e, "Read replica unreachable; configuration reads back on the primary");
                } else {
                    debug!(replica = %replica.host, error = %e, "Read replica probe failed");
                }
            }
        }
    }

    /// Force the replica's health flag (tests only).
    #[doc(hidden)]
    pub fn set_replica_healthy(&self, healthy: bool) {
        if let Some(replica) = &self.replica {
            replica.healthy.store(healthy, Ordering::Relaxed);
        }
    }

    // ── Encryption at rest helpers ───────────────────────────────────────

    /// Seal a secret for storage (identity when encryption is disabled).
    fn seal_opt(&self, value: Option<&str>) -> Result<Option<String>> {
        match (&self.encryption, value) {
            (_, None) => Ok(None),
            (None, Some(v)) => Ok(Some(v.to_string())),
            (Some(enc), Some(v)) => Ok(Some(enc.seal(v)?)),
        }
    }

    /// Open a stored secret. Legacy plaintext passes through; a sealed value
    /// on a backend without encryption configured is an error rather than
    /// a silently wrong credential.
    async fn open_opt(&self, value: Option<String>) -> Result<Option<String>> {
        let Some(v) = value else { return Ok(None) };
        match &self.encryption {
            None if FieldCipher::is_sealed(&v) => {
                Err(fcp_crypto::CryptoError::NotConfigured.into())
            }
            None => Ok(Some(v)),
            Some(enc) => Ok(Some(enc.open(&self.pool, &v).await?)),
        }
    }

    async fn open_user(&self, mut user: User) -> Result<User> {
        user.digest_password = self.open_opt(user.digest_password).await?;
        user.sip_ha1_hash = self.open_opt(user.sip_ha1_hash).await?;
        user.sip_provisioning_password = self.open_opt(user.sip_provisioning_password).await?;
        Ok(user)
    }

    async fn open_device(&self, mut device: Device) -> Result<Device> {
        device.sip_ha1_hash = self.open_opt(device.sip_ha1_hash).await?;
        device.sip_provisioning_password = self.open_opt(device.sip_provisioning_password).await?;
        Ok(device)
    }

    async fn open_users(&self, users: Vec<User>) -> Result<Vec<User>> {
        let mut out = Vec::with_capacity(users.len());
        for user in users {
            out.push(self.open_user(user).await?);
        }
        Ok(out)
    }

    async fn open_trunk(&self, mut trunk: Trunk) -> Result<Trunk> {
        if let Some(credentials) = trunk.credentials.as_mut() {
            credentials.password = self
                .open_opt(Some(std::mem::take(&mut credentials.password)))
                .await?
                .unwrap_or_default();
        }
        Ok(trunk)
    }

    async fn open_trunks(&self, trunks: Vec<Trunk>) -> Result<Vec<Trunk>> {
        let mut out = Vec::with_capacity(trunks.len());
        for trunk in trunks {
            out.push(self.open_trunk(trunk).await?);
        }
        Ok(out)
    }

    async fn open_meeting(&self, mut meeting: fcp_core::Meeting) -> Result<fcp_core::Meeting> {
        meeting.guest_pin = self.open_opt(meeting.guest_pin).await?;
        meeting.host_pin = self.open_opt(meeting.host_pin).await?;
        Ok(meeting)
    }

    async fn open_meetings(
        &self,
        meetings: Vec<fcp_core::Meeting>,
    ) -> Result<Vec<fcp_core::Meeting>> {
        let mut out = Vec::with_capacity(meetings.len());
        for meeting in meetings {
            out.push(self.open_meeting(meeting).await?);
        }
        Ok(out)
    }

    fn seal_webhook(&self, mut endpoint: WebhookEndpoint) -> Result<WebhookEndpoint> {
        endpoint.secret = self.seal_opt(Some(&endpoint.secret))?.unwrap_or_default();
        Ok(endpoint)
    }

    async fn open_webhook(&self, mut endpoint: WebhookEndpoint) -> Result<WebhookEndpoint> {
        endpoint.secret = self
            .open_opt(Some(endpoint.secret))
            .await?
            .unwrap_or_default();
        Ok(endpoint)
    }

    /// Get a reference to the underlying connection pool.
    ///
    /// This is useful for running custom queries not covered by the trait methods.
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    /// Close the connection pool and wait for all connections to be released.
    pub async fn close(&self) {
        self.pool.close().await;
        if let Some(replica) = &self.replica {
            replica.pool.close().await;
        }
    }

    /// Helper method to get device bindings for a line binding
    async fn get_device_bindings(
        &self,
        line_binding_id: &LineBindingId,
    ) -> Result<Vec<DeviceBinding>> {
        let rows = sqlx::query(
            "SELECT line_binding_id, device_id, priority, line_appearance, shared_line, enabled
             FROM device_bindings
             WHERE line_binding_id = $1
             ORDER BY priority",
        )
        .bind(line_binding_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let bindings = rows
            .into_iter()
            .filter_map(|row| {
                let device_id = DeviceId::from_str(&row.get::<String, _>("device_id")).ok()?;
                Some(DeviceBinding {
                    device_id,
                    priority: row.get::<i32, _>("priority") as u8,
                    line_appearance: row.get::<i32, _>("line_appearance") as u8,
                    shared_line: row.get("shared_line"),
                    enabled: row.get("enabled"),
                })
            })
            .collect();

        Ok(bindings)
    }

    /// Helper method to store a device binding
    async fn put_device_binding(
        &self,
        line_binding_id: &LineBindingId,
        binding: &DeviceBinding,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO device_bindings (line_binding_id, device_id, priority, line_appearance, shared_line, enabled)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT(line_binding_id, device_id) DO UPDATE SET
                 priority = EXCLUDED.priority,
                 line_appearance = EXCLUDED.line_appearance,
                 shared_line = EXCLUDED.shared_line,
                 enabled = EXCLUDED.enabled",
        )
        .bind(line_binding_id.to_string())
        .bind(binding.device_id.to_string())
        .bind(binding.priority as i32)
        .bind(binding.line_appearance as i32)
        .bind(binding.shared_line)
        .bind(binding.enabled)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

// ============================================================================
// CredentialStore Implementation
// ============================================================================

#[async_trait]
impl CredentialStore for PostgresStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        let rows = sqlx::query(
            "SELECT id, username, password_hash, digest_password, sip_ha1_hash, sip_provisioning_password, display_name, tenant_id,
                    role, department_id, enabled, created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, location_id, language, timezone,
                    account_status, last_login, emergency_contact_json,
                    primary_phone_number_id, voicemail_box_id, profile_picture,
                    failed_login_attempts, account_locked_until
             FROM users
             WHERE username = $1",
        )
        .bind(username)
        .fetch_all(self.reads())
        .await?;

        match rows.len() {
            0 => Ok(None),
            1 => Ok(Some(
                self.open_user(row_to_user(rows.into_iter().next().unwrap())?)
                    .await?,
            )),
            _ => anyhow::bail!("username '{}' exists in multiple tenants", username),
        }
    }

    async fn get_user_in_tenant(
        &self,
        username: &str,
        tenant_id: &TenantId,
    ) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, digest_password, sip_ha1_hash, sip_provisioning_password, display_name, tenant_id,
                    role, department_id, enabled, created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, location_id, language, timezone,
                    account_status, last_login, emergency_contact_json,
                    primary_phone_number_id, voicemail_box_id, profile_picture,
                    failed_login_attempts, account_locked_until
             FROM users
             WHERE username = $1 AND tenant_id = $2",
        )
        .bind(username)
        .bind(tenant_id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(self.open_user(row_to_user(row)?).await?)),
            None => Ok(None),
        }
    }

    async fn list_users_by_username(&self, username: &str) -> Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, username, password_hash, digest_password, sip_ha1_hash, sip_provisioning_password, display_name, tenant_id,
                    role, department_id, enabled, created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, location_id, language, timezone,
                    account_status, last_login, emergency_contact_json,
                    primary_phone_number_id, voicemail_box_id, profile_picture,
                    failed_login_attempts, account_locked_until
             FROM users
             WHERE username = $1
             ORDER BY tenant_id",
        )
        .bind(username)
        .fetch_all(self.reads())
        .await?;

        let users = rows
            .into_iter()
            .filter_map(|row| row_to_user(row).ok())
            .collect();
        let users = self.open_users(users).await?;

        Ok(users)
    }

    async fn get_user_by_id(&self, user_id: &fcp_core::UserId) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, digest_password, sip_ha1_hash, sip_provisioning_password, display_name, tenant_id,
                    role, department_id, enabled, created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, location_id, language, timezone,
                    account_status, last_login, emergency_contact_json,
                    primary_phone_number_id, voicemail_box_id, profile_picture,
                    failed_login_attempts, account_locked_until
             FROM users
             WHERE id = $1",
        )
        .bind(user_id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(self.open_user(row_to_user(row)?).await?)),
            None => Ok(None),
        }
    }

    async fn put_user(&self, user: User) -> Result<()> {
        let id = user.id.to_string();
        let tenant_id = user.tenant_id.to_string();
        let role = serde_json::to_string(&user.role)?
            .trim_matches('"')
            .to_string();
        let account_status = user.account_status.to_string();
        let media_preferences = serde_json::to_value(&user.media_preferences)?;
        let emergency_contact = serde_json::to_value(&user.emergency_contact)?;
        let created_at = system_time_to_datetime(user.created_at);
        let updated_at = system_time_to_datetime(user.updated_at);
        let password_last_reset = user.password_last_reset.map(system_time_to_datetime);
        let last_login = user.last_login.map(system_time_to_datetime);
        let primary_phone_number_id = user.primary_phone_number_id.map(|id| id.to_string());
        let voicemail_box_id = user.voicemail_box_id.map(|id| id.to_string());
        let account_locked_until = user.account_locked_until.map(system_time_to_datetime);
        let location_id = user.location_id.map(|id| id.to_string());
        let digest_password = self.seal_opt(user.digest_password.as_deref())?;
        let sip_ha1_hash = self.seal_opt(user.sip_ha1_hash.as_deref())?;
        let sip_provisioning_password = self.seal_opt(user.sip_provisioning_password.as_deref())?;

        sqlx::query(
            "INSERT INTO users (
                id, username, password_hash, digest_password, sip_ha1_hash, sip_provisioning_password, display_name, tenant_id,
                role, department_id, enabled, created_at, updated_at, media_preferences,
                email, password_last_reset, phone_number, mobile_number, pin,
                extension, department, job_title, location, location_id, language, timezone,
                account_status, last_login, emergency_contact_json,
                primary_phone_number_id, voicemail_box_id, profile_picture,
                failed_login_attempts, account_locked_until
             )
	             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34)
	             ON CONFLICT(username, tenant_id) DO UPDATE SET
	                 password_hash = EXCLUDED.password_hash,
	                 digest_password = EXCLUDED.digest_password,
	                 sip_ha1_hash = EXCLUDED.sip_ha1_hash,
	                 sip_provisioning_password = EXCLUDED.sip_provisioning_password,
	                 display_name = EXCLUDED.display_name,
                 role = EXCLUDED.role,
                 department_id = EXCLUDED.department_id,
                 enabled = EXCLUDED.enabled,
                 updated_at = EXCLUDED.updated_at,
                 media_preferences = EXCLUDED.media_preferences,
                 email = EXCLUDED.email,
                 password_last_reset = EXCLUDED.password_last_reset,
                 phone_number = EXCLUDED.phone_number,
                 mobile_number = EXCLUDED.mobile_number,
                 pin = EXCLUDED.pin,
                 extension = EXCLUDED.extension,
                 department = EXCLUDED.department,
                 job_title = EXCLUDED.job_title,
                 location = EXCLUDED.location,
                 location_id = EXCLUDED.location_id,
                 language = EXCLUDED.language,
                 timezone = EXCLUDED.timezone,
                 account_status = EXCLUDED.account_status,
                 last_login = EXCLUDED.last_login,
                 emergency_contact_json = EXCLUDED.emergency_contact_json,
                 primary_phone_number_id = EXCLUDED.primary_phone_number_id,
                 voicemail_box_id = EXCLUDED.voicemail_box_id,
                 profile_picture = EXCLUDED.profile_picture,
                 failed_login_attempts = EXCLUDED.failed_login_attempts,
                 account_locked_until = EXCLUDED.account_locked_until",
        )
        .bind(&id)
        .bind(&user.username)
        .bind(&user.password)
        .bind(&digest_password)
        .bind(&sip_ha1_hash)
        .bind(&sip_provisioning_password)
        .bind(&user.display_name)
        .bind(&tenant_id)
        .bind(&role)
        .bind(&user.department_id)
        .bind(user.enabled)
        .bind(created_at)
        .bind(updated_at)
        .bind(&media_preferences)
        .bind(&user.email)
        .bind(password_last_reset)
        .bind(&user.phone_number)
        .bind(&user.mobile_number)
        .bind(&user.pin)
        .bind(&user.extension)
        .bind(&user.department)
        .bind(&user.job_title)
        .bind(&user.location)
        .bind(&location_id)
        .bind(&user.language)
        .bind(&user.timezone)
        .bind(&account_status)
        .bind(last_login)
        .bind(&emergency_contact)
        .bind(&primary_phone_number_id)
        .bind(&voicemail_box_id)
        .bind(&user.profile_picture)
        .bind(user.failed_login_attempts)
        .bind(account_locked_until)
        .execute(self.writes())
        .await?;

        debug!(username = %user.username, "User stored");
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        let rows = sqlx::query("SELECT tenant_id FROM users WHERE username = $1")
            .bind(username)
            .fetch_all(self.writes())
            .await?;

        if rows.len() > 1 {
            anyhow::bail!("username '{}' exists in multiple tenants", username);
        }

        sqlx::query("DELETE FROM users WHERE username = $1")
            .bind(username)
            .execute(self.writes())
            .await?;

        debug!(username = %username, "User deleted");
        Ok(())
    }

    async fn delete_user_in_tenant(&self, username: &str, tenant_id: &TenantId) -> Result<()> {
        sqlx::query("DELETE FROM users WHERE username = $1 AND tenant_id = $2")
            .bind(username)
            .bind(tenant_id.to_string())
            .execute(self.writes())
            .await?;

        debug!(username = %username, tenant_id = %tenant_id, "User deleted");
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, username, password_hash, digest_password, sip_ha1_hash, sip_provisioning_password, display_name, tenant_id,
                    role, department_id, enabled, created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, location_id, language, timezone,
                    account_status, last_login, emergency_contact_json,
                    primary_phone_number_id, voicemail_box_id, profile_picture,
                    failed_login_attempts, account_locked_until
             FROM users
             ORDER BY username",
        )
        .fetch_all(self.reads())
        .await?;

        let users = rows
            .into_iter()
            .filter_map(|row| row_to_user(row).ok())
            .collect();
        let users = self.open_users(users).await?;

        Ok(users)
    }
}

// ============================================================================
// RegistrationStore Implementation
// ============================================================================

#[async_trait]
impl RegistrationStore for PostgresStorage {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        let rows = sqlx::query(
            "SELECT tenant_id, aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at, path, flow_token, source_address
             FROM registrations
             WHERE aor = $1
             ORDER BY q DESC NULLS LAST",
        )
        .bind(aor)
        .fetch_all(&self.pool)
        .await?;

        let registrations = rows
            .into_iter()
            .filter_map(|row| row_to_registration(row).ok())
            .collect();

        Ok(registrations)
    }

    async fn get_registrations_in_tenant(
        &self,
        tenant_id: &TenantId,
        aor: &str,
    ) -> Result<Vec<Registration>> {
        let rows = sqlx::query(
            "SELECT tenant_id, aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at, path, flow_token, source_address
             FROM registrations
             WHERE tenant_id = $1 AND aor = $2
             ORDER BY q DESC NULLS LAST",
        )
        .bind(tenant_id.to_string())
        .bind(aor)
        .fetch_all(&self.pool)
        .await?;

        let registrations = rows
            .into_iter()
            .filter_map(|row| row_to_registration(row).ok())
            .collect();

        Ok(registrations)
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        let expires_at = system_time_to_datetime(registration.expires_at);
        let registered_at = system_time_to_datetime(registration.registered_at);

        sqlx::query(
            "INSERT INTO registrations (tenant_id, aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at, path, flow_token, source_address)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (tenant_id, aor, contact)
             DO UPDATE SET
                 expires_at = EXCLUDED.expires_at,
                 call_id = EXCLUDED.call_id,
                 cseq = EXCLUDED.cseq,
                 user_agent = EXCLUDED.user_agent,
                 q = EXCLUDED.q,
                 registered_at = EXCLUDED.registered_at,
                 path = EXCLUDED.path,
                 flow_token = EXCLUDED.flow_token,
                 source_address = EXCLUDED.source_address",
        )
        .bind(registration.tenant_id.to_string())
        .bind(&registration.aor)
        .bind(&registration.contact)
        .bind(expires_at)
        .bind(&registration.call_id)
        .bind(registration.cseq as i64)
        .bind(&registration.user_agent)
        .bind(registration.q)
        .bind(registered_at)
        .bind(&registration.path)
        .bind(&registration.flow_token)
        .bind(&registration.source_address)
        .execute(self.writes())
        .await?;

        debug!(aor = %registration.aor, contact = %registration.contact, "Registration stored");
        Ok(())
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        sqlx::query("DELETE FROM registrations WHERE aor = $1 AND contact = $2")
            .bind(aor)
            .bind(contact)
            .execute(self.writes())
            .await?;

        debug!(aor = %aor, contact = %contact, "Registration deleted");
        Ok(())
    }

    async fn delete_registration_in_tenant(
        &self,
        tenant_id: &TenantId,
        aor: &str,
        contact: &str,
    ) -> Result<()> {
        sqlx::query("DELETE FROM registrations WHERE tenant_id = $1 AND aor = $2 AND contact = $3")
            .bind(tenant_id.to_string())
            .bind(aor)
            .bind(contact)
            .execute(self.writes())
            .await?;

        debug!(tenant_id = %tenant_id, aor = %aor, contact = %contact, "Registration deleted");
        Ok(())
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM registrations WHERE aor = $1")
            .bind(aor)
            .execute(self.writes())
            .await?;

        debug!(aor = %aor, "All registrations for AOR deleted");
        Ok(())
    }

    async fn delete_all_registrations_in_tenant(
        &self,
        tenant_id: &TenantId,
        aor: &str,
    ) -> Result<()> {
        sqlx::query("DELETE FROM registrations WHERE tenant_id = $1 AND aor = $2")
            .bind(tenant_id.to_string())
            .bind(aor)
            .execute(self.writes())
            .await?;

        debug!(tenant_id = %tenant_id, aor = %aor, "All registrations for AOR deleted");
        Ok(())
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        let result = sqlx::query("DELETE FROM registrations WHERE expires_at <= NOW()")
            .execute(self.writes())
            .await?;

        let count = result.rows_affected() as usize;
        if count > 0 {
            debug!(count = count, "Expired registrations cleaned up");
        }

        Ok(count)
    }

    async fn registration_count(&self) -> Result<usize> {
        let row =
            sqlx::query("SELECT COUNT(*) as count FROM registrations WHERE expires_at > NOW()")
                .fetch_one(&self.pool)
                .await?;
        Ok(row.get::<i64, _>("count") as usize)
    }

    async fn get_all_registrations(&self) -> Result<Vec<Registration>> {
        let rows = sqlx::query(
            "SELECT tenant_id, aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at, path, flow_token, source_address
             FROM registrations
             WHERE expires_at > NOW()
             ORDER BY aor, q DESC NULLS LAST",
        )
        .fetch_all(&self.pool)
        .await?;

        let registrations = rows
            .into_iter()
            .filter_map(|row| row_to_registration(row).ok())
            .collect();

        Ok(registrations)
    }

    async fn get_all_registrations_in_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<Registration>> {
        let rows = sqlx::query(
            "SELECT tenant_id, aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at, path, flow_token, source_address
             FROM registrations
             WHERE tenant_id = $1 AND expires_at > NOW()
             ORDER BY aor, q DESC NULLS LAST",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let registrations = rows
            .into_iter()
            .filter_map(|row| row_to_registration(row).ok())
            .collect();

        Ok(registrations)
    }
}

// ============================================================================
// TenantStore Implementation
// ============================================================================

/// Columns returned by every tenant SELECT. Keep this list in sync with
/// `row_to_tenant` below.
const TENANT_SELECT_COLS: &str = "id, name, description, status, contact_email, contact_phone,
    max_users, max_phone_numbers, max_concurrent_calls, billing_id,
    created_at, updated_at, trial_expires_at, metadata, sip_domains,
    platform_trunk_fallback, calling_json";

#[async_trait]
impl TenantStore for PostgresStorage {
    async fn get_tenant(&self, id: &TenantId) -> Result<Option<Tenant>> {
        let row = sqlx::query(&format!(
            "SELECT {} FROM tenants WHERE id = $1",
            TENANT_SELECT_COLS
        ))
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_tenant(row)?)),
            None => Ok(None),
        }
    }

    async fn get_tenant_by_name(&self, name: &str) -> Result<Option<Tenant>> {
        let row = sqlx::query(&format!(
            "SELECT {} FROM tenants WHERE name = $1",
            TENANT_SELECT_COLS
        ))
        .bind(name)
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_tenant(row)?)),
            None => Ok(None),
        }
    }

    async fn get_tenant_by_sip_domain(&self, domain: &str) -> Result<Option<Tenant>> {
        // Normalize the input the same way we normalize on write: trim and
        // lowercase. Reject empty after normalization so we never match a
        // tenant that accidentally has an empty entry in its sip_domains.
        let needle = domain.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return Ok(None);
        }

        // `$1 = ANY(sip_domains)` leverages the GIN index created by
        // migration 042. Limit 1 is a defensive guard — the application
        // layer enforces uniqueness so there should be at most one match.
        let row = sqlx::query(&format!(
            "SELECT {} FROM tenants WHERE $1 = ANY(sip_domains) LIMIT 1",
            TENANT_SELECT_COLS
        ))
        .bind(&needle)
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_tenant(row)?)),
            None => Ok(None),
        }
    }

    async fn list_tenants(&self) -> Result<Vec<Tenant>> {
        let rows = sqlx::query(&format!(
            "SELECT {} FROM tenants ORDER BY name",
            TENANT_SELECT_COLS
        ))
        .fetch_all(self.reads())
        .await?;

        let tenants = rows
            .into_iter()
            .filter_map(|row| row_to_tenant(row).ok())
            .collect();

        Ok(tenants)
    }

    async fn put_tenant(&self, tenant: Tenant) -> Result<()> {
        let id = tenant.id.to_string();
        let status = serde_json::to_string(&tenant.status)?
            .trim_matches('"')
            .to_string();
        let created_at = system_time_to_datetime(tenant.created_at);
        let updated_at = system_time_to_datetime(tenant.updated_at);
        let trial_expires_at = tenant.trial_expires_at.map(system_time_to_datetime);

        // Normalize sip_domains on the write path so that reads never need
        // to worry about case or whitespace, and so that the GIN index
        // lookups match what the application layer queries for.
        let sip_domains: Vec<String> = tenant
            .sip_domains
            .iter()
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect();

        sqlx::query(
            "INSERT INTO tenants (
                id, name, description, status, contact_email, contact_phone,
                max_users, max_phone_numbers, max_concurrent_calls, billing_id,
                created_at, updated_at, trial_expires_at, metadata, sip_domains,
                platform_trunk_fallback, data, calling_json
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, '{}'::jsonb, $17)
             ON CONFLICT(id) DO UPDATE SET
                 name = EXCLUDED.name,
                 description = EXCLUDED.description,
                 status = EXCLUDED.status,
                 contact_email = EXCLUDED.contact_email,
                 contact_phone = EXCLUDED.contact_phone,
                 max_users = EXCLUDED.max_users,
                 max_phone_numbers = EXCLUDED.max_phone_numbers,
                 max_concurrent_calls = EXCLUDED.max_concurrent_calls,
                 billing_id = EXCLUDED.billing_id,
                 updated_at = EXCLUDED.updated_at,
                 trial_expires_at = EXCLUDED.trial_expires_at,
                 metadata = EXCLUDED.metadata,
                 sip_domains = EXCLUDED.sip_domains,
                 platform_trunk_fallback = EXCLUDED.platform_trunk_fallback,
                 calling_json = EXCLUDED.calling_json",
        )
        .bind(&id)
        .bind(&tenant.name)
        .bind(&tenant.description)
        .bind(&status)
        .bind(&tenant.contact_email)
        .bind(&tenant.contact_phone)
        .bind(tenant.max_users.map(|u| u as i64))
        .bind(tenant.max_phone_numbers.map(|p| p as i64))
        .bind(tenant.max_concurrent_calls.map(|c| c as i64))
        .bind(&tenant.billing_id)
        .bind(created_at)
        .bind(updated_at)
        .bind(trial_expires_at)
        .bind(&tenant.metadata)
        .bind(&sip_domains)
        .bind(tenant.platform_trunk_fallback)
        .bind(serde_json::to_value(&tenant.calling)?)
        .execute(self.writes())
        .await?;

        debug!(id = %tenant.id, name = %tenant.name, "Tenant stored");
        Ok(())
    }

    async fn delete_tenant(&self, id: &TenantId) -> Result<()> {
        // Refused while the tenant holds anything (migration 063).
        deletion::execute(
            self.writes(),
            sqlx::query("DELETE FROM tenants WHERE id = $1").bind(id.to_string()),
        )
        .await?;

        debug!(id = %id, "Tenant deleted");
        Ok(())
    }
}

fn row_to_tenant(row: sqlx::postgres::PgRow) -> Result<Tenant> {
    let id = TenantId::from_str(&row.get::<String, _>("id"))?;
    let status_str: String = row.get("status");
    let status = serde_json::from_str::<fcp_core::TenantStatus>(&format!("\"{}\"", status_str))
        .unwrap_or(fcp_core::TenantStatus::Active);

    let created_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at"));
    let updated_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at"));
    let trial_expires_at = row
        .get::<Option<DateTime<Utc>>, _>("trial_expires_at")
        .map(datetime_to_system_time);
    let sip_domains: Vec<String> = row
        .try_get::<Vec<String>, _>("sip_domains")
        .unwrap_or_default();

    Ok(Tenant {
        id,
        name: row.get("name"),
        description: row.get("description"),
        status,
        contact_email: row.get("contact_email"),
        contact_phone: row.get("contact_phone"),
        max_users: row.get::<Option<i32>, _>("max_users").map(|u| u as u32),
        max_phone_numbers: row
            .get::<Option<i32>, _>("max_phone_numbers")
            .map(|p| p as u32),
        max_concurrent_calls: row
            .get::<Option<i32>, _>("max_concurrent_calls")
            .map(|c| c as u32),
        billing_id: row.get("billing_id"),
        created_at,
        updated_at,
        trial_expires_at,
        metadata: row.get("metadata"),
        sip_domains,
        // A tenant whose row predates the column falls back to the
        // platform's trunks, as every tenant did before (`docs/TENANT_TRUNKS.md` §5).
        // Selected by name like every other column: a mapping that quietly
        // defaults is a mapping that can lie about what a tenant said
        // (`docs/TENANT_TRUNKS.md` §5).
        platform_trunk_fallback: row.get("platform_trunk_fallback"),
        calling: serde_json::from_value(row.get("calling_json")).context("a tenant's calling")?,
    })
}

// ============================================================================
// CdrStore Implementation
// ============================================================================

#[async_trait]
impl CdrStore for PostgresStorage {
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()> {
        self.insert_cdrs(vec![cdr]).await.map(|_| ())
    }

    async fn insert_cdrs(&self, cdrs: Vec<CallDetailRecord>) -> Result<Vec<uuid::Uuid>> {
        if cdrs.is_empty() {
            return Ok(Vec::new());
        }
        let rows = cdrs
            .iter()
            .map(CdrRow::from_record)
            .collect::<Result<Vec<_>>>()?;

        // One transaction; the rows go in chunks that keep a statement's
        // binds under Postgres's limit. A replayed cdr_id is skipped, and
        // RETURNING says which rows were new (docs/CDR_PIPELINE.md §6).
        let mut tx = self.writes().begin().await?;
        let mut new_ids = Vec::with_capacity(rows.len());
        for chunk in rows.chunks(CDR_INSERT_CHUNK) {
            let mut query = sqlx::QueryBuilder::<Postgres>::new(
                "INSERT INTO cdrs (
                    call_id, tenant_id, sip_call_id,
                    from_uri, from_display_name, to_uri, to_display_name,
                    start_time, answer_time, end_time,
                    setup_duration_ms, call_duration_ms, total_duration_ms,
                    disposition, hangup_cause, hangup_party,
                    sip_status_code, sip_reason,
                    route_id, gateway_id,
                    media_anchored, codec_a_leg, codec_b_leg,
                    rtp_packets_sent, rtp_packets_received, rtp_bytes_sent, rtp_bytes_received,
                    recording_url, cost, currency, custom_data,
                    cdr_id, kind, schema_version, direction, node_id, region,
                    correlation_id, trunk_name, trunk_id, rtp_packets_lost, rtp_jitter_ms,
                    rate_plan_id, matched_prefix, billable_seconds, rated_at, created_at,
                    recording_consent, site_id
                 ) ",
            );
            query.push_values(chunk, |mut b, row| {
                b.push_bind(&row.call_id)
                    .push_bind(&row.tenant_id)
                    .push_bind(&row.r.sip_call_id)
                    .push_bind(&row.r.from_uri)
                    .push_bind(&row.r.from_display_name)
                    .push_bind(&row.r.to_uri)
                    .push_bind(&row.r.to_display_name)
                    .push_bind(row.start_time)
                    .push_bind(row.answer_time)
                    .push_bind(row.end_time)
                    .push_bind(row.r.setup_duration_ms.map(|d| d as i64))
                    .push_bind(row.r.call_duration_ms.map(|d| d as i64))
                    .push_bind(row.r.total_duration_ms.map(|d| d as i64))
                    .push_bind(&row.disposition)
                    .push_bind(&row.hangup_cause)
                    .push_bind(&row.hangup_party)
                    .push_bind(row.r.sip_status_code.map(|c| c as i64))
                    .push_bind(&row.r.sip_reason)
                    .push_bind(&row.r.route_id)
                    .push_bind(&row.r.gateway_id)
                    .push_bind(row.r.media_anchored)
                    .push_bind(&row.r.codec_a_leg)
                    .push_bind(&row.r.codec_b_leg)
                    .push_bind(row.r.rtp_packets_sent.map(|p| p as i64))
                    .push_bind(row.r.rtp_packets_received.map(|p| p as i64))
                    .push_bind(row.r.rtp_bytes_sent.map(|b| b as i64))
                    .push_bind(row.r.rtp_bytes_received.map(|b| b as i64))
                    .push_bind(&row.r.recording_url)
                    .push_bind(row.r.cost)
                    .push_bind(&row.r.currency)
                    .push_bind(&row.custom_data)
                    .push_bind(row.r.cdr_id)
                    .push_bind(row.r.kind.as_str())
                    .push_bind(row.r.schema_version as i16)
                    .push_bind(row.direction)
                    .push_bind(&row.r.node_id)
                    .push_bind(&row.r.region)
                    .push_bind(row.r.correlation_id.map(|c| c.to_string()))
                    .push_bind(&row.r.trunk_name)
                    .push_bind(row.r.trunk_id.as_ref().map(|t| t.to_string()))
                    .push_bind(row.r.rtp_packets_lost.map(|p| p as i64))
                    .push_bind(row.r.rtp_jitter_ms)
                    .push_bind(&row.r.rate_plan_id)
                    .push_bind(&row.r.matched_prefix)
                    .push_bind(row.r.billable_seconds.map(|s| s as i32))
                    .push_bind(row.rated_at)
                    .push_bind(row.created_at)
                    .push_bind(row.r.recording_consent.map(|c| c.as_str()))
                    .push_bind(row.r.site_id.map(|id| id.to_string()));
            });
            query.push(" ON CONFLICT (cdr_id, start_time) DO NOTHING RETURNING cdr_id");
            let returned = query
                .build()
                .fetch_all(&mut *tx)
                .await
                .map_err(cdr_insert_error)?;
            new_ids.extend(returned.iter().map(|r| r.get::<uuid::Uuid, _>("cdr_id")));
        }
        tx.commit().await.map_err(cdr_insert_error)?;

        debug!(
            records = rows.len(),
            stored = new_ids.len(),
            "CDR batch stored"
        );
        Ok(new_ids)
    }

    async fn get_cdr(&self, call_id: &fcp_core::CallId) -> Result<Option<CallDetailRecord>> {
        // The call's own record first; the other kinds sit beside it.
        let row = sqlx::query(
            "SELECT * FROM cdrs WHERE call_id = $1
             ORDER BY (kind = 'call') DESC, start_time DESC LIMIT 1",
        )
        .bind(call_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_cdr(row)?)),
            None => Ok(None),
        }
    }

    async fn cdrs_after(&self, after: i64, limit: usize) -> Result<Vec<(i64, CallDetailRecord)>> {
        let rows = sqlx::query("SELECT * FROM cdrs WHERE ship_seq > $1 ORDER BY ship_seq LIMIT $2")
            .bind(after)
            .bind(limit.clamp(1, 10_000) as i64)
            .fetch_all(self.reads())
            .await?;
        rows.into_iter()
            .map(|row| {
                let seq: i64 = row.try_get("ship_seq")?;
                Ok((seq, row_to_cdr(row)?))
            })
            .collect()
    }

    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>> {
        let (mut sql, binds) = cdr_filter_sql("SELECT * FROM cdrs", &query)?;
        // Newest first, the record id breaking ties, so a cursor names one
        // position exactly (docs/CDR_PIPELINE.md §9).
        sql.push_str(" ORDER BY start_time DESC, cdr_id DESC");
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }
        if query.cursor.is_none() {
            if let Some(offset) = query.offset {
                sql.push_str(&format!(" OFFSET {}", offset));
            }
        }

        let rows = bind_cdr_filters(sqlx::query(&sql), binds)
            .fetch_all(&self.pool)
            .await?;

        // A row that does not decode is an error, not a record quietly
        // left out of the listing.
        rows.into_iter().map(row_to_cdr).collect()
    }

    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize> {
        // Same filter set as query_cdrs so paginated totals match the rows.
        let (sql, binds) = cdr_filter_sql("SELECT COUNT(*) AS count FROM cdrs", &query)?;
        let row = bind_cdr_filters(sqlx::query(&sql), binds)
            .fetch_one(&self.pool)
            .await?;
        let count: i64 = row.get("count");
        Ok(count as usize)
    }

    async fn delete_cdrs_before(&self, tenant_id: &TenantId, before: SystemTime) -> Result<u64> {
        // In batches, so a tenant's first sweep over years of records is
        // many short transactions rather than one long one.
        const BATCH: i64 = 10_000;
        // A held record is left where it is (docs/COMPLIANCE_FOLLOWUPS.md §5).
        let held = if deletion::tenant_has_holds(self.writes(), &tenant_id.to_string()).await? {
            " AND fcp_cdr_hold(tenant_id, start_time, from_uri, to_uri, call_id) IS NULL"
        } else {
            ""
        };
        let sql = format!(
            "DELETE FROM cdrs WHERE (cdr_id, start_time) IN (
                 SELECT cdr_id, start_time FROM cdrs
                 WHERE tenant_id = $1 AND start_time < $2{held}
                 LIMIT $3)"
        );
        let mut deleted = 0;
        loop {
            let batch = deletion::execute(
                self.writes(),
                sqlx::query(&sql)
                    .bind(tenant_id.to_string())
                    .bind(system_time_to_datetime(before))
                    .bind(BATCH),
            )
            .await?;
            deleted += batch;
            if batch < BATCH as u64 {
                break;
            }
        }
        Ok(deleted)
    }

    async fn aggregate_usage(
        &self,
        tenant_id: &fcp_core::TenantId,
        from: SystemTime,
        to: SystemTime,
    ) -> Result<UsageAggregation> {
        // Closed hours the rollups cover come from the rollups; the edges of
        // the range, and the hours not yet rolled up, from the records
        // (docs/CDR_PIPELINE.md §9).
        let (from_dt, to_dt) = (system_time_to_datetime(from), system_time_to_datetime(to));
        let Some((lo, hi)) = cdr_volume::rollup_span(&self.pool, from_dt, to_dt).await? else {
            return self.live_usage(tenant_id, from_dt, to_dt).await;
        };
        let mut total = self.rolled_up_usage(tenant_id, lo, hi).await?;
        for (a, b) in [(from_dt, lo), (hi, to_dt)] {
            if a < b {
                let part = self.live_usage(tenant_id, a, b).await?;
                total.total_calls += part.total_calls;
                total.answered_calls += part.answered_calls;
                total.total_duration_ms += part.total_duration_ms;
                total.inbound_calls += part.inbound_calls;
                total.outbound_calls += part.outbound_calls;
                total.total_cost += part.total_cost;
            }
        }
        Ok(total)
    }

    async fn aggregate_usage_by_number(
        &self,
        tenant_id: &fcp_core::TenantId,
        from: SystemTime,
        to: SystemTime,
    ) -> Result<Vec<NumberUsage>> {
        let (from_dt, to_dt) = (system_time_to_datetime(from), system_time_to_datetime(to));
        let Some((lo, hi)) = cdr_volume::rollup_span(&self.pool, from_dt, to_dt).await? else {
            return self
                .live_usage_by_number(tenant_id, from_dt, to_dt, Some(100))
                .await;
        };
        let mut by_number: HashMap<String, NumberUsage> = HashMap::new();
        let mut add = |usage: NumberUsage| {
            let entry = by_number
                .entry(usage.number.clone())
                .or_insert_with(|| NumberUsage {
                    number: usage.number.clone(),
                    ..Default::default()
                });
            entry.total_calls += usage.total_calls;
            entry.answered_calls += usage.answered_calls;
            entry.total_duration_ms += usage.total_duration_ms;
            entry.inbound_calls += usage.inbound_calls;
            entry.outbound_calls += usage.outbound_calls;
        };
        for usage in self.rolled_up_usage_by_number(tenant_id, lo, hi).await? {
            add(usage);
        }
        for (a, b) in [(from_dt, lo), (hi, to_dt)] {
            if a < b {
                for usage in self.live_usage_by_number(tenant_id, a, b, None).await? {
                    add(usage);
                }
            }
        }
        let mut out: Vec<NumberUsage> = by_number.into_values().collect();
        out.sort_by(|a, b| {
            b.total_calls
                .cmp(&a.total_calls)
                .then(a.number.cmp(&b.number))
        });
        out.truncate(100);
        Ok(out)
    }

    async fn count_cdrs_within(&self, query: CdrQuery, budget: Duration) -> Result<CdrCount> {
        let (sql, binds) = cdr_filter_sql("SELECT COUNT(*) AS count FROM cdrs", &query)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL statement_timeout = {}",
            budget.as_millis().max(1)
        ))
        .execute(&mut *tx)
        .await?;
        match bind_cdr_filters(sqlx::query(&sql), binds)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(row) => {
                tx.commit().await?;
                Ok(CdrCount {
                    count: row.get::<i64, _>("count") as u64,
                    exact: true,
                })
            }
            // Too slow to count: the planner's estimate of the same query.
            Err(e)
                if e.as_database_error()
                    .and_then(|d| d.code())
                    .is_some_and(|c| c == "57014") =>
            {
                drop(tx);
                let (explain, binds) =
                    cdr_filter_sql("EXPLAIN (FORMAT JSON) SELECT 1 FROM cdrs", &query)?;
                let row = bind_cdr_filters(sqlx::query(&explain), binds)
                    .fetch_one(&self.pool)
                    .await?;
                let plan: serde_json::Value = row.try_get(0)?;
                let estimate = plan[0]["Plan"]["Plan Rows"].as_f64().unwrap_or(0.0);
                Ok(CdrCount {
                    count: estimate.max(0.0) as u64,
                    exact: false,
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn ensure_cdr_partitions(&self) -> Result<Vec<String>> {
        cdr_volume::ensure_partitions(self.writes()).await
    }

    async fn drop_cdr_partitions_before(&self, before: SystemTime) -> Result<Vec<String>> {
        cdr_volume::drop_partitions_before(self.writes(), system_time_to_datetime(before)).await
    }

    async fn rebuild_usage_rollups(&self, window: Duration) -> Result<RollupRun> {
        cdr_volume::rebuild_rollups(
            self.writes(),
            window,
            &phone_number_uri_match_sql("c.to_uri"),
            &phone_number_uri_match_sql("c.from_uri"),
        )
        .await
    }

    async fn try_maintenance_lock(
        &self,
        name: &str,
    ) -> Result<Option<Box<dyn fcp_core::storage::MaintenanceLock>>> {
        cdr_volume::try_lock(self.writes(), name).await
    }

    // The archive reads and writes the primary: a record the trigger has
    // already noted must be visible to the walk that answers the note.

    async fn cdr_archive_next_day(&self) -> Result<Option<chrono::NaiveDate>> {
        cdr_archive::next_day(self.writes()).await
    }

    async fn advance_cdr_archive(&self, day: chrono::NaiveDate) -> Result<()> {
        cdr_archive::advance(self.writes(), day).await
    }

    async fn cdr_tenants_on(&self, day: chrono::NaiveDate) -> Result<Vec<fcp_core::TenantId>> {
        cdr_archive::tenants_on(&self.pool, day).await
    }

    async fn pending_cdr_archives(
        &self,
        before: chrono::NaiveDate,
        limit: usize,
    ) -> Result<Vec<(fcp_core::TenantId, chrono::NaiveDate)>> {
        cdr_archive::pending(&self.pool, before, limit).await
    }

    async fn begin_cdr_archive(
        &self,
        tenant_id: &fcp_core::TenantId,
        day: chrono::NaiveDate,
    ) -> Result<u64> {
        cdr_archive::begin(self.writes(), tenant_id, day).await
    }

    async fn cdr_archive_page(
        &self,
        tenant_id: &fcp_core::TenantId,
        day: chrono::NaiveDate,
        after: Option<fcp_core::storage::CdrCursor>,
        limit: usize,
    ) -> Result<Vec<CallDetailRecord>> {
        cdr_archive::page(&self.pool, tenant_id, day, after, limit).await
    }

    async fn complete_cdr_archive(
        &self,
        write: &fcp_core::cdr_archive::CdrArchiveWrite,
    ) -> Result<bool> {
        cdr_archive::complete(self.writes(), write).await
    }

    async fn get_cdr_archive(
        &self,
        tenant_id: &fcp_core::TenantId,
        day: chrono::NaiveDate,
    ) -> Result<Option<fcp_core::cdr_archive::CdrArchiveEntry>> {
        cdr_archive::get(&self.pool, tenant_id, day).await
    }

    async fn cdr_archive_frontier(
        &self,
        tenant_id: &fcp_core::TenantId,
    ) -> Result<Option<SystemTime>> {
        cdr_archive::frontier(&self.pool, tenant_id).await
    }

    async fn cdr_archive_status(&self) -> Result<fcp_core::cdr_archive::CdrArchiveStatus> {
        cdr_archive::status(&self.pool).await
    }

    async fn cdr_archive_lock_changes(
        &self,
        limit: usize,
    ) -> Result<Vec<(TenantId, chrono::NaiveDate, String, bool)>> {
        cdr_archive::lock_changes(&self.pool, limit).await
    }

    async fn set_cdr_archive_locked(
        &self,
        tenant_id: &TenantId,
        day: chrono::NaiveDate,
        locked: bool,
    ) -> Result<()> {
        cdr_archive::set_locked(self.writes(), tenant_id, day, locked).await
    }
}

/// Usage from the records and from the hourly rollups (docs/CDR_PIPELINE.md §9).
impl PostgresStorage {
    /// Usage in `[from, to)` counted from the call records themselves.
    async fn live_usage(
        &self,
        tenant_id: &fcp_core::TenantId,
        from_dt: DateTime<Utc>,
        to_dt: DateTime<Utc>,
    ) -> Result<UsageAggregation> {
        let tenant_id_str = tenant_id.to_string();
        // A record the pipeline has enriched carries its direction; older
        // ones are classified against the tenant's numbers here, row by row.
        let inbound_match = phone_number_uri_match_sql("c.to_uri");
        let outbound_match = phone_number_uri_match_sql("c.from_uri");
        let sql = format!(
            "SELECT
                COUNT(*) as total_calls,
                COUNT(*) FILTER (WHERE c.answer_time IS NOT NULL) as answered_calls,
                COALESCE(SUM(c.call_duration_ms) FILTER (WHERE c.answer_time IS NOT NULL), 0)::BIGINT as total_duration_ms,
                COUNT(*) FILTER (
                    WHERE CASE WHEN c.direction IS NOT NULL THEN c.direction = 'inbound'
                    ELSE EXISTS (
                        SELECT 1
                        FROM phone_numbers pn
                        WHERE pn.tenant_id = $1
                          AND ({inbound_match})
                    ) END
                ) as inbound_calls,
                COUNT(*) FILTER (
                    WHERE CASE WHEN c.direction IS NOT NULL THEN c.direction = 'outbound'
                    ELSE EXISTS (
                        SELECT 1
                        FROM phone_numbers pn
                        WHERE pn.tenant_id = $1
                          AND ({outbound_match})
                    ) END
                ) as outbound_calls,
                COALESCE(SUM(c.cost), 0) as total_cost
            FROM cdrs c
            WHERE c.tenant_id = $1 AND c.kind = 'call' AND c.start_time >= $2 AND c.start_time < $3"
        );

        let row = sqlx::query(&sql)
            .bind(&tenant_id_str)
            .bind(from_dt)
            .bind(to_dt)
            .fetch_one(&self.pool)
            .await?;

        Ok(UsageAggregation {
            total_calls: row.get::<i64, _>("total_calls") as u64,
            answered_calls: row.get::<i64, _>("answered_calls") as u64,
            total_duration_ms: row.get::<i64, _>("total_duration_ms") as u64,
            inbound_calls: row.get::<i64, _>("inbound_calls") as u64,
            outbound_calls: row.get::<i64, _>("outbound_calls") as u64,
            total_cost: row.get::<fcp_core::billing::Decimal, _>("total_cost"),
        })
    }

    /// Usage per number in `[from, to)` counted from the call records, the
    /// busiest `limit` numbers.
    async fn live_usage_by_number(
        &self,
        tenant_id: &fcp_core::TenantId,
        from_dt: DateTime<Utc>,
        to_dt: DateTime<Utc>,
        limit: Option<usize>,
    ) -> Result<Vec<NumberUsage>> {
        let tenant_id_str = tenant_id.to_string();
        let inbound_match = phone_number_uri_match_sql("c.to_uri");
        let outbound_match = phone_number_uri_match_sql("c.from_uri");
        let limit_sql = limit.map(|n| format!("LIMIT {n}")).unwrap_or_default();
        let sql = format!(
            "WITH matched_numbers AS (
                SELECT DISTINCT
                    c.call_id,
                    pn.e164 AS number,
                    'outbound' AS direction,
                    c.call_duration_ms,
                    c.answer_time
                FROM cdrs c
                JOIN phone_numbers pn
                  ON pn.tenant_id = $1
                 AND ({outbound_match})
                WHERE c.tenant_id = $1 AND c.kind = 'call' AND c.start_time >= $2 AND c.start_time < $3

                UNION ALL

                SELECT DISTINCT
                    c.call_id,
                    pn.e164 AS number,
                    'inbound' AS direction,
                    c.call_duration_ms,
                    c.answer_time
                FROM cdrs c
                JOIN phone_numbers pn
                  ON pn.tenant_id = $1
                 AND ({inbound_match})
                WHERE c.tenant_id = $1 AND c.kind = 'call' AND c.start_time >= $2 AND c.start_time < $3
            ),
            deduped_calls AS (
                SELECT
                    number,
                    call_id,
                    BOOL_OR(answer_time IS NOT NULL) AS answered,
                    MAX(COALESCE(call_duration_ms, 0)) AS call_duration_ms,
                    BOOL_OR(direction = 'inbound') AS has_inbound,
                    BOOL_OR(direction = 'outbound') AS has_outbound
                FROM matched_numbers
                GROUP BY number, call_id
            )
            SELECT
                number,
                COUNT(*) AS total_calls,
                COUNT(*) FILTER (WHERE answered) AS answered_calls,
                COALESCE(
                    SUM(CASE WHEN answered THEN call_duration_ms ELSE 0 END),
                    0
                )::BIGINT AS total_duration_ms,
                COUNT(*) FILTER (WHERE has_inbound) AS inbound_calls,
                COUNT(*) FILTER (WHERE has_outbound) AS outbound_calls
            FROM deduped_calls
            GROUP BY number
            ORDER BY total_calls DESC, number
            {limit_sql}"
        );

        let rows = sqlx::query(&sql)
            .bind(&tenant_id_str)
            .bind(from_dt)
            .bind(to_dt)
            .fetch_all(&self.pool)
            .await?;

        let results = rows
            .into_iter()
            .map(|row| NumberUsage {
                number: row.get("number"),
                total_calls: row.get::<i64, _>("total_calls") as u64,
                answered_calls: row.get::<i64, _>("answered_calls") as u64,
                total_duration_ms: row.get::<i64, _>("total_duration_ms") as u64,
                inbound_calls: row.get::<i64, _>("inbound_calls") as u64,
                outbound_calls: row.get::<i64, _>("outbound_calls") as u64,
            })
            .collect();

        Ok(results)
    }

    /// Usage in the rolled-up hours `[lo, hi)`: call records only.
    async fn rolled_up_usage(
        &self,
        tenant_id: &fcp_core::TenantId,
        lo: DateTime<Utc>,
        hi: DateTime<Utc>,
    ) -> Result<UsageAggregation> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(calls), 0)::BIGINT AS total_calls,
                    COALESCE(SUM(answered), 0)::BIGINT AS answered_calls,
                    COALESCE(SUM(duration_ms), 0)::BIGINT AS total_duration_ms,
                    COALESCE(SUM(calls) FILTER (WHERE direction = 'inbound'), 0)::BIGINT AS inbound_calls,
                    COALESCE(SUM(calls) FILTER (WHERE direction = 'outbound'), 0)::BIGINT AS outbound_calls,
                    COALESCE(SUM(cost), 0) AS total_cost
             FROM cdr_usage_hourly
             WHERE tenant_id = $1 AND kind = 'call' AND hour >= $2 AND hour < $3",
        )
        .bind(tenant_id.to_string())
        .bind(lo)
        .bind(hi)
        .fetch_one(&self.pool)
        .await?;
        Ok(UsageAggregation {
            total_calls: row.get::<i64, _>("total_calls") as u64,
            answered_calls: row.get::<i64, _>("answered_calls") as u64,
            total_duration_ms: row.get::<i64, _>("total_duration_ms") as u64,
            inbound_calls: row.get::<i64, _>("inbound_calls") as u64,
            outbound_calls: row.get::<i64, _>("outbound_calls") as u64,
            total_cost: row.get::<fcp_core::billing::Decimal, _>("total_cost"),
        })
    }

    /// Usage per number in the rolled-up hours `[lo, hi)`.
    async fn rolled_up_usage_by_number(
        &self,
        tenant_id: &fcp_core::TenantId,
        lo: DateTime<Utc>,
        hi: DateTime<Utc>,
    ) -> Result<Vec<NumberUsage>> {
        let rows = sqlx::query(
            "SELECT number,
                    SUM(calls)::BIGINT AS total_calls,
                    SUM(answered)::BIGINT AS answered_calls,
                    SUM(duration_ms)::BIGINT AS total_duration_ms,
                    SUM(inbound)::BIGINT AS inbound_calls,
                    SUM(outbound)::BIGINT AS outbound_calls
             FROM cdr_number_usage_hourly
             WHERE tenant_id = $1 AND hour >= $2 AND hour < $3
             GROUP BY number",
        )
        .bind(tenant_id.to_string())
        .bind(lo)
        .bind(hi)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| NumberUsage {
                number: row.get("number"),
                total_calls: row.get::<i64, _>("total_calls") as u64,
                answered_calls: row.get::<i64, _>("answered_calls") as u64,
                total_duration_ms: row.get::<i64, _>("total_duration_ms") as u64,
                inbound_calls: row.get::<i64, _>("inbound_calls") as u64,
                outbound_calls: row.get::<i64, _>("outbound_calls") as u64,
            })
            .collect())
    }
}

#[async_trait]
impl fcp_core::billing::BillingStore for PostgresStorage {
    async fn create_rate_plan(&self, plan: fcp_core::billing::RatePlan) -> Result<()> {
        let mut tx = self.writes().begin().await?;
        if plan.is_default {
            sqlx::query(
                "UPDATE rate_plans SET is_default = FALSE WHERE tenant_id = $1 AND is_default",
            )
            .bind(plan.tenant_id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO rate_plans (
                id, tenant_id, name, currency, default_rate_per_minute,
                connect_fee, billing_increment_secs, is_default, enabled,
                created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(plan.id.to_string())
        .bind(plan.tenant_id.to_string())
        .bind(&plan.name)
        .bind(&plan.currency)
        .bind(plan.default_rate_per_minute)
        .bind(plan.connect_fee)
        .bind(plan.billing_increment_secs as i32)
        .bind(plan.is_default)
        .bind(plan.enabled)
        .bind(system_time_to_datetime(plan.created_at))
        .bind(system_time_to_datetime(plan.updated_at))
        .execute(&mut *tx)
        .await
        .map_err(map_rate_plan_write_error)?;
        tx.commit().await?;
        Ok(())
    }

    async fn get_rate_plan(
        &self,
        tenant_id: &fcp_core::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<Option<fcp_core::billing::RatePlan>> {
        let row = sqlx::query("SELECT * FROM rate_plans WHERE id = $1 AND tenant_id = $2")
            .bind(plan_id.to_string())
            .bind(tenant_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_rate_plan).transpose()
    }

    async fn list_rate_plans(
        &self,
        tenant_id: &fcp_core::TenantId,
    ) -> Result<Vec<fcp_core::billing::RatePlan>> {
        let rows = sqlx::query("SELECT * FROM rate_plans WHERE tenant_id = $1 ORDER BY name")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(row_to_rate_plan).collect()
    }

    async fn update_rate_plan(&self, plan: fcp_core::billing::RatePlan) -> Result<()> {
        let mut tx = self.writes().begin().await?;
        if plan.is_default {
            sqlx::query(
                "UPDATE rate_plans SET is_default = FALSE
                 WHERE tenant_id = $1 AND is_default AND id <> $2",
            )
            .bind(plan.tenant_id.to_string())
            .bind(plan.id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        let result = sqlx::query(
            "UPDATE rate_plans SET
                name = $3, currency = $4, default_rate_per_minute = $5,
                connect_fee = $6, billing_increment_secs = $7,
                is_default = $8, enabled = $9, updated_at = NOW()
             WHERE id = $1 AND tenant_id = $2",
        )
        .bind(plan.id.to_string())
        .bind(plan.tenant_id.to_string())
        .bind(&plan.name)
        .bind(&plan.currency)
        .bind(plan.default_rate_per_minute)
        .bind(plan.connect_fee)
        .bind(plan.billing_increment_secs as i32)
        .bind(plan.is_default)
        .bind(plan.enabled)
        .execute(&mut *tx)
        .await
        .map_err(map_rate_plan_write_error)?;
        if result.rows_affected() == 0 {
            return Err(anyhow::Error::new(fcp_core::Error::Storage(
                fcp_core::StorageError::NotFound(format!("rate plan {}", plan.id)),
            )));
        }
        tx.commit().await?;
        Ok(())
    }

    async fn delete_rate_plan(
        &self,
        tenant_id: &fcp_core::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<()> {
        let result = sqlx::query("DELETE FROM rate_plans WHERE id = $1 AND tenant_id = $2")
            .bind(plan_id.to_string())
            .bind(tenant_id.to_string())
            .execute(self.writes())
            .await?;
        if result.rows_affected() == 0 {
            return Err(anyhow::Error::new(fcp_core::Error::Storage(
                fcp_core::StorageError::NotFound(format!("rate plan {plan_id}")),
            )));
        }
        Ok(())
    }

    async fn upsert_rate(
        &self,
        tenant_id: &fcp_core::TenantId,
        rate: fcp_core::billing::Rate,
    ) -> Result<()> {
        // The tenant check rides in the INSERT..SELECT: zero rows affected
        // means the plan does not exist in this tenant.
        let result = sqlx::query(
            "INSERT INTO rates (id, rate_plan_id, prefix, rate_per_minute, connect_fee, description)
             SELECT $1, p.id, $3, $4, $5, $6
             FROM rate_plans p WHERE p.id = $2 AND p.tenant_id = $7
             ON CONFLICT (rate_plan_id, prefix) DO UPDATE SET
                rate_per_minute = EXCLUDED.rate_per_minute,
                connect_fee = EXCLUDED.connect_fee,
                description = EXCLUDED.description",
        )
        .bind(rate.id.to_string())
        .bind(rate.rate_plan_id.to_string())
        .bind(&rate.prefix)
        .bind(rate.rate_per_minute)
        .bind(rate.connect_fee)
        .bind(&rate.description)
        .bind(tenant_id.to_string())
        .execute(self.writes())
        .await?;
        if result.rows_affected() == 0 {
            return Err(anyhow::Error::new(fcp_core::Error::Storage(
                fcp_core::StorageError::NotFound(format!("rate plan {}", rate.rate_plan_id)),
            )));
        }
        Ok(())
    }

    async fn list_rates(
        &self,
        tenant_id: &fcp_core::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<Vec<fcp_core::billing::Rate>> {
        let rows = sqlx::query(
            "SELECT r.* FROM rates r
             JOIN rate_plans p ON p.id = r.rate_plan_id
             WHERE r.rate_plan_id = $1 AND p.tenant_id = $2
             ORDER BY LENGTH(r.prefix) DESC, r.prefix",
        )
        .bind(plan_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_rate).collect()
    }

    async fn delete_rate(
        &self,
        tenant_id: &fcp_core::TenantId,
        plan_id: uuid::Uuid,
        rate_id: uuid::Uuid,
    ) -> Result<()> {
        let result = sqlx::query(
            "DELETE FROM rates r
             USING rate_plans p
             WHERE r.id = $1 AND r.rate_plan_id = $2
               AND p.id = r.rate_plan_id AND p.tenant_id = $3",
        )
        .bind(rate_id.to_string())
        .bind(plan_id.to_string())
        .bind(tenant_id.to_string())
        .execute(self.writes())
        .await?;
        if result.rows_affected() == 0 {
            return Err(anyhow::Error::new(fcp_core::Error::Storage(
                fcp_core::StorageError::NotFound(format!("rate {rate_id}")),
            )));
        }
        Ok(())
    }

    async fn get_default_rate_plan_with_rates(
        &self,
        tenant_id: &fcp_core::TenantId,
    ) -> Result<Option<(fcp_core::billing::RatePlan, Vec<fcp_core::billing::Rate>)>> {
        let row =
            sqlx::query("SELECT * FROM rate_plans WHERE tenant_id = $1 AND is_default AND enabled")
                .bind(tenant_id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        let Some(row) = row else { return Ok(None) };
        let plan = row_to_rate_plan(row)?;
        let rates = sqlx::query("SELECT * FROM rates WHERE rate_plan_id = $1")
            .bind(plan.id.to_string())
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(row_to_rate)
            .collect::<Result<Vec<_>>>()?;
        Ok(Some((plan, rates)))
    }

    async fn get_balance(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Option<fcp_core::billing::TenantBalance>> {
        let row = sqlx::query("SELECT * FROM tenant_balances WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_tenant_balance).transpose()
    }

    async fn upsert_balance(&self, balance: fcp_core::billing::TenantBalance) -> Result<()> {
        // The amount is only taken on insert; settings are replaced on conflict.
        sqlx::query(
            "INSERT INTO tenant_balances (
                tenant_id, balance, currency, credit_limit, enforce,
                low_balance_threshold, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, NOW())
             ON CONFLICT (tenant_id) DO UPDATE SET
                currency = EXCLUDED.currency,
                credit_limit = EXCLUDED.credit_limit,
                enforce = EXCLUDED.enforce,
                low_balance_threshold = EXCLUDED.low_balance_threshold,
                updated_at = NOW()",
        )
        .bind(balance.tenant_id.to_string())
        .bind(balance.balance)
        .bind(&balance.currency)
        .bind(balance.credit_limit)
        .bind(balance.enforce)
        .bind(balance.low_balance_threshold)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn apply_balance_transaction(
        &self,
        request: fcp_core::billing::BalanceTransactionRequest,
    ) -> Result<fcp_core::billing::BalanceTransaction> {
        let mut tx = self.writes().begin().await?;

        // Upsert-and-add in one statement so concurrent settlements serialize
        // on the row lock rather than racing on a read-modify-write.
        let row = sqlx::query(
            "INSERT INTO tenant_balances (tenant_id, balance, currency, updated_at)
             VALUES ($1, $2, $3, NOW())
             ON CONFLICT (tenant_id) DO UPDATE SET
                balance = tenant_balances.balance + EXCLUDED.balance,
                updated_at = NOW()
             RETURNING balance, currency",
        )
        .bind(request.tenant_id.to_string())
        .bind(request.amount)
        .bind(&request.currency)
        .fetch_one(&mut *tx)
        .await?;
        let balance_after: fcp_core::billing::Decimal = row.get("balance");
        let currency: String = row.get("currency");

        let id = uuid::Uuid::new_v4();
        let created_at = SystemTime::now();
        let insert = sqlx::query(
            "INSERT INTO balance_transactions (
                id, tenant_id, kind, amount, balance_after, currency,
                call_id, description, external_ref, created_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id.to_string())
        .bind(request.tenant_id.to_string())
        .bind(request.kind.as_str())
        .bind(request.amount)
        .bind(balance_after)
        .bind(&currency)
        .bind(request.call_id.as_ref().map(|c| c.to_string()))
        .bind(&request.description)
        .bind(&request.external_ref)
        .bind(system_time_to_datetime(created_at))
        .execute(&mut *tx)
        .await;

        if let Err(err) = insert {
            // A second settlement for the same call rolls back the debit and
            // reports Conflict so the caller can treat it as already applied.
            let conflict = err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23505"))
                .unwrap_or(false);
            tx.rollback().await?;
            if conflict {
                return Err(anyhow::Error::new(fcp_core::Error::Storage(
                    fcp_core::StorageError::Conflict(format!(
                        "call {} has already been settled",
                        request.call_id.map(|c| c.to_string()).unwrap_or_default()
                    )),
                )));
            }
            return Err(err.into());
        }
        tx.commit().await?;

        Ok(fcp_core::billing::BalanceTransaction {
            id,
            tenant_id: request.tenant_id,
            kind: request.kind,
            amount: request.amount,
            balance_after,
            currency,
            call_id: request.call_id,
            description: request.description,
            external_ref: request.external_ref,
            created_at,
        })
    }

    async fn list_balance_transactions(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<fcp_core::billing::BalanceTransaction>> {
        let rows = sqlx::query(
            "SELECT * FROM balance_transactions WHERE tenant_id = $1
             ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(tenant_id.to_string())
        .bind(limit.min(1000) as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_balance_transaction).collect()
    }
}

#[async_trait]
impl EncryptionStore for PostgresStorage {
    async fn encryption_status(&self) -> Result<EncryptionStatus> {
        Ok(match &self.encryption {
            Some(enc) => enc.status(),
            None => EncryptionStatus::disabled(),
        })
    }

    async fn rotate_encryption_key(&self) -> Result<EncryptionStatus> {
        let enc = self
            .encryption
            .as_ref()
            .context("encryption at rest is not enabled (set FCP_ENCRYPTION_PROVIDER)")?;
        enc.rotate(self.writes()).await
    }

    async fn reencrypt_secrets(&self) -> Result<ReencryptReport> {
        let enc = self
            .encryption
            .as_ref()
            .context("encryption at rest is not enabled (set FCP_ENCRYPTION_PROVIDER)")?;
        enc.reencrypt(self.writes()).await
    }

    async fn seal_secret(&self, plaintext: &str) -> Result<String> {
        let enc = self
            .encryption
            .as_ref()
            .context("encryption at rest is not enabled (set FCP_ENCRYPTION_PROVIDER)")?;
        enc.seal(plaintext)
    }

    async fn open_secret(&self, value: &str) -> Result<String> {
        Ok(self
            .open_opt(Some(value.to_string()))
            .await?
            .unwrap_or_default())
    }
}

// ============================================================================
// CDR query helpers
// ============================================================================

/// A bound CDR filter value.
enum CdrBind {
    Text(String),
    Time(DateTime<Utc>),
    Uuid(uuid::Uuid),
}

/// Build `<select> WHERE …` from a [`CdrQuery`], with numbered placeholders
/// and typed bind values in placeholder order. Shared by `query_cdrs` and
/// `count_cdrs` so the two can never disagree about which rows match.
/// Rows per `INSERT` statement: 46 binds each, under Postgres's 65535.
const CDR_INSERT_CHUNK: usize = 1000;

/// A record's column values that need converting before they are bound.
struct CdrRow<'a> {
    r: &'a CallDetailRecord,
    call_id: String,
    tenant_id: String,
    disposition: String,
    hangup_cause: String,
    hangup_party: Option<String>,
    custom_data: Option<serde_json::Value>,
    start_time: DateTime<Utc>,
    answer_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
    rated_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    direction: Option<&'static str>,
}

impl<'a> CdrRow<'a> {
    fn from_record(r: &'a CallDetailRecord) -> Result<Self> {
        let unquote = |s: String| s.trim_matches('"').to_string();
        Ok(Self {
            r,
            call_id: r.call_id.to_string(),
            tenant_id: r.tenant_id.to_string(),
            disposition: unquote(serde_json::to_string(&r.disposition)?),
            hangup_cause: match r.hangup_cause {
                HangupCause::Other(code) => format!("other:{}", code),
                _ => unquote(serde_json::to_string(&r.hangup_cause)?),
            },
            hangup_party: r
                .hangup_party
                .map(|p| serde_json::to_string(&p).map(unquote))
                .transpose()?,
            custom_data: r.custom_data.clone(),
            start_time: system_time_to_datetime(r.start_time),
            answer_time: r.answer_time.map(system_time_to_datetime),
            end_time: r.end_time.map(system_time_to_datetime),
            rated_at: r.rated_at.map(system_time_to_datetime),
            created_at: r
                .created_at
                .map(system_time_to_datetime)
                .unwrap_or_else(Utc::now),
            direction: r.direction.map(call_direction_name),
        })
    }
}

/// A batch insert's failure: a row Postgres refuses for what it holds
/// (an integrity or data exception, SQLSTATE classes 23 and 22) is a
/// [`CdrRejected`](fcp_core::storage::CdrRejected), anything else the
/// database being unavailable.
fn cdr_insert_error(err: sqlx::Error) -> anyhow::Error {
    let refused = err
        .as_database_error()
        .and_then(|e| e.code())
        .is_some_and(|code| code.starts_with("23") || code.starts_with("22"));
    if refused {
        anyhow::Error::new(fcp_core::storage::CdrRejected(err.to_string()))
    } else {
        err.into()
    }
}

fn cdr_filter_sql(select: &str, query: &CdrQuery) -> Result<(String, Vec<CdrBind>)> {
    let mut sql = format!("{select} WHERE 1=1");
    let mut binds: Vec<CdrBind> = Vec::new();

    let push = |sql: &mut String, clause: &str, bind: CdrBind, binds: &mut Vec<CdrBind>| {
        binds.push(bind);
        sql.push_str(&format!(" AND {clause} ${}", binds.len()));
    };

    if let Some(tenant_id) = &query.tenant_id {
        push(
            &mut sql,
            "tenant_id =",
            CdrBind::Text(tenant_id.to_string()),
            &mut binds,
        );
    }
    if let Some(call_id) = &query.call_id {
        push(
            &mut sql,
            "call_id =",
            CdrBind::Text(call_id.to_string()),
            &mut binds,
        );
    }
    if let Some(from_uri) = &query.from_uri {
        push(
            &mut sql,
            "from_uri LIKE",
            CdrBind::Text(format!("%{from_uri}%")),
            &mut binds,
        );
    }
    if let Some(to_uri) = &query.to_uri {
        push(
            &mut sql,
            "to_uri LIKE",
            CdrBind::Text(format!("%{to_uri}%")),
            &mut binds,
        );
    }
    if let Some(from) = query.start_time_from {
        push(
            &mut sql,
            "start_time >=",
            CdrBind::Time(system_time_to_datetime(from)),
            &mut binds,
        );
    }
    if let Some(to) = query.start_time_to {
        push(
            &mut sql,
            "start_time <=",
            CdrBind::Time(system_time_to_datetime(to)),
            &mut binds,
        );
    }
    if let Some(disposition) = &query.disposition {
        let disp = serde_json::to_string(disposition)?
            .trim_matches('"')
            .to_string();
        push(&mut sql, "disposition =", CdrBind::Text(disp), &mut binds);
    }
    if let Some(kind) = &query.kind {
        push(
            &mut sql,
            "kind =",
            CdrBind::Text(kind.as_str().to_string()),
            &mut binds,
        );
    }
    if let Some(cursor) = &query.cursor {
        let at = DateTime::from_timestamp_micros(cursor.start_micros).unwrap_or_default();
        binds.push(CdrBind::Time(at));
        binds.push(CdrBind::Uuid(cursor.cdr_id));
        sql.push_str(&format!(
            " AND (start_time, cdr_id) < (${}, ${})",
            binds.len() - 1,
            binds.len()
        ));
    }

    Ok((sql, binds))
}

/// The name a call direction is stored under.
fn call_direction_name(direction: fcp_core::CallDirection) -> &'static str {
    match direction {
        fcp_core::CallDirection::Inbound => "inbound",
        fcp_core::CallDirection::Outbound => "outbound",
        fcp_core::CallDirection::Internal => "internal",
    }
}

fn call_direction_from_name(name: &str) -> Option<fcp_core::CallDirection> {
    match name {
        "inbound" => Some(fcp_core::CallDirection::Inbound),
        "outbound" => Some(fcp_core::CallDirection::Outbound),
        "internal" => Some(fcp_core::CallDirection::Internal),
        _ => None,
    }
}

fn bind_cdr_filters<'q>(
    mut query: sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>,
    binds: Vec<CdrBind>,
) -> sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments> {
    for bind in binds {
        query = match bind {
            CdrBind::Text(v) => query.bind(v),
            CdrBind::Time(v) => query.bind(v),
            CdrBind::Uuid(v) => query.bind(v),
        };
    }
    query
}

// ============================================================================
// Helper Functions for Row Conversion
// ============================================================================

/// Map unique-constraint violations on rate-plan writes to Conflict (409).
fn map_rate_plan_write_error(err: sqlx::Error) -> anyhow::Error {
    if let Some(db_err) = err.as_database_error() {
        if db_err.code().as_deref() == Some("23505") {
            return anyhow::Error::new(fcp_core::Error::Storage(fcp_core::StorageError::Conflict(
                "a rate plan with this name (or default flag) already exists for the tenant"
                    .to_string(),
            )));
        }
    }
    err.into()
}

/// Convert PostgreSQL row to RatePlan
fn row_to_rate_plan(row: sqlx::postgres::PgRow) -> Result<fcp_core::billing::RatePlan> {
    Ok(fcp_core::billing::RatePlan {
        id: parse_uuid_column(&row.get::<String, _>("id"))?,
        tenant_id: TenantId::from_str(&row.get::<String, _>("tenant_id"))?,
        name: row.get("name"),
        currency: row.get("currency"),
        default_rate_per_minute: row.get("default_rate_per_minute"),
        connect_fee: row.get("connect_fee"),
        billing_increment_secs: row.get::<i32, _>("billing_increment_secs") as u32,
        is_default: row.get("is_default"),
        enabled: row.get("enabled"),
        created_at: datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at")),
        updated_at: datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at")),
    })
}

/// Convert PostgreSQL row to Rate
fn row_to_rate(row: sqlx::postgres::PgRow) -> Result<fcp_core::billing::Rate> {
    Ok(fcp_core::billing::Rate {
        id: parse_uuid_column(&row.get::<String, _>("id"))?,
        rate_plan_id: parse_uuid_column(&row.get::<String, _>("rate_plan_id"))?,
        prefix: row.get("prefix"),
        rate_per_minute: row.get("rate_per_minute"),
        connect_fee: row.get("connect_fee"),
        description: row.get("description"),
    })
}

/// Convert PostgreSQL row to TenantBalance
fn row_to_tenant_balance(row: sqlx::postgres::PgRow) -> Result<fcp_core::billing::TenantBalance> {
    Ok(fcp_core::billing::TenantBalance {
        tenant_id: TenantId::from_str(&row.get::<String, _>("tenant_id"))?,
        balance: row.get("balance"),
        currency: row.get("currency"),
        credit_limit: row.get("credit_limit"),
        enforce: row.get("enforce"),
        low_balance_threshold: row.get("low_balance_threshold"),
        updated_at: datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at")),
    })
}

/// Convert PostgreSQL row to BalanceTransaction
fn row_to_balance_transaction(
    row: sqlx::postgres::PgRow,
) -> Result<fcp_core::billing::BalanceTransaction> {
    let kind: String = row.get("kind");
    let call_id: Option<String> = row.get("call_id");
    Ok(fcp_core::billing::BalanceTransaction {
        id: parse_uuid_column(&row.get::<String, _>("id"))?,
        tenant_id: TenantId::from_str(&row.get::<String, _>("tenant_id"))?,
        kind: kind.parse().map_err(|e: String| anyhow::anyhow!("{e}"))?,
        amount: row.get("amount"),
        balance_after: row.get("balance_after"),
        currency: row.get("currency"),
        call_id: call_id
            .map(|c| fcp_core::CallId::from_str(&c))
            .transpose()?,
        description: row.get("description"),
        external_ref: row.get("external_ref"),
        created_at: datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at")),
    })
}

fn parse_uuid_column(value: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::from_str(value).map_err(|e| {
        anyhow::Error::new(fcp_core::Error::Storage(
            fcp_core::StorageError::Serialization(format!("invalid uuid '{value}': {e}")),
        ))
    })
}

/// Convert PostgreSQL row to User
fn row_to_user(row: sqlx::postgres::PgRow) -> Result<User> {
    let id = UserId::from_str(&row.get::<String, _>("id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;

    let role_str: String = row.get("role");
    let role = serde_json::from_str::<fcp_core::UserRole>(&format!("\"{}\"", role_str))
        .unwrap_or(fcp_core::UserRole::EndUser);

    let account_status_str: String = row.get("account_status");
    let account_status =
        AccountStatus::from_str(&account_status_str).unwrap_or(AccountStatus::Active);

    let media_preferences: MediaPreferences =
        serde_json::from_value(row.get::<serde_json::Value, _>("media_preferences"))
            .map_err(|e| anyhow::anyhow!("Failed to parse media_preferences: {}", e))?;

    let emergency_contact: EmergencyContact =
        serde_json::from_value(row.get::<serde_json::Value, _>("emergency_contact_json"))
            .map_err(|e| anyhow::anyhow!("Failed to parse emergency_contact: {}", e))?;

    let created_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at"));
    let updated_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at"));
    let password_last_reset = row
        .get::<Option<DateTime<Utc>>, _>("password_last_reset")
        .map(datetime_to_system_time);
    let last_login = row
        .get::<Option<DateTime<Utc>>, _>("last_login")
        .map(datetime_to_system_time);

    let primary_phone_number_id = row
        .get::<Option<String>, _>("primary_phone_number_id")
        .and_then(|s| PhoneNumberId::from_str(&s).ok());

    let voicemail_box_id = row
        .get::<Option<String>, _>("voicemail_box_id")
        .and_then(|s| fcp_core::VoicemailBoxId::from_str(&s).ok());

    let failed_login_attempts: i32 = row.get("failed_login_attempts");
    let account_locked_until = row
        .get::<Option<DateTime<Utc>>, _>("account_locked_until")
        .map(datetime_to_system_time);

    Ok(User {
        id,
        username: row.get("username"),
        password: row.get("password_hash"),
        digest_password: row.get("digest_password"),
        sip_ha1_hash: row.get("sip_ha1_hash"),
        sip_provisioning_password: row.get("sip_provisioning_password"),
        display_name: row.get("display_name"),
        tenant_id,
        role,
        department_id: row.get("department_id"),
        enabled: row.get("enabled"),
        created_at,
        updated_at,
        media_preferences,
        email: row.get("email"),
        password_last_reset,
        phone_number: row.get("phone_number"),
        mobile_number: row.get("mobile_number"),
        pin: row.get("pin"),
        extension: row.get("extension"),
        department: row.get("department"),
        job_title: row.get("job_title"),
        location: row.get("location"),
        location_id: row
            .get::<Option<String>, _>("location_id")
            .and_then(|s| fcp_core::LocationId::from_str(&s).ok()),
        language: row.get("language"),
        timezone: row.get("timezone"),
        account_status,
        last_login,
        emergency_contact,
        primary_phone_number_id,
        voicemail_box_id,
        profile_picture: row.get("profile_picture"),
        failed_login_attempts,
        account_locked_until,
    })
}

/// Convert PostgreSQL row to Registration
fn row_to_registration(row: sqlx::postgres::PgRow) -> Result<Registration> {
    let expires_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("expires_at"));
    let registered_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("registered_at"));

    Ok(Registration {
        tenant_id: TenantId::from_str(&row.get::<String, _>("tenant_id"))
            .unwrap_or_else(|_| TenantId::nil()),
        aor: row.get("aor"),
        contact: row.get("contact"),
        expires_at,
        call_id: row.get("call_id"),
        cseq: row.get::<i32, _>("cseq") as u32,
        user_agent: row.get("user_agent"),
        q: row.get("q"),
        registered_at,
        path: row.try_get("path").ok(),
        flow_token: row.try_get("flow_token").ok().flatten(),
        source_address: row.try_get("source_address").ok().flatten(),
    })
}

fn row_to_cdr(row: sqlx::postgres::PgRow) -> Result<CallDetailRecord> {
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;
    let call_id = fcp_core::CallId::from_str(&row.get::<String, _>("call_id"))?;

    let hangup_cause_str: String = row.get("hangup_cause");
    let hangup_cause = if hangup_cause_str.starts_with("other:") {
        let code = hangup_cause_str
            .strip_prefix("other:")
            .unwrap()
            .parse()
            .unwrap_or(0);
        HangupCause::Other(code)
    } else {
        serde_json::from_str::<HangupCause>(&format!("\"{}\"", hangup_cause_str))
            .unwrap_or(HangupCause::NormalClearing)
    };

    let hangup_party: Option<HangupParty> = row
        .get::<Option<String>, _>("hangup_party")
        .and_then(|s| serde_json::from_str::<HangupParty>(&format!("\"{}\"", s)).ok());

    let disposition_str: String = row.get("disposition");
    let disposition = serde_json::from_str::<CallDisposition>(&format!("\"{}\"", disposition_str))
        .unwrap_or(CallDisposition::NoAnswer);

    let custom_data: Option<serde_json::Value> =
        row.get::<Option<serde_json::Value>, _>("custom_data");

    let start_time = datetime_to_system_time_micros(row.get::<DateTime<Utc>, _>("start_time"));
    let answer_time = row
        .get::<Option<DateTime<Utc>>, _>("answer_time")
        .map(datetime_to_system_time_micros);
    let end_time = row
        .get::<Option<DateTime<Utc>>, _>("end_time")
        .map(datetime_to_system_time_micros);

    let kind = row
        .get::<String, _>("kind")
        .parse::<fcp_core::storage::CdrKind>()
        .unwrap_or_default();
    let correlation_id = row
        .get::<Option<String>, _>("correlation_id")
        .and_then(|s| fcp_core::CallId::from_str(&s).ok());

    Ok(CallDetailRecord {
        cdr_id: row.get("cdr_id"),
        kind,
        schema_version: row.get::<i16, _>("schema_version") as u16,
        call_id,
        tenant_id,
        sip_call_id: row.get("sip_call_id"),
        from_uri: row.get("from_uri"),
        from_display_name: row.get("from_display_name"),
        to_uri: row.get("to_uri"),
        to_display_name: row.get("to_display_name"),
        start_time,
        answer_time,
        end_time,
        setup_duration_ms: row
            .get::<Option<i64>, _>("setup_duration_ms")
            .map(|d| d as u64),
        call_duration_ms: row
            .get::<Option<i64>, _>("call_duration_ms")
            .map(|d| d as u64),
        total_duration_ms: row
            .get::<Option<i64>, _>("total_duration_ms")
            .map(|d| d as u64),
        disposition,
        hangup_cause,
        hangup_party,
        sip_status_code: row
            .get::<Option<i32>, _>("sip_status_code")
            .map(|c| c as u16),
        sip_reason: row.get("sip_reason"),
        direction: row
            .get::<Option<String>, _>("direction")
            .and_then(|d| call_direction_from_name(&d)),
        node_id: row.get("node_id"),
        site_id: row
            .try_get::<Option<String>, _>("site_id")
            .ok()
            .flatten()
            .and_then(|s| fcp_core::SiteId::from_str(&s).ok()),
        region: row.get("region"),
        correlation_id,
        trunk_name: row.get("trunk_name"),
        trunk_id: row
            .get::<Option<String>, _>("trunk_id")
            .map(fcp_core::TrunkId::from_string),
        route_id: row.get("route_id"),
        gateway_id: row.get("gateway_id"),
        media_anchored: row.get("media_anchored"),
        codec_a_leg: row.get("codec_a_leg"),
        codec_b_leg: row.get("codec_b_leg"),
        rtp_packets_sent: row
            .get::<Option<i64>, _>("rtp_packets_sent")
            .map(|p| p as u64),
        rtp_packets_received: row
            .get::<Option<i64>, _>("rtp_packets_received")
            .map(|p| p as u64),
        rtp_bytes_sent: row
            .get::<Option<i64>, _>("rtp_bytes_sent")
            .map(|b| b as u64),
        rtp_bytes_received: row
            .get::<Option<i64>, _>("rtp_bytes_received")
            .map(|b| b as u64),
        rtp_packets_lost: row
            .get::<Option<i64>, _>("rtp_packets_lost")
            .map(|p| p as u64),
        rtp_jitter_ms: row.get("rtp_jitter_ms"),
        recording_url: row.get("recording_url"),
        recording_consent: row
            .get::<Option<String>, _>("recording_consent")
            .and_then(|c| fcp_core::RecordingConsent::parse(&c)),
        cost: row.get("cost"),
        currency: row.get("currency"),
        rate_plan_id: row.get("rate_plan_id"),
        matched_prefix: row.get("matched_prefix"),
        billable_seconds: row
            .get::<Option<i32>, _>("billable_seconds")
            .map(|s| s as u32),
        rated_at: row
            .get::<Option<DateTime<Utc>>, _>("rated_at")
            .map(datetime_to_system_time_micros),
        created_at: Some(datetime_to_system_time_micros(
            row.get::<DateTime<Utc>, _>("created_at"),
        )),
        custom_data,
    })
}

// ============================================================================
// Phone Number, Line Binding, Extension, and Device Store Implementations
// ============================================================================

#[async_trait]
impl PhoneNumberStore for PostgresStorage {
    async fn get_phone_number(&self, id: &PhoneNumberId) -> Result<Option<PhoneNumber>> {
        let row = sqlx::query(
            "SELECT id, e164, country_code, npa, nxx, line, state, tenant_id,
                    e911_address_json, e911_callback, cnam, spam_score,
                    carrier, lrn, ocn, rate_center, lata,
                    acquired_at, activated_at, ported_at, loa_date, foc_date,
                    recording_enabled, block_anonymous, allow_list, block_list, tags,
                    location_id, site_id, created_at, updated_at
             FROM phone_numbers
             WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_phone_number(row)?)),
            None => Ok(None),
        }
    }

    async fn get_phone_number_by_e164(&self, e164: &str) -> Result<Option<PhoneNumber>> {
        let row = sqlx::query(
            "SELECT id, e164, country_code, npa, nxx, line, state, tenant_id,
                    e911_address_json, e911_callback, cnam, spam_score,
                    carrier, lrn, ocn, rate_center, lata,
                    acquired_at, activated_at, ported_at, loa_date, foc_date,
                    recording_enabled, block_anonymous, allow_list, block_list, tags,
                    location_id, site_id, created_at, updated_at
             FROM phone_numbers
             WHERE e164 = $1",
        )
        .bind(e164)
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_phone_number(row)?)),
            None => Ok(None),
        }
    }

    async fn list_phone_numbers(&self, tenant_id: &TenantId) -> Result<Vec<PhoneNumber>> {
        let rows = sqlx::query(
            "SELECT id, e164, country_code, npa, nxx, line, state, tenant_id,
                    e911_address_json, e911_callback, cnam, spam_score,
                    carrier, lrn, ocn, rate_center, lata,
                    acquired_at, activated_at, ported_at, loa_date, foc_date,
                    recording_enabled, block_anonymous, allow_list, block_list, tags,
                    location_id, site_id, created_at, updated_at
             FROM phone_numbers
             WHERE tenant_id = $1
             ORDER BY e164",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let numbers = rows
            .into_iter()
            .filter_map(|row| row_to_phone_number(row).ok())
            .collect();

        Ok(numbers)
    }

    async fn put_phone_number(&self, phone_number: PhoneNumber) -> Result<()> {
        let id = phone_number.id.to_string();
        let tenant_id = phone_number.tenant_id.to_string();
        let state = phone_number.state.to_string();
        let e911_address_json = phone_number
            .e911_address
            .map(|a| serde_json::to_value(&a))
            .transpose()?;
        let allow_list = serde_json::to_value(&phone_number.allow_list)?;
        let block_list = serde_json::to_value(&phone_number.block_list)?;
        let tags = serde_json::to_value(&phone_number.tags)?;
        let acquired_at = system_time_to_datetime(phone_number.acquired_at);
        let activated_at = phone_number.activated_at.map(system_time_to_datetime);
        let ported_at = phone_number.ported_at.map(system_time_to_datetime);
        let loa_date = phone_number.loa_date.map(system_time_to_datetime);
        let foc_date = phone_number.foc_date.map(system_time_to_datetime);
        let created_at = system_time_to_datetime(phone_number.created_at);
        let updated_at = system_time_to_datetime(phone_number.updated_at);
        let location_id = phone_number.location_id.map(|id| id.to_string());

        sqlx::query(
            "INSERT INTO phone_numbers (
                id, e164, country_code, npa, nxx, line, state, tenant_id,
                e911_address_json, e911_callback, cnam, spam_score,
                carrier, lrn, ocn, rate_center, lata,
                acquired_at, activated_at, ported_at, loa_date, foc_date,
                recording_enabled, block_anonymous, allow_list, block_list, tags,
                location_id, site_id, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31)
             ON CONFLICT(id) DO UPDATE SET
                 tenant_id = EXCLUDED.tenant_id,
                 state = EXCLUDED.state,
                 acquired_at = EXCLUDED.acquired_at,
                 e911_address_json = EXCLUDED.e911_address_json,
                 e911_callback = EXCLUDED.e911_callback,
                 cnam = EXCLUDED.cnam,
                 spam_score = EXCLUDED.spam_score,
                 carrier = EXCLUDED.carrier,
                 lrn = EXCLUDED.lrn,
                 ocn = EXCLUDED.ocn,
                 rate_center = EXCLUDED.rate_center,
                 lata = EXCLUDED.lata,
                 activated_at = EXCLUDED.activated_at,
                 ported_at = EXCLUDED.ported_at,
                 loa_date = EXCLUDED.loa_date,
                 foc_date = EXCLUDED.foc_date,
                 recording_enabled = EXCLUDED.recording_enabled,
                 block_anonymous = EXCLUDED.block_anonymous,
                 allow_list = EXCLUDED.allow_list,
                 block_list = EXCLUDED.block_list,
                 tags = EXCLUDED.tags,
                 location_id = EXCLUDED.location_id,
                 site_id = EXCLUDED.site_id,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(&id)
        .bind(&phone_number.e164)
        .bind(&phone_number.country_code)
        .bind(&phone_number.npa)
        .bind(&phone_number.nxx)
        .bind(&phone_number.line)
        .bind(&state)
        .bind(&tenant_id)
        .bind(&e911_address_json)
        .bind(&phone_number.e911_callback)
        .bind(&phone_number.cnam)
        .bind(phone_number.spam_score)
        .bind(&phone_number.carrier)
        .bind(&phone_number.lrn)
        .bind(&phone_number.ocn)
        .bind(&phone_number.rate_center)
        .bind(&phone_number.lata)
        .bind(acquired_at)
        .bind(activated_at)
        .bind(ported_at)
        .bind(loa_date)
        .bind(foc_date)
        .bind(phone_number.recording_enabled)
        .bind(phone_number.block_anonymous)
        .bind(&allow_list)
        .bind(&block_list)
        .bind(&tags)
        .bind(&location_id)
        .bind(phone_number.site_id.map(|id| id.to_string()))
        .bind(created_at)
        .bind(updated_at)
        .execute(self.writes())
        .await?;

        debug!(id = %phone_number.id, e164 = %phone_number.e164, "Phone number stored");
        Ok(())
    }

    async fn purchase_available_phone_number(
        &self,
        id: &PhoneNumberId,
        tenant_id: &TenantId,
        acquired_at: SystemTime,
    ) -> Result<Option<PhoneNumber>> {
        let row = sqlx::query(
            "UPDATE phone_numbers
             SET tenant_id = $2,
                 state = 'assigned',
                 acquired_at = $3,
                 updated_at = $3
             WHERE id = $1
               AND tenant_id = $4
               AND state = 'available'
             RETURNING *",
        )
        .bind(id.to_string())
        .bind(tenant_id.to_string())
        .bind(system_time_to_datetime(acquired_at))
        .bind(TenantId::nil().to_string())
        .fetch_optional(self.writes())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_phone_number(row)?)),
            None => Ok(None),
        }
    }

    async fn delete_phone_number(&self, id: &PhoneNumberId) -> Result<()> {
        // Cascades to line bindings, mailboxes and messages: attributed, so
        // the messages' deletion is logged and their audio queued.
        deletion::execute(
            self.writes(),
            sqlx::query("DELETE FROM phone_numbers WHERE id = $1").bind(id.to_string()),
        )
        .await?;

        debug!(id = %id, "Phone number deleted");
        Ok(())
    }

    async fn search_available_numbers(
        &self,
        query: AvailableNumberQuery,
    ) -> Result<Vec<PhoneNumber>> {
        // Pool numbers have nil tenant_id AND state = 'available'
        let nil_tenant = TenantId::nil().to_string();
        let mut sql = String::from(
            "SELECT id, e164, country_code, npa, nxx, line, state, tenant_id,
                    e911_address_json, e911_callback, cnam, spam_score,
                    carrier, lrn, ocn, rate_center, lata,
                    acquired_at, activated_at, ported_at, loa_date, foc_date,
                    recording_enabled, block_anonymous, allow_list, block_list, tags,
                    location_id, site_id, created_at, updated_at
             FROM phone_numbers
             WHERE tenant_id = $1 AND state = 'available'",
        );
        let mut binds: Vec<String> = vec![nil_tenant];
        let mut param_idx: usize = 2;

        if let Some(ref area_code) = query.area_code {
            sql.push_str(&format!(" AND npa = ${}", param_idx));
            binds.push(area_code.clone());
            param_idx += 1;
        }

        if let Some(ref pattern) = query.pattern {
            // Convert glob pattern (* -> %) to SQL LIKE
            let like_pattern = pattern.replace('*', "%");
            sql.push_str(&format!(" AND e164 LIKE ${}", param_idx));
            binds.push(like_pattern);
            param_idx += 1;
        }

        if let Some(ref rate_center) = query.rate_center {
            sql.push_str(&format!(" AND rate_center = ${}", param_idx));
            binds.push(rate_center.clone());
            param_idx += 1;
        }

        let limit = if query.limit == 0 { 50 } else { query.limit };
        sql.push_str(&format!(" ORDER BY e164 LIMIT ${}", param_idx));
        let _ = param_idx;

        let mut q = sqlx::query(&sql);
        for bind in &binds {
            q = q.bind(bind);
        }
        q = q.bind(limit as i64);

        let rows = q.fetch_all(self.reads()).await?;

        let numbers = rows
            .into_iter()
            .filter_map(|row| row_to_phone_number(row).ok())
            .collect();

        Ok(numbers)
    }
}

// ============================================================================
// LineBindingStore Implementation
// ============================================================================

#[async_trait]
#[async_trait]
impl LineBindingStore for PostgresStorage {
    async fn get_line_binding(&self, id: &LineBindingId) -> Result<Option<LineBinding>> {
        let row = sqlx::query(
            "SELECT id, phone_number_id, owner_user_id, binding_type, ring_strategy, tenant_id,
                    outbound_caller_id, outbound_cnam,
                    flow_id, schedule_id, closed_target, holiday_target, shared, max_appearances, barge_in,
                    created_at, updated_at
             FROM line_bindings
             WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => {
                let binding = row_to_line_binding(row)?;
                // Fetch device bindings
                let devices = self.get_device_bindings(id).await?;
                Ok(Some(LineBinding { devices, ..binding }))
            }
            None => Ok(None),
        }
    }

    async fn get_line_binding_by_phone_number(
        &self,
        phone_number_id: &PhoneNumberId,
    ) -> Result<Option<LineBinding>> {
        let row = sqlx::query(
            "SELECT id, phone_number_id, owner_user_id, binding_type, ring_strategy, tenant_id,
                    outbound_caller_id, outbound_cnam,
                    flow_id, schedule_id, closed_target, holiday_target, shared, max_appearances, barge_in,
                    created_at, updated_at
             FROM line_bindings
             WHERE phone_number_id = $1",
        )
        .bind(phone_number_id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => {
                let binding = row_to_line_binding(row)?;
                let devices = self.get_device_bindings(&binding.id).await?;
                Ok(Some(LineBinding { devices, ..binding }))
            }
            None => Ok(None),
        }
    }

    async fn list_line_bindings(&self, tenant_id: &TenantId) -> Result<Vec<LineBinding>> {
        let rows = sqlx::query(
            "SELECT id, phone_number_id, owner_user_id, binding_type, ring_strategy, tenant_id,
                    outbound_caller_id, outbound_cnam,
                    flow_id, schedule_id, closed_target, holiday_target, shared, max_appearances, barge_in,
                    created_at, updated_at
             FROM line_bindings
             WHERE tenant_id = $1
             ORDER BY created_at",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let mut bindings = Vec::new();
        for row in rows {
            let binding = row_to_line_binding(row)?;
            let devices = self.get_device_bindings(&binding.id).await?;
            bindings.push(LineBinding { devices, ..binding });
        }

        Ok(bindings)
    }

    async fn list_line_bindings_by_user(&self, user_id: &UserId) -> Result<Vec<LineBinding>> {
        let rows = sqlx::query(
            "SELECT id, phone_number_id, owner_user_id, binding_type, ring_strategy, tenant_id,
                    outbound_caller_id, outbound_cnam,
                    flow_id, schedule_id, closed_target, holiday_target, shared, max_appearances, barge_in,
                    created_at, updated_at
             FROM line_bindings
             WHERE owner_user_id = $1
             ORDER BY created_at",
        )
        .bind(user_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let mut bindings = Vec::new();
        for row in rows {
            let binding = row_to_line_binding(row)?;
            let devices = self.get_device_bindings(&binding.id).await?;
            bindings.push(LineBinding { devices, ..binding });
        }

        Ok(bindings)
    }

    async fn list_line_bindings_by_device(&self, device_id: &DeviceId) -> Result<Vec<LineBinding>> {
        let rows = sqlx::query(
            "SELECT lb.id, lb.phone_number_id, lb.owner_user_id, lb.binding_type, lb.ring_strategy, lb.tenant_id,
                    lb.outbound_caller_id, lb.outbound_cnam,
                    lb.flow_id, lb.schedule_id, lb.closed_target, lb.holiday_target,
                    lb.shared, lb.max_appearances, lb.barge_in,
                    lb.created_at, lb.updated_at
             FROM line_bindings lb
             INNER JOIN device_bindings db ON lb.id = db.line_binding_id
             WHERE db.device_id = $1
             ORDER BY lb.created_at",
        )
        .bind(device_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let mut bindings = Vec::new();
        for row in rows {
            let binding = row_to_line_binding(row)?;
            let devices = self.get_device_bindings(&binding.id).await?;
            bindings.push(LineBinding { devices, ..binding });
        }

        Ok(bindings)
    }

    async fn put_line_binding(&self, binding: LineBinding) -> Result<()> {
        let id = binding.id.to_string();
        let phone_number_id = binding.phone_number_id.to_string();
        let owner_user_id = binding.owner_user_id.map(|u| u.to_string());
        let binding_type = binding.binding_type.to_string();
        let ring_strategy = binding.ring_strategy.to_string();
        let tenant_id = binding.tenant_id.to_string();
        let created_at = system_time_to_datetime(binding.created_at);
        let updated_at = system_time_to_datetime(binding.updated_at);

        sqlx::query(
            "INSERT INTO line_bindings (
                id, phone_number_id, owner_user_id, binding_type, ring_strategy, tenant_id,
                outbound_caller_id, outbound_cnam,
                flow_id, created_at, updated_at,
                schedule_id, closed_target, holiday_target, shared, max_appearances, barge_in
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
             ON CONFLICT(id) DO UPDATE SET
                 shared = EXCLUDED.shared,
                 barge_in = EXCLUDED.barge_in,
                 max_appearances = EXCLUDED.max_appearances,
                 schedule_id = EXCLUDED.schedule_id,
                 closed_target = EXCLUDED.closed_target,
                 holiday_target = EXCLUDED.holiday_target,
                 ring_strategy = EXCLUDED.ring_strategy,
                 outbound_caller_id = EXCLUDED.outbound_caller_id,
                 outbound_cnam = EXCLUDED.outbound_cnam,
                 flow_id = EXCLUDED.flow_id,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(&id)
        .bind(&phone_number_id)
        .bind(&owner_user_id)
        .bind(&binding_type)
        .bind(&ring_strategy)
        .bind(&tenant_id)
        .bind(&binding.outbound_caller_id)
        .bind(&binding.outbound_cnam)
        .bind(&binding.flow_id)
        .bind(created_at)
        .bind(updated_at)
        .bind(binding.schedule_id.map(|id| id.to_string()))
        .bind(binding.closed_target.as_ref().map(|t| t.to_string()))
        .bind(binding.holiday_target.as_ref().map(|t| t.to_string()))
        .bind(binding.shared)
        .bind(i16::from(binding.max_appearances))
        .bind(binding.barge_in)
        .execute(self.writes())
        .await?;

        // Delete existing device bindings
        sqlx::query("DELETE FROM device_bindings WHERE line_binding_id = $1")
            .bind(&id)
            .execute(self.writes())
            .await?;

        // Insert new device bindings
        for device in &binding.devices {
            self.put_device_binding(&binding.id, device).await?;
        }

        debug!(id = %binding.id, "Line binding stored");
        Ok(())
    }

    async fn delete_line_binding(&self, id: &LineBindingId) -> Result<()> {
        deletion::execute(
            self.writes(),
            sqlx::query("DELETE FROM line_bindings WHERE id = $1").bind(id.to_string()),
        )
        .await?;

        debug!(id = %id, "Line binding deleted");
        Ok(())
    }
}

#[async_trait]
impl ExtensionStore for PostgresStorage {
    async fn get_extension(&self, id: &ExtensionId) -> Result<Option<Extension>> {
        let row = sqlx::query(
            "SELECT id, number, target_type, target_id, tenant_id, description, location_id, enabled,
                    created_at, updated_at
             FROM extensions
             WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_extension(row)?)),
            None => Ok(None),
        }
    }

    async fn get_extension_by_number(
        &self,
        tenant_id: &TenantId,
        number: &str,
    ) -> Result<Option<Extension>> {
        let row = sqlx::query(
            "SELECT id, number, target_type, target_id, tenant_id, description, location_id, enabled,
                    created_at, updated_at
             FROM extensions
             WHERE tenant_id = $1 AND number = $2",
        )
        .bind(tenant_id.to_string())
        .bind(number)
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_extension(row)?)),
            None => Ok(None),
        }
    }

    async fn list_extensions(&self, tenant_id: &TenantId) -> Result<Vec<Extension>> {
        let rows = sqlx::query(
            "SELECT id, number, target_type, target_id, tenant_id, description, location_id, enabled,
                    created_at, updated_at
             FROM extensions
             WHERE tenant_id = $1
             ORDER BY number",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let extensions = rows
            .into_iter()
            .filter_map(|row| row_to_extension(row).ok())
            .collect();

        Ok(extensions)
    }

    async fn put_extension(&self, extension: Extension) -> Result<()> {
        let id = extension.id.to_string();
        let tenant_id = extension.tenant_id.to_string();
        let (target_type, target_id) = match &extension.target {
            ExtensionTarget::User(user_id) => ("user", user_id.to_string()),
            ExtensionTarget::PhoneNumber(num_id) => ("phone_number", num_id.to_string()),
            ExtensionTarget::Device(dev_id) => ("device", dev_id.to_string()),
            ExtensionTarget::Service(svc_id) => ("service", svc_id.to_string()),
        };
        let created_at = system_time_to_datetime(extension.created_at);
        let updated_at = system_time_to_datetime(extension.updated_at);
        let location_id = extension.location_id.map(|id| id.to_string());

        sqlx::query(
            "INSERT INTO extensions (
                id, number, target_type, target_id, tenant_id, description, location_id, enabled,
                created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT(tenant_id, number) DO UPDATE SET
                 target_type = EXCLUDED.target_type,
                 target_id = EXCLUDED.target_id,
                 description = EXCLUDED.description,
                 location_id = EXCLUDED.location_id,
                 enabled = EXCLUDED.enabled,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(&id)
        .bind(&extension.number)
        .bind(target_type)
        .bind(&target_id)
        .bind(&tenant_id)
        .bind(&extension.description)
        .bind(&location_id)
        .bind(extension.enabled)
        .bind(created_at)
        .bind(updated_at)
        .execute(self.writes())
        .await?;

        debug!(id = %extension.id, number = %extension.number, "Extension stored");
        Ok(())
    }

    async fn delete_extension(&self, id: &ExtensionId) -> Result<()> {
        sqlx::query("DELETE FROM extensions WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;

        debug!(id = %id, "Extension deleted");
        Ok(())
    }
}

// ============================================================================
// DeviceStore Implementation
// ============================================================================

/// The columns [`row_to_device`] reads.
macro_rules! device_columns {
    () => {
        "id, name, device_type, mac_address, model, firmware_version, tenant_id, owner_user_id,
         sip_username, sip_ha1_hash, sip_provisioning_password, hoteling_host,
         sip_uri, registration_state, last_registration, last_ip_address, user_agent,
         max_lines, supports_video, supports_blf, codec_support,
         location, location_id, e911_override_json, e911_trust_location, provisioning_url,
         config_template_id, provisioning_token_hash, enabled, created_at, updated_at"
    };
}

#[async_trait]
impl DeviceStore for PostgresStorage {
    async fn get_device(&self, id: &DeviceId) -> Result<Option<Device>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            device_columns!(),
            " FROM devices
             WHERE id = $1"
        ))
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(self.open_device(row_to_device(row)?).await?)),
            None => Ok(None),
        }
    }

    async fn get_device_by_sip_uri(&self, sip_uri: &str) -> Result<Option<Device>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            device_columns!(),
            " FROM devices
             WHERE sip_uri = $1"
        ))
        .bind(sip_uri)
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(self.open_device(row_to_device(row)?).await?)),
            None => Ok(None),
        }
    }

    async fn list_devices(&self, tenant_id: &TenantId) -> Result<Vec<Device>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            device_columns!(),
            " FROM devices
             WHERE tenant_id = $1
             ORDER BY name"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let mut devices = Vec::with_capacity(rows.len());
        for row in rows {
            devices.push(self.open_device(row_to_device(row)?).await?);
        }
        Ok(devices)
    }

    async fn list_devices_by_user(&self, user_id: &UserId) -> Result<Vec<Device>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            device_columns!(),
            " FROM devices
             WHERE owner_user_id = $1
             ORDER BY name"
        ))
        .bind(user_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let mut devices = Vec::with_capacity(rows.len());
        for row in rows {
            devices.push(self.open_device(row_to_device(row)?).await?);
        }
        Ok(devices)
    }

    async fn put_device(&self, device: Device) -> Result<()> {
        let id = device.id.to_string();
        let device_type = device.device_type.to_string();
        let tenant_id = device.tenant_id.to_string();
        let owner_user_id = device.owner_user_id.map(|u| u.to_string());
        let registration_state = device.registration_state.to_string();
        let last_registration = device.last_registration.map(system_time_to_datetime);
        let codec_support = serde_json::to_value(
            device
                .codec_support
                .into_iter()
                .map(|c| format!("{:?}", c))
                .collect::<Vec<_>>(),
        )?;
        let e911_override_json = device
            .e911_override
            .map(|a| serde_json::to_value(&a))
            .transpose()?;
        let created_at = system_time_to_datetime(device.created_at);
        let updated_at = system_time_to_datetime(device.updated_at);
        let location_id = device.location_id.map(|id| id.to_string());
        let sip_ha1_hash = self.seal_opt(device.sip_ha1_hash.as_deref())?;
        let sip_provisioning_password =
            self.seal_opt(device.sip_provisioning_password.as_deref())?;

        sqlx::query(
            "INSERT INTO devices (
                id, name, device_type, mac_address, model, firmware_version, tenant_id, owner_user_id,
                sip_username, sip_ha1_hash, sip_provisioning_password, hoteling_host,
                sip_uri, registration_state, last_registration, last_ip_address, user_agent,
                max_lines, supports_video, supports_blf, codec_support,
                location, location_id, e911_override_json, e911_trust_location, provisioning_url, config_template_id, provisioning_token_hash, enabled,
                created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31)
             ON CONFLICT(id) DO UPDATE SET
                 name = EXCLUDED.name,
                 mac_address = EXCLUDED.mac_address,
                 model = EXCLUDED.model,
                 firmware_version = EXCLUDED.firmware_version,
                 owner_user_id = EXCLUDED.owner_user_id,
                 sip_username = EXCLUDED.sip_username,
                 sip_ha1_hash = EXCLUDED.sip_ha1_hash,
                 sip_provisioning_password = EXCLUDED.sip_provisioning_password,
                 hoteling_host = EXCLUDED.hoteling_host,
                 sip_uri = EXCLUDED.sip_uri,
                 registration_state = EXCLUDED.registration_state,
                 last_registration = EXCLUDED.last_registration,
                 last_ip_address = EXCLUDED.last_ip_address,
                 user_agent = EXCLUDED.user_agent,
                 max_lines = EXCLUDED.max_lines,
                 supports_video = EXCLUDED.supports_video,
                 supports_blf = EXCLUDED.supports_blf,
                 codec_support = EXCLUDED.codec_support,
                 location = EXCLUDED.location,
                 location_id = EXCLUDED.location_id,
                 e911_override_json = EXCLUDED.e911_override_json,
                 e911_trust_location = EXCLUDED.e911_trust_location,
                 provisioning_url = EXCLUDED.provisioning_url,
                 config_template_id = EXCLUDED.config_template_id,
                 provisioning_token_hash = EXCLUDED.provisioning_token_hash,
                 enabled = EXCLUDED.enabled,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(&id)
        .bind(&device.name)
        .bind(&device_type)
        .bind(&device.mac_address)
        .bind(&device.model)
        .bind(&device.firmware_version)
        .bind(&tenant_id)
        .bind(&owner_user_id)
        .bind(&device.sip_username)
        .bind(&sip_ha1_hash)
        .bind(&sip_provisioning_password)
        .bind(device.hoteling_host)
        .bind(&device.sip_uri)
        .bind(&registration_state)
        .bind(last_registration)
        .bind(&device.last_ip_address)
        .bind(&device.user_agent)
        .bind(device.max_lines as i64)
        .bind(device.supports_video)
        .bind(device.supports_blf)
        .bind(&codec_support)
        .bind(&device.location)
        .bind(&location_id)
        .bind(&e911_override_json)
        .bind(device.e911_trust_location)
        .bind(&device.provisioning_url)
        .bind(&device.config_template_id)
        .bind(&device.provisioning_token_hash)
        .bind(device.enabled)
        .bind(created_at)
        .bind(updated_at)
        .execute(self.writes())
        .await
        .map_err(map_device_write_error)?;

        debug!(id = %device.id, name = %device.name, "Device stored");
        Ok(())
    }

    async fn get_device_by_mac_address(&self, mac_address: &str) -> Result<Option<Device>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            device_columns!(),
            " FROM devices
             WHERE LOWER(REPLACE(REPLACE(REPLACE(mac_address, ':', ''), '-', ''), '.', '')) = $1"
        ))
        .bind(mac_address)
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(self.open_device(row_to_device(row)?).await?)),
            None => Ok(None),
        }
    }

    async fn get_device_by_sip_name(
        &self,
        tenant_id: &TenantId,
        name: &str,
    ) -> Result<Option<Device>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            device_columns!(),
            " FROM devices
             WHERE tenant_id = $1
               AND (LOWER(sip_username) = LOWER($2)
                    OR LOWER(split_part(regexp_replace(sip_uri, '^sips?:', ''), '@', 1)) = LOWER($2))
             ORDER BY (LOWER(sip_username) = LOWER($2)) IS TRUE DESC, id
             LIMIT 1"
        ))
        .bind(tenant_id.to_string())
        .bind(name)
        .fetch_optional(self.reads())
        .await?;
        match row {
            Some(row) => Ok(Some(self.open_device(row_to_device(row)?).await?)),
            None => Ok(None),
        }
    }

    async fn delete_device(&self, id: &DeviceId) -> Result<()> {
        sqlx::query("DELETE FROM devices WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;

        debug!(id = %id, "Device deleted");
        Ok(())
    }
}

#[async_trait]
impl fcp_core::storage::ConfigTemplateStore for PostgresStorage {
    async fn get_config_template(
        &self,
        id: &fcp_core::ConfigTemplateId,
    ) -> Result<Option<fcp_core::config_template::ConfigTemplate>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, name, vendor, model_pattern, template_body, description,
                    created_at, updated_at
             FROM config_templates
             WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_config_template(row)?)),
            None => Ok(None),
        }
    }

    async fn list_config_templates(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<fcp_core::config_template::ConfigTemplate>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, name, vendor, model_pattern, template_body, description,
                    created_at, updated_at
             FROM config_templates
             WHERE tenant_id = $1
             ORDER BY name",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        rows.into_iter().map(row_to_config_template).collect()
    }

    async fn put_config_template(
        &self,
        template: fcp_core::config_template::ConfigTemplate,
    ) -> Result<()> {
        let id = template.id.to_string();
        let tenant_id = template.tenant_id.to_string();
        let vendor = template.vendor.to_string();
        let created_at = system_time_to_datetime(template.created_at);
        let updated_at = system_time_to_datetime(template.updated_at);

        sqlx::query(
            "INSERT INTO config_templates (id, tenant_id, name, vendor, model_pattern, template_body, description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT(id) DO UPDATE SET
                 name = EXCLUDED.name,
                 vendor = EXCLUDED.vendor,
                 model_pattern = EXCLUDED.model_pattern,
                 template_body = EXCLUDED.template_body,
                 description = EXCLUDED.description,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(&id)
        .bind(&tenant_id)
        .bind(&template.name)
        .bind(&vendor)
        .bind(&template.model_pattern)
        .bind(&template.template_body)
        .bind(&template.description)
        .bind(created_at)
        .bind(updated_at)
        .execute(self.writes())
        .await?;

        Ok(())
    }

    async fn delete_config_template(&self, id: &fcp_core::ConfigTemplateId) -> Result<()> {
        sqlx::query("DELETE FROM config_templates WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

#[async_trait]
impl ConferenceStore for PostgresStorage {
    async fn get_meeting(&self, id: &fcp_core::MeetingId) -> Result<Option<fcp_core::Meeting>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, room_number, name, guest_pin, host_pin,
                    require_guest_pin, max_participants, wait_for_moderator,
                    enabled, auto_record, default_locked, permanent, owner_id,
                    pin_prompt_sound, invalid_pin_sound, moh_sound,
                    pin_prompt_tts, invalid_pin_tts, welcome_tts,
                    play_entry_tone, play_exit_tone, join_sound, exit_sound,
                    video_enabled, video_layout, video_max_participants,
                    video_max_resolution, video_fps, video_exclude_self, video_codecs, video_passthrough,
                    video_content_share, video_content_layout,
                    created_at, updated_at
             FROM conference_meetings WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(self.open_meeting(map_meeting_row_pg(row)?).await?)),
            None => Ok(None),
        }
    }

    async fn get_meeting_by_room_number(
        &self,
        tenant_id: &TenantId,
        room_number: &str,
    ) -> Result<Option<fcp_core::Meeting>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, room_number, name, guest_pin, host_pin,
                    require_guest_pin, max_participants, wait_for_moderator,
                    enabled, auto_record, default_locked, permanent, owner_id,
                    pin_prompt_sound, invalid_pin_sound, moh_sound,
                    pin_prompt_tts, invalid_pin_tts, welcome_tts,
                    play_entry_tone, play_exit_tone, join_sound, exit_sound,
                    video_enabled, video_layout, video_max_participants,
                    video_max_resolution, video_fps, video_exclude_self, video_codecs, video_passthrough,
                    video_content_share, video_content_layout,
                    created_at, updated_at
             FROM conference_meetings WHERE tenant_id = $1 AND room_number = $2",
        )
        .bind(tenant_id.to_string())
        .bind(room_number)
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(self.open_meeting(map_meeting_row_pg(row)?).await?)),
            None => Ok(None),
        }
    }

    async fn list_meetings(&self, tenant_id: &TenantId) -> Result<Vec<fcp_core::Meeting>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, room_number, name, guest_pin, host_pin,
                    require_guest_pin, max_participants, wait_for_moderator,
                    enabled, auto_record, default_locked, permanent, owner_id,
                    pin_prompt_sound, invalid_pin_sound, moh_sound,
                    pin_prompt_tts, invalid_pin_tts, welcome_tts,
                    play_entry_tone, play_exit_tone, join_sound, exit_sound,
                    video_enabled, video_layout, video_max_participants,
                    video_max_resolution, video_fps, video_exclude_self, video_codecs, video_passthrough,
                    video_content_share, video_content_layout,
                    created_at, updated_at
             FROM conference_meetings WHERE tenant_id = $1 ORDER BY room_number",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let meetings = rows
            .into_iter()
            .filter_map(|row| map_meeting_row_pg(row).ok())
            .collect();
        let meetings = self.open_meetings(meetings).await?;

        Ok(meetings)
    }

    async fn list_meetings_by_owner(
        &self,
        tenant_id: &fcp_core::TenantId,
        owner_id: &str,
    ) -> Result<Vec<fcp_core::Meeting>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, room_number, name, guest_pin, host_pin,
                    require_guest_pin, max_participants, wait_for_moderator,
                    enabled, auto_record, default_locked, permanent, owner_id,
                    pin_prompt_sound, invalid_pin_sound, moh_sound,
                    pin_prompt_tts, invalid_pin_tts, welcome_tts,
                    play_entry_tone, play_exit_tone, join_sound, exit_sound,
                    video_enabled, video_layout, video_max_participants,
                    video_max_resolution, video_fps, video_exclude_self, video_codecs, video_passthrough,
                    video_content_share, video_content_layout,
                    created_at, updated_at
             FROM conference_meetings
             WHERE tenant_id = $1 AND owner_id = $2
             ORDER BY room_number",
        )
        .bind(tenant_id.to_string())
        .bind(owner_id)
        .fetch_all(self.reads())
        .await?;

        let meetings = rows
            .into_iter()
            .filter_map(|row| map_meeting_row_pg(row).ok())
            .collect();
        let meetings = self.open_meetings(meetings).await?;
        Ok(meetings)
    }

    async fn put_meeting(&self, meeting: fcp_core::Meeting) -> Result<()> {
        let created_at = system_time_to_datetime(meeting.created_at);
        let updated_at = system_time_to_datetime(SystemTime::now());

        sqlx::query(
            "INSERT INTO conference_meetings
             (id, tenant_id, room_number, name, guest_pin, host_pin,
              require_guest_pin, max_participants, wait_for_moderator,
              enabled, auto_record, default_locked, permanent, owner_id,
              pin_prompt_sound, invalid_pin_sound, moh_sound,
              pin_prompt_tts, invalid_pin_tts, welcome_tts,
              play_entry_tone, play_exit_tone, join_sound, exit_sound,
              video_enabled, video_layout, video_max_participants,
              video_max_resolution, video_fps, video_exclude_self, video_codecs, video_passthrough,
                    video_content_share, video_content_layout,
              created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34, $35, $36)
             ON CONFLICT (id) DO UPDATE SET
              tenant_id = EXCLUDED.tenant_id,
              room_number = EXCLUDED.room_number,
              name = EXCLUDED.name,
              guest_pin = EXCLUDED.guest_pin,
              host_pin = EXCLUDED.host_pin,
              require_guest_pin = EXCLUDED.require_guest_pin,
              max_participants = EXCLUDED.max_participants,
              wait_for_moderator = EXCLUDED.wait_for_moderator,
              enabled = EXCLUDED.enabled,
              auto_record = EXCLUDED.auto_record,
              default_locked = EXCLUDED.default_locked,
              permanent = EXCLUDED.permanent,
              owner_id = EXCLUDED.owner_id,
              pin_prompt_sound = EXCLUDED.pin_prompt_sound,
              invalid_pin_sound = EXCLUDED.invalid_pin_sound,
              moh_sound = EXCLUDED.moh_sound,
              pin_prompt_tts = EXCLUDED.pin_prompt_tts,
              invalid_pin_tts = EXCLUDED.invalid_pin_tts,
              welcome_tts = EXCLUDED.welcome_tts,
              play_entry_tone = EXCLUDED.play_entry_tone,
              play_exit_tone = EXCLUDED.play_exit_tone,
              join_sound = EXCLUDED.join_sound,
              exit_sound = EXCLUDED.exit_sound,
              video_enabled = EXCLUDED.video_enabled,
              video_layout = EXCLUDED.video_layout,
              video_max_participants = EXCLUDED.video_max_participants,
              video_max_resolution = EXCLUDED.video_max_resolution,
              video_fps = EXCLUDED.video_fps,
              video_exclude_self = EXCLUDED.video_exclude_self,
              video_codecs = EXCLUDED.video_codecs,
              video_passthrough = EXCLUDED.video_passthrough,
              video_content_share = EXCLUDED.video_content_share,
              video_content_layout = EXCLUDED.video_content_layout,
              updated_at = EXCLUDED.updated_at",
        )
        .bind(meeting.id.to_string())
        .bind(meeting.tenant_id.to_string())
        .bind(&meeting.room_number)
        .bind(&meeting.name)
        .bind(self.seal_opt(meeting.guest_pin.as_deref())?)
        .bind(self.seal_opt(meeting.host_pin.as_deref())?)
        .bind(meeting.require_guest_pin)
        .bind(meeting.max_participants as i32)
        .bind(meeting.wait_for_moderator)
        .bind(meeting.enabled)
        .bind(meeting.auto_record)
        .bind(meeting.default_locked)
        .bind(meeting.permanent)
        .bind(meeting.owner_id.as_deref())
        .bind(meeting.pin_prompt_sound.as_deref())
        .bind(meeting.invalid_pin_sound.as_deref())
        .bind(meeting.moh_sound.as_deref())
        .bind(meeting.pin_prompt_tts.as_deref())
        .bind(meeting.invalid_pin_tts.as_deref())
        .bind(meeting.welcome_tts.as_deref())
        .bind(meeting.play_entry_tone)
        .bind(meeting.play_exit_tone)
        .bind(meeting.join_sound.as_deref())
        .bind(meeting.exit_sound.as_deref())
        .bind(meeting.video_enabled)
        .bind(&meeting.video_layout)
        .bind(meeting.video_max_participants as i32)
        .bind(&meeting.video_max_resolution)
        .bind(meeting.video_fps as i32)
        .bind(meeting.video_exclude_self)
        .bind(video_codecs_to_column(&meeting.video_codecs))
        .bind(&meeting.video_passthrough)
        .bind(&meeting.video_content_share)
        .bind(&meeting.video_content_layout)
        .bind(created_at)
        .bind(updated_at)
        .execute(self.writes())
        .await?;

        debug!(id = %meeting.id, "Meeting stored");
        Ok(())
    }

    async fn delete_meeting(&self, id: &fcp_core::MeetingId) -> Result<()> {
        sqlx::query("DELETE FROM conference_meetings WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;

        debug!(id = %id, "Meeting deleted");
        Ok(())
    }
}

fn map_meeting_row_pg(row: sqlx::postgres::PgRow) -> Result<fcp_core::Meeting> {
    let id_str: String = row.get("id");
    let tenant_str: String = row.get("tenant_id");
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");

    Ok(fcp_core::Meeting {
        id: fcp_core::MeetingId::from_str(&id_str)
            .map_err(|e| anyhow::anyhow!("Invalid MeetingId: {}", e))?,
        tenant_id: TenantId::from_str(&tenant_str)
            .map_err(|e| anyhow::anyhow!("Invalid TenantId: {}", e))?,
        room_number: row.get("room_number"),
        name: row.get("name"),
        guest_pin: row.get("guest_pin"),
        host_pin: row.get("host_pin"),
        require_guest_pin: row.get("require_guest_pin"),
        max_participants: row.get::<i32, _>("max_participants") as usize,
        wait_for_moderator: row.get("wait_for_moderator"),
        enabled: row.get("enabled"),
        auto_record: row.get("auto_record"),
        default_locked: row.get("default_locked"),
        permanent: row.get("permanent"),
        owner_id: row.get("owner_id"),
        pin_prompt_sound: row.get("pin_prompt_sound"),
        invalid_pin_sound: row.get("invalid_pin_sound"),
        moh_sound: row.get("moh_sound"),
        pin_prompt_tts: row.get("pin_prompt_tts"),
        invalid_pin_tts: row.get("invalid_pin_tts"),
        welcome_tts: row.get("welcome_tts"),
        play_entry_tone: row.get("play_entry_tone"),
        play_exit_tone: row.get("play_exit_tone"),
        join_sound: row.get("join_sound"),
        exit_sound: row.get("exit_sound"),
        video_enabled: row.get("video_enabled"),
        video_layout: row.get("video_layout"),
        video_max_participants: row.get::<i32, _>("video_max_participants").max(0) as u32,
        video_max_resolution: row.get("video_max_resolution"),
        video_fps: row.get::<i32, _>("video_fps").max(0) as u32,
        video_exclude_self: row.get("video_exclude_self"),
        video_codecs: video_codecs_from_column(&row.get::<String, _>("video_codecs")),
        video_passthrough: row
            .try_get::<String, _>("video_passthrough")
            .unwrap_or_else(|_| fcp_core::meeting::default_video_passthrough()),
        video_content_share: row
            .try_get::<String, _>("video_content_share")
            .unwrap_or_else(|_| fcp_core::meeting::default_video_content_share()),
        video_content_layout: row
            .try_get::<String, _>("video_content_layout")
            .unwrap_or_else(|_| fcp_core::meeting::default_video_content_layout()),
        created_at: datetime_to_system_time(created_at),
        updated_at: datetime_to_system_time(updated_at),
    })
}

/// Serialise the codec preference list for the `video_codecs` column:
/// SDP encoding names joined by ','.
fn video_codecs_to_column(codecs: &[fcp_core::VideoCodec]) -> String {
    codecs
        .iter()
        .map(|c| c.sdp_name())
        .collect::<Vec<_>>()
        .join(",")
}

/// Parse the `video_codecs` column, skipping names this build does not know.
fn video_codecs_from_column(value: &str) -> Vec<fcp_core::VideoCodec> {
    value
        .split(',')
        .filter_map(fcp_core::VideoCodec::from_sdp_name)
        .collect()
}

fn row_to_phone_number(row: sqlx::postgres::PgRow) -> Result<PhoneNumber> {
    let id = PhoneNumberId::from_str(&row.get::<String, _>("id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;
    let state_str: String = row.get("state");
    let state = NumberState::from_str(&state_str).unwrap_or(NumberState::Unassigned);

    let e911_address = row
        .get::<Option<serde_json::Value>, _>("e911_address_json")
        .and_then(|v| serde_json::from_value(v).ok());

    let allow_list: Vec<String> = row
        .get::<Option<serde_json::Value>, _>("allow_list")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let block_list: Vec<String> = row
        .get::<Option<serde_json::Value>, _>("block_list")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let tags: Vec<String> = row
        .get::<Option<serde_json::Value>, _>("tags")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let acquired_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("acquired_at"));
    let activated_at = row
        .get::<Option<DateTime<Utc>>, _>("activated_at")
        .map(datetime_to_system_time);
    let ported_at = row
        .get::<Option<DateTime<Utc>>, _>("ported_at")
        .map(datetime_to_system_time);
    let loa_date = row
        .get::<Option<DateTime<Utc>>, _>("loa_date")
        .map(datetime_to_system_time);
    let foc_date = row
        .get::<Option<DateTime<Utc>>, _>("foc_date")
        .map(datetime_to_system_time);
    let created_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at"));
    let updated_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at"));

    Ok(PhoneNumber {
        id,
        e164: row.get("e164"),
        country_code: row.get("country_code"),
        npa: row.get("npa"),
        nxx: row.get("nxx"),
        line: row.get("line"),
        state,
        tenant_id,
        e911_address,
        e911_callback: row.get("e911_callback"),
        cnam: row.get("cnam"),
        spam_score: row.get("spam_score"),
        carrier: row.get("carrier"),
        lrn: row.get("lrn"),
        ocn: row.get("ocn"),
        rate_center: row.get("rate_center"),
        lata: row.get("lata"),
        acquired_at,
        activated_at,
        ported_at,
        loa_date,
        foc_date,
        recording_enabled: row.get::<bool, _>("recording_enabled"),
        block_anonymous: row.get::<bool, _>("block_anonymous"),
        allow_list,
        block_list,
        tags,
        location_id: row
            .get::<Option<String>, _>("location_id")
            .and_then(|s| LocationId::from_str(&s).ok()),
        site_id: row
            .try_get::<Option<String>, _>("site_id")
            .ok()
            .flatten()
            .and_then(|s| fcp_core::SiteId::from_str(&s).ok()),
        created_at,
        updated_at,
    })
}

/// Convert SQLite row to LineBinding
fn row_to_line_binding(row: sqlx::postgres::PgRow) -> Result<LineBinding> {
    let id = LineBindingId::from_str(&row.get::<String, _>("id"))?;
    let phone_number_id = PhoneNumberId::from_str(&row.get::<String, _>("phone_number_id"))?;
    let owner_user_id = row
        .get::<Option<String>, _>("owner_user_id")
        .and_then(|s| UserId::from_str(&s).ok());
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;

    let binding_type_str: String = row.get("binding_type");
    let binding_type = match binding_type_str.as_str() {
        "user" => BindingType::User,
        "ivr" => BindingType::Ivr,
        "queue" => BindingType::Queue,
        "hunt_group" => BindingType::HuntGroup,
        "conference_bridge" => BindingType::ConferenceBridge,
        "voicemail" => BindingType::Voicemail,
        "parking" => BindingType::Parking,
        "paging" => BindingType::Paging,
        "fax" => BindingType::Fax,
        "emergency" => BindingType::Emergency,
        _ => BindingType::User,
    };

    let ring_strategy_str: String = row.get("ring_strategy");
    let ring_strategy = match ring_strategy_str.as_str() {
        "simultaneous" => RingStrategy::Simultaneous,
        "sequential" => RingStrategy::Sequential,
        "round_robin" => RingStrategy::RoundRobin,
        "longest_idle" => RingStrategy::LongestIdle,
        "primary" => RingStrategy::Primary,
        _ => RingStrategy::Simultaneous,
    };

    let created_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at"));
    let updated_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at"));

    Ok(LineBinding {
        id,
        phone_number_id,
        owner_user_id,
        binding_type,
        devices: Vec::new(), // Will be populated by caller
        ring_strategy,
        tenant_id,
        outbound_caller_id: row.get("outbound_caller_id"),
        outbound_cnam: row.get("outbound_cnam"),
        flow_id: row.try_get("flow_id").unwrap_or(None),
        schedule_id: row
            .try_get::<Option<String>, _>("schedule_id")
            .ok()
            .flatten()
            .and_then(|id| id.parse().ok()),
        closed_target: row
            .try_get::<Option<String>, _>("closed_target")
            .ok()
            .flatten()
            .and_then(|t| fcp_core::CallTarget::parse(&t).ok()),
        holiday_target: row
            .try_get::<Option<String>, _>("holiday_target")
            .ok()
            .flatten()
            .and_then(|t| fcp_core::CallTarget::parse(&t).ok()),
        shared: row.try_get("shared").unwrap_or(false),
        max_appearances: row
            .try_get::<i16, _>("max_appearances")
            .ok()
            .and_then(|n| u8::try_from(n).ok())
            .unwrap_or(fcp_core::line_binding::DEFAULT_MAX_APPEARANCES),
        barge_in: row.try_get("barge_in").unwrap_or(false),
        created_at,
        updated_at,
    })
}

/// Convert SQLite row to Extension
fn row_to_extension(row: sqlx::postgres::PgRow) -> Result<Extension> {
    let id = ExtensionId::from_str(&row.get::<String, _>("id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;
    let target_type: String = row.get("target_type");
    let target_id: String = row.get("target_id");

    let target = match target_type.as_str() {
        "user" => ExtensionTarget::User(UserId::from_str(&target_id)?),
        "phone_number" => ExtensionTarget::PhoneNumber(PhoneNumberId::from_str(&target_id)?),
        "device" => ExtensionTarget::Device(DeviceId::from_str(&target_id)?),
        "service" => ExtensionTarget::Service(
            fcp_core::ServiceId::from_str(&target_id)
                .unwrap_or_else(|_| fcp_core::ServiceId::new()),
        ),
        _ => {
            return Err(anyhow::anyhow!(
                "Unknown extension target type: {}",
                target_type
            ));
        }
    };

    let created_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at"));
    let updated_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at"));

    Ok(Extension {
        id,
        number: row.get("number"),
        target,
        tenant_id,
        description: row.get("description"),
        location_id: row
            .get::<Option<String>, _>("location_id")
            .and_then(|s| LocationId::from_str(&s).ok()),
        enabled: row.get::<bool, _>("enabled"),
        created_at,
        updated_at,
    })
}

/// Translate Postgres errors raised by device INSERT/UPDATE into typed
/// conflicts the API layer can map to HTTP 409.
///
/// The `ensure_mac_available` check in the API layer narrows the race window
/// but can't eliminate it; the unique index on the normalized MAC is the real
/// gate. Without this mapping the second writer would see HTTP 500 instead of
/// the meaningful 409.
///
/// Any SQLSTATE 23505 unique_violation on the devices table is mapped to a
/// Conflict — not just the normalized-MAC index — so future unique constraints
/// (e.g. a tenant-scoped name index) surface as 409 automatically instead of
/// silently regressing to 500 until this helper is updated.
fn map_device_write_error(err: sqlx::Error) -> anyhow::Error {
    if let Some(db_err) = err.as_database_error() {
        if db_err.code().as_deref() == Some("23505") {
            let constraint = db_err.constraint().unwrap_or("<unknown>");
            let client_msg = if constraint == "idx_devices_normalized_mac_unique" {
                "MAC address is already assigned to another device".to_string()
            } else {
                "Device violates a uniqueness constraint".to_string()
            };
            // Preserve the specific constraint name in the server log for ops
            // triage — the client response is intentionally generic.
            tracing::warn!(
                constraint = %constraint,
                message = %db_err.message(),
                "Device write rejected by unique constraint"
            );
            return anyhow::Error::new(fcp_core::Error::Storage(fcp_core::StorageError::Conflict(
                client_msg,
            )));
        }
    }
    anyhow::Error::from(err)
}

/// Convert a row to a Device, its secrets still sealed.
fn row_to_device(row: sqlx::postgres::PgRow) -> Result<Device> {
    let id = DeviceId::from_str(&row.get::<String, _>("id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;
    let owner_user_id = row
        .get::<Option<String>, _>("owner_user_id")
        .and_then(|s| UserId::from_str(&s).ok());

    let device_type_str: String = row.get("device_type");
    let device_type = match device_type_str.as_str() {
        "desk_phone" => DeviceType::DeskPhone,
        "softphone" => DeviceType::Softphone,
        "webrtc" => DeviceType::WebRtc,
        "conference_room" => DeviceType::ConferenceRoom,
        "ata" => DeviceType::Ata,
        "gateway" => DeviceType::Gateway,
        "mobile" => DeviceType::Mobile,
        _ => DeviceType::Other,
    };

    let registration_state_str: String = row.get("registration_state");
    let registration_state = match registration_state_str.as_str() {
        "unregistered" => RegistrationState::Unregistered,
        "registered" => RegistrationState::Registered,
        "failed" => RegistrationState::Failed,
        "expired" => RegistrationState::Expired,
        _ => RegistrationState::Unregistered,
    };

    let last_registration = row
        .get::<Option<DateTime<Utc>>, _>("last_registration")
        .map(datetime_to_system_time);

    let codec_support_json: serde_json::Value = row.get("codec_support");
    let codec_names: Vec<String> = serde_json::from_value(codec_support_json).unwrap_or_default();
    let codec_support: Vec<AudioCodec> = codec_names
        .into_iter()
        .filter_map(|name| match name.as_str() {
            "Pcmu" => Some(AudioCodec::Pcmu),
            "Pcma" => Some(AudioCodec::Pcma),
            "G722" => Some(AudioCodec::G722),
            "G729" => Some(AudioCodec::G729),
            "Opus" => Some(AudioCodec::Opus),
            "Ilbc" => Some(AudioCodec::Ilbc),
            _ => None,
        })
        .collect();

    let e911_override = row
        .get::<Option<serde_json::Value>, _>("e911_override_json")
        .and_then(|v| serde_json::from_value(v).ok());

    let created_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at"));
    let updated_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at"));

    Ok(Device {
        id,
        name: row.get("name"),
        device_type,
        mac_address: row.get("mac_address"),
        model: row.get("model"),
        firmware_version: row.get("firmware_version"),
        tenant_id,
        owner_user_id,
        sip_username: row.get("sip_username"),
        sip_ha1_hash: row.get("sip_ha1_hash"),
        sip_provisioning_password: row.get("sip_provisioning_password"),
        hoteling_host: row.get("hoteling_host"),
        sip_uri: row.get("sip_uri"),
        registration_state,
        last_registration,
        last_ip_address: row.get("last_ip_address"),
        user_agent: row.get("user_agent"),
        max_lines: row.get::<i32, _>("max_lines") as u8,
        supports_video: row.get::<bool, _>("supports_video"),
        supports_blf: row.get::<bool, _>("supports_blf"),
        codec_support,
        location: row.get("location"),
        location_id: row
            .get::<Option<String>, _>("location_id")
            .and_then(|s| LocationId::from_str(&s).ok()),
        e911_override,
        e911_trust_location: row.try_get("e911_trust_location").unwrap_or(None),
        provisioning_url: row.get("provisioning_url"),
        config_template_id: row.get("config_template_id"),
        provisioning_token_hash: row.get("provisioning_token_hash"),
        enabled: row.get::<bool, _>("enabled"),
        created_at,
        updated_at,
    })
}

fn row_to_config_template(
    row: sqlx::postgres::PgRow,
) -> Result<fcp_core::config_template::ConfigTemplate> {
    use sqlx::Row;
    let id = fcp_core::ConfigTemplateId::from_str(&row.get::<String, _>("id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;
    let vendor_str: String = row.get("vendor");
    let vendor: fcp_core::config_template::PhoneVendor = vendor_str
        .parse()
        .unwrap_or(fcp_core::config_template::PhoneVendor::Generic);
    let created_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at"));
    let updated_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at"));

    Ok(fcp_core::config_template::ConfigTemplate {
        id,
        tenant_id,
        name: row.get("name"),
        vendor,
        model_pattern: row.get("model_pattern"),
        template_body: row.get("template_body"),
        description: row.get("description"),
        created_at,
        updated_at,
    })
}

fn row_to_site_location(row: sqlx::postgres::PgRow) -> Result<SiteLocation> {
    use sqlx::Row;
    let id = LocationId::from_str(&row.get::<String, _>("id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;
    let created_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("created_at"));
    let updated_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("updated_at"));

    Ok(SiteLocation {
        id,
        tenant_id,
        name: row.get("name"),
        description: row.get("description"),
        address_line1: row.get("address_line1"),
        address_line2: row.get("address_line2"),
        city: row.get("city"),
        state: row.get("state"),
        postal_code: row.get("postal_code"),
        country: row.get("country"),
        floor: row.get("floor"),
        building: row.get("building"),
        latitude: row.get("latitude"),
        longitude: row.get("longitude"),
        phone_number: row.get("phone_number"),
        timezone: row.get("timezone"),
        emergency_contact_name: row.get("emergency_contact_name"),
        emergency_contact_phone: row.get("emergency_contact_phone"),
        e911_callback_number: row.try_get("e911_callback_number").unwrap_or(None),
        civic: row
            .try_get::<Option<serde_json::Value>, _>("civic")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        e911_validated_at: row
            .try_get::<Option<DateTime<Utc>>, _>("e911_validated_at")
            .ok()
            .flatten()
            .map(datetime_to_system_time),
        e911_validated_by: row.try_get("e911_validated_by").unwrap_or(None),
        e911_validation_reference: row.try_get("e911_validation_reference").unwrap_or(None),
        created_at,
        updated_at,
    })
}

// ============================================================================
// SiteLocationStore Implementation
// ============================================================================
// Trunk Store
// ============================================================================

fn row_to_trunk(row: sqlx::postgres::PgRow) -> Result<Trunk> {
    use fcp_core::trunk::{
        NumberTransform, SrtpMode, TrunkCodecConfig, TrunkCredentials, TrunkDtmfConfig,
        TrunkDtmfMode, TrunkFailover, TrunkSrtpConfig, TrunkStirShakenConfig, TrunkTransport,
    };

    let id_str: String = row.try_get("id")?;
    let tenant_id_str: String = row.try_get("tenant_id")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at")?;
    let direction = row
        .try_get::<String, _>("direction")?
        .parse::<TrunkDirection>()
        .unwrap_or(TrunkDirection::Upstream);

    // The columns that are one structured setting on the trunk.
    let auth_username: Option<String> = row.try_get("auth_username")?;
    let credentials = auth_username.map(|username| TrunkCredentials {
        username,
        password: row
            .try_get("auth_password")
            .unwrap_or(None)
            .unwrap_or_default(),
        realm: row.try_get("auth_realm").unwrap_or(None),
    });
    let strip_leading: i32 = row.try_get("strip_digits")?;
    let strip_prefixes: Vec<String> = row.try_get("strip_prefixes").unwrap_or_default();
    let prepend: Option<String> = row.try_get("prepend_digits")?;
    let prepend = prepend.unwrap_or_default();
    let egress_number_transform = (strip_leading > 0
        || !strip_prefixes.is_empty()
        || !prepend.is_empty())
    .then(|| NumberTransform {
        strip_prefixes,
        strip_leading: strip_leading.max(0) as usize,
        prepend,
    });
    let header_rules: Option<String> = row.try_get("header_rules")?;
    let emergency_location: Vec<String> = row.try_get("emergency_location").unwrap_or_default();

    Ok(Trunk {
        id: TrunkId::from_string(id_str),
        // A stored trunk always belongs to a tenant; only the platform
        // trunks a node reads from its own TOML have none.
        tenant_id: Some(TenantId::from_str(&tenant_id_str).unwrap_or_else(|_| TenantId::nil())),
        name: row.try_get("name")?,
        enabled: row.try_get("enabled")?,
        direction,
        priority: row.try_get::<i32, _>("priority")?.max(0) as u32,
        weight: row.try_get::<i32, _>("weight")?.max(0) as u32,
        hosts: row.try_get("hosts")?,
        inbound_sources: row.get("inbound_sources"),
        site_id: row
            .try_get::<Option<String>, _>("site_id")
            .ok()
            .flatten()
            .and_then(|s| fcp_core::SiteId::from_str(&s).ok()),
        link: row
            .try_get::<Option<String>, _>("link_end")
            .ok()
            .flatten()
            .and_then(|end| match end.as_str() {
                "cloud" => Some(fcp_core::LinkEnd::Cloud),
                "site" => Some(fcp_core::LinkEnd::Site),
                _ => None,
            }),
        transport: TrunkTransport {
            protocol: parse_transport(&row.try_get::<String, _>("transport_protocol")?),
            // Unset means the transport's default (migration 077).
            port: row
                .try_get::<Option<i32>, _>("transport_port")?
                .and_then(|p| u16::try_from(p).ok())
                .filter(|p| *p > 0),
            verify_tls: row.try_get("verify_tls")?,
        },
        credentials,
        register: row.try_get("register")?,
        register_expiry: u32::try_from(row.try_get::<i32, _>("register_expiry")?)
            .ok()
            .filter(|e| *e > 0),
        failover: TrunkFailover {
            max_failures: row.try_get::<i32, _>("max_failures")?.max(1) as u32,
            recovery_interval_secs: row.try_get::<i32, _>("recovery_interval_secs")?.max(1) as u64,
            health_check_enabled: row.try_get("health_check_enabled")?,
            health_check_interval_secs: row.try_get::<i32, _>("health_check_interval_secs")?.max(1)
                as u64,
        },
        max_concurrent_calls: row.try_get::<i32, _>("max_concurrent_calls")?.max(0) as u32,
        cps_limit: row.try_get::<i32, _>("cps_limit")?.max(0) as u32,
        header_rules: header_rules
            .as_deref()
            .map(|rules| {
                rules
                    .split(',')
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        emergency_capable: row.try_get("emergency_capable").unwrap_or(false),
        emergency_location: if emergency_location.is_empty() {
            vec![fcp_core::e911::LocationDelivery::Elin]
        } else {
            emergency_location
                .iter()
                .filter_map(|d| match d.as_str() {
                    "pidf_lo" => Some(fcp_core::e911::LocationDelivery::PidfLo),
                    "elin" => Some(fcp_core::e911::LocationDelivery::Elin),
                    _ => None,
                })
                .collect()
        },
        srtp: TrunkSrtpConfig {
            mode: SrtpMode::parse(&row.try_get::<String, _>("srtp_mode")?),
            allowed_suites: row.try_get("srtp_suites").unwrap_or_default(),
        },
        dtmf: TrunkDtmfConfig {
            mode: TrunkDtmfMode::parse(&row.try_get::<String, _>("dtmf_mode")?),
        },
        codec: TrunkCodecConfig {
            allowed_codecs: row.try_get("codec_prefs")?,
            allow_transcoding: row.try_get("allow_transcoding").unwrap_or(true),
        },
        stir_shaken: TrunkStirShakenConfig {
            verify_mode: parse_stir_verify(row.try_get("stir_verify_mode").unwrap_or(None)),
            sign_mode: parse_stir_sign(row.try_get("stir_sign_mode").unwrap_or(None)),
            attestation: row
                .try_get::<Option<String>, _>("stir_attestation")
                .ok()
                .flatten()
                .and_then(|a| fcp_core::trunk::AttestationLevel::from_passport_str(&a))
                .unwrap_or_default(),
            max_iat_age_secs: row
                .try_get::<Option<i64>, _>("stir_max_iat_age_secs")
                .ok()
                .flatten()
                .map(|v| v.max(1) as u64)
                .unwrap_or(60),
        },
        egress_number_transform,
        caller_id_override: row.try_get("caller_id_override")?,
        from_domain: row.try_get("from_domain")?,
        session_timer_secs: row.try_get::<i32, _>("session_timer_secs")?.max(0) as u32,
        t38_enabled: row.try_get("t38_enabled")?,
        outbound_proxy: row.try_get("outbound_proxy")?,
        trunk_group: row.try_get("trunk_group")?,
        description: row.try_get("description")?,
        created_at: datetime_to_system_time(created_at),
        updated_at: datetime_to_system_time(updated_at),
    })
}

/// The transport a stored trunk names.
fn parse_transport(value: &str) -> fcp_core::transport::TransportType {
    use fcp_core::transport::TransportType;
    match value.to_ascii_lowercase().as_str() {
        "tcp" => TransportType::Tcp,
        "tls" => TransportType::Tls,
        "ws" => TransportType::Ws,
        "wss" => TransportType::Wss,
        _ => TransportType::Udp,
    }
}

fn parse_stir_verify(value: Option<String>) -> fcp_core::trunk::StirVerifyMode {
    use fcp_core::trunk::StirVerifyMode;
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "enforce" => StirVerifyMode::Enforce,
        "log_only" | "logonly" => StirVerifyMode::LogOnly,
        _ => StirVerifyMode::Disabled,
    }
}

fn parse_stir_sign(value: Option<String>) -> fcp_core::trunk::StirSignMode {
    use fcp_core::trunk::StirSignMode;
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "enabled" => StirSignMode::Enabled,
        _ => StirSignMode::Disabled,
    }
}

/// The columns a least-cost route is read from, named so a column added to
/// the table is a column added here (`docs/TENANT_TRUNKS.md` §12, phase 6c).
const LCR_ROUTE_COLS: &str = "id, tenant_id, prefix, trunk_id, cost_per_minute, priority, \
     enabled, description, created_at, updated_at";

fn row_to_lcr_route(row: sqlx::postgres::PgRow) -> Result<fcp_core::LcrRoute> {
    let tenant: String = row.get("tenant_id");
    Ok(fcp_core::LcrRoute {
        id: fcp_core::LcrRouteId::from_string(row.get("id")),
        tenant_id: TenantId::from_str(&tenant)?,
        prefix: row.get("prefix"),
        trunk_id: TrunkId::from_string(row.get("trunk_id")),
        cost_per_minute: row.get::<fcp_core::billing::Decimal, _>("cost_per_minute"),
        priority: row.get::<i32, _>("priority").max(0) as u32,
        enabled: row.get("enabled"),
        description: row.get("description"),
        created_at: datetime_to_system_time(row.get("created_at")),
        updated_at: datetime_to_system_time(row.get("updated_at")),
    })
}

#[async_trait]
impl TrunkStore for PostgresStorage {
    async fn get_trunk(&self, id: &TrunkId) -> Result<Option<Trunk>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, name, enabled, direction, priority, weight, hosts,
                    transport_protocol, transport_port, verify_tls,
                    auth_username, auth_password, auth_realm,
                    register, register_expiry, max_concurrent_calls,
                    max_failures, recovery_interval_secs,
                    health_check_enabled, health_check_interval_secs,
                    codec_prefs, caller_id_override, from_domain,
                    strip_digits, prepend_digits, cps_limit, description,
                    srtp_mode, dtmf_mode, session_timer_secs, t38_enabled,
                    outbound_proxy, trunk_group, header_rules,
                    created_at, updated_at, emergency_capable,
                    strip_prefixes, srtp_suites, allow_transcoding,
                    stir_verify_mode, stir_sign_mode, stir_attestation,
                    stir_max_iat_age_secs, emergency_location, inbound_sources, site_id, link_end
             FROM trunks WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(self.open_trunk(row_to_trunk(row)?).await?)),
            None => Ok(None),
        }
    }

    async fn list_trunks(&self, tenant_id: &TenantId) -> Result<Vec<Trunk>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, name, enabled, direction, priority, weight, hosts,
                    transport_protocol, transport_port, verify_tls,
                    auth_username, auth_password, auth_realm,
                    register, register_expiry, max_concurrent_calls,
                    max_failures, recovery_interval_secs,
                    health_check_enabled, health_check_interval_secs,
                    codec_prefs, caller_id_override, from_domain,
                    strip_digits, prepend_digits, cps_limit, description,
                    srtp_mode, dtmf_mode, session_timer_secs, t38_enabled,
                    outbound_proxy, trunk_group, header_rules,
                    created_at, updated_at, emergency_capable,
                    strip_prefixes, srtp_suites, allow_transcoding,
                    stir_verify_mode, stir_sign_mode, stir_attestation,
                    stir_max_iat_age_secs, emergency_location, inbound_sources, site_id, link_end
             FROM trunks WHERE tenant_id = $1
             ORDER BY priority, name",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let trunks = rows
            .into_iter()
            .filter_map(|row| row_to_trunk(row).ok())
            .collect();
        let trunks = self.open_trunks(trunks).await?;
        Ok(trunks)
    }

    /// Every tenant's trunks, for a node building what it routes by
    /// (`docs/TENANT_TRUNKS.md` §4).
    async fn list_all_trunks(&self) -> Result<Vec<Trunk>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, name, enabled, direction, priority, weight, hosts,
                    transport_protocol, transport_port, verify_tls,
                    auth_username, auth_password, auth_realm,
                    register, register_expiry, max_concurrent_calls,
                    max_failures, recovery_interval_secs,
                    health_check_enabled, health_check_interval_secs,
                    codec_prefs, caller_id_override, from_domain,
                    strip_digits, prepend_digits, cps_limit, description,
                    srtp_mode, dtmf_mode, session_timer_secs, t38_enabled,
                    outbound_proxy, trunk_group, header_rules,
                    created_at, updated_at, emergency_capable,
                    strip_prefixes, srtp_suites, allow_transcoding,
                    stir_verify_mode, stir_sign_mode, stir_attestation,
                    stir_max_iat_age_secs, emergency_location, inbound_sources, site_id, link_end
             FROM trunks
             ORDER BY tenant_id, priority, name",
        )
        .fetch_all(self.reads())
        .await?;

        let trunks = rows
            .into_iter()
            .filter_map(|row| row_to_trunk(row).ok())
            .collect();
        self.open_trunks(trunks).await
    }

    async fn put_trunk(&self, trunk: Trunk) -> Result<()> {
        sqlx::query(
            "INSERT INTO trunks (id, tenant_id, name, enabled, direction, priority, weight, hosts,
                    transport_protocol, transport_port, verify_tls,
                    auth_username, auth_password, auth_realm,
                    register, register_expiry, max_concurrent_calls,
                    max_failures, recovery_interval_secs,
                    health_check_enabled, health_check_interval_secs,
                    codec_prefs, caller_id_override, from_domain,
                    strip_digits, prepend_digits, cps_limit, description,
                    srtp_mode, dtmf_mode, session_timer_secs, t38_enabled,
                    outbound_proxy, trunk_group, header_rules,
                    created_at, updated_at, emergency_capable,
                    strip_prefixes, srtp_suites, allow_transcoding,
                    stir_verify_mode, stir_sign_mode, stir_attestation,
                    stir_max_iat_age_secs, emergency_location, inbound_sources, site_id, link_end)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34, $35, $36, $37, $38, $39, $40, $41, $42, $43, $44, $45, $46, $47, $48, $49)
             ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                enabled = EXCLUDED.enabled,
                direction = EXCLUDED.direction,
                priority = EXCLUDED.priority,
                weight = EXCLUDED.weight,
                hosts = EXCLUDED.hosts,
                transport_protocol = EXCLUDED.transport_protocol,
                transport_port = EXCLUDED.transport_port,
                verify_tls = EXCLUDED.verify_tls,
                auth_username = EXCLUDED.auth_username,
                auth_password = EXCLUDED.auth_password,
                auth_realm = EXCLUDED.auth_realm,
                register = EXCLUDED.register,
                register_expiry = EXCLUDED.register_expiry,
                max_concurrent_calls = EXCLUDED.max_concurrent_calls,
                max_failures = EXCLUDED.max_failures,
                recovery_interval_secs = EXCLUDED.recovery_interval_secs,
                health_check_enabled = EXCLUDED.health_check_enabled,
                health_check_interval_secs = EXCLUDED.health_check_interval_secs,
                codec_prefs = EXCLUDED.codec_prefs,
                caller_id_override = EXCLUDED.caller_id_override,
                from_domain = EXCLUDED.from_domain,
                strip_digits = EXCLUDED.strip_digits,
                prepend_digits = EXCLUDED.prepend_digits,
                cps_limit = EXCLUDED.cps_limit,
                description = EXCLUDED.description,
                srtp_mode = EXCLUDED.srtp_mode,
                dtmf_mode = EXCLUDED.dtmf_mode,
                session_timer_secs = EXCLUDED.session_timer_secs,
                t38_enabled = EXCLUDED.t38_enabled,
                outbound_proxy = EXCLUDED.outbound_proxy,
                trunk_group = EXCLUDED.trunk_group,
                header_rules = EXCLUDED.header_rules,
                emergency_capable = EXCLUDED.emergency_capable,
                strip_prefixes = EXCLUDED.strip_prefixes,
                srtp_suites = EXCLUDED.srtp_suites,
                allow_transcoding = EXCLUDED.allow_transcoding,
                stir_verify_mode = EXCLUDED.stir_verify_mode,
                stir_sign_mode = EXCLUDED.stir_sign_mode,
                stir_attestation = EXCLUDED.stir_attestation,
                stir_max_iat_age_secs = EXCLUDED.stir_max_iat_age_secs,
                emergency_location = EXCLUDED.emergency_location,
                inbound_sources = EXCLUDED.inbound_sources,
                site_id = EXCLUDED.site_id,
                link_end = EXCLUDED.link_end,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(trunk.id.to_string())
        .bind(
            trunk
                .tenant_id
                .map(|t| t.to_string())
                .context("a platform trunk has no tenant and cannot be stored")?,
        )
        .bind(&trunk.name)
        .bind(trunk.enabled)
        .bind(trunk.direction.to_string())
        .bind(trunk.priority as i32)
        .bind(trunk.weight as i32)
        .bind(&trunk.hosts)
        .bind(trunk.transport.protocol.to_string().to_lowercase())
        .bind(trunk.transport.port.map(i32::from))
        .bind(trunk.transport.verify_tls)
        .bind(trunk.credentials.as_ref().map(|c| c.username.clone()))
        .bind(self.seal_opt(trunk.credentials.as_ref().map(|c| c.password.as_str()))?)
        .bind(trunk.credentials.as_ref().and_then(|c| c.realm.clone()))
        .bind(trunk.register)
        .bind(trunk.register_expiry.unwrap_or(3600) as i32)
        .bind(trunk.max_concurrent_calls as i32)
        .bind(trunk.failover.max_failures as i32)
        .bind(trunk.failover.recovery_interval_secs as i32)
        .bind(trunk.failover.health_check_enabled)
        .bind(trunk.failover.health_check_interval_secs as i32)
        .bind(&trunk.codec.allowed_codecs)
        .bind(&trunk.caller_id_override)
        .bind(&trunk.from_domain)
        .bind(
            trunk
                .egress_number_transform
                .as_ref()
                .map(|t| t.strip_leading as i32)
                .unwrap_or(0),
        )
        .bind(
            trunk
                .egress_number_transform
                .as_ref()
                .map(|t| t.prepend.clone())
                .filter(|p| !p.is_empty()),
        )
        .bind(trunk.cps_limit as i32)
        .bind(&trunk.description)
        .bind(trunk.srtp.mode.as_str())
        .bind(trunk.dtmf.mode.as_str())
        .bind(trunk.session_timer_secs as i32)
        .bind(trunk.t38_enabled)
        .bind(&trunk.outbound_proxy)
        .bind(&trunk.trunk_group)
        .bind((!trunk.header_rules.is_empty()).then(|| trunk.header_rules.join(", ")))
        .bind(system_time_to_datetime(trunk.created_at))
        .bind(system_time_to_datetime(trunk.updated_at))
        .bind(trunk.emergency_capable)
        .bind(
            trunk
                .egress_number_transform
                .as_ref()
                .map(|t| t.strip_prefixes.clone())
                .unwrap_or_default(),
        )
        .bind(&trunk.srtp.allowed_suites)
        .bind(trunk.codec.allow_transcoding)
        .bind(format!("{:?}", trunk.stir_shaken.verify_mode).to_lowercase())
        .bind(format!("{:?}", trunk.stir_shaken.sign_mode).to_lowercase())
        .bind(trunk.stir_shaken.attestation.as_str())
        .bind(trunk.stir_shaken.max_iat_age_secs as i64)
        .bind(
            trunk
                .emergency_location
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>(),
        )
        .bind(&trunk.inbound_sources)
        .bind(trunk.site_id.map(|id| id.to_string()))
        .bind(trunk.link.map(|end| match end {
            fcp_core::LinkEnd::Cloud => "cloud",
            fcp_core::LinkEnd::Site => "site",
        }))
        .execute(self.writes())
        .await?;

        Ok(())
    }

    async fn delete_trunk(&self, id: &TrunkId) -> Result<()> {
        sqlx::query("DELETE FROM trunks WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn get_lcr_route(&self, id: &fcp_core::LcrRouteId) -> Result<Option<fcp_core::LcrRoute>> {
        let row = sqlx::query(&format!(
            "SELECT {LCR_ROUTE_COLS} FROM lcr_routes WHERE id = $1"
        ))
        .bind(id.as_str())
        .fetch_optional(self.reads())
        .await?;
        row.map(row_to_lcr_route).transpose()
    }

    async fn list_lcr_routes(&self, tenant_id: &TenantId) -> Result<Vec<fcp_core::LcrRoute>> {
        let rows = sqlx::query(&format!(
            "SELECT {LCR_ROUTE_COLS} FROM lcr_routes WHERE tenant_id = $1
             ORDER BY prefix, cost_per_minute, priority, id"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.into_iter().map(row_to_lcr_route).collect()
    }

    async fn list_all_lcr_routes(&self) -> Result<Vec<fcp_core::LcrRoute>> {
        let rows = sqlx::query(&format!(
            "SELECT {LCR_ROUTE_COLS} FROM lcr_routes
             ORDER BY tenant_id, prefix, cost_per_minute, priority, id"
        ))
        .fetch_all(self.reads())
        .await?;
        rows.into_iter().map(row_to_lcr_route).collect()
    }

    async fn put_lcr_route(&self, route: fcp_core::LcrRoute) -> Result<()> {
        sqlx::query(
            "INSERT INTO lcr_routes
                (id, tenant_id, prefix, trunk_id, cost_per_minute, priority, enabled,
                 description, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (id) DO UPDATE SET
                prefix = EXCLUDED.prefix,
                trunk_id = EXCLUDED.trunk_id,
                cost_per_minute = EXCLUDED.cost_per_minute,
                priority = EXCLUDED.priority,
                enabled = EXCLUDED.enabled,
                description = EXCLUDED.description,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(route.id.as_str())
        .bind(route.tenant_id.to_string())
        .bind(&route.prefix)
        .bind(route.trunk_id.as_str())
        .bind(route.cost_per_minute)
        .bind(route.priority as i32)
        .bind(route.enabled)
        .bind(&route.description)
        .bind(system_time_to_datetime(route.created_at))
        .bind(system_time_to_datetime(route.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_lcr_route(&self, id: &fcp_core::LcrRouteId) -> Result<()> {
        sqlx::query("DELETE FROM lcr_routes WHERE id = $1")
            .bind(id.as_str())
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn get_trunk_stats(&self, trunk_name: &str) -> Result<fcp_core::TrunkStats> {
        let cutoff = system_time_to_datetime(
            std::time::SystemTime::now() - std::time::Duration::from_secs(86400),
        );

        // Aggregate stats from CDRs where gateway_id matches trunk name
        let row = sqlx::query(
            "SELECT
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE disposition = 'answered') as answered,
                COALESCE(AVG(call_duration_ms) FILTER (WHERE disposition = 'answered'), 0) as avg_duration_ms,
                COALESCE(AVG(setup_duration_ms) FILTER (WHERE disposition = 'answered'), 0) as avg_setup_ms
             FROM cdrs
             WHERE gateway_id = $1 AND start_time >= $2",
        )
        .bind(trunk_name)
        .bind(cutoff)
        .fetch_one(self.reads())
        .await?;

        let total: i64 = row.try_get("total").unwrap_or(0);
        let answered: i64 = row.try_get("answered").unwrap_or(0);
        let avg_duration_ms: f64 = row.try_get::<f64, _>("avg_duration_ms").unwrap_or(0.0);
        let avg_setup_ms: f64 = row.try_get::<f64, _>("avg_setup_ms").unwrap_or(0.0);

        let asr = if total > 0 {
            (answered as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        // Hourly volume: 24 buckets over last 24h
        let rows = sqlx::query(
            "SELECT
                date_trunc('hour', start_time) as bucket,
                COUNT(*) as cnt
             FROM cdrs
             WHERE gateway_id = $1 AND start_time >= $2
             GROUP BY bucket
             ORDER BY bucket",
        )
        .bind(trunk_name)
        .bind(cutoff)
        .fetch_all(self.reads())
        .await?;

        // Build 24-slot vector
        let mut hourly = vec![0u64; 24];
        for row in &rows {
            let bucket: DateTime<Utc> = row.try_get("bucket")?;
            let cnt: i64 = row.try_get("cnt").unwrap_or(0);
            let hours_ago = (chrono::Utc::now() - bucket).num_hours();
            if (0..24).contains(&hours_ago) {
                hourly[23 - hours_ago as usize] = cnt as u64;
            }
        }

        Ok(fcp_core::TrunkStats {
            total_calls_24h: total as u64,
            answered_calls_24h: answered as u64,
            asr,
            acd_secs: avg_duration_ms / 1000.0,
            pdd_ms: avg_setup_ms,
            hourly_volume: hourly,
        })
    }
}

// ============================================================================
// Routing Rule Store
// ============================================================================

fn row_to_routing_rule(row: sqlx::postgres::PgRow) -> Result<RoutingRule> {
    let id_str: String = row.try_get("id")?;
    let tenant_id_str: String = row.try_get("tenant_id")?;
    let action_str: String = row.try_get("action")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

    Ok(RoutingRule {
        id: RoutingRuleId::from_string(id_str),
        tenant_id: TenantId::from_str(&tenant_id_str).unwrap_or_else(|_| TenantId::nil()),
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        priority: row.try_get("priority")?,
        enabled: row.try_get("enabled")?,
        destination_pattern: row.try_get("destination_pattern")?,
        source_pattern: row.try_get("source_pattern")?,
        source_ip_ranges: row
            .try_get::<Vec<String>, _>("source_ip_ranges")
            .unwrap_or_default(),
        time_window_days: row
            .try_get::<Vec<String>, _>("time_window_days")
            .unwrap_or_default(),
        time_window_start: row.try_get("time_window_start")?,
        time_window_end: row.try_get("time_window_end")?,
        time_window_timezone: row
            .try_get::<String, _>("time_window_timezone")
            .unwrap_or_else(|_| "UTC".to_string()),
        // An action that does not read is an error, not an `allow`: a
        // rule decides calls.
        action: action_str
            .parse::<RoutingAction>()
            .map_err(|e| anyhow::anyhow!(e))?,
        target_trunk_id: row.try_get("target_trunk_id")?,
        target_trunk_group: row.try_get("target_trunk_group")?,
        reason: row.try_get("reason")?,
        created_at: datetime_to_system_time(created_at),
        updated_at: datetime_to_system_time(updated_at),
    })
}

/// Rows that read as rules. Since phase 6b a rule decides calls, so a row
/// that does not read is said out loud rather than dropped quietly.
fn rows_to_routing_rules(rows: Vec<sqlx::postgres::PgRow>) -> Vec<RoutingRule> {
    rows.into_iter()
        .filter_map(|row| {
            let id: Option<String> = row.try_get("id").ok();
            match row_to_routing_rule(row) {
                Ok(rule) => Some(rule),
                Err(e) => {
                    tracing::error!(rule_id = ?id, error = %e, "A stored routing rule does not read; it is skipped");
                    None
                }
            }
        })
        .collect()
}

#[async_trait]
impl RoutingRuleStore for PostgresStorage {
    async fn get_routing_rule(&self, id: &RoutingRuleId) -> Result<Option<RoutingRule>> {
        let row = sqlx::query("SELECT * FROM routing_rules WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(self.reads())
            .await?;
        match row {
            Some(row) => Ok(Some(row_to_routing_rule(row)?)),
            None => Ok(None),
        }
    }

    async fn list_routing_rules(&self, tenant_id: &TenantId) -> Result<Vec<RoutingRule>> {
        let rows =
            sqlx::query("SELECT * FROM routing_rules WHERE tenant_id = $1 ORDER BY priority, name")
                .bind(tenant_id.to_string())
                .fetch_all(self.reads())
                .await?;
        Ok(rows_to_routing_rules(rows))
    }

    async fn list_all_routing_rules(&self) -> Result<Vec<RoutingRule>> {
        let rows = sqlx::query("SELECT * FROM routing_rules ORDER BY tenant_id, priority, name")
            .fetch_all(self.reads())
            .await?;
        Ok(rows_to_routing_rules(rows))
    }

    async fn put_routing_rule(&self, rule: RoutingRule) -> Result<()> {
        sqlx::query(
            "INSERT INTO routing_rules (id, tenant_id, name, description, priority, enabled,
                    destination_pattern, source_pattern, source_ip_ranges,
                    time_window_days, time_window_start, time_window_end, time_window_timezone,
                    action, target_trunk_id, target_trunk_group, reason,
                    created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
             ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name, description = EXCLUDED.description,
                priority = EXCLUDED.priority, enabled = EXCLUDED.enabled,
                destination_pattern = EXCLUDED.destination_pattern,
                source_pattern = EXCLUDED.source_pattern,
                source_ip_ranges = EXCLUDED.source_ip_ranges,
                time_window_days = EXCLUDED.time_window_days,
                time_window_start = EXCLUDED.time_window_start,
                time_window_end = EXCLUDED.time_window_end,
                time_window_timezone = EXCLUDED.time_window_timezone,
                action = EXCLUDED.action,
                target_trunk_id = EXCLUDED.target_trunk_id,
                target_trunk_group = EXCLUDED.target_trunk_group,
                reason = EXCLUDED.reason,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(rule.id.to_string())
        .bind(rule.tenant_id.to_string())
        .bind(&rule.name)
        .bind(&rule.description)
        .bind(rule.priority)
        .bind(rule.enabled)
        .bind(&rule.destination_pattern)
        .bind(&rule.source_pattern)
        .bind(&rule.source_ip_ranges)
        .bind(&rule.time_window_days)
        .bind(&rule.time_window_start)
        .bind(&rule.time_window_end)
        .bind(&rule.time_window_timezone)
        .bind(rule.action.to_string())
        .bind(&rule.target_trunk_id)
        .bind(&rule.target_trunk_group)
        .bind(&rule.reason)
        .bind(system_time_to_datetime(rule.created_at))
        .bind(system_time_to_datetime(rule.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_routing_rule(&self, id: &RoutingRuleId) -> Result<()> {
        sqlx::query("DELETE FROM routing_rules WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

// ============================================================================
// Queue Config Store
// ============================================================================

fn row_to_queue_config(row: sqlx::postgres::PgRow) -> Result<QueueConfigRecord> {
    let id_str: String = row.try_get("id")?;
    let tenant_id_str: String = row.try_get("tenant_id")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

    Ok(QueueConfigRecord {
        id: QueueConfigId::from_string(id_str),
        tenant_id: TenantId::from_str(&tenant_id_str).unwrap_or_else(|_| TenantId::nil()),
        name: row.try_get("name")?,
        extension: row.try_get("extension")?,
        strategy: row
            .try_get::<String, _>("strategy")
            .unwrap_or_else(|_| "round_robin".to_string()),
        max_wait_time_secs: row.try_get("max_wait_time_secs").unwrap_or(300),
        max_queue_depth: row.try_get("max_queue_depth").unwrap_or(50),
        overflow_destination: row.try_get("overflow_destination")?,
        moh_class: row.try_get("moh_class")?,
        announce_position: row.try_get("announce_position").unwrap_or(true),
        announce_interval_secs: row.try_get("announce_interval_secs").unwrap_or(30),
        retry_on_no_answer: row.try_get("retry_on_no_answer").unwrap_or(true),
        agents: row.try_get::<Vec<String>, _>("agents").unwrap_or_default(),
        priority: row.try_get("priority").unwrap_or(0),
        required_skills: row
            .try_get::<Vec<String>, _>("required_skills")
            .unwrap_or_default(),
        record_calls: row.try_get("record_calls").unwrap_or(false),
        callback_enabled: row.try_get("callback_enabled").unwrap_or(false),
        schedule_id: row
            .try_get::<Option<String>, _>("schedule_id")
            .ok()
            .flatten()
            .and_then(|id| id.parse().ok()),
        closed_target: row
            .try_get::<Option<String>, _>("closed_target")
            .ok()
            .flatten()
            .and_then(|t| fcp_core::CallTarget::parse(&t).ok()),
        timezone: row
            .try_get::<String, _>("timezone")
            .unwrap_or_else(|_| "UTC".to_string()),
        enabled: row.try_get("enabled").unwrap_or(true),
        created_at: datetime_to_system_time(created_at),
        updated_at: datetime_to_system_time(updated_at),
    })
}

#[async_trait]
impl QueueConfigStore for PostgresStorage {
    async fn get_queue_config(&self, id: &QueueConfigId) -> Result<Option<QueueConfigRecord>> {
        let row = sqlx::query("SELECT * FROM queue_configs WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(self.reads())
            .await?;
        match row {
            Some(row) => Ok(Some(row_to_queue_config(row)?)),
            None => Ok(None),
        }
    }

    async fn list_queue_configs(&self, tenant_id: &TenantId) -> Result<Vec<QueueConfigRecord>> {
        let rows =
            sqlx::query("SELECT * FROM queue_configs WHERE tenant_id = $1 ORDER BY priority, name")
                .bind(tenant_id.to_string())
                .fetch_all(self.reads())
                .await?;
        let configs = rows
            .into_iter()
            .filter_map(|r| row_to_queue_config(r).ok())
            .collect();
        Ok(configs)
    }

    async fn put_queue_config(&self, config: QueueConfigRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO queue_configs (id, tenant_id, name, extension, strategy,
                    max_wait_time_secs, max_queue_depth, overflow_destination,
                    moh_class, announce_position, announce_interval_secs,
                    retry_on_no_answer, agents, priority, required_skills,
                    record_calls, callback_enabled, timezone, enabled,
                    created_at, updated_at, schedule_id, closed_target)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)
             ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name, extension = EXCLUDED.extension,
                strategy = EXCLUDED.strategy,
                max_wait_time_secs = EXCLUDED.max_wait_time_secs,
                max_queue_depth = EXCLUDED.max_queue_depth,
                overflow_destination = EXCLUDED.overflow_destination,
                moh_class = EXCLUDED.moh_class,
                announce_position = EXCLUDED.announce_position,
                announce_interval_secs = EXCLUDED.announce_interval_secs,
                retry_on_no_answer = EXCLUDED.retry_on_no_answer,
                agents = EXCLUDED.agents,
                priority = EXCLUDED.priority,
                required_skills = EXCLUDED.required_skills,
                record_calls = EXCLUDED.record_calls,
                callback_enabled = EXCLUDED.callback_enabled,
                timezone = EXCLUDED.timezone,
                enabled = EXCLUDED.enabled,
                schedule_id = EXCLUDED.schedule_id,
                closed_target = EXCLUDED.closed_target,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(config.id.to_string())
        .bind(config.tenant_id.to_string())
        .bind(&config.name)
        .bind(&config.extension)
        .bind(&config.strategy)
        .bind(config.max_wait_time_secs)
        .bind(config.max_queue_depth)
        .bind(&config.overflow_destination)
        .bind(&config.moh_class)
        .bind(config.announce_position)
        .bind(config.announce_interval_secs)
        .bind(config.retry_on_no_answer)
        .bind(&config.agents)
        .bind(config.priority)
        .bind(&config.required_skills)
        .bind(config.record_calls)
        .bind(config.callback_enabled)
        .bind(&config.timezone)
        .bind(config.enabled)
        .bind(system_time_to_datetime(config.created_at))
        .bind(system_time_to_datetime(config.updated_at))
        .bind(config.schedule_id.map(|id| id.to_string()))
        .bind(config.closed_target.as_ref().map(|t| t.to_string()))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_queue_config(&self, id: &QueueConfigId) -> Result<()> {
        sqlx::query("DELETE FROM queue_configs WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

// ============================================================================

#[async_trait]
impl SiteLocationStore for PostgresStorage {
    async fn get_location(&self, id: &LocationId) -> Result<Option<SiteLocation>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, name, description, address_line1, address_line2,
                    city, state, postal_code, country, floor, building,
                    latitude, longitude, phone_number, timezone,
                    emergency_contact_name, emergency_contact_phone,
                    created_at, updated_at, e911_callback_number, civic,
                    e911_validated_at, e911_validated_by, e911_validation_reference
             FROM locations
             WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_site_location(row)?)),
            None => Ok(None),
        }
    }

    async fn list_locations(&self, tenant_id: &TenantId) -> Result<Vec<SiteLocation>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, name, description, address_line1, address_line2,
                    city, state, postal_code, country, floor, building,
                    latitude, longitude, phone_number, timezone,
                    emergency_contact_name, emergency_contact_phone,
                    created_at, updated_at, e911_callback_number, civic,
                    e911_validated_at, e911_validated_by, e911_validation_reference
             FROM locations
             WHERE tenant_id = $1
             ORDER BY name",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        let locations = rows
            .into_iter()
            .filter_map(|row| row_to_site_location(row).ok())
            .collect();

        Ok(locations)
    }

    async fn put_location(&self, location: SiteLocation) -> Result<()> {
        let id = location.id.to_string();
        let tenant_id = location.tenant_id.to_string();
        let created_at = system_time_to_datetime(location.created_at);
        let updated_at = system_time_to_datetime(location.updated_at);

        sqlx::query(
            "INSERT INTO locations (
                id, tenant_id, name, description, address_line1, address_line2,
                city, state, postal_code, country, floor, building,
                latitude, longitude, phone_number, timezone,
                emergency_contact_name, emergency_contact_phone,
                created_at, updated_at, e911_callback_number, civic,
                e911_validated_at, e911_validated_by, e911_validation_reference
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25)
             ON CONFLICT(id) DO UPDATE SET
                 name = EXCLUDED.name,
                 description = EXCLUDED.description,
                 address_line1 = EXCLUDED.address_line1,
                 address_line2 = EXCLUDED.address_line2,
                 city = EXCLUDED.city,
                 state = EXCLUDED.state,
                 postal_code = EXCLUDED.postal_code,
                 country = EXCLUDED.country,
                 floor = EXCLUDED.floor,
                 building = EXCLUDED.building,
                 latitude = EXCLUDED.latitude,
                 longitude = EXCLUDED.longitude,
                 phone_number = EXCLUDED.phone_number,
                 timezone = EXCLUDED.timezone,
                 emergency_contact_name = EXCLUDED.emergency_contact_name,
                 emergency_contact_phone = EXCLUDED.emergency_contact_phone,
                 e911_callback_number = EXCLUDED.e911_callback_number,
                 civic = EXCLUDED.civic,
                 e911_validated_at = EXCLUDED.e911_validated_at,
                 e911_validated_by = EXCLUDED.e911_validated_by,
                 e911_validation_reference = EXCLUDED.e911_validation_reference,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(&id)
        .bind(&tenant_id)
        .bind(&location.name)
        .bind(&location.description)
        .bind(&location.address_line1)
        .bind(&location.address_line2)
        .bind(&location.city)
        .bind(&location.state)
        .bind(&location.postal_code)
        .bind(&location.country)
        .bind(&location.floor)
        .bind(&location.building)
        .bind(location.latitude)
        .bind(location.longitude)
        .bind(&location.phone_number)
        .bind(&location.timezone)
        .bind(&location.emergency_contact_name)
        .bind(&location.emergency_contact_phone)
        .bind(created_at)
        .bind(updated_at)
        .bind(&location.e911_callback_number)
        .bind(
            (location.civic != fcp_core::phone_number::CivicElements::default())
                .then(|| serde_json::to_value(&location.civic))
                .transpose()?,
        )
        .bind(location.e911_validated_at.map(system_time_to_datetime))
        .bind(&location.e911_validated_by)
        .bind(&location.e911_validation_reference)
        .execute(self.writes())
        .await?;

        debug!(id = %location.id, name = %location.name, "Location stored");
        Ok(())
    }

    async fn delete_location(&self, id: &LocationId) -> Result<()> {
        sqlx::query("DELETE FROM locations WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;

        debug!(id = %id, "Location deleted");
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[async_trait]
impl QueueStore for PostgresStorage {
    async fn save_agents(&self, tenant_id: &TenantId, agents: Vec<StoredAgent>) -> Result<()> {
        let mut tx = self.writes().begin().await?;

        sqlx::query("DELETE FROM queue_agents WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .execute(&mut *tx)
            .await?;

        for agent in agents {
            let queue_priorities = serde_json::to_value(&agent.queue_priorities)?;
            sqlx::query(
                "INSERT INTO queue_agents (
                    tenant_id, agent_id, extension, queues, skills, priority, state,
                    queue_priorities, max_concurrent_calls, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())",
            )
            .bind(tenant_id.to_string())
            .bind(&agent.id)
            .bind(&agent.extension)
            .bind(&agent.queues)
            .bind(&agent.skills)
            .bind(agent.priority as i32)
            .bind(&agent.state)
            .bind(queue_priorities)
            .bind(agent.max_concurrent_calls as i32)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn load_agents(&self, tenant_id: &TenantId) -> Result<Vec<StoredAgent>> {
        let rows = sqlx::query(
            "SELECT agent_id, extension, queues, skills, priority, state, queue_priorities,
                    max_concurrent_calls
             FROM queue_agents
             WHERE tenant_id = $1
             ORDER BY agent_id",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut agents = Vec::with_capacity(rows.len());
        for row in rows {
            let queue_priorities = row.get::<serde_json::Value, _>("queue_priorities");
            agents.push(StoredAgent {
                id: row.get("agent_id"),
                extension: row.get("extension"),
                queues: row.try_get::<Vec<String>, _>("queues").unwrap_or_default(),
                skills: row.try_get::<Vec<String>, _>("skills").unwrap_or_default(),
                priority: row.get::<i32, _>("priority") as u32,
                state: row.get("state"),
                queue_priorities: serde_json::from_value(queue_priorities).unwrap_or_default(),
                max_concurrent_calls: row.get::<i32, _>("max_concurrent_calls") as u32,
            });
        }

        Ok(agents)
    }

    async fn remove_agent(&self, tenant_id: &TenantId, agent_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM queue_agents WHERE tenant_id = $1 AND agent_id = $2")
            .bind(tenant_id.to_string())
            .bind(agent_id)
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn clear_agents(&self, tenant_id: &TenantId) -> Result<()> {
        sqlx::query("DELETE FROM queue_agents WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

// ============================================================================
// PresenceStore Implementation
// ============================================================================

#[async_trait]
impl PresenceStore for PostgresStorage {
    async fn save_subscription(&self, sub: StoredSubscription) -> Result<()> {
        sqlx::query(
            "INSERT INTO presence_subscriptions \
             (subscription_id, watcher_aor, presentity_aor, tenant_id, contact_uri, \
              event_package, expires_at_epoch, sip_call_id, sip_from_tag, sip_to_tag) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT (subscription_id) DO UPDATE SET \
                watcher_aor = EXCLUDED.watcher_aor, \
                presentity_aor = EXCLUDED.presentity_aor, \
                tenant_id = EXCLUDED.tenant_id, \
                contact_uri = EXCLUDED.contact_uri, \
                event_package = EXCLUDED.event_package, \
                expires_at_epoch = EXCLUDED.expires_at_epoch, \
                sip_call_id = EXCLUDED.sip_call_id, \
                sip_from_tag = EXCLUDED.sip_from_tag, \
                sip_to_tag = EXCLUDED.sip_to_tag",
        )
        .bind(&sub.subscription_id)
        .bind(&sub.watcher_aor)
        .bind(&sub.presentity_aor)
        .bind(sub.tenant_id.to_string())
        .bind(&sub.contact_uri)
        .bind(&sub.event_package)
        .bind(sub.expires_at_epoch as i64)
        .bind(&sub.sip_call_id)
        .bind(&sub.sip_from_tag)
        .bind(&sub.sip_to_tag)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn load_subscriptions(&self, tenant_id: &TenantId) -> Result<Vec<StoredSubscription>> {
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let rows = sqlx::query(
            "SELECT subscription_id, watcher_aor, presentity_aor, tenant_id, \
             contact_uri, event_package, expires_at_epoch, sip_call_id, sip_from_tag, sip_to_tag \
             FROM presence_subscriptions WHERE tenant_id = $1 AND expires_at_epoch > $2",
        )
        .bind(tenant_id.to_string())
        .bind(now_epoch)
        .fetch_all(&self.pool)
        .await?;

        let mut subs = Vec::with_capacity(rows.len());
        for row in rows {
            let tid_str: String = row.try_get("tenant_id")?;
            let tid = tid_str
                .parse::<TenantId>()
                .unwrap_or_else(|_| TenantId::nil());
            subs.push(StoredSubscription {
                subscription_id: row.try_get("subscription_id")?,
                watcher_aor: row.try_get("watcher_aor")?,
                presentity_aor: row.try_get("presentity_aor")?,
                tenant_id: tid,
                contact_uri: row.try_get("contact_uri")?,
                event_package: row.try_get("event_package")?,
                expires_at_epoch: row.try_get::<i64, _>("expires_at_epoch")? as u64,
                sip_call_id: row.try_get("sip_call_id")?,
                sip_from_tag: row.try_get("sip_from_tag")?,
                sip_to_tag: row.try_get("sip_to_tag")?,
            });
        }

        Ok(subs)
    }

    async fn remove_subscription(&self, subscription_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM presence_subscriptions WHERE subscription_id = $1")
            .bind(subscription_id)
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn remove_subscriptions_for_presentity(
        &self,
        tenant_id: &TenantId,
        presentity_aor: &str,
    ) -> Result<()> {
        sqlx::query(
            "DELETE FROM presence_subscriptions WHERE tenant_id = $1 AND presentity_aor = $2",
        )
        .bind(tenant_id.to_string())
        .bind(presentity_aor)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn cleanup_expired_subscriptions(&self) -> Result<usize> {
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let result = sqlx::query("DELETE FROM presence_subscriptions WHERE expires_at_epoch <= $1")
            .bind(now_epoch)
            .execute(self.writes())
            .await?;

        Ok(result.rows_affected() as usize)
    }

    async fn save_presentity(&self, p: StoredPresentity) -> Result<()> {
        sqlx::query(
            "INSERT INTO presence_presentities \
             (aor, state, note, tenant_id, updated_at_epoch) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (tenant_id, aor) DO UPDATE SET \
                state = EXCLUDED.state, \
                note = EXCLUDED.note, \
                updated_at_epoch = EXCLUDED.updated_at_epoch",
        )
        .bind(&p.aor)
        .bind(&p.state)
        .bind(&p.note)
        .bind(p.tenant_id.to_string())
        .bind(p.updated_at_epoch as i64)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn load_presentities(&self, tenant_id: &TenantId) -> Result<Vec<StoredPresentity>> {
        let rows = sqlx::query(
            "SELECT aor, state, note, tenant_id, updated_at_epoch \
             FROM presence_presentities WHERE tenant_id = $1",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let tid_str: String = row.try_get("tenant_id")?;
            let tid = tid_str
                .parse::<TenantId>()
                .unwrap_or_else(|_| TenantId::nil());
            result.push(StoredPresentity {
                aor: row.try_get("aor")?,
                state: row.try_get("state")?,
                note: row.try_get("note")?,
                tenant_id: tid,
                updated_at_epoch: row.try_get::<i64, _>("updated_at_epoch")? as u64,
            });
        }
        Ok(result)
    }

    async fn remove_presentity(&self, tenant_id: &TenantId, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM presence_presentities WHERE tenant_id = $1 AND aor = $2")
            .bind(tenant_id.to_string())
            .bind(aor)
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn save_auth_policy(&self, policy: StoredAuthPolicy) -> Result<()> {
        sqlx::query(
            "INSERT INTO presence_auth_policies \
             (aor, default_allow, allow_list_json, block_list_json, tenant_id, \
              event_allow_json, event_block_json, time_windows_json, \
              allow_patterns_json, block_patterns_json) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT (tenant_id, aor) DO UPDATE SET \
                default_allow = EXCLUDED.default_allow, \
                allow_list_json = EXCLUDED.allow_list_json, \
                block_list_json = EXCLUDED.block_list_json, \
                event_allow_json = EXCLUDED.event_allow_json, \
                event_block_json = EXCLUDED.event_block_json, \
                time_windows_json = EXCLUDED.time_windows_json, \
                allow_patterns_json = EXCLUDED.allow_patterns_json, \
                block_patterns_json = EXCLUDED.block_patterns_json",
        )
        .bind(&policy.aor)
        .bind(policy.default_allow)
        .bind(&policy.allow_list_json)
        .bind(&policy.block_list_json)
        .bind(policy.tenant_id.to_string())
        .bind(&policy.event_allow_json)
        .bind(&policy.event_block_json)
        .bind(&policy.time_windows_json)
        .bind(&policy.allow_patterns_json)
        .bind(&policy.block_patterns_json)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn load_auth_policies(&self, tenant_id: &TenantId) -> Result<Vec<StoredAuthPolicy>> {
        let rows = sqlx::query(
            "SELECT aor, default_allow, allow_list_json, block_list_json, tenant_id, \
                    event_allow_json, event_block_json, time_windows_json, \
                    allow_patterns_json, block_patterns_json \
             FROM presence_auth_policies WHERE tenant_id = $1",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut policies = Vec::with_capacity(rows.len());
        for row in rows {
            let tid_str: String = row.try_get("tenant_id")?;
            let tid = tid_str
                .parse::<TenantId>()
                .unwrap_or_else(|_| TenantId::nil());
            policies.push(StoredAuthPolicy {
                aor: row.try_get("aor")?,
                default_allow: row.try_get::<bool, _>("default_allow")?,
                allow_list_json: row.try_get("allow_list_json")?,
                block_list_json: row.try_get("block_list_json")?,
                tenant_id: tid,
                event_allow_json: row.try_get("event_allow_json").unwrap_or_default(),
                event_block_json: row.try_get("event_block_json").unwrap_or_default(),
                time_windows_json: row.try_get("time_windows_json").unwrap_or_default(),
                allow_patterns_json: row.try_get("allow_patterns_json").unwrap_or_default(),
                block_patterns_json: row.try_get("block_patterns_json").unwrap_or_default(),
            });
        }
        Ok(policies)
    }

    async fn remove_auth_policy(&self, tenant_id: &TenantId, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM presence_auth_policies WHERE tenant_id = $1 AND aor = $2")
            .bind(tenant_id.to_string())
            .bind(aor)
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

#[async_trait]
impl PresenceHistoryStore for PostgresStorage {
    async fn record_change(&self, change: StoredPresenceChange) -> Result<()> {
        sqlx::query(
            "INSERT INTO presence_history \
             (id, tenant_id, aor, old_state, new_state, timestamp_epoch) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&change.id)
        .bind(change.tenant_id.to_string())
        .bind(&change.aor)
        .bind(&change.old_state)
        .bind(&change.new_state)
        .bind(change.timestamp_epoch as i64)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn query_history(
        &self,
        tenant_id: &TenantId,
        aor: &str,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        limit: usize,
    ) -> Result<Vec<StoredPresenceChange>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, aor, old_state, new_state, timestamp_epoch
             FROM presence_history
             WHERE tenant_id = $1
               AND aor = $2
               AND ($3::BIGINT IS NULL OR timestamp_epoch >= $3)
               AND ($4::BIGINT IS NULL OR timestamp_epoch <= $4)
             ORDER BY timestamp_epoch DESC
             LIMIT $5",
        )
        .bind(tenant_id.to_string())
        .bind(aor)
        .bind(from_epoch.map(|value| value as i64))
        .bind(to_epoch.map(|value| value as i64))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut history = Vec::with_capacity(rows.len());
        for row in rows {
            let tenant_id_str: String = row.try_get("tenant_id")?;
            history.push(StoredPresenceChange {
                id: row.try_get("id")?,
                aor: row.try_get("aor")?,
                old_state: row.try_get("old_state")?,
                new_state: row.try_get("new_state")?,
                tenant_id: tenant_id_str.parse().unwrap_or_default(),
                timestamp_epoch: row.try_get::<i64, _>("timestamp_epoch")? as u64,
            });
        }

        Ok(history)
    }

    async fn query_summary(
        &self,
        tenant_id: &TenantId,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
    ) -> Result<Vec<PresenceSummary>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, aor, old_state, new_state, timestamp_epoch
             FROM presence_history
             WHERE tenant_id = $1
               AND ($2::BIGINT IS NULL OR timestamp_epoch >= $2)
               AND ($3::BIGINT IS NULL OR timestamp_epoch <= $3)
             ORDER BY aor ASC, timestamp_epoch ASC",
        )
        .bind(tenant_id.to_string())
        .bind(from_epoch.map(|value| value as i64))
        .bind(to_epoch.map(|value| value as i64))
        .fetch_all(&self.pool)
        .await?;

        let mut changes_by_aor: HashMap<String, Vec<StoredPresenceChange>> = HashMap::new();
        for row in rows {
            let tenant_id_str: String = row.try_get("tenant_id")?;
            let change = StoredPresenceChange {
                id: row.try_get("id")?,
                aor: row.try_get("aor")?,
                old_state: row.try_get("old_state")?,
                new_state: row.try_get("new_state")?,
                tenant_id: tenant_id_str.parse().unwrap_or_default(),
                timestamp_epoch: row.try_get::<i64, _>("timestamp_epoch")? as u64,
            };
            changes_by_aor
                .entry(change.aor.clone())
                .or_default()
                .push(change);
        }

        let mut summaries = Vec::with_capacity(changes_by_aor.len());
        for (aor, changes) in changes_by_aor {
            let total_changes = changes.len() as u64;
            let mut time_in_open_secs = 0u64;
            let mut time_in_busy_secs = 0u64;
            let mut time_in_away_secs = 0u64;
            let mut time_in_closed_secs = 0u64;

            for window in changes.windows(2) {
                let current = &window[0];
                let next = &window[1];
                let duration = next.timestamp_epoch.saturating_sub(current.timestamp_epoch);

                match current.new_state.as_str() {
                    "open" => time_in_open_secs += duration,
                    "busy" => time_in_busy_secs += duration,
                    "away" => time_in_away_secs += duration,
                    "closed" => time_in_closed_secs += duration,
                    _ => {}
                }
            }

            summaries.push(PresenceSummary {
                aor,
                total_changes,
                time_in_open_secs,
                time_in_busy_secs,
                time_in_away_secs,
                time_in_closed_secs,
            });
        }

        summaries.sort_by(|left, right| left.aor.cmp(&right.aor));
        Ok(summaries)
    }
}

#[async_trait]
impl ResourceListStore for PostgresStorage {
    async fn save_resource_list(&self, list: &StoredResourceList) -> Result<()> {
        sqlx::query(
            "INSERT INTO resource_lists (uri, name, entries_json, tenant_id)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(tenant_id, uri) DO UPDATE SET
                name = EXCLUDED.name,
                entries_json = EXCLUDED.entries_json",
        )
        .bind(&list.uri)
        .bind(&list.name)
        .bind(&list.entries_json)
        .bind(list.tenant_id.to_string())
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn get_resource_list(
        &self,
        tenant_id: &TenantId,
        uri: &str,
    ) -> Result<Option<StoredResourceList>> {
        let row = sqlx::query(
            "SELECT uri, name, entries_json, tenant_id
             FROM resource_lists
             WHERE tenant_id = $1 AND uri = $2",
        )
        .bind(tenant_id.to_string())
        .bind(uri)
        .fetch_optional(self.reads())
        .await?;

        Ok(row.map(|r| StoredResourceList {
            uri: r.get("uri"),
            name: r.get("name"),
            entries_json: r.get("entries_json"),
            tenant_id: r.get::<String, _>("tenant_id").parse().unwrap_or_default(),
        }))
    }

    async fn get_resource_lists_by_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<StoredResourceList>> {
        let rows = sqlx::query(
            "SELECT uri, name, entries_json, tenant_id FROM resource_lists WHERE tenant_id = $1",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| StoredResourceList {
                uri: r.get("uri"),
                name: r.get("name"),
                entries_json: r.get("entries_json"),
                tenant_id: r.get::<String, _>("tenant_id").parse().unwrap_or_default(),
            })
            .collect())
    }

    async fn delete_resource_list(&self, tenant_id: &TenantId, uri: &str) -> Result<()> {
        sqlx::query("DELETE FROM resource_lists WHERE tenant_id = $1 AND uri = $2")
            .bind(tenant_id.to_string())
            .bind(uri)
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

#[async_trait]
impl WebhookStore for PostgresStorage {
    async fn create_endpoint(&self, endpoint: WebhookEndpoint) -> Result<()> {
        let endpoint = self.seal_webhook(endpoint)?;
        let endpoint_json = serde_json::to_value(&endpoint)?;
        sqlx::query(
            "INSERT INTO webhook_endpoints (id, tenant_id, endpoint_json, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO UPDATE
             SET tenant_id = EXCLUDED.tenant_id,
                 endpoint_json = EXCLUDED.endpoint_json,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(endpoint.id.to_string())
        .bind(endpoint.tenant_id.to_string())
        .bind(endpoint_json)
        .bind(system_time_to_datetime(endpoint.created_at))
        .bind(system_time_to_datetime(endpoint.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn get_endpoint(&self, id: &WebhookId) -> Result<Option<WebhookEndpoint>> {
        let row = sqlx::query("SELECT endpoint_json FROM webhook_endpoints WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => Ok(Some(
                self.open_webhook(webhook_endpoint_from_value(row.get("endpoint_json"))?)
                    .await?,
            )),
            None => Ok(None),
        }
    }

    async fn list_endpoints(&self, tenant_id: &TenantId) -> Result<Vec<WebhookEndpoint>> {
        let rows = sqlx::query(
            "SELECT endpoint_json
             FROM webhook_endpoints
             WHERE tenant_id = $1
             ORDER BY updated_at DESC",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut endpoints = Vec::with_capacity(rows.len());
        for row in rows {
            endpoints.push(
                self.open_webhook(webhook_endpoint_from_value(row.get("endpoint_json"))?)
                    .await?,
            );
        }
        Ok(endpoints)
    }

    async fn update_endpoint(&self, endpoint: WebhookEndpoint) -> Result<()> {
        let endpoint = self.seal_webhook(endpoint)?;
        let endpoint_json = serde_json::to_value(&endpoint)?;
        sqlx::query(
            "UPDATE webhook_endpoints
             SET tenant_id = $2,
                 endpoint_json = $3,
                 updated_at = $4
             WHERE id = $1",
        )
        .bind(endpoint.id.to_string())
        .bind(endpoint.tenant_id.to_string())
        .bind(endpoint_json)
        .bind(system_time_to_datetime(endpoint.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_endpoint(&self, id: &WebhookId) -> Result<()> {
        sqlx::query("DELETE FROM webhook_endpoints WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn log_delivery(&self, log: WebhookDeliveryLog) -> Result<()> {
        let delivery_json = serde_json::to_value(&log)?;
        sqlx::query(
            "INSERT INTO webhook_delivery_logs (id, endpoint_id, delivery_json, delivered, next_retry_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&log.id)
        .bind(log.endpoint_id.to_string())
        .bind(delivery_json)
        .bind(log.delivered)
        .bind(log.next_retry_at.map(system_time_to_datetime))
        .bind(system_time_to_datetime(log.created_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn get_pending_retries(&self, limit: usize) -> Result<Vec<WebhookDeliveryLog>> {
        let rows = sqlx::query(
            "SELECT delivery_json
             FROM webhook_delivery_logs
             WHERE delivered = FALSE
               AND next_retry_at IS NOT NULL
               AND next_retry_at <= NOW()
             ORDER BY next_retry_at ASC, created_at ASC
             LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| webhook_delivery_log_from_value(row.get("delivery_json")))
            .collect()
    }

    async fn update_delivery_log(&self, log: WebhookDeliveryLog) -> Result<()> {
        let delivery_json = serde_json::to_value(&log)?;
        sqlx::query(
            "UPDATE webhook_delivery_logs
             SET delivery_json = $2,
                 delivered = $3,
                 next_retry_at = $4
             WHERE id = $1",
        )
        .bind(&log.id)
        .bind(delivery_json)
        .bind(log.delivered)
        .bind(log.next_retry_at.map(system_time_to_datetime))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn get_delivery_logs(
        &self,
        endpoint_id: &WebhookId,
        limit: usize,
    ) -> Result<Vec<WebhookDeliveryLog>> {
        let rows = sqlx::query(
            "SELECT delivery_json
             FROM webhook_delivery_logs
             WHERE endpoint_id = $1
             ORDER BY created_at DESC
             LIMIT $2",
        )
        .bind(endpoint_id.to_string())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| webhook_delivery_log_from_value(row.get("delivery_json")))
            .collect()
    }

    async fn get_delivery_log(&self, id: &str) -> Result<Option<WebhookDeliveryLog>> {
        let row = sqlx::query(
            "SELECT delivery_json
             FROM webhook_delivery_logs
             WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(webhook_delivery_log_from_value(
                row.get("delivery_json"),
            )?)),
            None => Ok(None),
        }
    }
}

// ============================================================================
// FlowStore Implementation
// ============================================================================

/// Parse a flow from a PostgreSQL database row.
fn parse_flow_from_row_pg(
    row: &sqlx::postgres::PgRow,
) -> std::result::Result<FlowDefinition, FlowError> {
    let id: String = row.get("id");
    let name: String = row.get("name");
    let description: Option<String> = row.get("description");
    let version: i32 = row.get("version");
    let initial_node: String = row.get("initial_node");
    let nodes_json: serde_json::Value = row.get("nodes");
    let variables_json: serde_json::Value = row.get("variables");
    let tenant_id: Option<String> = row.get("tenant_id");
    let enabled: bool = row.get("enabled");
    let webhook_url: Option<String> = row.get("webhook_url");
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");

    let mut nodes: HashMap<String, FlowNode> = serde_json::from_value(nodes_json)?;
    // Backfill node id from the HashMap key for flows that were stored without it
    for (key, node) in nodes.iter_mut() {
        if node.id.is_empty() {
            node.id = key.clone();
        }
    }
    let variables: HashMap<String, serde_json::Value> = serde_json::from_value(variables_json)?;

    Ok(FlowDefinition {
        id,
        name,
        description,
        version: version as u32,
        initial_node,
        nodes,
        variables,
        tenant_id,
        enabled,
        webhook_url,
        created_at: Some(created_at),
        updated_at: Some(updated_at),
    })
}

#[async_trait]
impl FlowStore for PostgresStorage {
    async fn get_flow(&self, id: &str) -> std::result::Result<Option<FlowDefinition>, FlowError> {
        let row = sqlx::query(
            "SELECT id, name, description, version, initial_node, nodes, variables,
                    tenant_id, enabled, webhook_url, created_at, updated_at
             FROM flows
             WHERE id = $1
             ORDER BY version DESC
             LIMIT 1",
        )
        .bind(id)
        .fetch_optional(self.reads())
        .await
        .map_err(|e| FlowError::StorageError(e.to_string()))?;

        match row {
            Some(row) => Ok(Some(parse_flow_from_row_pg(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_flow_version(
        &self,
        id: &str,
        version: u32,
    ) -> std::result::Result<Option<FlowDefinition>, FlowError> {
        let row = sqlx::query(
            "SELECT id, name, description, version, initial_node, nodes, variables,
                    tenant_id, enabled, webhook_url, created_at, updated_at
             FROM flows
             WHERE id = $1 AND version = $2",
        )
        .bind(id)
        .bind(version as i32)
        .fetch_optional(self.reads())
        .await
        .map_err(|e| FlowError::StorageError(e.to_string()))?;

        match row {
            Some(row) => Ok(Some(parse_flow_from_row_pg(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_flows(
        &self,
        tenant_id: Option<&str>,
    ) -> std::result::Result<Vec<FlowDefinition>, FlowError> {
        let rows = if let Some(tid) = tenant_id {
            sqlx::query(
                "SELECT id, name, description, version, initial_node, nodes, variables,
                        tenant_id, enabled, webhook_url, created_at, updated_at
                 FROM flows
                 WHERE tenant_id = $1
                 ORDER BY updated_at DESC",
            )
            .bind(tid)
            .fetch_all(self.reads())
            .await
            .map_err(|e| FlowError::StorageError(e.to_string()))?
        } else {
            sqlx::query(
                "SELECT id, name, description, version, initial_node, nodes, variables,
                        tenant_id, enabled, webhook_url, created_at, updated_at
                 FROM flows
                 ORDER BY updated_at DESC",
            )
            .fetch_all(self.reads())
            .await
            .map_err(|e| FlowError::StorageError(e.to_string()))?
        };

        let mut flows = Vec::new();
        for row in &rows {
            match parse_flow_from_row_pg(row) {
                Ok(flow) => flows.push(flow),
                Err(e) => {
                    let id: String = row.get("id");
                    tracing::warn!(flow_id = %id, error = %e, "Skipping flow with incompatible schema");
                }
            }
        }

        Ok(flows)
    }

    async fn query_flows(
        &self,
        query: &FlowQuery,
    ) -> std::result::Result<FlowQueryResult, FlowError> {
        // Build WHERE clause with numbered PostgreSQL placeholders
        let mut where_clauses = Vec::new();
        let mut param_num = 0usize;

        if query.tenant_id.is_some() {
            param_num += 1;
            where_clauses.push(format!("tenant_id = ${}", param_num));
        }
        if query.enabled.is_some() {
            param_num += 1;
            where_clauses.push(format!("enabled = ${}", param_num));
        }
        if query.name_contains.is_some() {
            param_num += 1;
            where_clauses.push(format!("name LIKE ${}", param_num));
        }
        if query.description_contains.is_some() {
            param_num += 1;
            where_clauses.push(format!("description LIKE ${}", param_num));
        }

        let where_clause = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        // Build ORDER BY clause
        let order_by = match query.sort_by.unwrap_or(SortField::UpdatedAt) {
            SortField::Name => "name",
            SortField::CreatedAt => "created_at",
            SortField::UpdatedAt => "updated_at",
            SortField::Version => "version",
        };

        let order_direction = match query.sort_order {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        };

        // Count total matching rows
        let count_query = format!("SELECT COUNT(*) as count FROM flows {}", where_clause);
        let mut count_q = sqlx::query(&count_query);

        if let Some(ref tid) = query.tenant_id {
            count_q = count_q.bind(tid.clone());
        }
        if let Some(enabled) = query.enabled {
            count_q = count_q.bind(enabled);
        }
        if let Some(ref name) = query.name_contains {
            count_q = count_q.bind(format!("%{}%", name));
        }
        if let Some(ref desc) = query.description_contains {
            count_q = count_q.bind(format!("%{}%", desc));
        }

        let count_row = count_q
            .fetch_one(self.reads())
            .await
            .map_err(|e| FlowError::StorageError(e.to_string()))?;
        let total: i64 = count_row.get("count");

        // Query with pagination
        let limit_param = param_num + 1;
        let offset_param = param_num + 2;
        let select_query = format!(
            "SELECT id, name, description, version, initial_node, nodes, variables,
                    tenant_id, enabled, webhook_url, created_at, updated_at
             FROM flows
             {}
             ORDER BY {} {}
             LIMIT ${} OFFSET ${}",
            where_clause, order_by, order_direction, limit_param, offset_param
        );

        let mut select_q = sqlx::query(&select_query);

        if let Some(ref tid) = query.tenant_id {
            select_q = select_q.bind(tid.clone());
        }
        if let Some(enabled) = query.enabled {
            select_q = select_q.bind(enabled);
        }
        if let Some(ref name) = query.name_contains {
            select_q = select_q.bind(format!("%{}%", name));
        }
        if let Some(ref desc) = query.description_contains {
            select_q = select_q.bind(format!("%{}%", desc));
        }

        select_q = select_q.bind(query.limit as i64);
        select_q = select_q.bind(query.offset as i64);

        let rows = select_q
            .fetch_all(self.reads())
            .await
            .map_err(|e| FlowError::StorageError(e.to_string()))?;

        let mut flows = Vec::new();
        for row in &rows {
            match parse_flow_from_row_pg(row) {
                Ok(flow) => flows.push(flow),
                Err(e) => {
                    let id: String = row.get("id");
                    tracing::warn!(flow_id = %id, error = %e, "Skipping flow with incompatible schema");
                }
            }
        }

        Ok(FlowQueryResult {
            flows,
            total: total as usize,
            limit: query.limit,
            offset: query.offset,
        })
    }

    async fn insert_flow(&self, flow: FlowDefinition) -> std::result::Result<(), FlowError> {
        let nodes_json = serde_json::to_value(&flow.nodes)?;
        let variables_json = serde_json::to_value(&flow.variables)?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO flows (
                id, name, description, version, initial_node, nodes, variables,
                tenant_id, enabled, webhook_url, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(&flow.id)
        .bind(&flow.name)
        .bind(&flow.description)
        .bind(flow.version as i32)
        .bind(&flow.initial_node)
        .bind(&nodes_json)
        .bind(&variables_json)
        .bind(&flow.tenant_id)
        .bind(flow.enabled)
        .bind(&flow.webhook_url)
        .bind(now)
        .bind(now)
        .execute(self.writes())
        .await
        .map_err(|e| FlowError::StorageError(e.to_string()))?;

        debug!(flow_id = %flow.id, version = flow.version, "Flow inserted into PostgreSQL");
        Ok(())
    }

    async fn put_flow(&self, flow: FlowDefinition) -> std::result::Result<(), FlowError> {
        let nodes_json = serde_json::to_value(&flow.nodes)?;
        let variables_json = serde_json::to_value(&flow.variables)?;
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO flows (
                id, name, description, version, initial_node, nodes, variables,
                tenant_id, enabled, webhook_url, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name, description = EXCLUDED.description,
                version = EXCLUDED.version, initial_node = EXCLUDED.initial_node,
                nodes = EXCLUDED.nodes, variables = EXCLUDED.variables,
                tenant_id = EXCLUDED.tenant_id, enabled = EXCLUDED.enabled,
                webhook_url = EXCLUDED.webhook_url, updated_at = EXCLUDED.updated_at",
        )
        .bind(&flow.id)
        .bind(&flow.name)
        .bind(&flow.description)
        .bind(flow.version as i32)
        .bind(&flow.initial_node)
        .bind(&nodes_json)
        .bind(&variables_json)
        .bind(&flow.tenant_id)
        .bind(flow.enabled)
        .bind(&flow.webhook_url)
        .bind(flow.created_at.unwrap_or(now))
        .bind(flow.updated_at.unwrap_or(now))
        .execute(self.writes())
        .await
        .map_err(|e| FlowError::StorageError(e.to_string()))?;
        debug!(flow_id = %flow.id, version = flow.version, "Flow written as given");
        Ok(())
    }

    async fn update_flow(&self, flow: FlowDefinition) -> std::result::Result<(), FlowError> {
        let nodes_json = serde_json::to_value(&flow.nodes)?;
        let variables_json = serde_json::to_value(&flow.variables)?;

        let result = sqlx::query(
            "UPDATE flows SET
                name = $1, description = $2, version = version + 1,
                initial_node = $3, nodes = $4, variables = $5,
                tenant_id = $6, enabled = $7, webhook_url = $8, updated_at = NOW()
             WHERE id = $9",
        )
        .bind(&flow.name)
        .bind(&flow.description)
        .bind(&flow.initial_node)
        .bind(&nodes_json)
        .bind(&variables_json)
        .bind(&flow.tenant_id)
        .bind(flow.enabled)
        .bind(&flow.webhook_url)
        .bind(&flow.id)
        .execute(self.writes())
        .await
        .map_err(|e| FlowError::StorageError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(FlowError::NotFound(format!("Flow not found: {}", flow.id)));
        }

        debug!(flow_id = %flow.id, "Flow updated in PostgreSQL");
        Ok(())
    }

    async fn delete_flow(&self, id: &str) -> std::result::Result<(), FlowError> {
        let result = sqlx::query("DELETE FROM flows WHERE id = $1")
            .bind(id)
            .execute(self.writes())
            .await
            .map_err(|e| FlowError::StorageError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(FlowError::NotFound(format!("Flow not found: {}", id)));
        }

        debug!(flow_id = %id, "Flow deleted from PostgreSQL");
        Ok(())
    }

    async fn list_flow_versions(
        &self,
        id: &str,
    ) -> std::result::Result<Vec<FlowDefinition>, FlowError> {
        let rows = sqlx::query(
            "SELECT id, name, description, version, initial_node, nodes, variables,
                    tenant_id, enabled, webhook_url, created_at, updated_at
             FROM flows
             WHERE id = $1
             ORDER BY version DESC",
        )
        .bind(id)
        .fetch_all(self.reads())
        .await
        .map_err(|e| FlowError::StorageError(e.to_string()))?;

        let mut flows = Vec::new();
        for row in &rows {
            match parse_flow_from_row_pg(row) {
                Ok(flow) => flows.push(flow),
                Err(e) => {
                    tracing::warn!(flow_id = %id, error = %e, "Skipping flow version with incompatible schema");
                }
            }
        }

        Ok(flows)
    }

    async fn enable_flow(&self, id: &str) -> std::result::Result<(), FlowError> {
        let result =
            sqlx::query("UPDATE flows SET enabled = true, updated_at = NOW() WHERE id = $1")
                .bind(id)
                .execute(self.writes())
                .await
                .map_err(|e| FlowError::StorageError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(FlowError::NotFound(format!("Flow not found: {}", id)));
        }

        debug!(flow_id = %id, "Flow enabled in PostgreSQL");
        Ok(())
    }

    async fn disable_flow(&self, id: &str) -> std::result::Result<(), FlowError> {
        let result =
            sqlx::query("UPDATE flows SET enabled = false, updated_at = NOW() WHERE id = $1")
                .bind(id)
                .execute(self.writes())
                .await
                .map_err(|e| FlowError::StorageError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(FlowError::NotFound(format!("Flow not found: {}", id)));
        }

        debug!(flow_id = %id, "Flow disabled in PostgreSQL");
        Ok(())
    }
}

// ============================================================================
// VoicemailStore Implementation
// ============================================================================

impl PostgresStorage {
    /// Map a mailbox row from PostgreSQL to VoicemailMailbox
    async fn map_mailbox_row_pg(
        &self,
        row: sqlx::postgres::PgRow,
    ) -> Result<fcp_core::VoicemailMailbox> {
        let id: String = row.try_get("id")?;
        let tenant_id = fcp_core::TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?;
        let user_id = fcp_core::UserId::from_str(&row.try_get::<String, _>("user_id")?)?;
        let line_binding_id =
            fcp_core::LineBindingId::from_str(&row.try_get::<String, _>("line_binding_id")?)?;
        let extension: String = row.try_get("extension")?;
        let pin: String = row.try_get("pin")?;
        let enabled: bool = row.try_get("enabled")?;
        let max_message_duration: i32 = row.try_get("max_message_duration")?;
        let max_messages: i32 = row.try_get("max_messages")?;
        let email_notification: bool = row.try_get("email_notification")?;
        let email_address: Option<String> = row.try_get("email_address")?;
        let attach_audio: bool = row.try_get("attach_audio")?;
        let include_transcription: bool = row.try_get("include_transcription")?;
        let transcription_enabled: bool = row.try_get("transcription_enabled")?;
        let retention_days: Option<i32> = row.try_get("retention_days")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

        // Fetch greetings for this mailbox
        let greeting_rows = sqlx::query(
            "SELECT greeting_type, audio_path, is_active, created_at, expires_at
             FROM voicemail_greetings WHERE mailbox_id = $1",
        )
        .bind(&id)
        .fetch_all(&self.pool)
        .await?;

        let mut greetings = Vec::new();
        for greeting_row in greeting_rows {
            let greeting_type_str: String = greeting_row.try_get("greeting_type")?;
            let greeting_type = match greeting_type_str.as_str() {
                "unavailable" => fcp_core::GreetingType::Unavailable,
                "busy" => fcp_core::GreetingType::Busy,
                "name" => fcp_core::GreetingType::Name,
                "temporary" => fcp_core::GreetingType::Temporary,
                _ => continue,
            };

            let audio_path: String = greeting_row.try_get("audio_path")?;
            let is_active: bool = greeting_row.try_get("is_active").unwrap_or(true);
            let g_created_at: DateTime<Utc> = greeting_row.try_get("created_at")?;
            let expires_at: Option<DateTime<Utc>> = greeting_row.try_get("expires_at")?;

            greetings.push(fcp_core::VoicemailGreeting {
                greeting_type,
                audio_path: std::path::PathBuf::from(audio_path),
                is_active,
                created_at: datetime_to_system_time(g_created_at),
                expires_at: expires_at.map(datetime_to_system_time),
            });
        }

        Ok(fcp_core::VoicemailMailbox {
            id,
            tenant_id,
            user_id,
            line_binding_id,
            extension,
            pin,
            enabled,
            max_message_duration: max_message_duration as u32,
            max_messages: max_messages as u32,
            email_notification,
            email_address,
            attach_audio,
            include_transcription,
            transcription_enabled,
            retention_days: retention_days.map(|d| d as u32),
            greetings,
            created_at: datetime_to_system_time(created_at),
            updated_at: datetime_to_system_time(updated_at),
        })
    }

    /// Map a message row from PostgreSQL to VoicemailMessage
    fn map_message_row_pg(row: sqlx::postgres::PgRow) -> Result<fcp_core::VoicemailMessage> {
        let id: String = row.try_get("id")?;
        let mailbox_id: String = row.try_get("mailbox_id")?;
        let caller: String = row.try_get("caller")?;
        let caller_name: Option<String> = row.try_get("caller_name")?;
        let status_str: String = row.try_get("status")?;
        let priority_str: String = row.try_get("priority")?;
        let duration: i32 = row.try_get("duration")?;
        let recording_path: String = row.try_get("recording_path")?;
        let transcription: Option<String> = row.try_get("transcription")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let accessed_at: Option<DateTime<Utc>> = row.try_get("accessed_at")?;

        let status = match status_str.as_str() {
            "new" => fcp_core::MessageStatus::New,
            "heard" => fcp_core::MessageStatus::Heard,
            "saved" => fcp_core::MessageStatus::Saved,
            "deleted" => fcp_core::MessageStatus::Deleted,
            _ => fcp_core::MessageStatus::New,
        };

        let priority = match priority_str.as_str() {
            "urgent" => fcp_core::MessagePriority::Urgent,
            _ => fcp_core::MessagePriority::Normal,
        };

        Ok(fcp_core::VoicemailMessage {
            id,
            mailbox_id,
            caller,
            caller_name,
            status,
            priority,
            duration: duration as u32,
            recording_path: std::path::PathBuf::from(recording_path),
            site_id: row
                .try_get::<Option<String>, _>("site_id")
                .ok()
                .flatten()
                .and_then(|s| fcp_core::SiteId::from_str(&s).ok()),
            transcription,
            created_at: datetime_to_system_time(created_at),
            accessed_at: accessed_at.map(datetime_to_system_time),
        })
    }
}

#[async_trait]
impl VoicemailStore for PostgresStorage {
    async fn get_mailbox(&self, id: &str) -> Result<Option<fcp_core::VoicemailMailbox>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, user_id, line_binding_id, extension, pin, enabled,
                    max_message_duration, max_messages, email_notification, email_address,
                    attach_audio, include_transcription, transcription_enabled,
                    retention_days, created_at, updated_at
             FROM voicemail_mailboxes WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let mailbox = self.map_mailbox_row_pg(row).await?;
                Ok(Some(mailbox))
            }
            None => Ok(None),
        }
    }

    async fn get_mailbox_by_line_binding(
        &self,
        line_binding_id: &fcp_core::LineBindingId,
    ) -> Result<Option<fcp_core::VoicemailMailbox>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, user_id, line_binding_id, extension, pin, enabled,
                    max_message_duration, max_messages, email_notification, email_address,
                    attach_audio, include_transcription, transcription_enabled,
                    retention_days, created_at, updated_at
             FROM voicemail_mailboxes WHERE line_binding_id = $1",
        )
        .bind(line_binding_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let mailbox = self.map_mailbox_row_pg(row).await?;
                Ok(Some(mailbox))
            }
            None => Ok(None),
        }
    }

    async fn get_mailbox_by_user(
        &self,
        user_id: &fcp_core::UserId,
    ) -> Result<Option<fcp_core::VoicemailMailbox>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, user_id, line_binding_id, extension, pin, enabled,
                    max_message_duration, max_messages, email_notification, email_address,
                    attach_audio, include_transcription, transcription_enabled,
                    retention_days, created_at, updated_at
             FROM voicemail_mailboxes WHERE user_id = $1",
        )
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let mailbox = self.map_mailbox_row_pg(row).await?;
                Ok(Some(mailbox))
            }
            None => Ok(None),
        }
    }

    async fn get_mailbox_by_extension(
        &self,
        tenant_id: &fcp_core::TenantId,
        extension: &str,
    ) -> Result<Option<fcp_core::VoicemailMailbox>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, user_id, line_binding_id, extension, pin, enabled,
                    max_message_duration, max_messages, email_notification, email_address,
                    attach_audio, include_transcription, transcription_enabled,
                    retention_days, created_at, updated_at
             FROM voicemail_mailboxes WHERE tenant_id = $1 AND extension = $2",
        )
        .bind(tenant_id.to_string())
        .bind(extension)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let mailbox = self.map_mailbox_row_pg(row).await?;
                Ok(Some(mailbox))
            }
            None => Ok(None),
        }
    }

    async fn list_mailboxes(
        &self,
        tenant_id: &fcp_core::TenantId,
    ) -> Result<Vec<fcp_core::VoicemailMailbox>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, user_id, line_binding_id, extension, pin, enabled,
                    max_message_duration, max_messages, email_notification, email_address,
                    attach_audio, include_transcription, transcription_enabled,
                    retention_days, created_at, updated_at
             FROM voicemail_mailboxes WHERE tenant_id = $1 ORDER BY extension",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut mailboxes = Vec::new();
        for row in rows {
            let mailbox = self.map_mailbox_row_pg(row).await?;
            mailboxes.push(mailbox);
        }
        Ok(mailboxes)
    }

    async fn put_mailbox(&self, mailbox: fcp_core::VoicemailMailbox) -> Result<()> {
        let now = Utc::now();
        let created_at = system_time_to_datetime(mailbox.created_at);

        sqlx::query(
            "INSERT INTO voicemail_mailboxes
             (id, tenant_id, user_id, line_binding_id, extension, pin, enabled,
              max_message_duration, max_messages, email_notification, email_address,
              attach_audio, include_transcription, transcription_enabled,
              retention_days, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
             ON CONFLICT (id) DO UPDATE SET
                 tenant_id = EXCLUDED.tenant_id,
                 user_id = EXCLUDED.user_id,
                 line_binding_id = EXCLUDED.line_binding_id,
                 extension = EXCLUDED.extension,
                 pin = EXCLUDED.pin,
                 enabled = EXCLUDED.enabled,
                 max_message_duration = EXCLUDED.max_message_duration,
                 max_messages = EXCLUDED.max_messages,
                 email_notification = EXCLUDED.email_notification,
                 email_address = EXCLUDED.email_address,
                 attach_audio = EXCLUDED.attach_audio,
                 include_transcription = EXCLUDED.include_transcription,
                 transcription_enabled = EXCLUDED.transcription_enabled,
                 retention_days = EXCLUDED.retention_days,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(&mailbox.id)
        .bind(mailbox.tenant_id.to_string())
        .bind(mailbox.user_id.to_string())
        .bind(mailbox.line_binding_id.to_string())
        .bind(&mailbox.extension)
        .bind(&mailbox.pin)
        .bind(mailbox.enabled)
        .bind(mailbox.max_message_duration as i32)
        .bind(mailbox.max_messages as i32)
        .bind(mailbox.email_notification)
        .bind(mailbox.email_address.as_deref())
        .bind(mailbox.attach_audio)
        .bind(mailbox.include_transcription)
        .bind(mailbox.transcription_enabled)
        .bind(mailbox.retention_days.map(|d| d as i32))
        .bind(created_at)
        .bind(now)
        .execute(self.writes())
        .await?;

        // Delete existing greetings for this mailbox
        sqlx::query("DELETE FROM voicemail_greetings WHERE mailbox_id = $1")
            .bind(&mailbox.id)
            .execute(self.writes())
            .await?;

        // Insert greetings
        for greeting in &mailbox.greetings {
            let greeting_created_at = system_time_to_datetime(greeting.created_at);
            let expires_at = greeting.expires_at.map(system_time_to_datetime);

            sqlx::query(
                "INSERT INTO voicemail_greetings
                 (mailbox_id, greeting_type, audio_path, is_active, created_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&mailbox.id)
            .bind(greeting.greeting_type.to_string())
            .bind(greeting.audio_path.to_string_lossy().as_ref())
            .bind(greeting.is_active)
            .bind(greeting_created_at)
            .bind(expires_at)
            .execute(self.writes())
            .await?;
        }

        Ok(())
    }

    async fn delete_mailbox(&self, id: &str) -> Result<()> {
        deletion::execute(
            self.writes(),
            sqlx::query("DELETE FROM voicemail_mailboxes WHERE id = $1").bind(id),
        )
        .await?;
        Ok(())
    }

    async fn get_message(&self, id: &str) -> Result<Option<fcp_core::VoicemailMessage>> {
        let row = sqlx::query(
            "SELECT id, mailbox_id, caller, caller_name, status, priority, duration,
                    recording_path, transcription, created_at, accessed_at, site_id
             FROM voicemail_messages WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(Self::map_message_row_pg(row)?)),
            None => Ok(None),
        }
    }

    async fn list_messages(&self, mailbox_id: &str) -> Result<Vec<fcp_core::VoicemailMessage>> {
        let rows = sqlx::query(
            "SELECT id, mailbox_id, caller, caller_name, status, priority, duration,
                    recording_path, transcription, created_at, accessed_at, site_id
             FROM voicemail_messages
             WHERE mailbox_id = $1 AND status != 'deleted'
             ORDER BY created_at DESC",
        )
        .bind(mailbox_id)
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(Self::map_message_row_pg(row)?);
        }
        Ok(messages)
    }

    async fn put_message(&self, message: fcp_core::VoicemailMessage) -> Result<()> {
        let created_at = system_time_to_datetime(message.created_at);
        let accessed_at = message.accessed_at.map(system_time_to_datetime);

        sqlx::query(
            "INSERT INTO voicemail_messages
             (id, mailbox_id, caller, caller_name, status, priority, duration,
              recording_path, transcription, created_at, accessed_at, site_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (id) DO UPDATE SET
                 mailbox_id = EXCLUDED.mailbox_id,
                 caller = EXCLUDED.caller,
                 caller_name = EXCLUDED.caller_name,
                 status = EXCLUDED.status,
                 priority = EXCLUDED.priority,
                 duration = EXCLUDED.duration,
                 recording_path = EXCLUDED.recording_path,
                 transcription = EXCLUDED.transcription,
                 accessed_at = EXCLUDED.accessed_at,
                 site_id = EXCLUDED.site_id",
        )
        .bind(&message.id)
        .bind(&message.mailbox_id)
        .bind(&message.caller)
        .bind(message.caller_name.as_deref())
        .bind(message.status.to_string())
        .bind(match message.priority {
            fcp_core::MessagePriority::Normal => "normal",
            fcp_core::MessagePriority::Urgent => "urgent",
        })
        .bind(message.duration as i32)
        .bind(message.recording_path.to_string_lossy().as_ref())
        .bind(message.transcription.as_deref())
        .bind(created_at)
        .bind(accessed_at)
        .bind(message.site_id.map(|s| s.to_string()))
        .execute(self.writes())
        .await?;

        Ok(())
    }

    async fn messages_after(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<Vec<(i64, fcp_core::VoicemailMessage)>> {
        let rows = sqlx::query(
            "SELECT id, mailbox_id, caller, caller_name, status, priority, duration,
                    recording_path, transcription, created_at, accessed_at, site_id, ship_seq
             FROM voicemail_messages
             WHERE ship_seq > $1 ORDER BY ship_seq LIMIT $2",
        )
        .bind(after)
        .bind(limit.clamp(1, 10_000) as i64)
        .fetch_all(self.reads())
        .await?;
        rows.into_iter()
            .map(|row| {
                let seq: i64 = row.try_get("ship_seq")?;
                Ok((seq, Self::map_message_row_pg(row)?))
            })
            .collect()
    }

    async fn mailboxes_after(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<Vec<(i64, fcp_core::VoicemailMailbox)>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, user_id, line_binding_id, extension, pin, enabled,
                    max_message_duration, max_messages, email_notification, email_address,
                    attach_audio, include_transcription, transcription_enabled,
                    retention_days, created_at, updated_at, ship_seq
             FROM voicemail_mailboxes
             WHERE ship_seq > $1 ORDER BY ship_seq LIMIT $2",
        )
        .bind(after)
        .bind(limit.clamp(1, 10_000) as i64)
        .fetch_all(self.reads())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i64 = row.try_get("ship_seq")?;
            out.push((seq, self.map_mailbox_row_pg(row).await?));
        }
        Ok(out)
    }

    async fn delete_message(&self, id: &str) -> Result<()> {
        deletion::execute(
            self.writes(),
            sqlx::query("DELETE FROM voicemail_messages WHERE id = $1").bind(id),
        )
        .await?;
        Ok(())
    }

    async fn delete_messages_before(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> Result<u64> {
        Ok(deletion::execute(
            self.writes(),
            sqlx::query(
                "DELETE FROM voicemail_messages
                 WHERE tenant_id = $1 AND created_at < $2
                   AND fcp_voicemail_hold(tenant_id, created_at, caller, mailbox_id) IS NULL",
            )
            .bind(tenant_id.to_string())
            .bind(system_time_to_datetime(before)),
        )
        .await?)
    }

    async fn apply_mailbox_retention(&self) -> Result<u64> {
        Ok(deletion::execute(
            self.writes(),
            sqlx::query(
                "DELETE FROM voicemail_messages m USING voicemail_mailboxes mb
                 WHERE mb.id = m.mailbox_id AND mb.retention_days IS NOT NULL
                   AND mb.retention_days > 0
                   AND m.created_at < now() - make_interval(days => mb.retention_days)
                   AND fcp_voicemail_hold(m.tenant_id, m.created_at, m.caller, m.mailbox_id) IS NULL",
            ),
        )
        .await?)
    }

    async fn purge_deleted_messages(&self, before: SystemTime) -> Result<u64> {
        Ok(deletion::execute(
            self.writes(),
            sqlx::query(
                "DELETE FROM voicemail_messages
                 WHERE deleted_at < $1
                   AND fcp_voicemail_hold(tenant_id, created_at, caller, mailbox_id) IS NULL",
            )
            .bind(system_time_to_datetime(before)),
        )
        .await?)
    }

    async fn list_messages_by_status(
        &self,
        mailbox_id: &str,
        status: fcp_core::MessageStatus,
    ) -> Result<Vec<fcp_core::VoicemailMessage>> {
        let rows = sqlx::query(
            "SELECT id, mailbox_id, caller, caller_name, status, priority, duration,
                    recording_path, transcription, created_at, accessed_at, site_id
             FROM voicemail_messages
             WHERE mailbox_id = $1 AND status = $2
             ORDER BY created_at DESC",
        )
        .bind(mailbox_id)
        .bind(status.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(Self::map_message_row_pg(row)?);
        }
        Ok(messages)
    }

    async fn get_mwi_status(&self, mailbox_id: &str) -> Result<fcp_core::MwiStatus> {
        let mailbox = self
            .get_mailbox(mailbox_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Mailbox not found"))?;

        let row = sqlx::query(
            "SELECT
                COUNT(CASE WHEN status = 'new' AND priority = 'normal' THEN 1 END) as new_normal,
                COUNT(CASE WHEN status = 'new' AND priority = 'urgent' THEN 1 END) as new_urgent,
                COUNT(CASE WHEN status IN ('heard', 'saved') AND priority = 'normal' THEN 1 END) as old_normal,
                COUNT(CASE WHEN status IN ('heard', 'saved') AND priority = 'urgent' THEN 1 END) as old_urgent
             FROM voicemail_messages
             WHERE mailbox_id = $1",
        )
        .bind(mailbox_id)
        .fetch_one(&self.pool)
        .await?;

        let new_normal: i64 = row.try_get("new_normal")?;
        let new_urgent: i64 = row.try_get("new_urgent")?;
        let old_normal: i64 = row.try_get("old_normal")?;
        let old_urgent: i64 = row.try_get("old_urgent")?;

        Ok(fcp_core::MwiStatus {
            mailbox_id: mailbox_id.to_string(),
            line_binding_id: mailbox.line_binding_id,
            new_messages: new_normal as u32,
            old_messages: old_normal as u32,
            urgent_new_messages: new_urgent as u32,
            urgent_old_messages: old_urgent as u32,
        })
    }

    async fn get_mailbox_stats(&self, mailbox_id: &str) -> Result<fcp_core::VoicemailStats> {
        let row = sqlx::query(
            "SELECT
                COUNT(*) as total,
                COUNT(CASE WHEN status = 'new' THEN 1 END) as new_count,
                COUNT(CASE WHEN status = 'heard' THEN 1 END) as heard,
                COUNT(CASE WHEN status = 'saved' THEN 1 END) as saved,
                COALESCE(AVG(duration)::FLOAT8, 0.0) as avg_duration
             FROM voicemail_messages
             WHERE mailbox_id = $1 AND status != 'deleted'",
        )
        .bind(mailbox_id)
        .fetch_one(&self.pool)
        .await?;

        let total: i64 = row.try_get("total")?;
        let new_count: i64 = row.try_get("new_count")?;
        let heard: i64 = row.try_get("heard")?;
        let saved: i64 = row.try_get("saved")?;
        let avg_duration: f64 = row.try_get("avg_duration")?;

        Ok(fcp_core::VoicemailStats {
            total_messages: total as u32,
            new_messages: new_count as u32,
            heard_messages: heard as u32,
            saved_messages: saved as u32,
            storage_used: 0, // TODO: Calculate actual storage from file sizes
            avg_duration: avg_duration as u32,
        })
    }
}

// ============================================================================
// GeoRoutingStore Implementation (stub)
// ============================================================================

#[async_trait]
impl GeoRoutingStore for PostgresStorage {
    async fn get_geo_rule(&self, id: &str) -> Result<Option<fcp_core::GeoRoutingRule>> {
        let row = sqlx::query("SELECT rule_json FROM geo_routing_rules WHERE id = $1")
            .bind(id)
            .fetch_optional(self.reads())
            .await?;

        row.map(|row| geo_rule_from_value(row.get("rule_json")))
            .transpose()
    }

    async fn list_geo_rules(
        &self,
        tenant_id: &fcp_core::TenantId,
    ) -> Result<Vec<fcp_core::GeoRoutingRule>> {
        let rows = sqlx::query(
            "SELECT rule_json
             FROM geo_routing_rules
             WHERE tenant_id = $1
             ORDER BY priority ASC, updated_at DESC",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        rows.into_iter()
            .map(|row| geo_rule_from_value(row.get("rule_json")))
            .collect()
    }

    async fn put_geo_rule(&self, rule: fcp_core::GeoRoutingRule) -> Result<()> {
        let rule_json = serde_json::to_value(&rule)?;
        sqlx::query(
            "INSERT INTO geo_routing_rules (id, tenant_id, priority, updated_at, rule_json)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO UPDATE
             SET tenant_id = EXCLUDED.tenant_id,
                 priority = EXCLUDED.priority,
                 updated_at = EXCLUDED.updated_at,
                 rule_json = EXCLUDED.rule_json",
        )
        .bind(&rule.id)
        .bind(rule.tenant_id.to_string())
        .bind(rule.priority as i32)
        .bind(system_time_to_datetime(rule.updated_at))
        .bind(rule_json)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_geo_rule(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM geo_routing_rules WHERE id = $1")
            .bind(id)
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn get_phone_number_location(
        &self,
        phone_number_id: &fcp_core::PhoneNumberId,
    ) -> Result<Option<fcp_core::PhoneNumberLocation>> {
        let row = sqlx::query(
            "SELECT location_json
             FROM phone_number_locations
             WHERE phone_number_id = $1",
        )
        .bind(phone_number_id.to_string())
        .fetch_optional(self.reads())
        .await?;

        row.map(|row| phone_number_location_from_value(row.get("location_json")))
            .transpose()
    }

    async fn list_phone_number_locations(
        &self,
        tenant_id: &fcp_core::TenantId,
    ) -> Result<Vec<fcp_core::PhoneNumberLocation>> {
        let rows = sqlx::query(
            "SELECT location_json
             FROM phone_number_locations
             WHERE tenant_id = $1
             ORDER BY verified_at DESC",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;

        rows.into_iter()
            .map(|row| phone_number_location_from_value(row.get("location_json")))
            .collect()
    }

    async fn put_phone_number_location(
        &self,
        location: fcp_core::PhoneNumberLocation,
    ) -> Result<()> {
        let location_json = serde_json::to_value(&location)?;
        sqlx::query(
            "INSERT INTO phone_number_locations (phone_number_id, tenant_id, verified_at, location_json)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (phone_number_id) DO UPDATE
             SET tenant_id = EXCLUDED.tenant_id,
                 verified_at = EXCLUDED.verified_at,
                 location_json = EXCLUDED.location_json",
        )
        .bind(location.phone_number_id.to_string())
        .bind(location.tenant_id.to_string())
        .bind(system_time_to_datetime(location.verified_at))
        .bind(location_json)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_phone_number_location(
        &self,
        phone_number_id: &fcp_core::PhoneNumberId,
    ) -> Result<()> {
        sqlx::query("DELETE FROM phone_number_locations WHERE phone_number_id = $1")
            .bind(phone_number_id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

// ============================================================================
// PortingStore Implementation (stub)
// ============================================================================

#[async_trait]
impl fcp_core::sync::SyncStore for PostgresStorage {
    async fn journal_head(&self) -> Result<i64> {
        let head: Option<i64> = sqlx::query_scalar("SELECT max(seq) FROM sync_journal")
            .fetch_one(&self.pool)
            .await?;
        Ok(head.unwrap_or(0))
    }

    async fn journal_after(
        &self,
        tenant_id: &fcp_core::TenantId,
        since: i64,
        limit: usize,
    ) -> Result<Vec<fcp_core::JournalEntry>> {
        let rows = sqlx::query(
            "SELECT seq, tenant_id, table_name, row_id, op, at
             FROM sync_journal
             WHERE tenant_id = $1 AND seq > $2
             ORDER BY seq
             LIMIT $3",
        )
        .bind(tenant_id.to_string())
        .bind(since)
        .bind(limit.min(10_000) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let tenant: String = row.get("tenant_id");
                let op: String = row.get("op");
                let at: chrono::DateTime<chrono::Utc> = row.get("at");
                Ok(fcp_core::JournalEntry {
                    seq: row.get("seq"),
                    tenant_id: tenant.parse().context("journal tenant id")?,
                    table: row.get("table_name"),
                    row_id: row.get("row_id"),
                    op: fcp_core::SyncOp::parse(&op).with_context(|| format!("journal op {op}"))?,
                    at: at.into(),
                })
            })
            .collect()
    }
}

#[async_trait]
impl fcp_core::class_of_service::ClassOfServiceStore for PostgresStorage {
    async fn get_class_of_service(
        &self,
        id: &fcp_core::ClassOfServiceId,
    ) -> Result<Option<fcp_core::ClassOfService>> {
        let row = sqlx::query("SELECT class_json FROM classes_of_service WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(self.reads())
            .await?;
        row.map(|row| {
            serde_json::from_value(row.get("class_json")).context("a class of service row")
        })
        .transpose()
    }

    async fn list_classes_of_service(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<fcp_core::ClassOfService>> {
        let rows = sqlx::query(
            "SELECT class_json FROM classes_of_service WHERE tenant_id = $1 ORDER BY name",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.into_iter()
            .map(|row| {
                serde_json::from_value(row.get("class_json")).context("a class of service row")
            })
            .collect()
    }

    async fn put_class_of_service(&self, class: fcp_core::ClassOfService) -> Result<()> {
        let json = serde_json::to_value(&class)?;
        sqlx::query(
            "INSERT INTO classes_of_service (id, tenant_id, name, class_json, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 class_json = EXCLUDED.class_json,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(class.id.to_string())
        .bind(class.tenant_id.to_string())
        .bind(&class.name)
        .bind(json)
        .bind(system_time_to_datetime(class.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_class_of_service(&self, id: &fcp_core::ClassOfServiceId) -> Result<()> {
        sqlx::query("DELETE FROM classes_of_service WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

/// The columns [`row_to_hoteling_session`] reads.
const HOTELING_COLUMNS: &str =
    "id, tenant_id, device_id, user_id, started_at, expires_at, ended_at, ended_by";

fn row_to_hoteling_session(row: sqlx::postgres::PgRow) -> Result<fcp_core::HotelingSession> {
    Ok(fcp_core::HotelingSession {
        id: row.get::<String, _>("id").parse()?,
        tenant_id: TenantId::from_str(&row.get::<String, _>("tenant_id"))?,
        device_id: DeviceId::from_str(&row.get::<String, _>("device_id"))?,
        user_id: UserId::from_str(&row.get::<String, _>("user_id"))?,
        started_at: datetime_to_system_time(row.get("started_at")),
        expires_at: datetime_to_system_time(row.get("expires_at")),
        ended_at: row
            .get::<Option<DateTime<Utc>>, _>("ended_at")
            .map(datetime_to_system_time),
        ended_by: row
            .get::<Option<String>, _>("ended_by")
            .as_deref()
            .and_then(fcp_core::SessionEnd::parse),
    })
}

// Sessions are read from the primary: a guest who has just logged in is
// rung a moment later, sooner than a replica may have caught up.
#[async_trait]
impl fcp_core::hoteling::HotelingStore for PostgresStorage {
    async fn start_hoteling_session(
        &self,
        session: fcp_core::HotelingSession,
    ) -> Result<Vec<fcp_core::HotelingSession>> {
        let mut tx = self.writes().begin().await?;
        let ended = sqlx::query(&format!(
            "UPDATE hoteling_sessions SET ended_at = $3, ended_by = 'replaced'
             WHERE ended_at IS NULL AND (device_id = $1 OR user_id = $2)
             RETURNING {HOTELING_COLUMNS}"
        ))
        .bind(session.device_id.to_string())
        .bind(session.user_id.to_string())
        .bind(system_time_to_datetime(session.started_at))
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO hoteling_sessions (id, tenant_id, device_id, user_id, started_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session.id.to_string())
        .bind(session.tenant_id.to_string())
        .bind(session.device_id.to_string())
        .bind(session.user_id.to_string())
        .bind(system_time_to_datetime(session.started_at))
        .bind(system_time_to_datetime(session.expires_at))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        ended.into_iter().map(row_to_hoteling_session).collect()
    }

    async fn get_hoteling_session(
        &self,
        id: &fcp_core::HotelingSessionId,
    ) -> Result<Option<fcp_core::HotelingSession>> {
        sqlx::query(&format!(
            "SELECT {HOTELING_COLUMNS} FROM hoteling_sessions WHERE id = $1"
        ))
        .bind(id.to_string())
        .fetch_optional(self.writes())
        .await?
        .map(row_to_hoteling_session)
        .transpose()
    }

    async fn hoteling_session_for_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Option<fcp_core::HotelingSession>> {
        sqlx::query(&format!(
            "SELECT {HOTELING_COLUMNS} FROM hoteling_sessions
             WHERE device_id = $1 AND ended_at IS NULL AND expires_at > NOW()"
        ))
        .bind(device_id.to_string())
        .fetch_optional(self.writes())
        .await?
        .map(row_to_hoteling_session)
        .transpose()
    }

    async fn hoteling_session_for_user(
        &self,
        user_id: &UserId,
    ) -> Result<Option<fcp_core::HotelingSession>> {
        sqlx::query(&format!(
            "SELECT {HOTELING_COLUMNS} FROM hoteling_sessions
             WHERE user_id = $1 AND ended_at IS NULL AND expires_at > NOW()"
        ))
        .bind(user_id.to_string())
        .fetch_optional(self.writes())
        .await?
        .map(row_to_hoteling_session)
        .transpose()
    }

    async fn list_hoteling_sessions(
        &self,
        tenant_id: &TenantId,
        include_ended: bool,
    ) -> Result<Vec<fcp_core::HotelingSession>> {
        let filter = if include_ended {
            ""
        } else {
            "AND ended_at IS NULL AND expires_at > NOW()"
        };
        sqlx::query(&format!(
            "SELECT {HOTELING_COLUMNS} FROM hoteling_sessions
             WHERE tenant_id = $1 {filter}
             ORDER BY started_at DESC LIMIT 500"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?
        .into_iter()
        .map(row_to_hoteling_session)
        .collect()
    }

    async fn end_hoteling_session(
        &self,
        id: &fcp_core::HotelingSessionId,
        by: fcp_core::SessionEnd,
    ) -> Result<Option<fcp_core::HotelingSession>> {
        sqlx::query(&format!(
            "UPDATE hoteling_sessions SET ended_at = NOW(), ended_by = $2
             WHERE id = $1 AND ended_at IS NULL
             RETURNING {HOTELING_COLUMNS}"
        ))
        .bind(id.to_string())
        .bind(by.as_str())
        .fetch_optional(self.writes())
        .await?
        .map(row_to_hoteling_session)
        .transpose()
    }

    async fn expire_hoteling_sessions(
        &self,
        now: SystemTime,
    ) -> Result<Vec<fcp_core::HotelingSession>> {
        sqlx::query(&format!(
            "UPDATE hoteling_sessions SET ended_at = expires_at, ended_by = 'expired'
             WHERE ended_at IS NULL AND expires_at <= $1
             RETURNING {HOTELING_COLUMNS}"
        ))
        .bind(system_time_to_datetime(now))
        .fetch_all(self.writes())
        .await?
        .into_iter()
        .map(row_to_hoteling_session)
        .collect()
    }
}

#[async_trait]
impl fcp_core::schedule::ScheduleStore for PostgresStorage {
    async fn get_schedule(&self, id: &fcp_core::ScheduleId) -> Result<Option<fcp_core::Schedule>> {
        let row = sqlx::query("SELECT schedule_json FROM schedules WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(self.reads())
            .await?;
        row.map(|row| serde_json::from_value(row.get("schedule_json")).context("a schedule row"))
            .transpose()
    }

    async fn list_schedules(&self, tenant_id: &TenantId) -> Result<Vec<fcp_core::Schedule>> {
        let rows =
            sqlx::query("SELECT schedule_json FROM schedules WHERE tenant_id = $1 ORDER BY name")
                .bind(tenant_id.to_string())
                .fetch_all(self.reads())
                .await?;
        rows.into_iter()
            .map(|row| serde_json::from_value(row.get("schedule_json")).context("a schedule row"))
            .collect()
    }

    async fn put_schedule(&self, schedule: fcp_core::Schedule) -> Result<()> {
        let json = serde_json::to_value(&schedule)?;
        sqlx::query(
            "INSERT INTO schedules (id, tenant_id, name, schedule_json, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 schedule_json = EXCLUDED.schedule_json,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(schedule.id.to_string())
        .bind(schedule.tenant_id.to_string())
        .bind(&schedule.name)
        .bind(json)
        .bind(system_time_to_datetime(schedule.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_schedule(&self, id: &fcp_core::ScheduleId) -> Result<()> {
        sqlx::query("DELETE FROM schedules WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

#[async_trait]
impl fcp_core::calling::CallingStore for PostgresStorage {
    async fn get_calling_settings(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<Option<fcp_core::CallingSettings>> {
        let row = sqlx::query(
            "SELECT settings_json FROM user_calling_settings WHERE tenant_id = $1 AND user_id = $2",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.reads())
        .await?;
        row.map(|row| calling_settings_from_value(row.get("settings_json")))
            .transpose()
    }

    async fn list_calling_settings(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<fcp_core::CallingSettings>> {
        let rows = sqlx::query(
            "SELECT settings_json FROM user_calling_settings WHERE tenant_id = $1 ORDER BY user_id",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.into_iter()
            .map(|row| calling_settings_from_value(row.get("settings_json")))
            .collect()
    }

    async fn put_calling_settings(&self, settings: fcp_core::CallingSettings) -> Result<()> {
        let json = serde_json::to_value(&settings)?;
        sqlx::query(
            "INSERT INTO user_calling_settings (user_id, tenant_id, settings_json, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id) DO UPDATE SET
                 tenant_id = EXCLUDED.tenant_id,
                 settings_json = EXCLUDED.settings_json,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(settings.user_id.to_string())
        .bind(settings.tenant_id.to_string())
        .bind(json)
        .bind(system_time_to_datetime(settings.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_calling_settings(&self, tenant_id: &TenantId, user_id: &UserId) -> Result<()> {
        sqlx::query("DELETE FROM user_calling_settings WHERE tenant_id = $1 AND user_id = $2")
            .bind(tenant_id.to_string())
            .bind(user_id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }
}

fn calling_settings_from_value(value: serde_json::Value) -> Result<fcp_core::CallingSettings> {
    serde_json::from_value(value).context("a calling settings row")
}

#[async_trait]
impl fcp_core::site::SiteStore for PostgresStorage {
    async fn get_site(&self, id: &fcp_core::SiteId) -> Result<Option<fcp_core::Site>> {
        let row = sqlx::query("SELECT site_json FROM sites WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| site_from_value(row.get("site_json")))
            .transpose()
    }

    async fn list_sites(&self, tenant_id: &fcp_core::TenantId) -> Result<Vec<fcp_core::Site>> {
        let rows = sqlx::query(
            "SELECT site_json FROM sites WHERE tenant_id = $1 ORDER BY created_at DESC",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| site_from_value(row.get("site_json")))
            .collect()
    }

    async fn list_all_sites(&self) -> Result<Vec<fcp_core::Site>> {
        let rows = sqlx::query("SELECT site_json FROM sites ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| site_from_value(row.get("site_json")))
            .collect()
    }

    async fn put_site(&self, site: fcp_core::Site) -> Result<()> {
        let site_json = serde_json::to_value(&site)?;
        sqlx::query(
            "INSERT INTO sites (id, tenant_id, name, status, secret_hash, created_at, updated_at, site_json)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (id) DO UPDATE
             SET tenant_id = EXCLUDED.tenant_id,
                 name = EXCLUDED.name,
                 status = EXCLUDED.status,
                 secret_hash = EXCLUDED.secret_hash,
                 updated_at = EXCLUDED.updated_at,
                 site_json = EXCLUDED.site_json",
        )
        .bind(site.id.to_string())
        .bind(site.tenant_id.to_string())
        .bind(&site.name)
        .bind(site.status.as_str())
        .bind(&site.secret_hash)
        .bind(system_time_to_datetime(site.created_at))
        .bind(system_time_to_datetime(site.updated_at))
        .bind(site_json)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_site(&self, id: &fcp_core::SiteId) -> Result<()> {
        sqlx::query("DELETE FROM sites WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn put_enrollment_token(&self, token: fcp_core::EnrollmentToken) -> Result<()> {
        sqlx::query(
            "INSERT INTO site_enrollment_tokens (token_hash, site_id, created_at, expires_at, used_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (token_hash) DO NOTHING",
        )
        .bind(&token.token_hash)
        .bind(token.site_id.to_string())
        .bind(system_time_to_datetime(token.created_at))
        .bind(system_time_to_datetime(token.expires_at))
        .bind(token.used_at.map(system_time_to_datetime))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn consume_enrollment_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<fcp_core::EnrollmentToken>> {
        // Spent in the same statement that finds it: two links presenting
        // one token get it once.
        let row = sqlx::query(
            "UPDATE site_enrollment_tokens
             SET used_at = NOW()
             WHERE token_hash = $1 AND used_at IS NULL AND expires_at > NOW()
             RETURNING token_hash, site_id, created_at, expires_at, used_at",
        )
        .bind(token_hash)
        .fetch_optional(self.writes())
        .await?;
        Ok(row.map(|row| {
            let created: chrono::DateTime<chrono::Utc> = row.get("created_at");
            let expires: chrono::DateTime<chrono::Utc> = row.get("expires_at");
            let used: Option<chrono::DateTime<chrono::Utc>> = row.get("used_at");
            let site_id: String = row.get("site_id");
            fcp_core::EnrollmentToken {
                site_id: site_id.parse().unwrap_or_else(|_| fcp_core::SiteId::nil()),
                token_hash: row.get("token_hash"),
                created_at: created.into(),
                expires_at: expires.into(),
                used_at: used.map(Into::into),
            }
        }))
    }

    async fn get_site_by_secret_hash(&self, secret_hash: &str) -> Result<Option<fcp_core::Site>> {
        let row = sqlx::query("SELECT site_json FROM sites WHERE secret_hash = $1")
            .bind(secret_hash)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| site_from_value(row.get("site_json")))
            .transpose()
    }

    async fn put_site_request(&self, request: fcp_core::site::SiteRequest) -> Result<()> {
        let outcome = request.outcome.as_ref();
        sqlx::query(
            "INSERT INTO site_requests (id, site_id, tenant_id, request, created_at, taken_at, done_at, ok, detail)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO UPDATE SET
                request = EXCLUDED.request,
                taken_at = EXCLUDED.taken_at,
                done_at = EXCLUDED.done_at,
                ok = EXCLUDED.ok,
                detail = EXCLUDED.detail",
        )
        .bind(request.id.to_string())
        .bind(request.site_id.to_string())
        .bind(request.tenant_id.to_string())
        .bind(serde_json::to_value(&request)?)
        .bind(system_time_to_datetime(request.created_at))
        .bind(request.taken_at.map(system_time_to_datetime))
        .bind(request.done_at.map(system_time_to_datetime))
        .bind(outcome.map(|o| o.ok))
        .bind(outcome.and_then(|o| o.detail.clone()))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn get_site_request(
        &self,
        id: &fcp_core::SiteRequestId,
    ) -> Result<Option<fcp_core::site::SiteRequest>> {
        let row = sqlx::query("SELECT request FROM site_requests WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(self.reads())
            .await?;
        row.map(|row| Ok(serde_json::from_value(row.try_get("request")?)?))
            .transpose()
    }

    async fn pending_site_requests(
        &self,
        site_id: &fcp_core::SiteId,
        limit: usize,
    ) -> Result<Vec<fcp_core::site::SiteRequest>> {
        let rows = sqlx::query(
            "SELECT request FROM site_requests
             WHERE site_id = $1 AND done_at IS NULL
             ORDER BY created_at LIMIT $2",
        )
        .bind(site_id.to_string())
        .bind(limit.clamp(1, 1000) as i64)
        .fetch_all(self.reads())
        .await?;
        rows.into_iter()
            .map(|row| Ok(serde_json::from_value(row.try_get("request")?)?))
            .collect()
    }
}

fn site_from_value(value: serde_json::Value) -> Result<fcp_core::Site> {
    serde_json::from_value(value).context("Failed to deserialize site")
}

#[async_trait]
impl PortingStore for PostgresStorage {
    async fn get_port_request(
        &self,
        id: &fcp_core::PortRequestId,
    ) -> Result<Option<fcp_core::PortRequest>> {
        let row = sqlx::query("SELECT request_json FROM port_requests WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(|row| port_request_from_value(row.get("request_json")))
            .transpose()
    }

    async fn list_port_requests(
        &self,
        tenant_id: &fcp_core::TenantId,
    ) -> Result<Vec<fcp_core::PortRequest>> {
        let rows = sqlx::query(
            "SELECT request_json
             FROM port_requests
             WHERE tenant_id = $1
             ORDER BY updated_at DESC",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| port_request_from_value(row.get("request_json")))
            .collect()
    }

    async fn put_port_request(&self, request: fcp_core::PortRequest) -> Result<()> {
        let request_json = serde_json::to_value(&request)?;
        sqlx::query(
            "INSERT INTO port_requests (id, tenant_id, status, created_at, updated_at, request_json)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (id) DO UPDATE
             SET tenant_id = EXCLUDED.tenant_id,
                 status = EXCLUDED.status,
                 updated_at = EXCLUDED.updated_at,
                 request_json = EXCLUDED.request_json",
        )
        .bind(request.id.to_string())
        .bind(request.tenant_id.to_string())
        .bind(port_status_to_db(request.status))
        .bind(system_time_to_datetime(request.created_at))
        .bind(system_time_to_datetime(request.updated_at))
        .bind(request_json)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_port_request(&self, id: &fcp_core::PortRequestId) -> Result<()> {
        sqlx::query("DELETE FROM port_requests WHERE id = $1")
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(())
    }

    async fn list_port_requests_by_status(
        &self,
        tenant_id: &fcp_core::TenantId,
        status: fcp_core::PortStatus,
    ) -> Result<Vec<fcp_core::PortRequest>> {
        let rows = sqlx::query(
            "SELECT request_json
             FROM port_requests
             WHERE tenant_id = $1 AND status = $2
             ORDER BY updated_at DESC",
        )
        .bind(tenant_id.to_string())
        .bind(port_status_to_db(status))
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| port_request_from_value(row.get("request_json")))
            .collect()
    }
}

// Database-backed tests live in `tests/integration.rs`; they run whenever
// `FCP_TEST_DATABASE_URL` points at a PostgreSQL superuser connection (CI
// provides one) and skip otherwise.

#[cfg(test)]
mod replica_tests {
    use super::*;

    #[test]
    fn lsn_parses_as_a_64_bit_position() {
        assert_eq!(parse_lsn("0/0"), Some(0));
        assert_eq!(parse_lsn("0/16B3748"), Some(0x16B3748));
        assert_eq!(parse_lsn("1/0"), Some(1 << 32));
        assert_eq!(parse_lsn(" 2/A "), Some((2 << 32) | 0xA));
        assert!(parse_lsn("garbage").is_none());
        assert!(parse_lsn("1/xyz").is_none());
    }

    #[test]
    fn lag_is_zero_when_caught_up_or_not_a_standby() {
        // Fully replayed: the replay timestamp's age is irrelevant.
        assert_eq!(
            replication_lag(true, Some("0/100"), Some("0/100"), Some(900.0)),
            Duration::ZERO
        );
        assert_eq!(
            replication_lag(true, Some("0/100"), Some("0/200"), Some(900.0)),
            Duration::ZERO
        );
        // Not in recovery: a primary masquerading as a replica.
        assert_eq!(
            replication_lag(false, None, None, Some(900.0)),
            Duration::ZERO
        );
    }

    #[test]
    fn lag_is_the_replay_age_when_behind() {
        assert_eq!(
            replication_lag(true, Some("0/200"), Some("0/100"), Some(2.5)),
            Duration::from_millis(2500)
        );
        // Unknown positions: trust the replay age.
        assert_eq!(
            replication_lag(true, None, None, Some(1.0)),
            Duration::from_secs(1)
        );
        // No replayed transaction yet, or a negative clock skew: zero.
        assert_eq!(
            replication_lag(true, Some("0/200"), Some("0/100"), None),
            Duration::ZERO
        );
        assert_eq!(
            replication_lag(true, Some("0/200"), Some("0/100"), Some(-3.0)),
            Duration::ZERO
        );
    }

    #[test]
    fn url_host_strips_scheme_credentials_and_path() {
        assert_eq!(
            url_host("postgresql://fcp:s3cret@replica-eu.internal:5432/fcp?sslmode=require"),
            "replica-eu.internal:5432"
        );
        assert_eq!(url_host("postgres://db/fcp"), "db");
    }
}
