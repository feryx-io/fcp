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
    storage::{
        CdrStore, CredentialStore, DeviceStore, ExtensionStore, LineBindingStore,
        PhoneNumberStore, RegistrationStore,
    }, AudioCodec, BindingType, CallDetailRecord, CallDisposition, CdrQuery, Device,
    DeviceBinding, DeviceId, DeviceType, Extension, ExtensionId,
    ExtensionTarget, HangupCause, HangupParty, LineBinding, LineBindingId, MediaPreferences,
    NumberState, PhoneNumber, PhoneNumberId, Registration, RegistrationState, RingStrategy,
    TenantId, User, UserId,
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

        // Create migration tracking table if it doesn't exist
        Self::create_migration_table(pool).await?;

        // Migration 001: Initial schema
        let migration_001 = include_str!("../migrations/001_initial_schema.sql");
        Self::execute_migration(pool, migration_001, "001").await?;

        // Migration 002: Enterprise user fields
        let migration_002 = include_str!("../migrations/002_add_enterprise_user_fields.sql");
        Self::execute_migration(pool, migration_002, "002").await?;

        // Migration 003: Digest credentials (Cisco-style separate SIP password)
        let migration_003 = include_str!("../migrations/003_add_sip_ha1_hash.sql");
        Self::execute_migration(pool, migration_003, "003").await?;

        // Migration 004: Phone number support
        let migration_004 = include_str!("../migrations/004_add_phone_number_support.sql");
        Self::execute_migration(pool, migration_004, "004").await?;

        info!("SQLite migrations completed successfully");
        Ok(())
    }

    /// Create the schema_migrations table to track applied migrations.
    async fn create_migration_table(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            )"
        )
        .execute(pool)
        .await
        .context("Failed to create schema_migrations table")?;

        Ok(())
    }

    /// Check if a migration has already been applied.
    async fn is_migration_applied(pool: &SqlitePool, version: &str) -> Result<bool> {
        let result = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?"
        )
        .bind(version)
        .fetch_one(pool)
        .await?;

        Ok(result > 0)
    }

    /// Record a successfully applied migration.
    async fn record_migration(pool: &SqlitePool, version: &str) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?, ?)"
        )
        .bind(version)
        .bind(now)
        .execute(pool)
        .await
        .context(format!("Failed to record migration {}", version))?;

        Ok(())
    }

    /// Execute a single migration file.
    async fn execute_migration(pool: &SqlitePool, migration_sql: &str, version: &str) -> Result<()> {
        // Check if migration has already been applied
        if Self::is_migration_applied(pool, version).await? {
            debug!(version = %version, "Migration already applied, skipping");
            return Ok(());
        }

        info!(version = %version, "Applying migration");

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

        // Record successful migration
        Self::record_migration(pool, version).await?;
        info!(version = %version, "Migration applied successfully");

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
            "SELECT username, password_hash, digest_password, sip_ha1_hash, display_name, tenant_id, enabled,
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
                username, password_hash, digest_password, sip_ha1_hash, display_name, tenant_id, enabled,
                created_at, updated_at, media_preferences,
                email, password_last_reset, phone_number, mobile_number, pin,
                extension, department, job_title, location, language, timezone,
                account_status, last_login, emergency_contact_name, emergency_contact_phone
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(tenant_id, username) DO UPDATE SET
                 password_hash = excluded.password_hash,
                 digest_password = excluded.digest_password,
                 sip_ha1_hash = excluded.sip_ha1_hash,
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
        .bind(&user.digest_password)
        .bind(&user.sip_ha1_hash)
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
            "SELECT username, password_hash, digest_password, sip_ha1_hash, display_name, tenant_id, enabled,
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

/// Parse role string from database to UserRole enum
fn parse_role(role_str: &str) -> fcp_core::UserRole {
    use fcp_core::UserRole;

    // Try deserializing with serde first (handles snake_case like "telecom_admin")
    if let Ok(role) = serde_json::from_str::<UserRole>(&format!("\"{}\"", role_str)) {
        return role;
    }

    // Fallback to manual matching for backward compatibility
    match role_str.to_lowercase().as_str() {
        "superadmin" | "super_admin" => UserRole::SuperAdmin,
        "telecomadmin" | "telecom_admin" => UserRole::TelecomAdmin,
        "useradmin" | "user_admin" => UserRole::UserAdmin,
        "departmentadmin" | "department_admin" => UserRole::DepartmentAdmin,
        "readonly" | "read_only" => UserRole::ReadOnly,
        "enduser" | "end_user" => UserRole::EndUser,
        _ => {
            tracing::warn!("Unknown role '{}', defaulting to EndUser", role_str);
            UserRole::EndUser
        }
    }
}

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

    // Parse role from database (defaults to EndUser if not found or invalid)
    let role_str: String = row.try_get("role").unwrap_or_else(|_| "enduser".to_string());
    let role = parse_role(&role_str);

    let emergency_contact = EmergencyContact {
        name: row.get("emergency_contact_name"),
        phone: row.get("emergency_contact_phone"),
    };

    Ok(User {
        username: row.get("username"),
        password: row.get("password_hash"),
        digest_password: row.try_get("digest_password").ok(),
        sip_ha1_hash: row.try_get("sip_ha1_hash").ok(),
        display_name: row.get("display_name"),
        tenant_id: TenantId::from_str(&row.get::<String, _>("tenant_id"))
            .unwrap_or_else(|_| TenantId::nil()),
        enabled: row.get::<i64, _>("enabled") != 0,
        role, // User role for RBAC
        department_id: row.try_get("department_id").ok(), // Department ID for scoping
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
        primary_phone_number_id: None, // TODO: Add to database schema
        voicemail_box_id: None,        // TODO: Add to database schema
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
// PhoneNumberStore Implementation
// ============================================================================

#[async_trait]
impl PhoneNumberStore for SqliteStorage {
    async fn get_phone_number(&self, id: &PhoneNumberId) -> Result<Option<PhoneNumber>> {
        let row = sqlx::query(
            "SELECT id, e164, country_code, npa, nxx, line, state, tenant_id,
                    e911_address_json, e911_callback, cnam, spam_score,
                    carrier, lrn, ocn, rate_center, lata,
                    acquired_at, activated_at, ported_at, loa_date, foc_date,
                    recording_enabled, block_anonymous, allow_list, block_list, tags,
                    created_at, updated_at
             FROM phone_numbers
             WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
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
                    created_at, updated_at
             FROM phone_numbers
             WHERE e164 = ?",
        )
        .bind(e164)
        .fetch_optional(&self.pool)
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
                    created_at, updated_at
             FROM phone_numbers
             WHERE tenant_id = ?
             ORDER BY e164",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
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
            .map(|a| serde_json::to_string(&a))
            .transpose()?;
        let allow_list = serde_json::to_string(&phone_number.allow_list)?;
        let block_list = serde_json::to_string(&phone_number.block_list)?;
        let tags = serde_json::to_string(&phone_number.tags)?;
        let acquired_at = phone_number
            .acquired_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let activated_at = phone_number.activated_at.map(|t| {
            t.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        });
        let ported_at = phone_number.ported_at.map(|t| {
            t.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        });
        let loa_date = phone_number.loa_date.map(|t| {
            t.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        });
        let foc_date = phone_number.foc_date.map(|t| {
            t.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        });
        let created_at = phone_number
            .created_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let updated_at = phone_number
            .updated_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            "INSERT INTO phone_numbers (
                id, e164, country_code, npa, nxx, line, state, tenant_id,
                e911_address_json, e911_callback, cnam, spam_score,
                carrier, lrn, ocn, rate_center, lata,
                acquired_at, activated_at, ported_at, loa_date, foc_date,
                recording_enabled, block_anonymous, allow_list, block_list, tags,
                created_at, updated_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 state = excluded.state,
                 e911_address_json = excluded.e911_address_json,
                 e911_callback = excluded.e911_callback,
                 cnam = excluded.cnam,
                 spam_score = excluded.spam_score,
                 carrier = excluded.carrier,
                 lrn = excluded.lrn,
                 ocn = excluded.ocn,
                 rate_center = excluded.rate_center,
                 lata = excluded.lata,
                 activated_at = excluded.activated_at,
                 ported_at = excluded.ported_at,
                 loa_date = excluded.loa_date,
                 foc_date = excluded.foc_date,
                 recording_enabled = excluded.recording_enabled,
                 block_anonymous = excluded.block_anonymous,
                 allow_list = excluded.allow_list,
                 block_list = excluded.block_list,
                 tags = excluded.tags,
                 updated_at = excluded.updated_at",
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
        .bind(if phone_number.recording_enabled { 1 } else { 0 })
        .bind(if phone_number.block_anonymous { 1 } else { 0 })
        .bind(&allow_list)
        .bind(&block_list)
        .bind(&tags)
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        debug!(id = %phone_number.id, e164 = %phone_number.e164, "Phone number stored");
        Ok(())
    }

    async fn delete_phone_number(&self, id: &PhoneNumberId) -> Result<()> {
        sqlx::query("DELETE FROM phone_numbers WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        debug!(id = %id, "Phone number deleted");
        Ok(())
    }
}

// ============================================================================
// LineBindingStore Implementation
// ============================================================================

#[async_trait]
impl LineBindingStore for SqliteStorage {
    async fn get_line_binding(&self, id: &LineBindingId) -> Result<Option<LineBinding>> {
        let row = sqlx::query(
            "SELECT id, phone_number_id, owner_user_id, binding_type, ring_strategy, tenant_id,
                    no_answer_timeout_secs, forward_on_busy, forward_on_no_answer,
                    forward_unconditional, outbound_caller_id, outbound_cnam,
                    created_at, updated_at
             FROM line_bindings
             WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
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
                    no_answer_timeout_secs, forward_on_busy, forward_on_no_answer,
                    forward_unconditional, outbound_caller_id, outbound_cnam,
                    created_at, updated_at
             FROM line_bindings
             WHERE phone_number_id = ?",
        )
        .bind(phone_number_id.to_string())
        .fetch_optional(&self.pool)
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
                    no_answer_timeout_secs, forward_on_busy, forward_on_no_answer,
                    forward_unconditional, outbound_caller_id, outbound_cnam,
                    created_at, updated_at
             FROM line_bindings
             WHERE tenant_id = ?
             ORDER BY created_at",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
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
                    no_answer_timeout_secs, forward_on_busy, forward_on_no_answer,
                    forward_unconditional, outbound_caller_id, outbound_cnam,
                    created_at, updated_at
             FROM line_bindings
             WHERE owner_user_id = ?
             ORDER BY created_at",
        )
        .bind(user_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut bindings = Vec::new();
        for row in rows {
            let binding = row_to_line_binding(row)?;
            let devices = self.get_device_bindings(&binding.id).await?;
            bindings.push(LineBinding { devices, ..binding });
        }

        Ok(bindings)
    }

    async fn list_line_bindings_by_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Vec<LineBinding>> {
        let rows = sqlx::query(
            "SELECT lb.id, lb.phone_number_id, lb.owner_user_id, lb.binding_type, lb.ring_strategy, lb.tenant_id,
                    lb.no_answer_timeout_secs, lb.forward_on_busy, lb.forward_on_no_answer,
                    lb.forward_unconditional, lb.outbound_caller_id, lb.outbound_cnam,
                    lb.created_at, lb.updated_at
             FROM line_bindings lb
             INNER JOIN device_bindings db ON lb.id = db.line_binding_id
             WHERE db.device_id = ?
             ORDER BY lb.created_at",
        )
        .bind(device_id.to_string())
        .fetch_all(&self.pool)
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
        let created_at = binding
            .created_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let updated_at = binding
            .updated_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            "INSERT INTO line_bindings (
                id, phone_number_id, owner_user_id, binding_type, ring_strategy, tenant_id,
                no_answer_timeout_secs, forward_on_busy, forward_on_no_answer,
                forward_unconditional, outbound_caller_id, outbound_cnam,
                created_at, updated_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 ring_strategy = excluded.ring_strategy,
                 no_answer_timeout_secs = excluded.no_answer_timeout_secs,
                 forward_on_busy = excluded.forward_on_busy,
                 forward_on_no_answer = excluded.forward_on_no_answer,
                 forward_unconditional = excluded.forward_unconditional,
                 outbound_caller_id = excluded.outbound_caller_id,
                 outbound_cnam = excluded.outbound_cnam,
                 updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&phone_number_id)
        .bind(&owner_user_id)
        .bind(&binding_type)
        .bind(&ring_strategy)
        .bind(&tenant_id)
        .bind(binding.no_answer_timeout.as_secs() as i64)
        .bind(&binding.forward_on_busy)
        .bind(&binding.forward_on_no_answer)
        .bind(&binding.forward_unconditional)
        .bind(&binding.outbound_caller_id)
        .bind(&binding.outbound_cnam)
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        // Delete existing device bindings
        sqlx::query("DELETE FROM device_bindings WHERE line_binding_id = ?")
            .bind(&id)
            .execute(&self.pool)
            .await?;

        // Insert new device bindings
        for device in &binding.devices {
            self.put_device_binding(&binding.id, device).await?;
        }

        debug!(id = %binding.id, "Line binding stored");
        Ok(())
    }

    async fn delete_line_binding(&self, id: &LineBindingId) -> Result<()> {
        sqlx::query("DELETE FROM line_bindings WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        debug!(id = %id, "Line binding deleted");
        Ok(())
    }
}

impl SqliteStorage {
    async fn get_device_bindings(&self, line_binding_id: &LineBindingId) -> Result<Vec<DeviceBinding>> {
        let rows = sqlx::query(
            "SELECT device_id, priority, line_appearance, shared_line, enabled
             FROM device_bindings
             WHERE line_binding_id = ?
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
                    priority: row.get::<i64, _>("priority") as u8,
                    line_appearance: row.get::<i64, _>("line_appearance") as u8,
                    shared_line: row.get::<i64, _>("shared_line") != 0,
                    enabled: row.get::<i64, _>("enabled") != 0,
                })
            })
            .collect();

        Ok(bindings)
    }

    async fn put_device_binding(
        &self,
        line_binding_id: &LineBindingId,
        device: &DeviceBinding,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO device_bindings (line_binding_id, device_id, priority, line_appearance, shared_line, enabled)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(line_binding_id.to_string())
        .bind(device.device_id.to_string())
        .bind(device.priority as i64)
        .bind(device.line_appearance as i64)
        .bind(if device.shared_line { 1 } else { 0 })
        .bind(if device.enabled { 1 } else { 0 })
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

// ============================================================================
// ExtensionStore Implementation
// ============================================================================

#[async_trait]
impl ExtensionStore for SqliteStorage {
    async fn get_extension(&self, id: &ExtensionId) -> Result<Option<Extension>> {
        let row = sqlx::query(
            "SELECT id, number, target_type, target_id, tenant_id, description, enabled,
                    created_at, updated_at
             FROM extensions
             WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
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
            "SELECT id, number, target_type, target_id, tenant_id, description, enabled,
                    created_at, updated_at
             FROM extensions
             WHERE tenant_id = ? AND number = ?",
        )
        .bind(tenant_id.to_string())
        .bind(number)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_extension(row)?)),
            None => Ok(None),
        }
    }

    async fn list_extensions(&self, tenant_id: &TenantId) -> Result<Vec<Extension>> {
        let rows = sqlx::query(
            "SELECT id, number, target_type, target_id, tenant_id, description, enabled,
                    created_at, updated_at
             FROM extensions
             WHERE tenant_id = ?
             ORDER BY number",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
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
        let created_at = extension
            .created_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let updated_at = extension
            .updated_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            "INSERT INTO extensions (
                id, number, target_type, target_id, tenant_id, description, enabled,
                created_at, updated_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(tenant_id, number) DO UPDATE SET
                 target_type = excluded.target_type,
                 target_id = excluded.target_id,
                 description = excluded.description,
                 enabled = excluded.enabled,
                 updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&extension.number)
        .bind(target_type)
        .bind(&target_id)
        .bind(&tenant_id)
        .bind(&extension.description)
        .bind(if extension.enabled { 1 } else { 0 })
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        debug!(id = %extension.id, number = %extension.number, "Extension stored");
        Ok(())
    }

    async fn delete_extension(&self, id: &ExtensionId) -> Result<()> {
        sqlx::query("DELETE FROM extensions WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        debug!(id = %id, "Extension deleted");
        Ok(())
    }
}

// ============================================================================
// DeviceStore Implementation
// ============================================================================

#[async_trait]
impl DeviceStore for SqliteStorage {
    async fn get_device(&self, id: &DeviceId) -> Result<Option<Device>> {
        let row = sqlx::query(
            "SELECT id, name, device_type, mac_address, model, firmware_version, tenant_id, owner_user_id,
                    sip_uri, registration_state, last_registration, last_ip_address, user_agent,
                    max_lines, supports_video, supports_blf, codec_support,
                    location, e911_override_json, provisioning_url, config_template_id, enabled,
                    created_at, updated_at
             FROM devices
             WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_device(row)?)),
            None => Ok(None),
        }
    }

    async fn get_device_by_sip_uri(&self, sip_uri: &str) -> Result<Option<Device>> {
        let row = sqlx::query(
            "SELECT id, name, device_type, mac_address, model, firmware_version, tenant_id, owner_user_id,
                    sip_uri, registration_state, last_registration, last_ip_address, user_agent,
                    max_lines, supports_video, supports_blf, codec_support,
                    location, e911_override_json, provisioning_url, config_template_id, enabled,
                    created_at, updated_at
             FROM devices
             WHERE sip_uri = ?",
        )
        .bind(sip_uri)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_device(row)?)),
            None => Ok(None),
        }
    }

    async fn list_devices(&self, tenant_id: &TenantId) -> Result<Vec<Device>> {
        let rows = sqlx::query(
            "SELECT id, name, device_type, mac_address, model, firmware_version, tenant_id, owner_user_id,
                    sip_uri, registration_state, last_registration, last_ip_address, user_agent,
                    max_lines, supports_video, supports_blf, codec_support,
                    location, e911_override_json, provisioning_url, config_template_id, enabled,
                    created_at, updated_at
             FROM devices
             WHERE tenant_id = ?
             ORDER BY name",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let devices = rows
            .into_iter()
            .filter_map(|row| row_to_device(row).ok())
            .collect();

        Ok(devices)
    }

    async fn list_devices_by_user(&self, user_id: &UserId) -> Result<Vec<Device>> {
        let rows = sqlx::query(
            "SELECT id, name, device_type, mac_address, model, firmware_version, tenant_id, owner_user_id,
                    sip_uri, registration_state, last_registration, last_ip_address, user_agent,
                    max_lines, supports_video, supports_blf, codec_support,
                    location, e911_override_json, provisioning_url, config_template_id, enabled,
                    created_at, updated_at
             FROM devices
             WHERE owner_user_id = ?
             ORDER BY name",
        )
        .bind(user_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let devices = rows
            .into_iter()
            .filter_map(|row| row_to_device(row).ok())
            .collect();

        Ok(devices)
    }

    async fn put_device(&self, device: Device) -> Result<()> {
        let id = device.id.to_string();
        let device_type = device.device_type.to_string();
        let tenant_id = device.tenant_id.to_string();
        let owner_user_id = device.owner_user_id.map(|u| u.to_string());
        let registration_state = device.registration_state.to_string();
        let last_registration = device.last_registration.map(|t| {
            t.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        });
        let codec_support = serde_json::to_string(
            &device
                .codec_support
                .into_iter()
                .map(|c| format!("{:?}", c))
                .collect::<Vec<_>>(),
        )?;
        let e911_override_json = device
            .e911_override
            .map(|a| serde_json::to_string(&a))
            .transpose()?;
        let created_at = device
            .created_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let updated_at = device
            .updated_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            "INSERT INTO devices (
                id, name, device_type, mac_address, model, firmware_version, tenant_id, owner_user_id,
                sip_uri, registration_state, last_registration, last_ip_address, user_agent,
                max_lines, supports_video, supports_blf, codec_support,
                location, e911_override_json, provisioning_url, config_template_id, enabled,
                created_at, updated_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 mac_address = excluded.mac_address,
                 model = excluded.model,
                 firmware_version = excluded.firmware_version,
                 owner_user_id = excluded.owner_user_id,
                 sip_uri = excluded.sip_uri,
                 registration_state = excluded.registration_state,
                 last_registration = excluded.last_registration,
                 last_ip_address = excluded.last_ip_address,
                 user_agent = excluded.user_agent,
                 max_lines = excluded.max_lines,
                 supports_video = excluded.supports_video,
                 supports_blf = excluded.supports_blf,
                 codec_support = excluded.codec_support,
                 location = excluded.location,
                 e911_override_json = excluded.e911_override_json,
                 provisioning_url = excluded.provisioning_url,
                 config_template_id = excluded.config_template_id,
                 enabled = excluded.enabled,
                 updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&device.name)
        .bind(&device_type)
        .bind(&device.mac_address)
        .bind(&device.model)
        .bind(&device.firmware_version)
        .bind(&tenant_id)
        .bind(&owner_user_id)
        .bind(&device.sip_uri)
        .bind(&registration_state)
        .bind(last_registration)
        .bind(&device.last_ip_address)
        .bind(&device.user_agent)
        .bind(device.max_lines as i64)
        .bind(if device.supports_video { 1 } else { 0 })
        .bind(if device.supports_blf { 1 } else { 0 })
        .bind(&codec_support)
        .bind(&device.location)
        .bind(&e911_override_json)
        .bind(&device.provisioning_url)
        .bind(&device.config_template_id)
        .bind(if device.enabled { 1 } else { 0 })
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        debug!(id = %device.id, name = %device.name, "Device stored");
        Ok(())
    }

    async fn delete_device(&self, id: &DeviceId) -> Result<()> {
        sqlx::query("DELETE FROM devices WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        debug!(id = %id, "Device deleted");
        Ok(())
    }
}

// ============================================================================
// Helper Functions for New Types
// ============================================================================

/// Convert SQLite row to PhoneNumber
fn row_to_phone_number(row: sqlx::sqlite::SqliteRow) -> Result<PhoneNumber> {
    let id = PhoneNumberId::from_str(&row.get::<String, _>("id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;
    let state_str: String = row.get("state");
    let state = NumberState::from_str(&state_str).unwrap_or(NumberState::Unassigned);

    let e911_address = row
        .get::<Option<String>, _>("e911_address_json")
        .and_then(|s| serde_json::from_str(&s).ok());

    let allow_list: Vec<String> = row
        .get::<Option<String>, _>("allow_list")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let block_list: Vec<String> = row
        .get::<Option<String>, _>("block_list")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let tags: Vec<String> = row
        .get::<Option<String>, _>("tags")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let acquired_at = UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("acquired_at") as u64);
    let activated_at = row
        .get::<Option<i64>, _>("activated_at")
        .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64));
    let ported_at = row
        .get::<Option<i64>, _>("ported_at")
        .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64));
    let loa_date = row
        .get::<Option<i64>, _>("loa_date")
        .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64));
    let foc_date = row
        .get::<Option<i64>, _>("foc_date")
        .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64));
    let created_at = UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("created_at") as u64);
    let updated_at = UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("updated_at") as u64);

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
        recording_enabled: row.get::<i64, _>("recording_enabled") != 0,
        block_anonymous: row.get::<i64, _>("block_anonymous") != 0,
        allow_list,
        block_list,
        tags,
        created_at,
        updated_at,
    })
}

/// Convert SQLite row to LineBinding
fn row_to_line_binding(row: sqlx::sqlite::SqliteRow) -> Result<LineBinding> {
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

    let no_answer_timeout =
        Duration::from_secs(row.get::<i64, _>("no_answer_timeout_secs") as u64);
    let created_at = UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("created_at") as u64);
    let updated_at = UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("updated_at") as u64);

    Ok(LineBinding {
        id,
        phone_number_id,
        owner_user_id,
        binding_type,
        devices: Vec::new(), // Will be populated by caller
        ring_strategy,
        tenant_id,
        no_answer_timeout,
        forward_on_busy: row.get("forward_on_busy"),
        forward_on_no_answer: row.get("forward_on_no_answer"),
        forward_unconditional: row.get("forward_unconditional"),
        outbound_caller_id: row.get("outbound_caller_id"),
        outbound_cnam: row.get("outbound_cnam"),
        created_at,
        updated_at,
    })
}

/// Convert SQLite row to Extension
fn row_to_extension(row: sqlx::sqlite::SqliteRow) -> Result<Extension> {
    let id = ExtensionId::from_str(&row.get::<String, _>("id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;
    let target_type: String = row.get("target_type");
    let target_id: String = row.get("target_id");

    let target = match target_type.as_str() {
        "user" => ExtensionTarget::User(UserId::from_str(&target_id)?),
        "phone_number" => ExtensionTarget::PhoneNumber(PhoneNumberId::from_str(&target_id)?),
        "device" => ExtensionTarget::Device(DeviceId::from_str(&target_id)?),
        "service" => {
            ExtensionTarget::Service(fcp_core::ServiceId::from_str(&target_id).unwrap_or_else(
                |_| fcp_core::ServiceId::new(),
            ))
        }
        _ => {
            return Err(anyhow::anyhow!("Unknown extension target type: {}", target_type));
        }
    };

    let created_at = UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("created_at") as u64);
    let updated_at = UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("updated_at") as u64);

    Ok(Extension {
        id,
        number: row.get("number"),
        target,
        tenant_id,
        description: row.get("description"),
        enabled: row.get::<i64, _>("enabled") != 0,
        created_at,
        updated_at,
    })
}

/// Convert SQLite row to Device
fn row_to_device(row: sqlx::sqlite::SqliteRow) -> Result<Device> {
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
        .get::<Option<i64>, _>("last_registration")
        .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64));

    let codec_support_json: String = row.get("codec_support");
    let codec_names: Vec<String> = serde_json::from_str(&codec_support_json).unwrap_or_default();
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
        .get::<Option<String>, _>("e911_override_json")
        .and_then(|s| serde_json::from_str(&s).ok());

    let created_at = UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("created_at") as u64);
    let updated_at = UNIX_EPOCH + Duration::from_secs(row.get::<i64, _>("updated_at") as u64);

    Ok(Device {
        id,
        name: row.get("name"),
        device_type,
        mac_address: row.get("mac_address"),
        model: row.get("model"),
        firmware_version: row.get("firmware_version"),
        tenant_id,
        owner_user_id,
        sip_uri: row.get("sip_uri"),
        registration_state,
        last_registration,
        last_ip_address: row.get("last_ip_address"),
        user_agent: row.get("user_agent"),
        max_lines: row.get::<i64, _>("max_lines") as u8,
        supports_video: row.get::<i64, _>("supports_video") != 0,
        supports_blf: row.get::<i64, _>("supports_blf") != 0,
        codec_support,
        location: row.get("location"),
        e911_override,
        provisioning_url: row.get("provisioning_url"),
        config_template_id: row.get("config_template_id"),
        enabled: row.get::<i64, _>("enabled") != 0,
        created_at,
        updated_at,
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
            digest_password: None,
            sip_ha1_hash: None,
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
            primary_phone_number_id: None,
            voicemail_box_id: None,
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
