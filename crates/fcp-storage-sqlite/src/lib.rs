//! SQLite storage backend for FCP.
//!
//! This module provides a production-ready SQLite implementation of the FCP storage traits.
//! It uses sqlx for async database access with connection pooling and migrations.
//!
//! # Features
//! - WAL (Write-Ahead Logging) mode for better concurrency
//! - Connection pooling for performance
//! - Automatic migrations on startup
//! - Full implementation of CredentialStore, RegistrationStore, and CdrStore traits
//!
//! # Example
//! ```no_run
//! use fcp_storage_sqlite::{SqliteStorage, SqliteConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = SqliteConfig::new("/var/lib/fcp/fcp.db");
//!     let storage = SqliteStorage::new(config).await?;
//!
//!     // Use storage via traits
//!     // storage.get_user("alice").await?;
//!
//!     Ok(())
//! }
//! ```

use anyhow::{Context, Result};
use async_trait::async_trait;
use fcp_core::{
    AccountStatus, CallDetailRecord, CallDisposition, CdrQuery, CdrStore, CredentialStore,
    EmergencyContact, HangupCause, HangupParty, MediaPreferences, Registration,
    RegistrationStore, TenantId, User,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Pool, Row, Sqlite, SqlitePool};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

/// Configuration for SQLite storage backend.
#[derive(Debug, Clone)]
pub struct SqliteConfig {
    /// Path to the SQLite database file
    pub database_path: PathBuf,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Connection timeout
    pub connection_timeout: Duration,
    /// Enable WAL (Write-Ahead Logging) mode for better concurrency
    pub enable_wal: bool,
    /// Create database file if it doesn't exist
    pub create_if_missing: bool,
}

impl SqliteConfig {
    /// Create a new SQLite configuration with default settings.
    pub fn new<P: Into<PathBuf>>(database_path: P) -> Self {
        Self {
            database_path: database_path.into(),
            max_connections: 5,
            connection_timeout: Duration::from_secs(30),
            enable_wal: true,
            create_if_missing: true,
        }
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

impl Default for SqliteConfig {
    fn default() -> Self {
        Self::new("/var/lib/fcp/fcp.db")
    }
}

/// SQLite storage backend implementing all FCP storage traits.
#[derive(Clone)]
pub struct SqliteStorage {
    pool: Pool<Sqlite>,
}

impl SqliteStorage {
    /// Create a new SQLite storage backend.
    pub async fn new(config: SqliteConfig) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = config.database_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create database directory")?;
        }

        // Build connection options
        let mut options = SqliteConnectOptions::from_str(&format!(
            "sqlite://{}",
            config.database_path.display()
        ))?
        .create_if_missing(config.create_if_missing);

        // Enable WAL mode for better concurrency
        if config.enable_wal {
            options = options.journal_mode(SqliteJournalMode::Wal);
        }

        // Create connection pool
        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(config.connection_timeout)
            .connect_with(options)
            .await
            .context("Failed to create SQLite connection pool")?;

        info!(
            database = %config.database_path.display(),
            max_connections = config.max_connections,
            "SQLite storage initialized"
        );

        // Run migrations
        Self::run_migrations(&pool).await?;

        Ok(Self { pool })
    }

    /// Run database migrations.
    async fn run_migrations(pool: &SqlitePool) -> Result<()> {
        info!("Running SQLite migrations");

        // Migration 001: Initial schema
        let migration_001 = include_str!("../migrations/001_initial_schema.sql");
        Self::execute_migration(pool, migration_001, "001").await?;

        // Migration 002: Enterprise user fields
        let migration_002 = include_str!("../migrations/002_add_enterprise_user_fields.sql");
        Self::execute_migration(pool, migration_002, "002").await?;

        info!("SQLite migrations completed successfully");
        Ok(())
    }

    /// Execute a single migration file.
    async fn execute_migration(pool: &SqlitePool, migration_sql: &str, version: &str) -> Result<()> {
        // Split by semicolon and filter out comments and empty lines
        let statements: Vec<String> = migration_sql
            .split(';')
            .map(|s| {
                // Remove comments and trim
                s.lines()
                    .filter(|line| !line.trim().starts_with("--"))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .collect();

        for statement in statements {
            debug!(version = %version, statement = %statement, "Executing migration statement");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .context(format!("Failed to execute migration {} statement: {}", version, statement))?;
        }

        Ok(())
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

// ============================================================================
// CredentialStore Implementation
// ============================================================================

#[async_trait]
impl CredentialStore for SqliteStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT username, password_hash, display_name, tenant_id, enabled,
                    created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, language, timezone,
                    account_status, last_login, emergency_contact_name, emergency_contact_phone
             FROM users
             WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_user(row)?)),
            None => Ok(None),
        }
    }

    async fn put_user(&self, user: User) -> Result<()> {
        let tenant_id = user.tenant_id.to_string();
        let enabled = if user.enabled { 1 } else { 0 };
        let created_at = user
            .created_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let updated_at = user
            .updated_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let media_prefs = serde_json::to_string(&user.media_preferences)?;

        let password_last_reset = user.password_last_reset.map(|t| {
            t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
        });
        let last_login = user.last_login.map(|t| {
            t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
        });
        let account_status = user.account_status.to_string();

        sqlx::query(
            "INSERT INTO users (
                username, password_hash, display_name, tenant_id, enabled,
                created_at, updated_at, media_preferences,
                email, password_last_reset, phone_number, mobile_number, pin,
                extension, department, job_title, location, language, timezone,
                account_status, last_login, emergency_contact_name, emergency_contact_phone
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(tenant_id, username) DO UPDATE SET
                 password_hash = excluded.password_hash,
                 display_name = excluded.display_name,
                 enabled = excluded.enabled,
                 updated_at = excluded.updated_at,
                 media_preferences = excluded.media_preferences,
                 email = excluded.email,
                 password_last_reset = excluded.password_last_reset,
                 phone_number = excluded.phone_number,
                 mobile_number = excluded.mobile_number,
                 pin = excluded.pin,
                 extension = excluded.extension,
                 department = excluded.department,
                 job_title = excluded.job_title,
                 location = excluded.location,
                 language = excluded.language,
                 timezone = excluded.timezone,
                 account_status = excluded.account_status,
                 last_login = excluded.last_login,
                 emergency_contact_name = excluded.emergency_contact_name,
                 emergency_contact_phone = excluded.emergency_contact_phone",
        )
        .bind(&user.username)
        .bind(&user.password)
        .bind(&user.display_name)
        .bind(&tenant_id)
        .bind(enabled)
        .bind(created_at)
        .bind(updated_at)
        .bind(&media_prefs)
        .bind(&user.email)
        .bind(password_last_reset)
        .bind(&user.phone_number)
        .bind(&user.mobile_number)
        .bind(&user.pin)
        .bind(&user.extension)
        .bind(&user.department)
        .bind(&user.job_title)
        .bind(&user.location)
        .bind(&user.language)
        .bind(&user.timezone)
        .bind(&account_status)
        .bind(last_login)
        .bind(&user.emergency_contact.name)
        .bind(&user.emergency_contact.phone)
        .execute(&self.pool)
        .await?;

        debug!(username = %user.username, "User stored in SQLite");
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        sqlx::query("DELETE FROM users WHERE username = ?")
            .bind(username)
            .execute(&self.pool)
            .await?;

        debug!(username = %username, "User deleted from SQLite");
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT username, password_hash, display_name, tenant_id, enabled,
                    created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, language, timezone,
                    account_status, last_login, emergency_contact_name, emergency_contact_phone
             FROM users
             ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await?;

        let users = rows
            .into_iter()
            .filter_map(|row| row_to_user(row).ok())
            .collect();

        Ok(users)
    }
}

// ============================================================================
// RegistrationStore Implementation
// ============================================================================

#[async_trait]
impl RegistrationStore for SqliteStorage {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        let rows = sqlx::query(
            "SELECT aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at
             FROM registrations
             WHERE aor = ?
             ORDER BY q DESC",
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

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        let expires_at = registration
            .expires_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let registered_at = registration
            .registered_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            "INSERT INTO registrations (aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(aor, contact) DO UPDATE SET
                 expires_at = excluded.expires_at,
                 call_id = excluded.call_id,
                 cseq = excluded.cseq,
                 user_agent = excluded.user_agent,
                 q = excluded.q,
                 registered_at = excluded.registered_at",
        )
        .bind(&registration.aor)
        .bind(&registration.contact)
        .bind(expires_at)
        .bind(&registration.call_id)
        .bind(registration.cseq as i64)
        .bind(&registration.user_agent)
        .bind(registration.q)
        .bind(registered_at)
        .execute(&self.pool)
        .await?;

        debug!(aor = %registration.aor, contact = %registration.contact, "Registration stored");
        Ok(())
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        sqlx::query("DELETE FROM registrations WHERE aor = ? AND contact = ?")
            .bind(aor)
            .bind(contact)
            .execute(&self.pool)
            .await?;

        debug!(aor = %aor, contact = %contact, "Registration deleted");
        Ok(())
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM registrations WHERE aor = ?")
            .bind(aor)
            .execute(&self.pool)
            .await?;

        debug!(aor = %aor, "All registrations deleted");
        Ok(())
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let result = sqlx::query("DELETE FROM registrations WHERE expires_at <= ?")
            .bind(now)
            .execute(&self.pool)
            .await?;

        let count = result.rows_affected() as usize;
        if count > 0 {
            info!(count = count, "Cleaned up expired registrations");
        }

        Ok(count)
    }
}

// ============================================================================
// CdrStore Implementation
// ============================================================================

#[async_trait]
impl CdrStore for SqliteStorage {
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()> {
        let start_time = cdr
            .start_time
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let answer_time = cdr.answer_time.map(|t| {
            t.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        });
        let end_time = cdr.end_time.map(|t| {
            t.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        });

        let disposition = serde_json::to_string(&cdr.disposition)?.trim_matches('"').to_string();
        let hangup_cause = match cdr.hangup_cause {
            HangupCause::Other(code) => format!("other:{}", code),
            _ => serde_json::to_string(&cdr.hangup_cause)?
                .trim_matches('"')
                .to_string(),
        };
        let hangup_party = cdr
            .hangup_party
            .map(|p| serde_json::to_string(&p).unwrap_or_default().trim_matches('"').to_string());

        let custom_data = cdr
            .custom_data
            .map(|v| serde_json::to_string(&v))
            .transpose()?;

        sqlx::query(
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
                recording_url, cost, currency, custom_data
             ) VALUES (
                ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?, ?,
                ?, ?, ?,
                ?, ?, ?,
                ?, ?,
                ?, ?,
                ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?, ?, ?
             )",
        )
        .bind(cdr.call_id.to_string())
        .bind(cdr.tenant_id.to_string())
        .bind(&cdr.sip_call_id)
        .bind(&cdr.from_uri)
        .bind(&cdr.from_display_name)
        .bind(&cdr.to_uri)
        .bind(&cdr.to_display_name)
        .bind(start_time)
        .bind(answer_time)
        .bind(end_time)
        .bind(cdr.setup_duration_ms.map(|d| d as i64))
        .bind(cdr.call_duration_ms.map(|d| d as i64))
        .bind(cdr.total_duration_ms.map(|d| d as i64))
        .bind(&disposition)
        .bind(&hangup_cause)
        .bind(&hangup_party)
        .bind(cdr.sip_status_code.map(|c| c as i64))
        .bind(&cdr.sip_reason)
        .bind(&cdr.route_id)
        .bind(&cdr.gateway_id)
        .bind(if cdr.media_anchored { 1 } else { 0 })
        .bind(&cdr.codec_a_leg)
        .bind(&cdr.codec_b_leg)
        .bind(cdr.rtp_packets_sent.map(|p| p as i64))
        .bind(cdr.rtp_packets_received.map(|p| p as i64))
        .bind(cdr.rtp_bytes_sent.map(|b| b as i64))
        .bind(cdr.rtp_bytes_received.map(|b| b as i64))
        .bind(&cdr.recording_url)
        .bind(cdr.cost)
        .bind(&cdr.currency)
        .bind(&custom_data)
        .execute(&self.pool)
        .await?;

        debug!(call_id = %cdr.call_id, "CDR stored in SQLite");
        Ok(())
    }

    async fn get_cdr(&self, call_id: &fcp_core::CallId) -> Result<Option<CallDetailRecord>> {
        let row = sqlx::query("SELECT * FROM cdrs WHERE call_id = ?")
            .bind(call_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => Ok(Some(row_to_cdr(row)?)),
            None => Ok(None),
        }
    }

    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>> {
        let mut sql = String::from("SELECT * FROM cdrs WHERE 1=1");
        let mut conditions = Vec::new();

        if let Some(tenant_id) = &query.tenant_id {
            conditions.push(("tenant_id", tenant_id.to_string()));
        }

        if let Some(call_id) = &query.call_id {
            conditions.push(("call_id", call_id.to_string()));
        }

        if let Some(from_uri) = &query.from_uri {
            conditions.push(("from_uri_like", format!("%{}%", from_uri)));
        }

        if let Some(to_uri) = &query.to_uri {
            conditions.push(("to_uri_like", format!("%{}%", to_uri)));
        }

        if let Some(start_from) = query.start_time_from {
            let ts = start_from
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            conditions.push(("start_time_from", ts.to_string()));
        }

        if let Some(start_to) = query.start_time_to {
            let ts = start_to
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            conditions.push(("start_time_to", ts.to_string()));
        }

        if let Some(disposition) = &query.disposition {
            let disp_str = serde_json::to_string(disposition)?
                .trim_matches('"')
                .to_string();
            conditions.push(("disposition", disp_str));
        }

        // Build dynamic query
        for (key, _value) in &conditions {
            match *key {
                "from_uri_like" => {
                    sql.push_str(" AND from_uri LIKE ?");
                }
                "to_uri_like" => {
                    sql.push_str(" AND to_uri LIKE ?");
                }
                "start_time_from" => {
                    sql.push_str(" AND start_time >= ?");
                }
                "start_time_to" => {
                    sql.push_str(" AND start_time <= ?");
                }
                _ => {
                    sql.push_str(&format!(" AND {} = ?", key));
                }
            }
        }

        sql.push_str(" ORDER BY start_time DESC");

        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }

        if let Some(offset) = query.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }

        // Execute query with bindings
        let mut final_query = sqlx::query(&sql);
        for (_, value) in conditions {
            final_query = final_query.bind(value);
        }

        let rows = final_query.fetch_all(&self.pool).await?;

        let cdrs = rows
            .into_iter()
            .filter_map(|row| row_to_cdr(row).ok())
            .collect();

        Ok(cdrs)
    }

    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize> {
        let mut sql = String::from("SELECT COUNT(*) as count FROM cdrs WHERE 1=1");
        let mut conditions = Vec::new();

        if let Some(tenant_id) = &query.tenant_id {
            sql.push_str(" AND tenant_id = ?");
            conditions.push(tenant_id.to_string());
        }

        if let Some(disposition) = &query.disposition {
            sql.push_str(" AND disposition = ?");
            let disp_str = serde_json::to_string(disposition)?
                .trim_matches('"')
                .to_string();
            conditions.push(disp_str);
        }

        let mut query_builder = sqlx::query(&sql);
        for condition in conditions {
            query_builder = query_builder.bind(condition);
        }

        let row = query_builder.fetch_one(&self.pool).await?;
        let count: i64 = row.get("count");

        Ok(count as usize)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert SQLite row to User
fn row_to_user(row: sqlx::sqlite::SqliteRow) -> Result<User> {
    use fcp_core::{AccountStatus, EmergencyContact};

    let media_prefs: Option<String> = row.get("media_preferences");
    let media_preferences = if let Some(json) = media_prefs {
        serde_json::from_str(&json).unwrap_or_default()
    } else {
        MediaPreferences::default()
    };

    let password_last_reset = row
        .get::<Option<i64>, _>("password_last_reset")
        .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64));

    let last_login = row
        .get::<Option<i64>, _>("last_login")
        .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64));

    let account_status_str: String = row.get("account_status");
    let account_status = AccountStatus::from_str(&account_status_str).unwrap_or_default();

    let emergency_contact = EmergencyContact {
        name: row.get("emergency_contact_name"),
        phone: row.get("emergency_contact_phone"),
    };

    Ok(User {
        username: row.get("username"),
        password: row.get("password_hash"),
        display_name: row.get("display_name"),
        tenant_id: TenantId::from_str(&row.get::<String, _>("tenant_id"))
            .unwrap_or_else(|_| TenantId::nil()),
        enabled: row.get::<i64, _>("enabled") != 0,
        created_at: UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("created_at") as u64),
        updated_at: UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("updated_at") as u64),
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
        language: row.get("language"),
        timezone: row.get("timezone"),
        account_status,
        last_login,
        emergency_contact,
    })
}

/// Convert SQLite row to Registration
fn row_to_registration(row: sqlx::sqlite::SqliteRow) -> Result<Registration> {
    Ok(Registration {
        aor: row.get("aor"),
        contact: row.get("contact"),
        expires_at: UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("expires_at") as u64),
        call_id: row.get("call_id"),
        cseq: row.get::<i64, _>("cseq") as u32,
        user_agent: row.get("user_agent"),
        q: row.get("q"),
        registered_at: UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("registered_at") as u64),
    })
}

/// Convert SQLite row to CallDetailRecord
fn row_to_cdr(row: sqlx::sqlite::SqliteRow) -> Result<CallDetailRecord> {
    let call_id = fcp_core::CallId::from_str(&row.get::<String, _>("call_id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;

    let disposition_str: String = row.get("disposition");
    let disposition: CallDisposition =
        serde_json::from_str(&format!("\"{}\"", disposition_str))?;

    let hangup_cause_str: String = row.get("hangup_cause");
    let hangup_cause = if hangup_cause_str.starts_with("other:") {
        let code = hangup_cause_str
            .strip_prefix("other:")
            .unwrap()
            .parse()
            .unwrap_or(0);
        HangupCause::Other(code)
    } else {
        serde_json::from_str(&format!("\"{}\"", hangup_cause_str))?
    };

    let hangup_party: Option<HangupParty> = row
        .get::<Option<String>, _>("hangup_party")
        .and_then(|s| serde_json::from_str(&format!("\"{}\"", s)).ok());

    let custom_data: Option<serde_json::Value> = row
        .get::<Option<String>, _>("custom_data")
        .and_then(|s| serde_json::from_str(&s).ok());

    Ok(CallDetailRecord {
        call_id,
        tenant_id,
        sip_call_id: row.get("sip_call_id"),
        from_uri: row.get("from_uri"),
        from_display_name: row.get("from_display_name"),
        to_uri: row.get("to_uri"),
        to_display_name: row.get("to_display_name"),
        start_time: UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("start_time") as u64),
        answer_time: row
            .get::<Option<i64>, _>("answer_time")
            .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64)),
        end_time: row
            .get::<Option<i64>, _>("end_time")
            .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64)),
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
            .get::<Option<i64>, _>("sip_status_code")
            .map(|c| c as u16),
        sip_reason: row.get("sip_reason"),
        route_id: row.get("route_id"),
        gateway_id: row.get("gateway_id"),
        media_anchored: row.get::<i64, _>("media_anchored") != 0,
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
        recording_url: row.get("recording_url"),
        cost: row.get("cost"),
        currency: row.get("currency"),
        custom_data,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::CallId;
    use tempfile::TempDir;

    async fn create_test_storage() -> (SqliteStorage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let config = SqliteConfig::new(db_path);
        let storage = SqliteStorage::new(config).await.unwrap();
        (storage, temp_dir)
    }

    #[tokio::test]
    async fn test_user_crud() {
        let (storage, _temp) = create_test_storage().await;

        // Create user
        let user = User {
            username: "alice".to_string(),
            password: "hashed_password".to_string(),
            display_name: Some("Alice".to_string()),
            tenant_id: TenantId::nil(),
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            media_preferences: MediaPreferences::default(),
            email: None,
            password_last_reset: None,
            phone_number: None,
            mobile_number: None,
            pin: None,
            extension: None,
            department: None,
            job_title: None,
            location: None,
            language: "en-US".to_string(),
            timezone: "UTC".to_string(),
            account_status: AccountStatus::Active,
            last_login: None,
            emergency_contact: EmergencyContact::default(),
        };

        storage.put_user(user.clone()).await.unwrap();

        // Get user
        let retrieved = storage.get_user("alice").await.unwrap().unwrap();
        assert_eq!(retrieved.username, "alice");
        assert_eq!(retrieved.display_name, Some("Alice".to_string()));

        // List users
        let users = storage.list_users().await.unwrap();
        assert_eq!(users.len(), 1);

        // Update user
        let mut updated_user = retrieved.clone();
        updated_user.display_name = Some("Alice Updated".to_string());
        storage.put_user(updated_user).await.unwrap();

        let retrieved = storage.get_user("alice").await.unwrap().unwrap();
        assert_eq!(retrieved.display_name, Some("Alice Updated".to_string()));

        // Delete user
        storage.delete_user("alice").await.unwrap();
        assert!(storage.get_user("alice").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_registration_crud() {
        let (storage, _temp) = create_test_storage().await;

        // Create registration
        let reg = Registration {
            aor: "sip:alice@example.com".to_string(),
            contact: "sip:alice@192.168.1.100:5060".to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            call_id: "test-call-id".to_string(),
            cseq: 1,
            user_agent: Some("Test UA".to_string()),
            q: Some(1.0),
            registered_at: SystemTime::now(),
        };

        storage.put_registration(reg.clone()).await.unwrap();

        // Get registrations
        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].contact, "sip:alice@192.168.1.100:5060");

        // Add another registration for same AOR
        let reg2 = Registration {
            aor: "sip:alice@example.com".to_string(),
            contact: "sip:alice@192.168.1.101:5060".to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            call_id: "test-call-id-2".to_string(),
            cseq: 1,
            user_agent: Some("Test UA 2".to_string()),
            q: Some(0.5),
            registered_at: SystemTime::now(),
        };

        storage.put_registration(reg2).await.unwrap();

        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 2);

        // Delete specific registration
        storage
            .delete_registration("sip:alice@example.com", "sip:alice@192.168.1.100:5060")
            .await
            .unwrap();

        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 1);

        // Delete all registrations
        storage
            .delete_all_registrations("sip:alice@example.com")
            .await
            .unwrap();

        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 0);
    }

    #[tokio::test]
    async fn test_registration_cleanup() {
        let (storage, _temp) = create_test_storage().await;

        // Create expired registration
        let reg = Registration {
            aor: "sip:bob@example.com".to_string(),
            contact: "sip:bob@192.168.1.100:5060".to_string(),
            expires_at: SystemTime::now() - Duration::from_secs(60), // Expired
            call_id: "expired-call-id".to_string(),
            cseq: 1,
            user_agent: Some("Test UA".to_string()),
            q: Some(1.0),
            registered_at: SystemTime::now() - Duration::from_secs(3660),
        };

        storage.put_registration(reg).await.unwrap();

        // Cleanup expired
        let count = storage.cleanup_expired().await.unwrap();
        assert_eq!(count, 1);

        // Verify it's gone
        let regs = storage
            .get_registrations("sip:bob@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 0);
    }

    #[tokio::test]
    async fn test_cdr_storage() {
        let (storage, _temp) = create_test_storage().await;

        // Create CDR
        let cdr = CallDetailRecord {
            call_id: CallId::new(),
            tenant_id: TenantId::nil(),
            sip_call_id: "test-sip-call-id".to_string(),
            from_uri: "sip:alice@example.com".to_string(),
            from_display_name: Some("Alice".to_string()),
            to_uri: "sip:bob@example.com".to_string(),
            to_display_name: Some("Bob".to_string()),
            start_time: SystemTime::now(),
            answer_time: Some(SystemTime::now()),
            end_time: Some(SystemTime::now()),
            setup_duration_ms: Some(1000),
            call_duration_ms: Some(30000),
            total_duration_ms: Some(31000),
            disposition: CallDisposition::Answered,
            hangup_cause: HangupCause::NormalClearing,
            hangup_party: Some(HangupParty::Caller),
            sip_status_code: Some(200),
            sip_reason: Some("OK".to_string()),
            route_id: None,
            gateway_id: None,
            media_anchored: true,
            codec_a_leg: Some("PCMU".to_string()),
            codec_b_leg: Some("PCMU".to_string()),
            rtp_packets_sent: Some(1500),
            rtp_packets_received: Some(1450),
            rtp_bytes_sent: Some(240000),
            rtp_bytes_received: Some(232000),
            recording_url: None,
            cost: Some(0.05),
            currency: Some("USD".to_string()),
            custom_data: None,
        };

        storage.insert_cdr(cdr.clone()).await.unwrap();

        // Get CDR
        let retrieved = storage.get_cdr(&cdr.call_id).await.unwrap().unwrap();
        assert_eq!(retrieved.sip_call_id, "test-sip-call-id");
        assert_eq!(retrieved.disposition, CallDisposition::Answered);
        assert_eq!(retrieved.hangup_cause, HangupCause::NormalClearing);
        assert_eq!(retrieved.hangup_party, Some(HangupParty::Caller));

        // Query CDRs
        let query = CdrQuery {
            tenant_id: Some(TenantId::nil()),
            ..Default::default()
        };
        let cdrs = storage.query_cdrs(query).await.unwrap();
        assert_eq!(cdrs.len(), 1);

        // Count CDRs
        let query = CdrQuery {
            tenant_id: Some(TenantId::nil()),
            disposition: Some(CallDisposition::Answered),
            ..Default::default()
        };
        let count = storage.count_cdrs(query).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_cdr_query_filters() {
        let (storage, _temp) = create_test_storage().await;

        // Insert multiple CDRs
        for i in 0..5 {
            let cdr = CallDetailRecord {
                call_id: CallId::new(),
                tenant_id: TenantId::nil(),
                sip_call_id: format!("sip-call-{}", i),
                from_uri: format!("sip:alice{}@example.com", i),
                from_display_name: Some("Alice".to_string()),
                to_uri: "sip:bob@example.com".to_string(),
                to_display_name: Some("Bob".to_string()),
                start_time: SystemTime::now(),
                answer_time: Some(SystemTime::now()),
                end_time: Some(SystemTime::now()),
                setup_duration_ms: Some(1000),
                call_duration_ms: Some(30000),
                total_duration_ms: Some(31000),
                disposition: if i % 2 == 0 {
                    CallDisposition::Answered
                } else {
                    CallDisposition::Busy
                },
                hangup_cause: HangupCause::NormalClearing,
                hangup_party: Some(HangupParty::Caller),
                sip_status_code: Some(200),
                sip_reason: Some("OK".to_string()),
                route_id: None,
                gateway_id: None,
                media_anchored: true,
                codec_a_leg: Some("PCMU".to_string()),
                codec_b_leg: Some("PCMU".to_string()),
                rtp_packets_sent: Some(1500),
                rtp_packets_received: Some(1450),
                rtp_bytes_sent: Some(240000),
                rtp_bytes_received: Some(232000),
                recording_url: None,
                cost: Some(0.05),
                currency: Some("USD".to_string()),
                custom_data: None,
            };
            storage.insert_cdr(cdr).await.unwrap();
        }

        // Query by disposition
        let query = CdrQuery {
            disposition: Some(CallDisposition::Answered),
            ..Default::default()
        };
        let cdrs = storage.query_cdrs(query).await.unwrap();
        assert_eq!(cdrs.len(), 3); // 0, 2, 4

        // Query with limit
        let query = CdrQuery {
            limit: Some(2),
            ..Default::default()
        };
        let cdrs = storage.query_cdrs(query).await.unwrap();
        assert_eq!(cdrs.len(), 2);

        // Query by from_uri pattern
        let query = CdrQuery {
            from_uri: Some("alice0".to_string()),
            ..Default::default()
        };
        let cdrs = storage.query_cdrs(query).await.unwrap();
        assert_eq!(cdrs.len(), 1);
    }

    #[tokio::test]
    async fn test_hangup_cause_other() {
        let (storage, _temp) = create_test_storage().await;

        // Create CDR with Other hangup cause
        let cdr = CallDetailRecord {
            call_id: CallId::new(),
            tenant_id: TenantId::nil(),
            sip_call_id: "test-other-cause".to_string(),
            from_uri: "sip:alice@example.com".to_string(),
            from_display_name: None,
            to_uri: "sip:bob@example.com".to_string(),
            to_display_name: None,
            start_time: SystemTime::now(),
            answer_time: None,
            end_time: Some(SystemTime::now()),
            setup_duration_ms: None,
            call_duration_ms: None,
            total_duration_ms: Some(5000),
            disposition: CallDisposition::Failed,
            hangup_cause: HangupCause::Other(503), // Service Unavailable
            hangup_party: Some(HangupParty::System),
            sip_status_code: Some(503),
            sip_reason: Some("Service Unavailable".to_string()),
            route_id: None,
            gateway_id: None,
            media_anchored: false,
            codec_a_leg: None,
            codec_b_leg: None,
            rtp_packets_sent: None,
            rtp_packets_received: None,
            rtp_bytes_sent: None,
            rtp_bytes_received: None,
            recording_url: None,
            cost: None,
            currency: None,
            custom_data: None,
        };

        storage.insert_cdr(cdr.clone()).await.unwrap();

        // Retrieve and verify
        let retrieved = storage.get_cdr(&cdr.call_id).await.unwrap().unwrap();
        assert_eq!(retrieved.hangup_cause, HangupCause::Other(503));
    }
}
