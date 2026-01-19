//! SQLite storage backend implementation.
//!
//! This module provides a production-ready SQLite implementation of all FCP storage traits.
//! SQLite is the default storage backend for FCP, providing:
//! - Zero-configuration setup
//! - ACID compliance
//! - Excellent performance for single-node and small deployments
//! - Simple backup and restore
//!
//! ## Usage
//!
//! ```no_run
//! use fcp_core::storage::sqlite::SqliteStorage;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create storage with default settings
//! let storage = SqliteStorage::new("fcp.db").await?;
//!
//! // Or with custom path
//! let storage = SqliteStorage::new("/var/lib/fcp/data.db").await?;
//! # Ok(())
//! # }
//! ```

use crate::{
    geographic::{GeoRoutingRule, PhoneNumberLocation},
    identifiers::*,
    phone_number::PhoneNumber,
    porting::{PortRequest, PortRequestId, PortStatus},
    storage::*,
    Device, Extension, LineBinding,
    VoicemailMailbox, VoicemailMessage,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use std::path::Path;
use std::str::FromStr;
use std::time::SystemTime;
use tracing::{debug, info};

/// SQLite storage backend.
///
/// This implementation uses sqlx for async database operations with connection pooling.
#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Creates a new SQLite storage backend.
    ///
    /// This will create the database file if it doesn't exist and run all migrations.
    pub async fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        info!("Initializing SQLite storage at: {}", path.display());

        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create database directory")?;
        }

        // Configure SQLite for optimal performance
        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal) // Write-Ahead Logging for better concurrency
            .synchronous(SqliteSynchronous::Normal) // Balance between safety and performance
            .busy_timeout(std::time::Duration::from_secs(30));

        // Create connection pool
        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect_with(options)
            .await
            .context("Failed to connect to SQLite database")?;

        let storage = Self { pool };

        // Run migrations
        storage.run_migrations().await?;

        info!("SQLite storage initialized successfully");
        Ok(storage)
    }

    /// Runs database migrations.
    async fn run_migrations(&self) -> Result<()> {
        debug!("Running database migrations");

        // Enable foreign keys
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&self.pool)
            .await?;

        // Tenants table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS tenants (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                description TEXT,
                status TEXT NOT NULL,
                contact_email TEXT,
                contact_phone TEXT,
                max_users INTEGER,
                max_phone_numbers INTEGER,
                max_concurrent_calls INTEGER,
                billing_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                trial_expires_at INTEGER,
                metadata TEXT,
                data TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_tenants_name ON tenants(name)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_tenants_status ON tenants(status)")
            .execute(&self.pool)
            .await?;

        // Users table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id TEXT,
                username TEXT PRIMARY KEY,
                password TEXT NOT NULL,
                digest_password TEXT,
                sip_ha1_hash TEXT,
                display_name TEXT,
                tenant_id TEXT NOT NULL,
                enabled BOOLEAN NOT NULL DEFAULT 1,
                role TEXT NOT NULL DEFAULT 'end_user',
                department_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                data TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Add role column to existing tables (migration)
        let _ = sqlx::query(
            r#"
            ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'end_user'
            "#,
        )
        .execute(&self.pool)
        .await; // Ignore errors if column already exists

        // Add department_id column to existing tables (migration)
        let _ = sqlx::query(
            r#"
            ALTER TABLE users ADD COLUMN department_id TEXT
            "#,
        )
        .execute(&self.pool)
        .await; // Ignore errors if column already exists

        // Add id column to existing tables (migration)
        let _ = sqlx::query(
            r#"
            ALTER TABLE users ADD COLUMN id TEXT
            "#,
        )
        .execute(&self.pool)
        .await; // Ignore errors if column already exists

        // Registrations table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS registrations (
                aor TEXT NOT NULL,
                contact TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                call_id TEXT NOT NULL,
                cseq INTEGER NOT NULL,
                user_agent TEXT,
                q REAL,
                registered_at INTEGER NOT NULL,
                data TEXT NOT NULL,
                PRIMARY KEY (aor, contact)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_registrations_aor ON registrations(aor)")
            .execute(&self.pool)
            .await?;

        // Call Detail Records table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS call_detail_records (
                call_id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                sip_call_id TEXT NOT NULL,
                from_uri TEXT NOT NULL,
                to_uri TEXT NOT NULL,
                start_time INTEGER NOT NULL,
                answer_time INTEGER,
                end_time INTEGER,
                disposition TEXT NOT NULL,
                hangup_cause TEXT NOT NULL,
                hangup_party TEXT,
                data TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_cdr_tenant ON call_detail_records(tenant_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_cdr_call_id ON call_detail_records(call_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_cdr_start_time ON call_detail_records(start_time)",
        )
        .execute(&self.pool)
        .await?;

        // Phone Numbers table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS phone_numbers (
                id TEXT PRIMARY KEY,
                e164 TEXT NOT NULL UNIQUE,
                tenant_id TEXT NOT NULL,
                state TEXT NOT NULL,
                cnam TEXT,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_phone_numbers_tenant ON phone_numbers(tenant_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_phone_numbers_e164 ON phone_numbers(e164)",
        )
        .execute(&self.pool)
        .await?;

        // Line Bindings table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS line_bindings (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                phone_number_id TEXT NOT NULL,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Extensions table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS extensions (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                number TEXT NOT NULL,
                enabled BOOLEAN NOT NULL DEFAULT 1,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(tenant_id, number)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Devices table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS devices (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                name TEXT NOT NULL,
                device_type TEXT NOT NULL,
                enabled BOOLEAN NOT NULL DEFAULT 1,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Voicemail Mailboxes table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS voicemail_mailboxes (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                extension TEXT NOT NULL,
                pin TEXT,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(tenant_id, extension)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Voicemail Messages table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS voicemail_messages (
                id TEXT PRIMARY KEY,
                mailbox_id TEXT NOT NULL,
                caller TEXT NOT NULL,
                status TEXT NOT NULL,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(mailbox_id) REFERENCES voicemail_mailboxes(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Voicemail Greetings table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS voicemail_greetings (
                id TEXT PRIMARY KEY,
                mailbox_id TEXT NOT NULL,
                greeting_type TEXT NOT NULL,
                data TEXT NOT NULL,
                recorded_at INTEGER NOT NULL,
                FOREIGN KEY(mailbox_id) REFERENCES voicemail_mailboxes(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Geographic Routing Rules table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS geo_routing_rules (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                name TEXT NOT NULL,
                enabled BOOLEAN NOT NULL DEFAULT 1,
                priority INTEGER NOT NULL,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Phone Number Locations table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS phone_number_locations (
                phone_number_id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Port Requests table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS port_requests (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                status TEXT NOT NULL,
                customer_name TEXT NOT NULL,
                account_number TEXT NOT NULL,
                losing_carrier TEXT NOT NULL,
                winning_carrier TEXT NOT NULL,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_port_requests_tenant ON port_requests(tenant_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_port_requests_status ON port_requests(status)")
            .execute(&self.pool)
            .await?;

        debug!("Database migrations completed successfully");
        Ok(())
    }

    /// Helper to convert SystemTime to Unix timestamp.
    fn system_time_to_i64(time: SystemTime) -> i64 {
        time.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    /// Helper to convert Unix timestamp to SystemTime.
    fn i64_to_system_time(timestamp: i64) -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(timestamp as u64)
    }
}

// Implement CredentialStore
#[async_trait]
impl CredentialStore for SqliteStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            r#"
            SELECT data
            FROM users
            WHERE username = ?
            "#,
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let user: User = serde_json::from_str(&data)?;
                Ok(Some(user))
            }
            None => Ok(None),
        }
    }

    async fn put_user(&self, user: User) -> Result<()> {
        let data = serde_json::to_string(&user)?;
        let created_at = Self::system_time_to_i64(user.created_at);
        let updated_at = Self::system_time_to_i64(user.updated_at);

        // Serialize role to snake_case string using serde (e.g., "telecom_admin", not "telecomadmin")
        let role_str = serde_json::to_string(&user.role)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "end_user".to_string());

        sqlx::query(
            r#"
            INSERT INTO users (id, username, password, digest_password, sip_ha1_hash, display_name, tenant_id, enabled, role, department_id, created_at, updated_at, data)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(username) DO UPDATE SET
                id = excluded.id,
                password = excluded.password,
                digest_password = excluded.digest_password,
                sip_ha1_hash = excluded.sip_ha1_hash,
                display_name = excluded.display_name,
                tenant_id = excluded.tenant_id,
                enabled = excluded.enabled,
                role = excluded.role,
                department_id = excluded.department_id,
                updated_at = excluded.updated_at,
                data = excluded.data
            "#,
        )
        .bind(user.id.to_string())
        .bind(&user.username)
        .bind(&user.password)
        .bind(&user.digest_password)
        .bind(&user.sip_ha1_hash)
        .bind(&user.display_name)
        .bind(user.tenant_id.to_string())
        .bind(user.enabled)
        .bind(role_str)
        .bind(&user.department_id)
        .bind(created_at)
        .bind(updated_at)
        .bind(data)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        sqlx::query("DELETE FROM users WHERE username = ?")
            .bind(username)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query(
            r#"
            SELECT data
            FROM users
            ORDER BY username
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut users = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let user: User = serde_json::from_str(&data)?;
            users.push(user);
        }

        Ok(users)
    }
}

// Implement RegistrationStore
#[async_trait]
impl RegistrationStore for SqliteStorage {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        let rows = sqlx::query(
            r#"
            SELECT data
            FROM registrations
            WHERE aor = ?
            ORDER BY registered_at DESC
            "#,
        )
        .bind(aor)
        .fetch_all(&self.pool)
        .await?;

        let mut registrations = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let registration: Registration = serde_json::from_str(&data)?;
            registrations.push(registration);
        }

        Ok(registrations)
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        let data = serde_json::to_string(&registration)?;
        let expires_at = Self::system_time_to_i64(registration.expires_at);
        let registered_at = Self::system_time_to_i64(registration.registered_at);

        sqlx::query(
            r#"
            INSERT INTO registrations (aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at, data)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(aor, contact) DO UPDATE SET
                expires_at = excluded.expires_at,
                call_id = excluded.call_id,
                cseq = excluded.cseq,
                user_agent = excluded.user_agent,
                q = excluded.q,
                registered_at = excluded.registered_at,
                data = excluded.data
            "#,
        )
        .bind(&registration.aor)
        .bind(&registration.contact)
        .bind(expires_at)
        .bind(&registration.call_id)
        .bind(registration.cseq as i64)
        .bind(&registration.user_agent)
        .bind(registration.q)
        .bind(registered_at)
        .bind(data)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        sqlx::query("DELETE FROM registrations WHERE aor = ? AND contact = ?")
            .bind(aor)
            .bind(contact)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM registrations WHERE aor = ?")
            .bind(aor)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        let now = Self::system_time_to_i64(SystemTime::now());
        let result = sqlx::query("DELETE FROM registrations WHERE expires_at < ?")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() as usize)
    }
}

// Implement CdrStore
#[async_trait]
impl CdrStore for SqliteStorage {
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()> {
        let start_time = Self::system_time_to_i64(cdr.start_time);
        let answer_time = cdr.answer_time.map(Self::system_time_to_i64);
        let end_time = cdr.end_time.map(Self::system_time_to_i64);

        let data = serde_json::to_string(&cdr)?;

        let hangup_cause_str = format!("{:?}", cdr.hangup_cause);
        let hangup_party_str = cdr.hangup_party.as_ref().map(|p| format!("{:?}", p));

        sqlx::query(
            r#"
            INSERT INTO call_detail_records
            (call_id, tenant_id, sip_call_id, from_uri, to_uri, start_time, answer_time, end_time, disposition, hangup_cause, hangup_party, data)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(cdr.call_id.to_string())
        .bind(cdr.tenant_id.to_string())
        .bind(&cdr.sip_call_id)
        .bind(&cdr.from_uri)
        .bind(&cdr.to_uri)
        .bind(start_time)
        .bind(answer_time)
        .bind(end_time)
        .bind(format!("{:?}", cdr.disposition))
        .bind(hangup_cause_str)
        .bind(hangup_party_str)
        .bind(data)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_cdr(&self, call_id: &CallId) -> Result<Option<CallDetailRecord>> {
        let row = sqlx::query("SELECT data FROM call_detail_records WHERE call_id = ?")
            .bind(call_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let cdr: CallDetailRecord = serde_json::from_str(&data)?;
                Ok(Some(cdr))
            }
            None => Ok(None),
        }
    }

    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>> {
        let mut sql = String::from("SELECT data FROM call_detail_records WHERE 1=1");

        let mut has_tenant = false;
        let mut has_from = false;
        let mut has_to = false;
        let mut has_start_from = false;
        let mut has_start_to = false;

        if query.tenant_id.is_some() {
            sql.push_str(" AND tenant_id = ?");
            has_tenant = true;
        }
        if query.from_uri.is_some() {
            sql.push_str(" AND from_uri = ?");
            has_from = true;
        }
        if query.to_uri.is_some() {
            sql.push_str(" AND to_uri = ?");
            has_to = true;
        }
        if query.start_time_from.is_some() {
            sql.push_str(" AND start_time >= ?");
            has_start_from = true;
        }
        if query.start_time_to.is_some() {
            sql.push_str(" AND start_time <= ?");
            has_start_to = true;
        }

        sql.push_str(" ORDER BY start_time DESC LIMIT ?");

        let mut q = sqlx::query(&sql);

        if has_tenant {
            q = q.bind(query.tenant_id.unwrap().to_string());
        }
        if has_from {
            q = q.bind(query.from_uri.unwrap());
        }
        if has_to {
            q = q.bind(query.to_uri.unwrap());
        }
        if has_start_from {
            q = q.bind(Self::system_time_to_i64(query.start_time_from.unwrap()));
        }
        if has_start_to {
            q = q.bind(Self::system_time_to_i64(query.start_time_to.unwrap()));
        }

        q = q.bind(query.limit.unwrap_or(100) as i64);

        let rows = q.fetch_all(&self.pool).await?;

        let mut cdrs = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let cdr: CallDetailRecord = serde_json::from_str(&data)?;
            cdrs.push(cdr);
        }

        Ok(cdrs)
    }

    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize> {
        let mut sql = String::from("SELECT COUNT(*) as count FROM call_detail_records WHERE 1=1");

        let mut has_tenant = false;
        let mut has_from = false;
        let mut has_to = false;
        let mut has_start_from = false;
        let mut has_start_to = false;

        if query.tenant_id.is_some() {
            sql.push_str(" AND tenant_id = ?");
            has_tenant = true;
        }
        if query.from_uri.is_some() {
            sql.push_str(" AND from_uri = ?");
            has_from = true;
        }
        if query.to_uri.is_some() {
            sql.push_str(" AND to_uri = ?");
            has_to = true;
        }
        if query.start_time_from.is_some() {
            sql.push_str(" AND start_time >= ?");
            has_start_from = true;
        }
        if query.start_time_to.is_some() {
            sql.push_str(" AND start_time <= ?");
            has_start_to = true;
        }

        let mut q = sqlx::query(&sql);

        if has_tenant {
            q = q.bind(query.tenant_id.unwrap().to_string());
        }
        if has_from {
            q = q.bind(query.from_uri.unwrap());
        }
        if has_to {
            q = q.bind(query.to_uri.unwrap());
        }
        if has_start_from {
            q = q.bind(Self::system_time_to_i64(query.start_time_from.unwrap()));
        }
        if has_start_to {
            q = q.bind(Self::system_time_to_i64(query.start_time_to.unwrap()));
        }

        let row = q.fetch_one(&self.pool).await?;
        let count: i64 = row.get("count");
        Ok(count as usize)
    }
}

// Implement PhoneNumberStore
#[async_trait]
impl PhoneNumberStore for SqliteStorage {
    async fn get_phone_number(&self, id: &PhoneNumberId) -> Result<Option<PhoneNumber>> {
        let row = sqlx::query("SELECT data FROM phone_numbers WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let phone_number: PhoneNumber = serde_json::from_str(&data)?;
                Ok(Some(phone_number))
            }
            None => Ok(None),
        }
    }

    async fn get_phone_number_by_e164(&self, e164: &str) -> Result<Option<PhoneNumber>> {
        let row = sqlx::query("SELECT data FROM phone_numbers WHERE e164 = ?")
            .bind(e164)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let phone_number: PhoneNumber = serde_json::from_str(&data)?;
                Ok(Some(phone_number))
            }
            None => Ok(None),
        }
    }

    async fn list_phone_numbers(&self, tenant_id: &TenantId) -> Result<Vec<PhoneNumber>> {
        let rows = sqlx::query("SELECT data FROM phone_numbers WHERE tenant_id = ? ORDER BY e164")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut phone_numbers = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let phone_number: PhoneNumber = serde_json::from_str(&data)?;
            phone_numbers.push(phone_number);
        }

        Ok(phone_numbers)
    }

    async fn put_phone_number(&self, phone_number: PhoneNumber) -> Result<()> {
        let data = serde_json::to_string(&phone_number)?;
        let now = Self::system_time_to_i64(SystemTime::now());

        sqlx::query(
            r#"
            INSERT INTO phone_numbers (id, e164, tenant_id, state, cnam, data, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                e164 = excluded.e164,
                tenant_id = excluded.tenant_id,
                state = excluded.state,
                cnam = excluded.cnam,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(phone_number.id.to_string())
        .bind(&phone_number.e164)
        .bind(phone_number.tenant_id.to_string())
        .bind(format!("{:?}", phone_number.state))
        .bind(&phone_number.cnam)
        .bind(data)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_phone_number(&self, id: &PhoneNumberId) -> Result<()> {
        sqlx::query("DELETE FROM phone_numbers WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// Implement LineBindingStore
#[async_trait]
impl LineBindingStore for SqliteStorage {
    async fn get_line_binding(&self, id: &LineBindingId) -> Result<Option<LineBinding>> {
        let row = sqlx::query("SELECT data FROM line_bindings WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let binding: LineBinding = serde_json::from_str(&data)?;
                Ok(Some(binding))
            }
            None => Ok(None),
        }
    }

    async fn get_line_binding_by_phone_number(
        &self,
        phone_number_id: &PhoneNumberId,
    ) -> Result<Option<LineBinding>> {
        let rows = sqlx::query("SELECT data FROM line_bindings")
            .fetch_all(&self.pool)
            .await?;

        for row in rows {
            let data: String = row.get("data");
            let binding: LineBinding = serde_json::from_str(&data)?;
            // Check if this binding has the phone number
            if &binding.phone_number_id == phone_number_id {
                return Ok(Some(binding));
            }
        }

        Ok(None)
    }

    async fn list_line_bindings_by_user(
        &self,
        user_id: &UserId,
    ) -> Result<Vec<LineBinding>> {
        let rows = sqlx::query("SELECT data FROM line_bindings")
            .fetch_all(&self.pool)
            .await?;

        let mut bindings = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let binding: LineBinding = serde_json::from_str(&data)?;
            if matches!(binding.owner_user_id, Some(ref id) if id == user_id) {
                bindings.push(binding);
            }
        }

        Ok(bindings)
    }

    async fn list_line_bindings_by_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Vec<LineBinding>> {
        let rows = sqlx::query("SELECT data FROM line_bindings")
            .fetch_all(&self.pool)
            .await?;

        let mut bindings = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let binding: LineBinding = serde_json::from_str(&data)?;
            // Check if this binding has the device
            for device_binding in &binding.devices {
                if &device_binding.device_id == device_id {
                    bindings.push(binding);
                    break;
                }
            }
        }

        Ok(bindings)
    }

    async fn list_line_bindings(&self, tenant_id: &TenantId) -> Result<Vec<LineBinding>> {
        let rows = sqlx::query("SELECT data FROM line_bindings WHERE tenant_id = ?")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut bindings = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let binding: LineBinding = serde_json::from_str(&data)?;
            bindings.push(binding);
        }

        Ok(bindings)
    }

    async fn put_line_binding(&self, binding: LineBinding) -> Result<()> {
        let data = serde_json::to_string(&binding)?;
        let created_at = Self::system_time_to_i64(binding.created_at);
        let updated_at = Self::system_time_to_i64(binding.updated_at);

        sqlx::query(
            r#"
            INSERT INTO line_bindings (id, tenant_id, phone_number_id, data, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                tenant_id = excluded.tenant_id,
                phone_number_id = excluded.phone_number_id,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(binding.id.to_string())
        .bind(binding.tenant_id.to_string())
        .bind(binding.phone_number_id.to_string())
        .bind(data)
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_line_binding(&self, id: &LineBindingId) -> Result<()> {
        sqlx::query("DELETE FROM line_bindings WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// Implement ExtensionStore
#[async_trait]
impl ExtensionStore for SqliteStorage {
    async fn get_extension(&self, id: &ExtensionId) -> Result<Option<Extension>> {
        let row = sqlx::query("SELECT data FROM extensions WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let extension: Extension = serde_json::from_str(&data)?;
                Ok(Some(extension))
            }
            None => Ok(None),
        }
    }

    async fn get_extension_by_number(
        &self,
        tenant_id: &TenantId,
        extension: &str,
    ) -> Result<Option<Extension>> {
        let row = sqlx::query("SELECT data FROM extensions WHERE tenant_id = ? AND extension = ?")
            .bind(tenant_id.to_string())
            .bind(extension)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let ext: Extension = serde_json::from_str(&data)?;
                Ok(Some(ext))
            }
            None => Ok(None),
        }
    }

    async fn list_extensions(&self, tenant_id: &TenantId) -> Result<Vec<Extension>> {
        let rows = sqlx::query("SELECT data FROM extensions WHERE tenant_id = ? ORDER BY number")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut extensions = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let extension: Extension = serde_json::from_str(&data)?;
            extensions.push(extension);
        }

        Ok(extensions)
    }

    async fn put_extension(&self, extension: Extension) -> Result<()> {
        let data = serde_json::to_string(&extension)?;
        let now = Self::system_time_to_i64(SystemTime::now());

        sqlx::query(
            r#"
            INSERT INTO extensions (id, tenant_id, number, enabled, data, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                tenant_id = excluded.tenant_id,
                number = excluded.number,
                enabled = excluded.enabled,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(extension.id.to_string())
        .bind(extension.tenant_id.to_string())
        .bind(&extension.number)
        .bind(extension.enabled)
        .bind(data)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_extension(&self, id: &ExtensionId) -> Result<()> {
        sqlx::query("DELETE FROM extensions WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// Implement DeviceStore
#[async_trait]
impl DeviceStore for SqliteStorage {
    async fn get_device(&self, id: &DeviceId) -> Result<Option<Device>> {
        let row = sqlx::query("SELECT data FROM devices WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let device: Device = serde_json::from_str(&data)?;
                Ok(Some(device))
            }
            None => Ok(None),
        }
    }

    async fn list_devices(&self, tenant_id: &TenantId) -> Result<Vec<Device>> {
        let rows = sqlx::query("SELECT data FROM devices WHERE tenant_id = ? ORDER BY name")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut devices = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let device: Device = serde_json::from_str(&data)?;
            devices.push(device);
        }

        Ok(devices)
    }

    async fn put_device(&self, device: Device) -> Result<()> {
        let data = serde_json::to_string(&device)?;
        let now = Self::system_time_to_i64(SystemTime::now());

        sqlx::query(
            r#"
            INSERT INTO devices (id, tenant_id, name, device_type, enabled, data, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                tenant_id = excluded.tenant_id,
                name = excluded.name,
                device_type = excluded.device_type,
                enabled = excluded.enabled,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(device.id.to_string())
        .bind(device.tenant_id.to_string())
        .bind(&device.name)
        .bind(format!("{:?}", device.device_type))
        .bind(device.enabled)
        .bind(data)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_device(&self, id: &DeviceId) -> Result<()> {
        sqlx::query("DELETE FROM devices WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_device_by_sip_uri(&self, sip_uri: &str) -> Result<Option<Device>> {
        let rows = sqlx::query("SELECT data FROM devices")
            .fetch_all(&self.pool)
            .await?;

        for row in rows {
            let data: String = row.get("data");
            let device: Device = serde_json::from_str(&data)?;
            if device.sip_uri.as_deref() == Some(sip_uri) {
                return Ok(Some(device));
            }
        }

        Ok(None)
    }

    async fn list_devices_by_user(&self, user_id: &UserId) -> Result<Vec<Device>> {
        let rows = sqlx::query("SELECT data FROM devices")
            .fetch_all(&self.pool)
            .await?;

        let mut devices = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let device: Device = serde_json::from_str(&data)?;
            if matches!(device.owner_user_id, Some(ref id) if id == user_id) {
                devices.push(device);
            }
        }

        Ok(devices)
    }
}

// Implement VoicemailStore
#[async_trait]
impl VoicemailStore for SqliteStorage {
    async fn get_mailbox(&self, id: &str) -> Result<Option<VoicemailMailbox>> {
        let row = sqlx::query("SELECT data FROM voicemail_mailboxes WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let mailbox: VoicemailMailbox = serde_json::from_str(&data)?;
                Ok(Some(mailbox))
            }
            None => Ok(None),
        }
    }

    async fn get_mailbox_by_extension(
        &self,
        tenant_id: &TenantId,
        extension: &str,
    ) -> Result<Option<VoicemailMailbox>> {
        let row = sqlx::query("SELECT data FROM voicemail_mailboxes WHERE tenant_id = ? AND extension = ?")
            .bind(tenant_id.to_string())
            .bind(extension)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let mailbox: VoicemailMailbox = serde_json::from_str(&data)?;
                Ok(Some(mailbox))
            }
            None => Ok(None),
        }
    }

    async fn list_mailboxes(&self, tenant_id: &TenantId) -> Result<Vec<VoicemailMailbox>> {
        let rows = sqlx::query("SELECT data FROM voicemail_mailboxes WHERE tenant_id = ? ORDER BY extension")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut mailboxes = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let mailbox: VoicemailMailbox = serde_json::from_str(&data)?;
            mailboxes.push(mailbox);
        }

        Ok(mailboxes)
    }

    async fn put_mailbox(&self, mailbox: VoicemailMailbox) -> Result<()> {
        let data = serde_json::to_string(&mailbox)?;
        let now = Self::system_time_to_i64(SystemTime::now());

        sqlx::query(
            r#"
            INSERT INTO voicemail_mailboxes (id, tenant_id, extension, pin, data, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                tenant_id = excluded.tenant_id,
                extension = excluded.extension,
                pin = excluded.pin,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&mailbox.id)
        .bind(mailbox.tenant_id.to_string())
        .bind(&mailbox.extension)
        .bind(&mailbox.pin)
        .bind(data)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_mailbox(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM voicemail_mailboxes WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_message(&self, id: &str) -> Result<Option<VoicemailMessage>> {
        let row = sqlx::query("SELECT data FROM voicemail_messages WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let message: VoicemailMessage = serde_json::from_str(&data)?;
                Ok(Some(message))
            }
            None => Ok(None),
        }
    }

    async fn list_messages(&self, mailbox_id: &str) -> Result<Vec<VoicemailMessage>> {
        let rows = sqlx::query("SELECT data FROM voicemail_messages WHERE mailbox_id = ? ORDER BY created_at DESC")
            .bind(mailbox_id)
            .fetch_all(&self.pool)
            .await?;

        let mut messages = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let message: VoicemailMessage = serde_json::from_str(&data)?;
            messages.push(message);
        }

        Ok(messages)
    }

    async fn put_message(&self, message: VoicemailMessage) -> Result<()> {
        let data = serde_json::to_string(&message)?;
        let created_at = Self::system_time_to_i64(message.created_at);

        sqlx::query(
            r#"
            INSERT INTO voicemail_messages (id, mailbox_id, caller, status, data, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                mailbox_id = excluded.mailbox_id,
                caller = excluded.caller,
                status = excluded.status,
                data = excluded.data
            "#,
        )
        .bind(&message.id)
        .bind(&message.mailbox_id)
        .bind(&message.caller)
        .bind(format!("{:?}", message.status))
        .bind(data)
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_message(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM voicemail_messages WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_mailbox_by_line_binding(
        &self,
        line_binding_id: &LineBindingId,
    ) -> Result<Option<VoicemailMailbox>> {
        let rows = sqlx::query("SELECT data FROM voicemail_mailboxes")
            .fetch_all(&self.pool)
            .await?;

        for row in rows {
            let data: String = row.get("data");
            let mailbox: VoicemailMailbox = serde_json::from_str(&data)?;
            if &mailbox.line_binding_id == line_binding_id {
                return Ok(Some(mailbox));
            }
        }

        Ok(None)
    }

    async fn get_mailbox_by_user(
        &self,
        user_id: &UserId,
    ) -> Result<Option<VoicemailMailbox>> {
        let rows = sqlx::query("SELECT data FROM voicemail_mailboxes")
            .fetch_all(&self.pool)
            .await?;

        for row in rows {
            let data: String = row.get("data");
            let mailbox: VoicemailMailbox = serde_json::from_str(&data)?;
            if &mailbox.user_id == user_id {
                return Ok(Some(mailbox));
            }
        }

        Ok(None)
    }

    async fn list_messages_by_status(
        &self,
        mailbox_id: &str,
        status: crate::MessageStatus,
    ) -> Result<Vec<VoicemailMessage>> {
        let rows = sqlx::query("SELECT data FROM voicemail_messages WHERE mailbox_id = ?")
            .bind(mailbox_id)
            .fetch_all(&self.pool)
            .await?;

        let mut messages = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let message: VoicemailMessage = serde_json::from_str(&data)?;
            if message.status == status {
                messages.push(message);
            }
        }

        Ok(messages)
    }

    async fn get_mwi_status(&self, mailbox_id: &str) -> Result<crate::MwiStatus> {
        let mailbox = self
            .get_mailbox(mailbox_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Mailbox not found: {}", mailbox_id))?;

        let messages = self.list_messages(mailbox_id).await?;

        let new_count = messages
            .iter()
            .filter(|m| m.status == crate::MessageStatus::New)
            .count();

        let old_count = messages
            .iter()
            .filter(|m| m.status == crate::MessageStatus::Saved)
            .count();

        Ok(crate::MwiStatus {
            mailbox_id: mailbox_id.to_string(),
            line_binding_id: mailbox.line_binding_id,
            new_messages: new_count as u32,
            old_messages: old_count as u32,
            urgent_new_messages: 0,
            urgent_old_messages: 0,
        })
    }

    async fn get_mailbox_stats(&self, mailbox_id: &str) -> Result<crate::VoicemailStats> {
        let messages = self.list_messages(mailbox_id).await?;

        let total = messages.len();
        let new = messages
            .iter()
            .filter(|m| m.status == crate::MessageStatus::New)
            .count();
        let saved = messages
            .iter()
            .filter(|m| m.status == crate::MessageStatus::Saved)
            .count();
        let heard = messages
            .iter()
            .filter(|m| matches!(m.status, crate::MessageStatus::Saved))
            .count();

        let total_duration = messages
            .iter()
            .map(|m| m.duration as u64)
            .sum::<u64>();

        let avg_duration = if total > 0 {
            (total_duration / total as u64) as u32
        } else {
            0
        };

        Ok(crate::VoicemailStats {
            total_messages: total as u32,
            new_messages: new as u32,
            heard_messages: heard as u32,
            saved_messages: saved as u32,
            storage_used: 0, // TODO: Calculate actual storage used
            avg_duration,
        })
    }
}

// Implement GeoRoutingStore
#[async_trait]
impl GeoRoutingStore for SqliteStorage {
    async fn get_geo_rule(&self, id: &str) -> Result<Option<GeoRoutingRule>> {
        let row = sqlx::query("SELECT data FROM geo_routing_rules WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let rule: GeoRoutingRule = serde_json::from_str(&data)?;
                Ok(Some(rule))
            }
            None => Ok(None),
        }
    }

    async fn list_geo_rules(&self, tenant_id: &TenantId) -> Result<Vec<GeoRoutingRule>> {
        let rows = sqlx::query("SELECT data FROM geo_routing_rules WHERE tenant_id = ? ORDER BY priority, name")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut rules = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let rule: GeoRoutingRule = serde_json::from_str(&data)?;
            rules.push(rule);
        }

        Ok(rules)
    }

    async fn put_geo_rule(&self, rule: GeoRoutingRule) -> Result<()> {
        let data = serde_json::to_string(&rule)?;
        let now = Self::system_time_to_i64(SystemTime::now());

        sqlx::query(
            r#"
            INSERT INTO geo_routing_rules (id, tenant_id, name, enabled, priority, data, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                tenant_id = excluded.tenant_id,
                name = excluded.name,
                enabled = excluded.enabled,
                priority = excluded.priority,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&rule.id)
        .bind(rule.tenant_id.to_string())
        .bind(&rule.name)
        .bind(rule.enabled)
        .bind(rule.priority as i64)
        .bind(data)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_geo_rule(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM geo_routing_rules WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_phone_number_location(
        &self,
        phone_number_id: &PhoneNumberId,
    ) -> Result<Option<PhoneNumberLocation>> {
        let row = sqlx::query("SELECT data FROM phone_number_locations WHERE phone_number_id = ?")
            .bind(phone_number_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let location: PhoneNumberLocation = serde_json::from_str(&data)?;
                Ok(Some(location))
            }
            None => Ok(None),
        }
    }

    async fn list_phone_number_locations(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<PhoneNumberLocation>> {
        let rows = sqlx::query("SELECT data FROM phone_number_locations WHERE tenant_id = ?")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut locations = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let location: PhoneNumberLocation = serde_json::from_str(&data)?;
            locations.push(location);
        }

        Ok(locations)
    }

    async fn put_phone_number_location(&self, location: PhoneNumberLocation) -> Result<()> {
        let data = serde_json::to_string(&location)?;
        let now = Self::system_time_to_i64(SystemTime::now());

        sqlx::query(
            r#"
            INSERT INTO phone_number_locations (phone_number_id, tenant_id, data, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(phone_number_id) DO UPDATE SET
                tenant_id = excluded.tenant_id,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(location.phone_number_id.to_string())
        .bind(location.tenant_id.to_string())
        .bind(data)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_phone_number_location(&self, phone_number_id: &PhoneNumberId) -> Result<()> {
        sqlx::query("DELETE FROM phone_number_locations WHERE phone_number_id = ?")
            .bind(phone_number_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// Implement PortingStore
#[async_trait]
impl crate::porting::PortingStore for SqliteStorage {
    async fn get_port_request(
        &self,
        id: &PortRequestId,
    ) -> Result<Option<PortRequest>> {
        let row = sqlx::query("SELECT data FROM port_requests WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let request: PortRequest = serde_json::from_str(&data)?;
                Ok(Some(request))
            }
            None => Ok(None),
        }
    }

    async fn list_port_requests(&self, tenant_id: &TenantId) -> Result<Vec<PortRequest>> {
        let rows = sqlx::query("SELECT data FROM port_requests WHERE tenant_id = ? ORDER BY created_at DESC")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut requests = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let request: PortRequest = serde_json::from_str(&data)?;
            requests.push(request);
        }

        Ok(requests)
    }

    async fn list_port_requests_by_status(
        &self,
        tenant_id: &TenantId,
        status: PortStatus,
    ) -> Result<Vec<PortRequest>> {
        let rows = sqlx::query("SELECT data FROM port_requests WHERE tenant_id = ? AND status = ? ORDER BY created_at DESC")
            .bind(tenant_id.to_string())
            .bind(format!("{:?}", status))
            .fetch_all(&self.pool)
            .await?;

        let mut requests = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let request: PortRequest = serde_json::from_str(&data)?;
            requests.push(request);
        }

        Ok(requests)
    }

    async fn put_port_request(&self, request: PortRequest) -> Result<()> {
        let data = serde_json::to_string(&request)?;
        let now = Self::system_time_to_i64(SystemTime::now());

        sqlx::query(
            r#"
            INSERT INTO port_requests (id, tenant_id, status, customer_name, account_number, losing_carrier, winning_carrier, data, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                tenant_id = excluded.tenant_id,
                status = excluded.status,
                customer_name = excluded.customer_name,
                account_number = excluded.account_number,
                losing_carrier = excluded.losing_carrier,
                winning_carrier = excluded.winning_carrier,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(request.id.to_string())
        .bind(request.tenant_id.to_string())
        .bind(format!("{:?}", request.status))
        .bind(&request.customer_name)
        .bind(&request.account_number)
        .bind(&request.losing_carrier)
        .bind(&request.winning_carrier)
        .bind(data)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_port_request(&self, id: &PortRequestId) -> Result<()> {
        sqlx::query("DELETE FROM port_requests WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// Implement TenantStore
#[async_trait]
impl crate::storage::TenantStore for SqliteStorage {
    async fn get_tenant(&self, id: &TenantId) -> Result<Option<crate::Tenant>> {
        let row = sqlx::query("SELECT data FROM tenants WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let tenant: crate::Tenant = serde_json::from_str(&data)?;
                Ok(Some(tenant))
            }
            None => Ok(None),
        }
    }

    async fn get_tenant_by_name(&self, name: &str) -> Result<Option<crate::Tenant>> {
        let row = sqlx::query("SELECT data FROM tenants WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let tenant: crate::Tenant = serde_json::from_str(&data)?;
                Ok(Some(tenant))
            }
            None => Ok(None),
        }
    }

    async fn list_tenants(&self) -> Result<Vec<crate::Tenant>> {
        let rows = sqlx::query(
            r#"
            SELECT data
            FROM tenants
            ORDER BY name
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut tenants = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let tenant: crate::Tenant = serde_json::from_str(&data)?;
            tenants.push(tenant);
        }

        Ok(tenants)
    }

    async fn put_tenant(&self, tenant: crate::Tenant) -> Result<()> {
        let data = serde_json::to_string(&tenant)?;
        let created_at = Self::system_time_to_i64(tenant.created_at);
        let updated_at = Self::system_time_to_i64(tenant.updated_at);
        let trial_expires_at = tenant.trial_expires_at.map(Self::system_time_to_i64);

        // Serialize status to snake_case string using serde
        let status_str = serde_json::to_string(&tenant.status)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "active".to_string());

        sqlx::query(
            r#"
            INSERT INTO tenants (id, name, description, status, contact_email, contact_phone, max_users, max_phone_numbers, max_concurrent_calls, billing_id, created_at, updated_at, trial_expires_at, metadata, data)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                status = excluded.status,
                contact_email = excluded.contact_email,
                contact_phone = excluded.contact_phone,
                max_users = excluded.max_users,
                max_phone_numbers = excluded.max_phone_numbers,
                max_concurrent_calls = excluded.max_concurrent_calls,
                billing_id = excluded.billing_id,
                updated_at = excluded.updated_at,
                trial_expires_at = excluded.trial_expires_at,
                metadata = excluded.metadata,
                data = excluded.data
            "#,
        )
        .bind(tenant.id.to_string())
        .bind(&tenant.name)
        .bind(&tenant.description)
        .bind(status_str)
        .bind(&tenant.contact_email)
        .bind(&tenant.contact_phone)
        .bind(tenant.max_users.map(|v| v as i64))
        .bind(tenant.max_phone_numbers.map(|v| v as i64))
        .bind(tenant.max_concurrent_calls.map(|v| v as i64))
        .bind(&tenant.billing_id)
        .bind(created_at)
        .bind(updated_at)
        .bind(trial_expires_at)
        .bind(&tenant.metadata)
        .bind(data)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_tenant(&self, id: &TenantId) -> Result<()> {
        sqlx::query("DELETE FROM tenants WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "sqlite_tests.rs"]
mod tests;
