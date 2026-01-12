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
    AccountStatus, CallDetailRecord, CallDisposition, CdrQuery, CdrStore, CredentialStore,
    EmergencyContact, HangupCause, HangupParty, MediaPreferences, Registration,
    RegistrationStore, TenantId, User,
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
}

// ============================================================================
// CredentialStore Implementation
// ============================================================================

#[async_trait]
impl CredentialStore for MysqlStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT username, password_hash, display_name, tenant_id, enabled,
                    created_at, updated_at, media_preferences
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

        sqlx::query(
            "INSERT INTO users (username, password_hash, display_name, tenant_id, enabled,
                               created_at, updated_at, media_preferences)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                 password_hash = VALUES(password_hash),
                 display_name = VALUES(display_name),
                 enabled = VALUES(enabled),
                 updated_at = VALUES(updated_at),
                 media_preferences = VALUES(media_preferences)",
        )
        .bind(&user.username)
        .bind(&user.password)
        .bind(&user.display_name)
        .bind(&tenant_id)
        .bind(user.enabled)
        .bind(created_at)
        .bind(updated_at)
        .bind(&media_prefs)
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
            "SELECT username, password_hash, display_name, tenant_id, enabled,
                    created_at, updated_at, media_preferences
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
            "INSERT INTO registrations (aor, contact, expires_at, call_id, cseq, user_agent, q, registered_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                 expires_at = VALUES(expires_at),
                 call_id = VALUES(call_id),
                 cseq = VALUES(cseq),
                 user_agent = VALUES(user_agent),
                 q = VALUES(q),
                 registered_at = VALUES(registered_at)",
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
// Helper Functions
// ============================================================================

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
    let account_status_str: String = row.try_get("account_status").unwrap_or_else(|_| "active".to_string());
    let account_status = AccountStatus::from_str(&account_status_str).unwrap_or(AccountStatus::Active);

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
    })
}

/// Convert MySQL row to CallDetailRecord
fn row_to_cdr(row: sqlx::mysql::MySqlRow) -> Result<CallDetailRecord> {
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
