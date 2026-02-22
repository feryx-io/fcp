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

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::{
    storage::{
        CdrStore, ConferenceStore, CredentialStore, DeviceStore, ExtensionStore, GeoRoutingStore,
        LineBindingStore, PhoneNumberStore, PresenceStore, QueueStore, RegistrationStore,
        ResourceListStore, StoredAgent, StoredAuthPolicy, StoredPresentity, StoredResourceList,
        StoredSubscription, TenantStore, VoicemailStore, WebhookDeliveryLog, WebhookEndpoint,
        WebhookStore,
    },
    AccountStatus, AudioCodec, BindingType, CallDetailRecord, CallDisposition, CdrQuery, Device,
    DeviceBinding, DeviceId, DeviceType, EmergencyContact, Extension, ExtensionId, ExtensionTarget,
    HangupCause, HangupParty, LineBinding, LineBindingId, MediaPreferences, NumberState,
    PhoneNumber, PhoneNumberId, Registration, RegistrationState, RingStrategy, Tenant, TenantId,
    User, UserId, WebhookId,
};
use fcp_core::porting::PortingStore;
use fcp_flow::schema::{FlowDefinition, FlowNode};
use fcp_flow::storage::{FlowQuery, FlowQueryResult, FlowStore, SortField, SortOrder};
use fcp_flow::FlowError;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Pool, Postgres, Row};
use std::collections::HashMap;
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

/// Configuration for PostgreSQL storage backend.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// PostgreSQL connection string (e.g., "postgresql://user:pass@localhost/fcp")
    pub connection_string: String,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Connection timeout
    pub connection_timeout: Duration,
}

impl PostgresConfig {
    /// Create a new PostgreSQL configuration with default settings.
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

impl Default for PostgresConfig {
    fn default() -> Self {
        Self::new("postgresql://fcp:fcp@localhost/fcp")
    }
}

/// PostgreSQL storage backend implementing all FCP storage traits.
#[derive(Clone)]
pub struct PostgresStorage {
    pool: Pool<Postgres>,
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

        Ok(Self { pool })
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
        let row = sqlx::query(
            "SELECT id, username, password_hash, digest_password, sip_ha1_hash, display_name, tenant_id,
                    role, department_id, enabled, created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, language, timezone,
                    account_status, last_login, emergency_contact_json,
                    primary_phone_number_id, voicemail_box_id
             FROM users
             WHERE username = $1",
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

        sqlx::query(
            "INSERT INTO users (
                id, username, password_hash, digest_password, sip_ha1_hash, display_name, tenant_id,
                role, department_id, enabled, created_at, updated_at, media_preferences,
                email, password_last_reset, phone_number, mobile_number, pin,
                extension, department, job_title, location, language, timezone,
                account_status, last_login, emergency_contact_json,
                primary_phone_number_id, voicemail_box_id
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29)
             ON CONFLICT(username, tenant_id) DO UPDATE SET
                 password_hash = EXCLUDED.password_hash,
                 digest_password = EXCLUDED.digest_password,
                 sip_ha1_hash = EXCLUDED.sip_ha1_hash,
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
                 language = EXCLUDED.language,
                 timezone = EXCLUDED.timezone,
                 account_status = EXCLUDED.account_status,
                 last_login = EXCLUDED.last_login,
                 emergency_contact_json = EXCLUDED.emergency_contact_json,
                 primary_phone_number_id = EXCLUDED.primary_phone_number_id,
                 voicemail_box_id = EXCLUDED.voicemail_box_id",
        )
        .bind(&id)
        .bind(&user.username)
        .bind(&user.password)
        .bind(&user.digest_password)
        .bind(&user.sip_ha1_hash)
        .bind(&user.display_name)
        .bind(&tenant_id)
        .bind(&role)
        .bind(&user.department_id)
        .bind(user.enabled)
        .bind(&created_at)
        .bind(&updated_at)
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
        .bind(&user.language)
        .bind(&user.timezone)
        .bind(&account_status)
        .bind(last_login)
        .bind(&emergency_contact)
        .bind(&primary_phone_number_id)
        .bind(&voicemail_box_id)
        .execute(&self.pool)
        .await?;

        debug!(username = %user.username, "User stored");
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        sqlx::query("DELETE FROM users WHERE username = $1")
            .bind(username)
            .execute(&self.pool)
            .await?;

        debug!(username = %username, "User deleted");
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, username, password_hash, digest_password, sip_ha1_hash, display_name, tenant_id,
                    role, department_id, enabled, created_at, updated_at, media_preferences,
                    email, password_last_reset, phone_number, mobile_number, pin,
                    extension, department, job_title, location, language, timezone,
                    account_status, last_login, emergency_contact_json,
                    primary_phone_number_id, voicemail_box_id
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
impl RegistrationStore for PostgresStorage {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        let rows = sqlx::query(
            "SELECT aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at
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

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        let expires_at = system_time_to_datetime(registration.expires_at);
        let registered_at = system_time_to_datetime(registration.registered_at);

        sqlx::query(
            "INSERT INTO registrations (aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at, path)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (aor, contact)
             DO UPDATE SET
                 expires_at = EXCLUDED.expires_at,
                 call_id = EXCLUDED.call_id,
                 cseq = EXCLUDED.cseq,
                 user_agent = EXCLUDED.user_agent,
                 q = EXCLUDED.q,
                 registered_at = EXCLUDED.registered_at,
                 path = EXCLUDED.path",
        )
        .bind(&registration.aor)
        .bind(&registration.contact)
        .bind(expires_at)
        .bind(&registration.call_id)
        .bind(registration.cseq as i64)
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
        sqlx::query("DELETE FROM registrations WHERE aor = $1 AND contact = $2")
            .bind(aor)
            .bind(contact)
            .execute(&self.pool)
            .await?;

        debug!(aor = %aor, contact = %contact, "Registration deleted");
        Ok(())
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM registrations WHERE aor = $1")
            .bind(aor)
            .execute(&self.pool)
            .await?;

        debug!(aor = %aor, "All registrations for AOR deleted");
        Ok(())
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        let result = sqlx::query("DELETE FROM registrations WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await?;

        let count = result.rows_affected() as usize;
        if count > 0 {
            debug!(count = count, "Expired registrations cleaned up");
        }

        Ok(count)
    }
}

// ============================================================================
// TenantStore Implementation
// ============================================================================

#[async_trait]
impl TenantStore for PostgresStorage {
    async fn get_tenant(&self, id: &TenantId) -> Result<Option<Tenant>> {
        let row = sqlx::query(
            "SELECT id, name, description, status, contact_email, contact_phone,
                    max_users, max_phone_numbers, max_concurrent_calls, billing_id,
                    created_at, updated_at, trial_expires_at, metadata
             FROM tenants
             WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_tenant(row)?)),
            None => Ok(None),
        }
    }

    async fn get_tenant_by_name(&self, name: &str) -> Result<Option<Tenant>> {
        let row = sqlx::query(
            "SELECT id, name, description, status, contact_email, contact_phone,
                    max_users, max_phone_numbers, max_concurrent_calls, billing_id,
                    created_at, updated_at, trial_expires_at, metadata
             FROM tenants
             WHERE name = $1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_tenant(row)?)),
            None => Ok(None),
        }
    }

    async fn list_tenants(&self) -> Result<Vec<Tenant>> {
        let rows = sqlx::query(
            "SELECT id, name, description, status, contact_email, contact_phone,
                    max_users, max_phone_numbers, max_concurrent_calls, billing_id,
                    created_at, updated_at, trial_expires_at, metadata
             FROM tenants
             ORDER BY name",
        )
        .fetch_all(&self.pool)
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

        sqlx::query(
            "INSERT INTO tenants (
                id, name, description, status, contact_email, contact_phone,
                max_users, max_phone_numbers, max_concurrent_calls, billing_id,
                created_at, updated_at, trial_expires_at, metadata, data
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, '{}'::jsonb)
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
                 metadata = EXCLUDED.metadata",
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
        .execute(&self.pool)
        .await?;

        debug!(id = %tenant.id, name = %tenant.name, "Tenant stored");
        Ok(())
    }

    async fn delete_tenant(&self, id: &TenantId) -> Result<()> {
        sqlx::query("DELETE FROM tenants WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
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
    })
}

// ============================================================================
// CdrStore Implementation
// ============================================================================

#[async_trait]
impl CdrStore for PostgresStorage {
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()> {
        let call_id = cdr.call_id.to_string();
        let tenant_id = cdr.tenant_id.to_string();
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
                recording_url, cost, currency, custom_data, created_at
             ) VALUES (
                $1, $2, $3,
                $4, $5, $6, $7,
                $8, $9, $10,
                $11, $12, $13,
                $14, $15, $16,
                $17, $18,
                $19, $20,
                $21, $22, $23,
                $24, $25, $26, $27,
                $28, $29, $30, $31, NOW()
             )
             ON CONFLICT(call_id) DO UPDATE SET
                 answer_time = EXCLUDED.answer_time,
                 end_time = EXCLUDED.end_time,
                 setup_duration_ms = EXCLUDED.setup_duration_ms,
                 call_duration_ms = EXCLUDED.call_duration_ms,
                 total_duration_ms = EXCLUDED.total_duration_ms,
                 disposition = EXCLUDED.disposition,
                 hangup_cause = EXCLUDED.hangup_cause,
                 hangup_party = EXCLUDED.hangup_party,
                 rtp_packets_sent = EXCLUDED.rtp_packets_sent,
                 rtp_packets_received = EXCLUDED.rtp_packets_received,
                 rtp_bytes_sent = EXCLUDED.rtp_bytes_sent,
                 rtp_bytes_received = EXCLUDED.rtp_bytes_received,
                 recording_url = EXCLUDED.recording_url,
                 cost = EXCLUDED.cost,
                 currency = EXCLUDED.currency,
                 custom_data = EXCLUDED.custom_data",
        )
        .bind(&call_id)
        .bind(&tenant_id)
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

        debug!(call_id = %call_id, "CDR stored");
        Ok(())
    }

    async fn get_cdr(&self, call_id: &fcp_core::CallId) -> Result<Option<CallDetailRecord>> {
        let row = sqlx::query("SELECT * FROM cdrs WHERE call_id = $1")
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
            let dt = system_time_to_datetime(start_from);
            conditions.push(("start_time_from", dt.to_rfc3339()));
        }

        if let Some(start_to) = query.start_time_to {
            let dt = system_time_to_datetime(start_to);
            conditions.push(("start_time_to", dt.to_rfc3339()));
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
                    sql.push_str(" AND from_uri LIKE $");
                }
                "to_uri_like" => {
                    sql.push_str(" AND to_uri LIKE $");
                }
                "start_time_from" => {
                    sql.push_str(" AND start_time >= $");
                }
                "start_time_to" => {
                    sql.push_str(" AND start_time <= $");
                }
                _ => {
                    sql.push_str(&format!(" AND {} = $", key));
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
            sql.push_str(" AND tenant_id = $1");
            conditions.push(tenant_id.to_string());
        }

        if let Some(disposition) = &query.disposition {
            let param_num = if conditions.is_empty() {
                1
            } else {
                conditions.len() + 1
            };
            sql.push_str(&format!(" AND disposition = ${}", param_num));
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
// Helper Functions for Row Conversion
// ============================================================================

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

    Ok(User {
        id,
        username: row.get("username"),
        password: row.get("password_hash"),
        digest_password: row.get("digest_password"),
        sip_ha1_hash: row.get("sip_ha1_hash"),
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
        language: row.get("language"),
        timezone: row.get("timezone"),
        account_status,
        last_login,
        emergency_contact,
        primary_phone_number_id,
        voicemail_box_id,
    })
}

/// Convert PostgreSQL row to Registration
fn row_to_registration(row: sqlx::postgres::PgRow) -> Result<Registration> {
    let expires_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("expires_at"));
    let registered_at = datetime_to_system_time(row.get::<DateTime<Utc>, _>("registered_at"));

    Ok(Registration {
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

    let start_time = datetime_to_system_time(row.get::<DateTime<Utc>, _>("start_time"));
    let answer_time = row
        .get::<Option<DateTime<Utc>>, _>("answer_time")
        .map(datetime_to_system_time);
    let end_time = row
        .get::<Option<DateTime<Utc>>, _>("end_time")
        .map(datetime_to_system_time);

    Ok(CallDetailRecord {
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
                    created_at, updated_at
             FROM phone_numbers
             WHERE id = $1",
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
             WHERE e164 = $1",
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
             WHERE tenant_id = $1
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29)
             ON CONFLICT(id) DO UPDATE SET
                 state = EXCLUDED.state,
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
        sqlx::query("DELETE FROM phone_numbers WHERE id = $1")
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
impl LineBindingStore for PostgresStorage {
    async fn get_line_binding(&self, id: &LineBindingId) -> Result<Option<LineBinding>> {
        let row = sqlx::query(
            "SELECT id, phone_number_id, owner_user_id, binding_type, ring_strategy, tenant_id,
                    no_answer_timeout_secs, forward_on_busy, forward_on_no_answer,
                    forward_unconditional, outbound_caller_id, outbound_cnam,
                    created_at, updated_at
             FROM line_bindings
             WHERE id = $1",
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
             WHERE phone_number_id = $1",
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
             WHERE tenant_id = $1
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
             WHERE owner_user_id = $1
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
             WHERE db.device_id = $1
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             ON CONFLICT(id) DO UPDATE SET
                 ring_strategy = EXCLUDED.ring_strategy,
                 no_answer_timeout_secs = EXCLUDED.no_answer_timeout_secs,
                 forward_on_busy = EXCLUDED.forward_on_busy,
                 forward_on_no_answer = EXCLUDED.forward_on_no_answer,
                 forward_unconditional = EXCLUDED.forward_unconditional,
                 outbound_caller_id = EXCLUDED.outbound_caller_id,
                 outbound_cnam = EXCLUDED.outbound_cnam,
                 updated_at = EXCLUDED.updated_at",
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
        sqlx::query("DELETE FROM device_bindings WHERE line_binding_id = $1")
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
        sqlx::query("DELETE FROM line_bindings WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        debug!(id = %id, "Line binding deleted");
        Ok(())
    }
}

#[async_trait]
impl ExtensionStore for PostgresStorage {
    async fn get_extension(&self, id: &ExtensionId) -> Result<Option<Extension>> {
        let row = sqlx::query(
            "SELECT id, number, target_type, target_id, tenant_id, description, enabled,
                    created_at, updated_at
             FROM extensions
             WHERE id = $1",
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
             WHERE tenant_id = $1 AND number = $2",
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
             WHERE tenant_id = $1
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT(tenant_id, number) DO UPDATE SET
                 target_type = EXCLUDED.target_type,
                 target_id = EXCLUDED.target_id,
                 description = EXCLUDED.description,
                 enabled = EXCLUDED.enabled,
                 updated_at = EXCLUDED.updated_at",
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
        sqlx::query("DELETE FROM extensions WHERE id = $1")
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
impl DeviceStore for PostgresStorage {
    async fn get_device(&self, id: &DeviceId) -> Result<Option<Device>> {
        let row = sqlx::query(
            "SELECT id, name, device_type, mac_address, model, firmware_version, tenant_id, owner_user_id,
                    sip_uri, registration_state, last_registration, last_ip_address, user_agent,
                    max_lines, supports_video, supports_blf, codec_support,
                    location, e911_override_json, provisioning_url, config_template_id, enabled,
                    created_at, updated_at
             FROM devices
             WHERE id = $1",
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
             WHERE sip_uri = $1",
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
             WHERE tenant_id = $1
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
             WHERE owner_user_id = $1
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24)
             ON CONFLICT(id) DO UPDATE SET
                 name = EXCLUDED.name,
                 mac_address = EXCLUDED.mac_address,
                 model = EXCLUDED.model,
                 firmware_version = EXCLUDED.firmware_version,
                 owner_user_id = EXCLUDED.owner_user_id,
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
                 e911_override_json = EXCLUDED.e911_override_json,
                 provisioning_url = EXCLUDED.provisioning_url,
                 config_template_id = EXCLUDED.config_template_id,
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
        sqlx::query("DELETE FROM devices WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        debug!(id = %id, "Device deleted");
        Ok(())
    }
}

#[async_trait]
impl ConferenceStore for PostgresStorage {
    async fn get_meeting(
        &self,
        id: &fcp_core::MeetingId,
    ) -> Result<Option<fcp_core::Meeting>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, room_number, name, guest_pin, host_pin,
                    require_guest_pin, max_participants, wait_for_moderator,
                    enabled, auto_record, default_locked, permanent,
                    created_at, updated_at
             FROM conference_meetings WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(map_meeting_row_pg(row)?)),
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
             FROM conference_meetings WHERE tenant_id = $1 AND room_number = $2",
        )
        .bind(tenant_id.to_string())
        .bind(room_number)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(map_meeting_row_pg(row)?)),
            None => Ok(None),
        }
    }

    async fn list_meetings(&self, tenant_id: &TenantId) -> Result<Vec<fcp_core::Meeting>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, room_number, name, guest_pin, host_pin,
                    require_guest_pin, max_participants, wait_for_moderator,
                    enabled, auto_record, default_locked, permanent,
                    created_at, updated_at
             FROM conference_meetings WHERE tenant_id = $1 ORDER BY room_number",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let meetings = rows
            .into_iter()
            .filter_map(|row| map_meeting_row_pg(row).ok())
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
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
              updated_at = EXCLUDED.updated_at",
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
        sqlx::query("DELETE FROM conference_meetings WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
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
        created_at: datetime_to_system_time(created_at),
        updated_at: datetime_to_system_time(updated_at),
    })
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

    let no_answer_timeout = Duration::from_secs(row.get::<i32, _>("no_answer_timeout_secs") as u64);
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
        enabled: row.get::<bool, _>("enabled"),
        created_at,
        updated_at,
    })
}

/// Convert SQLite row to Device
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
        e911_override,
        provisioning_url: row.get("provisioning_url"),
        config_template_id: row.get("config_template_id"),
        enabled: row.get::<bool, _>("enabled"),
        created_at,
        updated_at,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[async_trait]
impl QueueStore for PostgresStorage {
    async fn save_agents(&self, _tenant_id: &TenantId, _agents: Vec<StoredAgent>) -> Result<()> {
        Ok(()) // TODO: implement when PostgreSQL queue persistence is needed
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
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn remove_subscriptions_for_presentity(&self, presentity_aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM presence_subscriptions WHERE presentity_aor = $1")
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
            sqlx::query("DELETE FROM presence_subscriptions WHERE expires_at_epoch <= $1")
                .bind(now_epoch)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() as usize)
    }

    async fn save_presentity(&self, p: StoredPresentity) -> Result<()> {
        sqlx::query(
            "INSERT INTO presence_presentities \
             (aor, state, note, tenant_id, updated_at_epoch) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (aor) DO UPDATE SET \
                state = EXCLUDED.state, \
                note = EXCLUDED.note, \
                tenant_id = EXCLUDED.tenant_id, \
                updated_at_epoch = EXCLUDED.updated_at_epoch",
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

    async fn remove_presentity(&self, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM presence_presentities WHERE aor = $1")
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT (aor) DO UPDATE SET \
                default_allow = EXCLUDED.default_allow, \
                allow_list_json = EXCLUDED.allow_list_json, \
                block_list_json = EXCLUDED.block_list_json, \
                tenant_id = EXCLUDED.tenant_id, \
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
        .execute(&self.pool)
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

    async fn remove_auth_policy(&self, aor: &str) -> Result<()> {
        sqlx::query("DELETE FROM presence_auth_policies WHERE aor = $1")
            .bind(aor)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl ResourceListStore for PostgresStorage {
    async fn save_resource_list(&self, list: &StoredResourceList) -> Result<()> {
        sqlx::query(
            "INSERT INTO resource_lists (uri, name, entries_json, tenant_id)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(uri) DO UPDATE SET
                name = EXCLUDED.name,
                entries_json = EXCLUDED.entries_json,
                tenant_id = EXCLUDED.tenant_id",
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
        let row = sqlx::query("SELECT uri, name, entries_json, tenant_id FROM resource_lists WHERE uri = $1")
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
        let rows = sqlx::query("SELECT uri, name, entries_json, tenant_id FROM resource_lists WHERE tenant_id = $1")
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
        sqlx::query("DELETE FROM resource_lists WHERE uri = $1")
            .bind(uri)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl WebhookStore for PostgresStorage {
    async fn create_endpoint(&self, _endpoint: WebhookEndpoint) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Postgres backend")
    }

    async fn get_endpoint(&self, _id: &WebhookId) -> Result<Option<WebhookEndpoint>> {
        anyhow::bail!("Webhook store not yet implemented for Postgres backend")
    }

    async fn list_endpoints(&self, _tenant_id: &TenantId) -> Result<Vec<WebhookEndpoint>> {
        anyhow::bail!("Webhook store not yet implemented for Postgres backend")
    }

    async fn update_endpoint(&self, _endpoint: WebhookEndpoint) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Postgres backend")
    }

    async fn delete_endpoint(&self, _id: &WebhookId) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Postgres backend")
    }

    async fn log_delivery(&self, _log: WebhookDeliveryLog) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Postgres backend")
    }

    async fn get_pending_retries(&self, _limit: usize) -> Result<Vec<WebhookDeliveryLog>> {
        anyhow::bail!("Webhook store not yet implemented for Postgres backend")
    }

    async fn update_delivery_log(&self, _log: WebhookDeliveryLog) -> Result<()> {
        anyhow::bail!("Webhook store not yet implemented for Postgres backend")
    }

    async fn get_delivery_logs(&self, _endpoint_id: &WebhookId, _limit: usize) -> Result<Vec<WebhookDeliveryLog>> {
        anyhow::bail!("Webhook store not yet implemented for Postgres backend")
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

    let nodes: HashMap<String, FlowNode> = serde_json::from_value(nodes_json)?;
    let variables: HashMap<String, String> = serde_json::from_value(variables_json)?;

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
        .fetch_optional(&self.pool)
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
        .fetch_optional(&self.pool)
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
            .fetch_all(&self.pool)
            .await
            .map_err(|e| FlowError::StorageError(e.to_string()))?
        } else {
            sqlx::query(
                "SELECT id, name, description, version, initial_node, nodes, variables,
                        tenant_id, enabled, webhook_url, created_at, updated_at
                 FROM flows
                 ORDER BY updated_at DESC",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| FlowError::StorageError(e.to_string()))?
        };

        let mut flows = Vec::new();
        for row in &rows {
            flows.push(parse_flow_from_row_pg(row)?);
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
            .fetch_one(&self.pool)
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
            .fetch_all(&self.pool)
            .await
            .map_err(|e| FlowError::StorageError(e.to_string()))?;

        let mut flows = Vec::new();
        for row in &rows {
            flows.push(parse_flow_from_row_pg(row)?);
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
        .execute(&self.pool)
        .await
        .map_err(|e| FlowError::StorageError(e.to_string()))?;

        debug!(flow_id = %flow.id, version = flow.version, "Flow inserted into PostgreSQL");
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
        .execute(&self.pool)
        .await
        .map_err(|e| FlowError::StorageError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(FlowError::NotFound(format!("Flow not found: {}", flow.id)));
        }

        debug!(flow_id = %flow.id, "Flow updated in PostgreSQL");
        Ok(())
    }

    async fn delete_flow(&self, id: &str) -> std::result::Result<(), FlowError> {
        let result =
            sqlx::query("UPDATE flows SET enabled = false, updated_at = NOW() WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| FlowError::StorageError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(FlowError::NotFound(format!("Flow not found: {}", id)));
        }

        debug!(flow_id = %id, "Flow deleted (soft delete) in PostgreSQL");
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
        .fetch_all(&self.pool)
        .await
        .map_err(|e| FlowError::StorageError(e.to_string()))?;

        let mut flows = Vec::new();
        for row in &rows {
            flows.push(parse_flow_from_row_pg(row)?);
        }

        Ok(flows)
    }

    async fn enable_flow(&self, id: &str) -> std::result::Result<(), FlowError> {
        let result =
            sqlx::query("UPDATE flows SET enabled = true, updated_at = NOW() WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
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
                .execute(&self.pool)
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
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

        // Fetch greetings for this mailbox
        let greeting_rows = sqlx::query(
            "SELECT greeting_type, audio_path, created_at, expires_at
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
            let g_created_at: DateTime<Utc> = greeting_row.try_get("created_at")?;
            let expires_at: Option<DateTime<Utc>> = greeting_row.try_get("expires_at")?;

            greetings.push(fcp_core::VoicemailGreeting {
                greeting_type,
                audio_path: std::path::PathBuf::from(audio_path),
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
                    created_at, updated_at
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
                    created_at, updated_at
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
                    created_at, updated_at
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
                    created_at, updated_at
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
                    created_at, updated_at
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
              created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
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
        .bind(created_at)
        .bind(now)
        .execute(&self.pool)
        .await?;

        // Delete existing greetings for this mailbox
        sqlx::query("DELETE FROM voicemail_greetings WHERE mailbox_id = $1")
            .bind(&mailbox.id)
            .execute(&self.pool)
            .await?;

        // Insert greetings
        for greeting in &mailbox.greetings {
            let greeting_created_at = system_time_to_datetime(greeting.created_at);
            let expires_at = greeting.expires_at.map(system_time_to_datetime);

            sqlx::query(
                "INSERT INTO voicemail_greetings
                 (mailbox_id, greeting_type, audio_path, created_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&mailbox.id)
            .bind(greeting.greeting_type.to_string())
            .bind(greeting.audio_path.to_string_lossy().as_ref())
            .bind(greeting_created_at)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    async fn delete_mailbox(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM voicemail_mailboxes WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_message(&self, id: &str) -> Result<Option<fcp_core::VoicemailMessage>> {
        let row = sqlx::query(
            "SELECT id, mailbox_id, caller, caller_name, status, priority, duration,
                    recording_path, transcription, created_at, accessed_at
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
                    recording_path, transcription, created_at, accessed_at
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
              recording_path, transcription, created_at, accessed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (id) DO UPDATE SET
                 mailbox_id = EXCLUDED.mailbox_id,
                 caller = EXCLUDED.caller,
                 caller_name = EXCLUDED.caller_name,
                 status = EXCLUDED.status,
                 priority = EXCLUDED.priority,
                 duration = EXCLUDED.duration,
                 recording_path = EXCLUDED.recording_path,
                 transcription = EXCLUDED.transcription,
                 accessed_at = EXCLUDED.accessed_at",
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
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_message(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM voicemail_messages WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_messages_by_status(
        &self,
        mailbox_id: &str,
        status: fcp_core::MessageStatus,
    ) -> Result<Vec<fcp_core::VoicemailMessage>> {
        let rows = sqlx::query(
            "SELECT id, mailbox_id, caller, caller_name, status, priority, duration,
                    recording_path, transcription, created_at, accessed_at
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
    async fn get_geo_rule(&self, _id: &str) -> Result<Option<fcp_core::GeoRoutingRule>> {
        Ok(None)
    }

    async fn list_geo_rules(
        &self,
        _tenant_id: &fcp_core::TenantId,
    ) -> Result<Vec<fcp_core::GeoRoutingRule>> {
        Ok(Vec::new())
    }

    async fn put_geo_rule(&self, _rule: fcp_core::GeoRoutingRule) -> Result<()> {
        Err(anyhow::anyhow!(
            "Geographic routing storage not yet implemented"
        ))
    }

    async fn delete_geo_rule(&self, _id: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "Geographic routing storage not yet implemented"
        ))
    }

    async fn get_phone_number_location(
        &self,
        _phone_number_id: &fcp_core::PhoneNumberId,
    ) -> Result<Option<fcp_core::PhoneNumberLocation>> {
        Ok(None)
    }

    async fn list_phone_number_locations(
        &self,
        _tenant_id: &fcp_core::TenantId,
    ) -> Result<Vec<fcp_core::PhoneNumberLocation>> {
        Ok(Vec::new())
    }

    async fn put_phone_number_location(
        &self,
        _location: fcp_core::PhoneNumberLocation,
    ) -> Result<()> {
        Err(anyhow::anyhow!(
            "Geographic routing storage not yet implemented"
        ))
    }

    async fn delete_phone_number_location(
        &self,
        _phone_number_id: &fcp_core::PhoneNumberId,
    ) -> Result<()> {
        Err(anyhow::anyhow!(
            "Geographic routing storage not yet implemented"
        ))
    }
}

// ============================================================================
// PortingStore Implementation (stub)
// ============================================================================

#[async_trait]
impl PortingStore for PostgresStorage {
    async fn get_port_request(
        &self,
        _id: &fcp_core::PortRequestId,
    ) -> Result<Option<fcp_core::PortRequest>> {
        Ok(None)
    }

    async fn list_port_requests(
        &self,
        _tenant_id: &fcp_core::TenantId,
    ) -> Result<Vec<fcp_core::PortRequest>> {
        Ok(Vec::new())
    }

    async fn put_port_request(&self, _request: fcp_core::PortRequest) -> Result<()> {
        Err(anyhow::anyhow!(
            "Number porting storage not yet implemented"
        ))
    }

    async fn delete_port_request(&self, _id: &fcp_core::PortRequestId) -> Result<()> {
        Err(anyhow::anyhow!(
            "Number porting storage not yet implemented"
        ))
    }

    async fn list_port_requests_by_status(
        &self,
        _tenant_id: &fcp_core::TenantId,
        _status: fcp_core::PortStatus,
    ) -> Result<Vec<fcp_core::PortRequest>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    // Note: Integration tests require a running PostgreSQL instance
    // Run these tests with: cargo test --features integration-tests -- --ignored
    // Or manually with a PostgreSQL instance:
    // 1. Start PostgreSQL: docker run --rm -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres
    // 2. Run: DATABASE_URL=postgresql://postgres:postgres@localhost/postgres cargo test

    #[tokio::test]
    #[ignore] // Requires PostgreSQL instance
    async fn test_connection() {
        // This test requires a running PostgreSQL instance
        // Uncomment and adjust connection string as needed
        /*
        let config = PostgresConfig::new("postgresql://postgres:postgres@localhost/fcp");
        let storage = PostgresStorage::new(config).await;
        assert!(storage.is_ok());
        */
    }
}
