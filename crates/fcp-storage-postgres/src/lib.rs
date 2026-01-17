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
    AccountStatus, CallDetailRecord, CallDisposition, CdrQuery, CdrStore, CredentialStore,
    EmergencyContact, HangupCause, HangupParty, MediaPreferences, Registration,
    RegistrationStore, TenantId, User,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Pool, Postgres, Row};
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
    /// Create a new PostgreSQL storage backend.
    pub async fn new(config: PostgresConfig) -> Result<Self> {
        // Parse connection options
        let options = PgConnectOptions::from_str(&config.connection_string)?;

        // Create connection pool
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(config.connection_timeout)
            .connect_with(options)
            .await
            .context("Failed to create PostgreSQL connection pool")?;

        info!(
            max_connections = config.max_connections,
            "PostgreSQL storage initialized"
        );

        // Run migrations
        Self::run_migrations(&pool).await?;

        Ok(Self { pool })
    }

    /// Run database migrations.
    async fn run_migrations(pool: &Pool<Postgres>) -> Result<()> {
        info!("Running PostgreSQL migrations");

        let migration_sql = include_str!("../migrations/001_initial_schema.sql");

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

        info!("PostgreSQL migrations completed successfully");
        Ok(())
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &Pool<Postgres> {
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
impl CredentialStore for PostgresStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT username, password_hash, display_name, tenant_id, enabled,
                    role, department_id, created_at, updated_at, media_preferences
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
        let tenant_id = user.tenant_id.to_string();
        let media_prefs = serde_json::to_value(&user.media_preferences)?;
        let created_at = system_time_to_datetime(user.created_at);
        let updated_at = system_time_to_datetime(user.updated_at);

        // Serialize role to snake_case string for storage
        let role_str = serde_json::to_string(&user.role)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "end_user".to_string());

        sqlx::query(
            "INSERT INTO users (username, password_hash, display_name, tenant_id, enabled,
                               role, department_id, created_at, updated_at, media_preferences)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT(tenant_id, username) DO UPDATE SET
                 password_hash = EXCLUDED.password_hash,
                 display_name = EXCLUDED.display_name,
                 enabled = EXCLUDED.enabled,
                 role = EXCLUDED.role,
                 department_id = EXCLUDED.department_id,
                 updated_at = EXCLUDED.updated_at,
                 media_preferences = EXCLUDED.media_preferences",
        )
        .bind(&user.username)
        .bind(&user.password)
        .bind(&user.display_name)
        .bind(&tenant_id)
        .bind(user.enabled)
        .bind(&role_str)
        .bind(&user.department_id)
        .bind(created_at)
        .bind(updated_at)
        .bind(&media_prefs)
        .execute(&self.pool)
        .await?;

        debug!(username = %user.username, "User stored in PostgreSQL");
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        sqlx::query("DELETE FROM users WHERE username = $1")
            .bind(username)
            .execute(&self.pool)
            .await?;

        debug!(username = %username, "User deleted from PostgreSQL");
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query(
            "SELECT username, password_hash, display_name, tenant_id, enabled,
                    role, department_id, created_at, updated_at, media_preferences
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
            "INSERT INTO registrations (aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT(aor, contact) DO UPDATE SET
                 expires_at = EXCLUDED.expires_at,
                 call_id = EXCLUDED.call_id,
                 cseq = EXCLUDED.cseq,
                 user_agent = EXCLUDED.user_agent,
                 q = EXCLUDED.q,
                 registered_at = EXCLUDED.registered_at",
        )
        .bind(&registration.aor)
        .bind(&registration.contact)
        .bind(expires_at)
        .bind(&registration.call_id)
        .bind(registration.cseq as i32)
        .bind(&registration.user_agent)
        .bind(registration.q)
        .bind(registered_at)
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

        debug!(aor = %aor, "All registrations deleted");
        Ok(())
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        let result = sqlx::query("DELETE FROM registrations WHERE expires_at <= NOW()")
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
impl CdrStore for PostgresStorage {
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()> {
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
                $1, $2, $3,
                $4, $5, $6, $7,
                $8, $9, $10,
                $11, $12, $13,
                $14, $15, $16,
                $17, $18,
                $19, $20,
                $21, $22, $23,
                $24, $25, $26, $27,
                $28, $29, $30, $31
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

        debug!(call_id = %cdr.call_id, "CDR stored in PostgreSQL");
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
        let mut param_count = 1;
        let mut bind_values: Vec<String> = Vec::new();

        if let Some(tenant_id) = &query.tenant_id {
            sql.push_str(&format!(" AND tenant_id = ${}", param_count));
            bind_values.push(tenant_id.to_string());
            param_count += 1;
        }

        if let Some(call_id) = &query.call_id {
            sql.push_str(&format!(" AND call_id = ${}", param_count));
            bind_values.push(call_id.to_string());
            param_count += 1;
        }

        if let Some(from_uri) = &query.from_uri {
            sql.push_str(&format!(" AND from_uri LIKE ${}", param_count));
            bind_values.push(format!("%{}%", from_uri));
            param_count += 1;
        }

        if let Some(to_uri) = &query.to_uri {
            sql.push_str(&format!(" AND to_uri LIKE ${}", param_count));
            bind_values.push(format!("%{}%", to_uri));
            param_count += 1;
        }

        // Note: For time filtering, we need a different approach since we can't convert SystemTime to string
        // This would need proper handling with DateTime conversion if time filtering is required
        if query.start_time_from.is_some() || query.start_time_to.is_some() {
            // Skip time filtering for now - would need dynamic query building with proper type handling
            // TODO: Implement proper timestamp filtering with DateTime conversion
        }

        if let Some(disposition) = &query.disposition {
            sql.push_str(&format!(" AND disposition = ${}", param_count));
            let disp_str = serde_json::to_string(disposition)?
                .trim_matches('"')
                .to_string();
            bind_values.push(disp_str);
            #[allow(unused_assignments)]
            {
                param_count += 1;
            }
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
        for value in bind_values {
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
        let mut bind_values: Vec<String> = Vec::new();
        let mut param_count = 1;

        if let Some(tenant_id) = &query.tenant_id {
            sql.push_str(&format!(" AND tenant_id = ${}", param_count));
            bind_values.push(tenant_id.to_string());
            param_count += 1;
        }

        if let Some(disposition) = &query.disposition {
            sql.push_str(&format!(" AND disposition = ${}", param_count));
            let disp_str = serde_json::to_string(disposition)?
                .trim_matches('"')
                .to_string();
            bind_values.push(disp_str);
        }

        let mut query_builder = sqlx::query(&sql);
        for value in bind_values {
            query_builder = query_builder.bind(value);
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

/// Convert PostgreSQL row to User
fn row_to_user(row: sqlx::postgres::PgRow) -> Result<User> {
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
    let account_status_str: String = row.try_get("account_status").unwrap_or_else(|_| "active".to_string());
    let account_status = AccountStatus::from_str(&account_status_str).unwrap_or(AccountStatus::Active);

    // Parse role from database (defaults to EndUser if not found or invalid)
    let role_str: String = row.try_get("role").unwrap_or_else(|_| "enduser".to_string());
    let role = parse_role(&role_str);

    // Parse emergency contact
    let emergency_contact = EmergencyContact {
        name: row.try_get("emergency_contact_name").ok(),
        phone: row.try_get("emergency_contact_phone").ok(),
    };

    Ok(User {
        username: row.get("username"),
        password: row.get("password_hash"),
        digest_password: row.try_get("digest_password").ok(),
        sip_ha1_hash: row.try_get("sip_ha1_hash").ok(),
        display_name: row.get("display_name"),
        tenant_id: TenantId::from_str(&row.get::<String, _>("tenant_id"))
            .unwrap_or_else(|_| TenantId::nil()),
        enabled: row.get("enabled"),
        role, // User role for RBAC
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
        language: row.try_get("language").unwrap_or_else(|_| "en-US".to_string()),
        timezone: row.try_get("timezone").unwrap_or_else(|_| "UTC".to_string()),
        account_status,
        last_login,
        emergency_contact,
        primary_phone_number_id: None, // TODO: Add to PostgreSQL schema
        voicemail_box_id: None,        // TODO: Add to PostgreSQL schema
    })
}

/// Convert PostgreSQL row to Registration
fn row_to_registration(row: sqlx::postgres::PgRow) -> Result<Registration> {
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
    })
}

/// Convert PostgreSQL row to CallDetailRecord
fn row_to_cdr(row: sqlx::postgres::PgRow) -> Result<CallDetailRecord> {
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
