//! MySQL storage backend for FCP.
//!
//! This module provides a production-ready MySQL implementation of the FCP storage traits.
//! It uses sqlx for async database access with connection pooling and migrations.
//!
//! # Features
//! - Connection pooling for high performance
//! - Automatic migrations on startup
//! - Full implementation of CredentialStore, RegistrationStore, and CdrStore traits
//! - JSON type for efficient JSON storage
//! - DATETIME(6) for microsecond precision timestamps
//! - InnoDB engine for ACID compliance
//!
//! # Example
//! ```no_run
//! use fcp_storage_mysql::{MysqlStorage, MysqlConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = MysqlConfig::new("mysql://user:pass@localhost/fcp");
//!     let storage = MysqlStorage::new(config).await?;
//!
//!     // Use storage via traits
//!     // storage.get_user("alice").await?;
//!
//!     Ok(())
//! }
//! ```

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::{
    storage::{
        CdrStore, ConferenceStore, CredentialStore, DeviceStore, ExtensionStore, LineBindingStore,
        PhoneNumberStore, PresenceStore, QueueStore, RegistrationStore, ResourceListStore,
        StoredAgent, StoredAuthPolicy, StoredPresentity, StoredResourceList, StoredSubscription,
        TenantStore, WebhookDeliveryLog, WebhookEndpoint, WebhookStore,
    },
    AccountStatus, AudioCodec, BindingType, CallDetailRecord, CallDisposition, CdrQuery, Device,
    DeviceBinding, DeviceId, DeviceType, EmergencyContact, Extension, ExtensionId, ExtensionTarget,
    HangupCause, HangupParty, LineBinding, LineBindingId, MediaPreferences, NumberState,
    PhoneNumber, PhoneNumberId, Registration, RegistrationState, RingStrategy, Tenant, TenantId,
    User, UserId, VoicemailBoxId, WebhookId,
};
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use sqlx::{MySql, Pool, Row};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

// Helper functions for SystemTime <-> chrono::DateTime conversion
fn system_time_to_datetime(st: SystemTime) -> DateTime<Utc> {
    let duration = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    DateTime::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos()).unwrap_or_default()
}

fn datetime_to_system_time(dt: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(dt.timestamp() as u64)
}

/// Configuration for MySQL storage backend.
#[derive(Debug, Clone)]
pub struct MysqlConfig {
    /// MySQL connection string (e.g., "mysql://user:pass@localhost/fcp")
    pub connection_string: String,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Connection timeout
    pub connection_timeout: Duration,
}

impl MysqlConfig {
    /// Create a new MySQL configuration with default settings.
    pub fn new<S: Into<String>>(connection_string: S) -> Self {
        Self {
            connection_string: connection_string.into(),
            max_connections: 10,
            connection_timeout: Duration::from_secs(30),
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

impl Default for MysqlConfig {
    fn default() -> Self {
        Self::new("mysql://fcp:fcp@localhost/fcp")
    }
}

/// MySQL storage backend implementing all FCP storage traits.
#[derive(Clone)]
pub struct MysqlStorage {
    pool: Pool<MySql>,
}

impl MysqlStorage {
    /// Create a new MySQL storage backend.
    pub async fn new(config: MysqlConfig) -> Result<Self> {
        // Parse connection options
        let options = MySqlConnectOptions::from_str(&config.connection_string)?;

        // Create connection pool
        let pool = MySqlPoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(config.connection_timeout)
            .connect_with(options)
            .await
            .context("Failed to create MySQL connection pool")?;

        info!(
            max_connections = config.max_connections,
            "MySQL storage initialized"
        );

        // Run migrations
        Self::run_migrations(&pool).await?;

        Ok(Self { pool })
    }

    /// Run database migrations.
    async fn run_migrations(pool: &Pool<MySql>) -> Result<()> {
        info!("Running MySQL migrations");

        let migration_files = [
            include_str!("../migrations/001_initial_schema.sql"),
            include_str!("../migrations/013_add_presence_support.sql"),
            include_str!("../migrations/014_add_resource_lists.sql"),
        ];
        let migration_sql = migration_files.join("\n");

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
            debug!(statement = %statement, "Executing migration statement");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .context(format!("Failed to execute migration: {}", statement))?;
        }

        info!("MySQL migrations completed successfully");
        Ok(())
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &Pool<MySql> {
        &self.pool
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Helper method to get device bindings for a line binding
    async fn get_device_bindings(
        &self,
        line_binding_id: &LineBindingId,
    ) -> Result<Vec<DeviceBinding>> {
        let rows = sqlx::query(
            "SELECT line_binding_id, device_id, priority, line_appearance, shared_line, enabled
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
             VALUES (?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                 priority = VALUES(priority),
                 line_appearance = VALUES(line_appearance),
                 shared_line = VALUES(shared_line),
                 enabled = VALUES(enabled)",
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
impl CredentialStore for MysqlStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, digest_password, sip_ha1_hash, display_name, tenant_id, enabled,
                    created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, language, timezone,
                    account_status, last_login, emergency_contact_name, emergency_contact_phone,
                    role, department_id, primary_phone_number_id, voicemail_box_id
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
        let media_prefs = serde_json::to_value(&user.media_preferences)?;
        let created_at = system_time_to_datetime(user.created_at);
        let updated_at = system_time_to_datetime(user.updated_at);
        let password_last_reset = user.password_last_reset.map(system_time_to_datetime);
        let last_login = user.last_login.map(system_time_to_datetime);
        let account_status = user.account_status.to_string();
        let primary_phone_number_id = user.primary_phone_number_id.map(|id| id.to_string());
        let voicemail_box_id = user.voicemail_box_id.map(|id| id.to_string());

        // Serialize role to snake_case string for storage
        let role_str = serde_json::to_string(&user.role)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "end_user".to_string());

        sqlx::query(
            "INSERT INTO users (id, username, password_hash, digest_password, sip_ha1_hash, display_name, tenant_id, enabled,
                               created_at, updated_at, media_preferences,
                               email, password_last_reset, phone_number, mobile_number, pin,
                               extension, department, job_title, location, language, timezone,
                               account_status, last_login, emergency_contact_name, emergency_contact_phone,
                               role, department_id, primary_phone_number_id, voicemail_box_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                 id = VALUES(id),
                 password_hash = VALUES(password_hash),
                 digest_password = VALUES(digest_password),
                 sip_ha1_hash = VALUES(sip_ha1_hash),
                 display_name = VALUES(display_name),
                 enabled = VALUES(enabled),
                 role = VALUES(role),
                 department_id = VALUES(department_id),
                 updated_at = VALUES(updated_at),
                 media_preferences = VALUES(media_preferences),
                 email = VALUES(email),
                 password_last_reset = VALUES(password_last_reset),
                 phone_number = VALUES(phone_number),
                 mobile_number = VALUES(mobile_number),
                 pin = VALUES(pin),
                 extension = VALUES(extension),
                 department = VALUES(department),
                 job_title = VALUES(job_title),
                 location = VALUES(location),
                 language = VALUES(language),
                 timezone = VALUES(timezone),
                 account_status = VALUES(account_status),
                 last_login = VALUES(last_login),
                 emergency_contact_name = VALUES(emergency_contact_name),
                 emergency_contact_phone = VALUES(emergency_contact_phone),
                 primary_phone_number_id = VALUES(primary_phone_number_id),
                 voicemail_box_id = VALUES(voicemail_box_id)",
        )
        .bind(user.id.to_string())
        .bind(&user.username)
        .bind(&user.password)
        .bind(&user.digest_password)
        .bind(&user.sip_ha1_hash)
        .bind(&user.display_name)
        .bind(&tenant_id)
        .bind(user.enabled)
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
        .bind(&role_str)
        .bind(&user.department_id)
        .bind(&primary_phone_number_id)
        .bind(&voicemail_box_id)
        .execute(&self.pool)
        .await?;

        debug!(username = %user.username, "User stored in MySQL");
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        sqlx::query("DELETE FROM users WHERE username = ?")
            .bind(username)
            .execute(&self.pool)
            .await?;

        debug!(username = %username, "User deleted from MySQL");
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, username, password_hash, digest_password, sip_ha1_hash, display_name, tenant_id, enabled,
                    created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, language, timezone,
                    account_status, last_login, emergency_contact_name, emergency_contact_phone,
                    role, department_id, primary_phone_number_id, voicemail_box_id
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
impl RegistrationStore for MysqlStorage {
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
        let expires_at = system_time_to_datetime(registration.expires_at);
        let registered_at = system_time_to_datetime(registration.registered_at);

        sqlx::query(
            "INSERT INTO registrations (aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at, path)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                 expires_at = VALUES(expires_at),
                 call_id = VALUES(call_id),
                 cseq = VALUES(cseq),
                 user_agent = VALUES(user_agent),
                 q = VALUES(q),
                 registered_at = VALUES(registered_at),
                 path = VALUES(path)",
        )
        .bind(&registration.aor)
        .bind(&registration.contact)
        .bind(expires_at)
        .bind(&registration.call_id)
        .bind(registration.cseq as i32)
        .bind(&registration.user_agent)
        .bind(registration.q)
        .bind(registered_at)
        .bind(&registration.path)
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
        let result = sqlx::query("DELETE FROM registrations WHERE expires_at <= NOW(6)")
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
impl CdrStore for MysqlStorage {
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()> {
        let disposition = serde_json::to_string(&cdr.disposition)?
            .trim_matches('"')
            .to_string();
        let hangup_cause = match cdr.hangup_cause {
            HangupCause::Other(code) => format!("other:{}", code),
            _ => serde_json::to_string(&cdr.hangup_cause)?
                .trim_matches('"')
                .to_string(),
        };
        let hangup_party = cdr.hangup_party.map(|p| {
            serde_json::to_string(&p)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string()
        });

        let custom_data = cdr
            .custom_data
            .map(|v| serde_json::to_value(&v))
            .transpose()?;

        let start_time = system_time_to_datetime(cdr.start_time);
        let answer_time = cdr.answer_time.map(system_time_to_datetime);
        let end_time = cdr.end_time.map(system_time_to_datetime);

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
        .bind(cdr.sip_status_code.map(|c| c as i32))
        .bind(&cdr.sip_reason)
        .bind(&cdr.route_id)
        .bind(&cdr.gateway_id)
        .bind(cdr.media_anchored)
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

        debug!(call_id = %cdr.call_id, "CDR stored in MySQL");
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
        let mut bindings: Vec<String> = Vec::new();

        if let Some(tenant_id) = &query.tenant_id {
            sql.push_str(" AND tenant_id = ?");
            bindings.push(tenant_id.to_string());
        }

        if let Some(call_id) = &query.call_id {
            sql.push_str(" AND call_id = ?");
            bindings.push(call_id.to_string());
        }

        if let Some(from_uri) = &query.from_uri {
            sql.push_str(" AND from_uri LIKE ?");
            bindings.push(format!("%{}%", from_uri));
        }

        if let Some(to_uri) = &query.to_uri {
            sql.push_str(" AND to_uri LIKE ?");
            bindings.push(format!("%{}%", to_uri));
        }

        if let Some(disposition) = &query.disposition {
            sql.push_str(" AND disposition = ?");
            let disp_str = serde_json::to_string(disposition)?
                .trim_matches('"')
                .to_string();
            bindings.push(disp_str);
        }

        sql.push_str(" ORDER BY start_time DESC");

        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }

        if let Some(offset) = query.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }

        // Build and execute query with dynamic bindings
        let mut query_builder = sqlx::query(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder.fetch_all(&self.pool).await?;

        let cdrs = rows
            .into_iter()
            .filter_map(|row| row_to_cdr(row).ok())
            .collect();

        Ok(cdrs)
    }

    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize> {
        let mut sql = String::from("SELECT COUNT(*) as count FROM cdrs WHERE 1=1");
        let mut bindings: Vec<String> = Vec::new();

        if let Some(tenant_id) = &query.tenant_id {
            sql.push_str(" AND tenant_id = ?");
            bindings.push(tenant_id.to_string());
        }

        if let Some(disposition) = &query.disposition {
            sql.push_str(" AND disposition = ?");
            let disp_str = serde_json::to_string(disposition)?
                .trim_matches('"')
                .to_string();
            bindings.push(disp_str);
        }

        let mut query_builder = sqlx::query(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let row = query_builder.fetch_one(&self.pool).await?;
        let count: i64 = row.get("count");

        Ok(count as usize)
    }
}

// ============================================================================
// TenantStore Implementation
// ============================================================================

#[async_trait]
impl TenantStore for MysqlStorage {
    async fn get_tenant(&self, id: &TenantId) -> Result<Option<Tenant>> {
        let row = sqlx::query("SELECT data FROM tenants WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let tenant: Tenant = serde_json::from_str(&data)?;
                Ok(Some(tenant))
            }
            None => Ok(None),
        }
    }

    async fn get_tenant_by_name(&self, name: &str) -> Result<Option<Tenant>> {
        let row = sqlx::query("SELECT data FROM tenants WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let data: String = row.get("data");
                let tenant: Tenant = serde_json::from_str(&data)?;
                Ok(Some(tenant))
            }
            None => Ok(None),
        }
    }

    async fn list_tenants(&self) -> Result<Vec<Tenant>> {
        let rows = sqlx::query("SELECT data FROM tenants ORDER BY name")
            .fetch_all(&self.pool)
            .await?;

        let mut tenants = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let tenant: Tenant = serde_json::from_str(&data)?;
            tenants.push(tenant);
        }

        Ok(tenants)
    }

    async fn put_tenant(&self, tenant: Tenant) -> Result<()> {
        let data = serde_json::to_string(&tenant)?;
        let created_at = tenant
            .created_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let updated_at = tenant
            .updated_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let trial_expires_at = tenant
            .trial_expires_at
            .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64);

        // Serialize status to snake_case string using serde
        let status_str = serde_json::to_string(&tenant.status)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "active".to_string());

        sqlx::query(
            "INSERT INTO tenants (id, name, description, status, contact_email, contact_phone,
                                  max_users, max_phone_numbers, max_concurrent_calls, billing_id,
                                  created_at, updated_at, trial_expires_at, metadata, data)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                 name = VALUES(name),
                 description = VALUES(description),
                 status = VALUES(status),
                 contact_email = VALUES(contact_email),
                 contact_phone = VALUES(contact_phone),
                 max_users = VALUES(max_users),
                 max_phone_numbers = VALUES(max_phone_numbers),
                 max_concurrent_calls = VALUES(max_concurrent_calls),
                 billing_id = VALUES(billing_id),
                 updated_at = VALUES(updated_at),
                 trial_expires_at = VALUES(trial_expires_at),
                 metadata = VALUES(metadata),
                 data = VALUES(data)",
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

        debug!(tenant_id = %tenant.id, tenant_name = %tenant.name, "Tenant stored in MySQL");
        Ok(())
    }

    async fn delete_tenant(&self, id: &TenantId) -> Result<()> {
        sqlx::query("DELETE FROM tenants WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        debug!(tenant_id = %id, "Tenant deleted from MySQL");
        Ok(())
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

/// Convert MySQL row to User
fn row_to_user(row: sqlx::mysql::MySqlRow) -> Result<User> {
    let media_prefs: Option<serde_json::Value> = row.get("media_preferences");
    let media_preferences = if let Some(json) = media_prefs {
        serde_json::from_value(json).unwrap_or_default()
    } else {
        MediaPreferences::default()
    };

    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");

    // Parse optional timestamp fields
    let password_last_reset: Option<DateTime<Utc>> = row.try_get("password_last_reset").ok();
    let password_last_reset = password_last_reset.map(datetime_to_system_time);

    let last_login: Option<DateTime<Utc>> = row.try_get("last_login").ok();
    let last_login = last_login.map(datetime_to_system_time);

    // Parse account status
    let account_status_str: String = row
        .try_get("account_status")
        .unwrap_or_else(|_| "active".to_string());
    let account_status =
        AccountStatus::from_str(&account_status_str).unwrap_or(AccountStatus::Active);

    // Parse role from database (defaults to EndUser if not found or invalid)
    let role_str: String = row
        .try_get("role")
        .unwrap_or_else(|_| "enduser".to_string());
    let role = parse_role(&role_str);

    // Parse emergency contact
    let emergency_contact = EmergencyContact {
        name: row.try_get("emergency_contact_name").ok(),
        phone: row.try_get("emergency_contact_phone").ok(),
    };

    // Parse user ID from database, or generate a new one if missing
    let id = row
        .try_get::<String, _>("id")
        .ok()
        .and_then(|s| UserId::from_str(&s).ok())
        .unwrap_or_else(UserId::new);

    Ok(User {
        id,
        username: row.get("username"),
        password: row.get("password_hash"),
        digest_password: row.try_get("digest_password").ok(),
        sip_ha1_hash: row.try_get("sip_ha1_hash").ok(),
        display_name: row.get("display_name"),
        tenant_id: TenantId::from_str(&row.get::<String, _>("tenant_id"))
            .unwrap_or_else(|_| TenantId::nil()),
        enabled: row.get("enabled"),
        role,                                             // User role for RBAC
        department_id: row.try_get("department_id").ok(), // Department ID for scoping
        created_at: datetime_to_system_time(created_at),
        updated_at: datetime_to_system_time(updated_at),
        media_preferences,
        email: row.try_get("email").ok(),
        password_last_reset,
        phone_number: row.try_get("phone_number").ok(),
        mobile_number: row.try_get("mobile_number").ok(),
        pin: row.try_get("pin").ok(),
        extension: row.try_get("extension").ok(),
        department: row.try_get("department").ok(),
        job_title: row.try_get("job_title").ok(),
        location: row.try_get("location").ok(),
        language: row
            .try_get("language")
            .unwrap_or_else(|_| "en-US".to_string()),
        timezone: row
            .try_get("timezone")
            .unwrap_or_else(|_| "UTC".to_string()),
        account_status,
        last_login,
        emergency_contact,
        primary_phone_number_id: row
            .try_get::<String, _>("primary_phone_number_id")
            .ok()
            .and_then(|s| PhoneNumberId::from_str(&s).ok()),
        voicemail_box_id: row
            .try_get::<String, _>("voicemail_box_id")
            .ok()
            .and_then(|s| VoicemailBoxId::from_str(&s).ok()),
    })
}

/// Convert MySQL row to Registration
fn row_to_registration(row: sqlx::mysql::MySqlRow) -> Result<Registration> {
    let expires_at: DateTime<Utc> = row.get("expires_at");
    let registered_at: DateTime<Utc> = row.get("registered_at");

    Ok(Registration {
        aor: row.get("aor"),
        contact: row.get("contact"),
        expires_at: datetime_to_system_time(expires_at),
        call_id: row.get("call_id"),
        cseq: row.get::<i32, _>("cseq") as u32,
        user_agent: row.get("user_agent"),
        q: row.get("q"),
        registered_at: datetime_to_system_time(registered_at),
        path: row.try_get("path").ok(),
        flow_token: row.try_get("flow_token").ok().flatten(),
    })
}

/// Convert MySQL row to CallDetailRecord
fn row_to_cdr(row: sqlx::mysql::MySqlRow) -> Result<CallDetailRecord> {
    let call_id = fcp_core::CallId::from_str(&row.get::<String, _>("call_id"))?;
    let tenant_id = TenantId::from_str(&row.get::<String, _>("tenant_id"))?;

    let disposition_str: String = row.get("disposition");
    let disposition: CallDisposition = serde_json::from_str(&format!("\"{}\"", disposition_str))?;

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

    let custom_data: Option<serde_json::Value> = row.get("custom_data");

    let start_time: DateTime<Utc> = row.get("start_time");
    let answer_time: Option<DateTime<Utc>> = row.get("answer_time");
    let end_time: Option<DateTime<Utc>> = row.get("end_time");

    Ok(CallDetailRecord {
        call_id,
        tenant_id,
        sip_call_id: row.get("sip_call_id"),
        from_uri: row.get("from_uri"),
        from_display_name: row.get("from_display_name"),
        to_uri: row.get("to_uri"),
        to_display_name: row.get("to_display_name"),
        start_time: datetime_to_system_time(start_time),
        answer_time: answer_time.map(datetime_to_system_time),
        end_time: end_time.map(datetime_to_system_time),
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
        recording_url: row.get("recording_url"),
        cost: row.get("cost"),
        currency: row.get("currency"),
        custom_data,
    })
}

// ============================================================================
// Tests
// ============================================================================

// ============================================================================
// Phone Number, Line Binding, Extension, and Device Store Implementations
// ============================================================================

#[async_trait]
impl PhoneNumberStore for MysqlStorage {
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
             ON DUPLICATE KEY UPDATE
                 state = VALUES(state),
                 e911_address_json = VALUES(e911_address_json),
                 e911_callback = VALUES(e911_callback),
                 cnam = VALUES(cnam),
                 spam_score = VALUES(spam_score),
                 carrier = VALUES(carrier),
                 lrn = VALUES(lrn),
                 ocn = VALUES(ocn),
                 rate_center = VALUES(rate_center),
                 lata = VALUES(lata),
                 activated_at = VALUES(activated_at),
                 ported_at = VALUES(ported_at),
                 loa_date = VALUES(loa_date),
                 foc_date = VALUES(foc_date),
                 recording_enabled = VALUES(recording_enabled),
                 block_anonymous = VALUES(block_anonymous),
                 allow_list = VALUES(allow_list),
                 block_list = VALUES(block_list),
                 tags = VALUES(tags),
                 updated_at = VALUES(updated_at)",
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
        .bind(&acquired_at)
        .bind(&activated_at)
        .bind(&ported_at)
        .bind(loa_date)
        .bind(foc_date)
        .bind(phone_number.recording_enabled)
        .bind(phone_number.block_anonymous)
        .bind(&allow_list)
        .bind(&block_list)
        .bind(&tags)
        .bind(&created_at)
        .bind(&updated_at)
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
#[async_trait]
impl LineBindingStore for MysqlStorage {
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

    async fn list_line_bindings_by_device(&self, device_id: &DeviceId) -> Result<Vec<LineBinding>> {
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
        let created_at = system_time_to_datetime(binding.created_at);
        let updated_at = system_time_to_datetime(binding.updated_at);

        sqlx::query(
            "INSERT INTO line_bindings (
                id, phone_number_id, owner_user_id, binding_type, ring_strategy, tenant_id,
                no_answer_timeout_secs, forward_on_busy, forward_on_no_answer,
                forward_unconditional, outbound_caller_id, outbound_cnam,
                created_at, updated_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                 ring_strategy = VALUES(ring_strategy),
                 no_answer_timeout_secs = VALUES(no_answer_timeout_secs),
                 forward_on_busy = VALUES(forward_on_busy),
                 forward_on_no_answer = VALUES(forward_on_no_answer),
                 forward_unconditional = VALUES(forward_unconditional),
                 outbound_caller_id = VALUES(outbound_caller_id),
                 outbound_cnam = VALUES(outbound_cnam),
                 updated_at = VALUES(updated_at)",
        )
        .bind(&id)
        .bind(&phone_number_id)
        .bind(&owner_user_id)
        .bind(&binding_type)
        .bind(&ring_strategy)
        .bind(&tenant_id)
        .bind(binding.no_answer_timeout.as_secs() as i32)
        .bind(&binding.forward_on_busy)
        .bind(&binding.forward_on_no_answer)
        .bind(&binding.forward_unconditional)
        .bind(&binding.outbound_caller_id)
        .bind(&binding.outbound_cnam)
        .bind(&created_at)
        .bind(&updated_at)
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

#[async_trait]
impl ExtensionStore for MysqlStorage {
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
        let created_at = system_time_to_datetime(extension.created_at);
        let updated_at = system_time_to_datetime(extension.updated_at);

        sqlx::query(
            "INSERT INTO extensions (
                id, number, target_type, target_id, tenant_id, description, enabled,
                created_at, updated_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                 target_type = VALUES(target_type),
                 target_id = VALUES(target_id),
                 description = VALUES(description),
                 enabled = VALUES(enabled),
                 updated_at = VALUES(updated_at)",
        )
        .bind(&id)
        .bind(&extension.number)
        .bind(target_type)
        .bind(&target_id)
        .bind(&tenant_id)
        .bind(&extension.description)
        .bind(extension.enabled)
        .bind(&created_at)
        .bind(&updated_at)
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

fn row_to_phone_number(row: sqlx::mysql::MySqlRow) -> Result<PhoneNumber> {
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

    let acquired_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("acquired_at"));
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
        created_at,
        updated_at,
    })
}

/// Convert SQLite row to LineBinding
fn row_to_line_binding(row: sqlx::mysql::MySqlRow) -> Result<LineBinding> {
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

    let no_answer_timeout = Duration::from_secs(row.get::<i64, _>("no_answer_timeout_secs") as u64);
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
fn row_to_extension(row: sqlx::mysql::MySqlRow) -> Result<Extension> {
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
        enabled: row.get::<bool, _>("enabled"),
        created_at,
        updated_at,
    })
}

/// Convert SQLite row to Device
fn row_to_device(row: sqlx::mysql::MySqlRow) -> Result<Device> {
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
        sip_uri: row.get("sip_uri"),
        registration_state,
        last_registration,
        last_ip_address: row.get("last_ip_address"),
        user_agent: row.get("user_agent"),
        max_lines: row.get::<i64, _>("max_lines") as u8,
        supports_video: row.get::<bool, _>("supports_video"),
        supports_blf: row.get::<bool, _>("supports_blf"),
        codec_support,
        location: row.get("location"),
        e911_override,
        provisioning_url: row.get("provisioning_url"),
        config_template_id: row.get("config_template_id"),
        enabled: row.get::<bool, _>("enabled"),
        created_at,
        updated_at,
    })
}

// ============================================================================
// DeviceStore Implementation
// ============================================================================

#[async_trait]
impl DeviceStore for MysqlStorage {
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
        let last_registration = device.last_registration.map(system_time_to_datetime);
        let codec_support = serde_json::to_value(
            &device
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

        sqlx::query(
            "INSERT INTO devices (
                id, name, device_type, mac_address, model, firmware_version, tenant_id, owner_user_id,
                sip_uri, registration_state, last_registration, last_ip_address, user_agent,
                max_lines, supports_video, supports_blf, codec_support,
                location, e911_override_json, provisioning_url, config_template_id, enabled,
                created_at, updated_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                 name = VALUES(name),
                 mac_address = VALUES(mac_address),
                 model = VALUES(model),
                 firmware_version = VALUES(firmware_version),
                 owner_user_id = VALUES(owner_user_id),
                 sip_uri = VALUES(sip_uri),
                 registration_state = VALUES(registration_state),
                 last_registration = VALUES(last_registration),
                 last_ip_address = VALUES(last_ip_address),
                 user_agent = VALUES(user_agent),
                 max_lines = VALUES(max_lines),
                 supports_video = VALUES(supports_video),
                 supports_blf = VALUES(supports_blf),
                 codec_support = VALUES(codec_support),
                 location = VALUES(location),
                 e911_override_json = VALUES(e911_override_json),
                 provisioning_url = VALUES(provisioning_url),
                 config_template_id = VALUES(config_template_id),
                 enabled = VALUES(enabled),
                 updated_at = VALUES(updated_at)",
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
        .bind(device.supports_video)
        .bind(device.supports_blf)
        .bind(&codec_support)
        .bind(&device.location)
        .bind(&e911_override_json)
        .bind(&device.provisioning_url)
        .bind(&device.config_template_id)
        .bind(device.enabled)
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

#[async_trait]
impl ConferenceStore for MysqlStorage {
    async fn get_meeting(
        &self,
        id: &fcp_core::MeetingId,
    ) -> Result<Option<fcp_core::Meeting>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, room_number, name, guest_pin, host_pin,
                    require_guest_pin, max_participants, wait_for_moderator,
                    enabled, auto_record, default_locked, permanent,
                    created_at, updated_at
             FROM conference_meetings WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(map_meeting_row_mysql(row)?)),
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
                    enabled, auto_record, default_locked, permanent,
                    created_at, updated_at
             FROM conference_meetings WHERE tenant_id = ? AND room_number = ?",
        )
        .bind(tenant_id.to_string())
        .bind(room_number)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(map_meeting_row_mysql(row)?)),
            None => Ok(None),
        }
    }

    async fn list_meetings(&self, tenant_id: &TenantId) -> Result<Vec<fcp_core::Meeting>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, room_number, name, guest_pin, host_pin,
                    require_guest_pin, max_participants, wait_for_moderator,
                    enabled, auto_record, default_locked, permanent,
                    created_at, updated_at
             FROM conference_meetings WHERE tenant_id = ? ORDER BY room_number",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let meetings = rows
            .into_iter()
            .filter_map(|row| map_meeting_row_mysql(row).ok())
            .collect();

        Ok(meetings)
    }

    async fn put_meeting(&self, meeting: fcp_core::Meeting) -> Result<()> {
        let created_at = system_time_to_datetime(meeting.created_at);
        let updated_at = system_time_to_datetime(SystemTime::now());

        sqlx::query(
            "INSERT INTO conference_meetings
             (id, tenant_id, room_number, name, guest_pin, host_pin,
              require_guest_pin, max_participants, wait_for_moderator,
              enabled, auto_record, default_locked, permanent,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
              tenant_id = VALUES(tenant_id),
              room_number = VALUES(room_number),
              name = VALUES(name),
              guest_pin = VALUES(guest_pin),
              host_pin = VALUES(host_pin),
              require_guest_pin = VALUES(require_guest_pin),
              max_participants = VALUES(max_participants),
              wait_for_moderator = VALUES(wait_for_moderator),
              enabled = VALUES(enabled),
              auto_record = VALUES(auto_record),
              default_locked = VALUES(default_locked),
              permanent = VALUES(permanent),
              updated_at = VALUES(updated_at)",
        )
        .bind(meeting.id.to_string())
        .bind(meeting.tenant_id.to_string())
        .bind(&meeting.room_number)
        .bind(&meeting.name)
        .bind(meeting.guest_pin.as_deref())
        .bind(meeting.host_pin.as_deref())
        .bind(meeting.require_guest_pin)
        .bind(meeting.max_participants as i32)
        .bind(meeting.wait_for_moderator)
        .bind(meeting.enabled)
        .bind(meeting.auto_record)
        .bind(meeting.default_locked)
        .bind(meeting.permanent)
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        debug!(id = %meeting.id, "Meeting stored");
        Ok(())
    }

    async fn delete_meeting(&self, id: &fcp_core::MeetingId) -> Result<()> {
        sqlx::query("DELETE FROM conference_meetings WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        debug!(id = %id, "Meeting deleted");
        Ok(())
    }
}

fn map_meeting_row_mysql(row: sqlx::mysql::MySqlRow) -> Result<fcp_core::Meeting> {
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
        created_at: datetime_to_system_time(created_at),
        updated_at: datetime_to_system_time(updated_at),
    })
}

#[async_trait]
impl QueueStore for MysqlStorage {
    async fn save_agents(&self, _tenant_id: &TenantId, _agents: Vec<StoredAgent>) -> Result<()> {
        Ok(()) // TODO: implement when MySQL queue persistence is needed
    }

    async fn load_agents(&self, _tenant_id: &TenantId) -> Result<Vec<StoredAgent>> {
        Ok(Vec::new())
    }

    async fn remove_agent(&self, _tenant_id: &TenantId, _agent_id: &str) -> Result<()> {
        Ok(())
    }

    async fn clear_agents(&self, _tenant_id: &TenantId) -> Result<()> {
        Ok(())
    }
}

// ============================================================================
// PresenceStore Implementation
// ============================================================================

#[async_trait]
impl PresenceStore for MysqlStorage {
    async fn save_subscription(&self, sub: StoredSubscription) -> Result<()> {
        sqlx::query(
            "INSERT INTO presence_subscriptions \
             (subscription_id, watcher_aor, presentity_aor, tenant_id, contact_uri, \
              event_package, expires_at_epoch, sip_call_id, sip_from_tag, sip_to_tag) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE \
                watcher_aor = VALUES(watcher_aor), \
                presentity_aor = VALUES(presentity_aor), \
                tenant_id = VALUES(tenant_id), \
                contact_uri = VALUES(contact_uri), \
                event_package = VALUES(event_package), \
                expires_at_epoch = VALUES(expires_at_epoch), \
                sip_call_id = VALUES(sip_call_id), \
                sip_from_tag = VALUES(sip_from_tag), \
                sip_to_tag = VALUES(sip_to_tag)",
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
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_subscriptions(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<StoredSubscription>> {
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let rows = sqlx::query(
            "SELECT subscription_id, watcher_aor, presentity_aor, tenant_id, \
             contact_uri, event_package, expires_at_epoch, sip_call_id, sip_from_tag, sip_to_tag \
             FROM presence_subscriptions WHERE tenant_id = ? AND expires_at_epoch > ?",
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
        sqlx::query("DELETE FROM presence_subscriptions WHERE subscription_id = ?")
            .bind(subscription_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn remove_subscriptions_for_presentity(&self, presentity_aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM presence_subscriptions WHERE presentity_aor = ?")
            .bind(presentity_aor)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn cleanup_expired_subscriptions(&self) -> Result<usize> {
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let result =
            sqlx::query("DELETE FROM presence_subscriptions WHERE expires_at_epoch <= ?")
                .bind(now_epoch)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() as usize)
    }

    async fn save_presentity(&self, p: StoredPresentity) -> Result<()> {
        sqlx::query(
            "INSERT INTO presence_presentities \
             (aor, state, note, tenant_id, updated_at_epoch) \
             VALUES (?, ?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE \
                state = VALUES(state), \
                note = VALUES(note), \
                tenant_id = VALUES(tenant_id), \
                updated_at_epoch = VALUES(updated_at_epoch)",
        )
        .bind(&p.aor)
        .bind(&p.state)
        .bind(&p.note)
        .bind(p.tenant_id.to_string())
        .bind(p.updated_at_epoch as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_presentities(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<StoredPresentity>> {
        let rows = sqlx::query(
            "SELECT aor, state, note, tenant_id, updated_at_epoch \
             FROM presence_presentities WHERE tenant_id = ?",
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

    async fn remove_presentity(&self, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM presence_presentities WHERE aor = ?")
            .bind(aor)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn save_auth_policy(&self, policy: StoredAuthPolicy) -> Result<()> {
        sqlx::query(
            "INSERT INTO presence_auth_policies \
             (aor, default_allow, allow_list_json, block_list_json, tenant_id, \
              event_allow_json, event_block_json, time_windows_json, \
              allow_patterns_json, block_patterns_json) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE \
                default_allow = VALUES(default_allow), \
                allow_list_json = VALUES(allow_list_json), \
                block_list_json = VALUES(block_list_json), \
                tenant_id = VALUES(tenant_id), \
                event_allow_json = VALUES(event_allow_json), \
                event_block_json = VALUES(event_block_json), \
                time_windows_json = VALUES(time_windows_json), \
                allow_patterns_json = VALUES(allow_patterns_json), \
                block_patterns_json = VALUES(block_patterns_json)",
        )
        .bind(&policy.aor)
        .bind(policy.default_allow as i32)
        .bind(&policy.allow_list_json)
        .bind(&policy.block_list_json)
        .bind(policy.tenant_id.to_string())
        .bind(&policy.event_allow_json)
        .bind(&policy.event_block_json)
        .bind(&policy.time_windows_json)
        .bind(&policy.allow_patterns_json)
        .bind(&policy.block_patterns_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_auth_policies(&self, tenant_id: &TenantId) -> Result<Vec<StoredAuthPolicy>> {
        let rows = sqlx::query(
            "SELECT aor, default_allow, allow_list_json, block_list_json, tenant_id, \
                    event_allow_json, event_block_json, time_windows_json, \
                    allow_patterns_json, block_patterns_json \
             FROM presence_auth_policies WHERE tenant_id = ?",
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
                default_allow: row.try_get::<i32, _>("default_allow")? != 0,
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

    async fn remove_auth_policy(&self, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM presence_auth_policies WHERE aor = ?")
            .bind(aor)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl ResourceListStore for MysqlStorage {
    async fn save_resource_list(&self, list: &StoredResourceList) -> Result<()> {
        sqlx::query(
            "INSERT INTO resource_lists (uri, name, entries_json, tenant_id)
             VALUES (?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                name = VALUES(name),
                entries_json = VALUES(entries_json),
                tenant_id = VALUES(tenant_id)",
        )
        .bind(&list.uri)
        .bind(&list.name)
        .bind(&list.entries_json)
        .bind(list.tenant_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_resource_list(&self, uri: &str) -> Result<Option<StoredResourceList>> {
        let row = sqlx::query("SELECT uri, name, entries_json, tenant_id FROM resource_lists WHERE uri = ?")
            .bind(uri)
            .fetch_optional(&self.pool)
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
        let rows = sqlx::query("SELECT uri, name, entries_json, tenant_id FROM resource_lists WHERE tenant_id = ?")
            .bind(tenant_id.to_string())
            .fetch_all(&self.pool)
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

    async fn delete_resource_list(&self, uri: &str) -> Result<()> {
        sqlx::query("DELETE FROM resource_lists WHERE uri = ?")
            .bind(uri)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl WebhookStore for MysqlStorage {
    async fn create_endpoint(&self, _endpoint: WebhookEndpoint) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Mysql backend")
    }

    async fn get_endpoint(&self, _id: &WebhookId) -> Result<Option<WebhookEndpoint>> {
        anyhow::bail!("Webhook store not yet implemented for Mysql backend")
    }

    async fn list_endpoints(&self, _tenant_id: &TenantId) -> Result<Vec<WebhookEndpoint>> {
        anyhow::bail!("Webhook store not yet implemented for Mysql backend")
    }

    async fn update_endpoint(&self, _endpoint: WebhookEndpoint) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Mysql backend")
    }

    async fn delete_endpoint(&self, _id: &WebhookId) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Mysql backend")
    }

    async fn log_delivery(&self, _log: WebhookDeliveryLog) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Mysql backend")
    }

    async fn get_pending_retries(&self, _limit: usize) -> Result<Vec<WebhookDeliveryLog>> {
        anyhow::bail!("Webhook store not yet implemented for Mysql backend")
    }

    async fn update_delivery_log(&self, _log: WebhookDeliveryLog) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Mysql backend")
    }

    async fn get_delivery_logs(&self, _endpoint_id: &WebhookId, _limit: usize) -> Result<Vec<WebhookDeliveryLog>> {
        anyhow::bail!("Webhook store not yet implemented for Mysql backend")
    }
}

#[cfg(test)]
mod tests {
    // Note: Integration tests require a running MySQL instance
    // Run these tests with: cargo test --features integration-tests -- --ignored
    // Or manually with a MySQL instance:
    // 1. Start MySQL: docker run --rm -p 3306:3306 -e MYSQL_ROOT_PASSWORD=root -e MYSQL_DATABASE=fcp mysql:8
    // 2. Run: DATABASE_URL=mysql://root:root@localhost/fcp cargo test

    #[tokio::test]
    #[ignore] // Requires MySQL instance
    async fn test_connection() {
        // This test requires a running MySQL instance
        // Uncomment and adjust connection string as needed
        /*
        use super::*;
        let config = MysqlConfig::new("mysql://root:root@localhost/fcp");
        let storage = MysqlStorage::new(config).await;
        assert!(storage.is_ok());
        */
    }
}
