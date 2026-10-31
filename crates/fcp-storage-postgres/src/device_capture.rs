//! Captures from people's apps and who may take them
//! (`docs/CALL_DIAGNOSTICS.md` §3; migration 132).

use std::str::FromStr;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::device_capture::{AllowSubject, CaptureAllow, DeviceCapture, DeviceCaptureStore};
use fcp_core::{CallId, DeviceId, TenantId, UserId};
use sqlx::postgres::PgRow;
use sqlx::Row;

use crate::{datetime_to_system_time_micros as to_time, system_time_to_datetime, PostgresStorage};

const ALLOW_COLUMNS: &str =
    "id, tenant_id, user_id, device_id, until, reason, created_by, created_at";
const CAPTURE_COLUMNS: &str = "id, tenant_id, device_id, user_id, call_id, size, sha256, file_key,
    subject_keys, uploaded_at, expires_at";

fn allow_from_row(row: &PgRow) -> anyhow::Result<CaptureAllow> {
    let user: Option<String> = row.try_get("user_id")?;
    let device: Option<String> = row.try_get("device_id")?;
    let subject = match (user, device) {
        (Some(user), _) => AllowSubject::User {
            user_id: UserId::from_str(&user)?,
        },
        (None, Some(device)) => AllowSubject::Device {
            device_id: DeviceId::from_str(&device)?,
        },
        (None, None) => anyhow::bail!("a capture allow for nobody"),
    };
    Ok(CaptureAllow {
        id: uuid::Uuid::from_str(&row.try_get::<String, _>("id")?)?,
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        subject,
        until: to_time(row.try_get::<DateTime<Utc>, _>("until")?),
        reason: row.try_get("reason")?,
        created_by: row.try_get("created_by")?,
        created_at: to_time(row.try_get::<DateTime<Utc>, _>("created_at")?),
    })
}

fn capture_from_row(row: &PgRow) -> anyhow::Result<DeviceCapture> {
    Ok(DeviceCapture {
        id: uuid::Uuid::from_str(&row.try_get::<String, _>("id")?)?,
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        device_id: DeviceId::from_str(&row.try_get::<String, _>("device_id")?)?,
        user_id: UserId::from_str(&row.try_get::<String, _>("user_id")?)?,
        call_id: row
            .try_get::<Option<String>, _>("call_id")?
            .map(|c| CallId::from_str(&c))
            .transpose()?,
        size: row.try_get::<i64, _>("size")?.max(0) as u64,
        sha256: row.try_get("sha256")?,
        file_key: row.try_get("file_key")?,
        subject_keys: row.try_get("subject_keys")?,
        uploaded_at: to_time(row.try_get::<DateTime<Utc>, _>("uploaded_at")?),
        expires_at: to_time(row.try_get::<DateTime<Utc>, _>("expires_at")?),
    })
}

#[async_trait]
impl DeviceCaptureStore for PostgresStorage {
    async fn put_capture_allow(&self, allow: &CaptureAllow) -> anyhow::Result<()> {
        let (user, device) = match &allow.subject {
            AllowSubject::User { user_id } => (Some(user_id.to_string()), None),
            AllowSubject::Device { device_id } => (None, Some(device_id.to_string())),
        };
        sqlx::query(&format!(
            "INSERT INTO capture_allows ({ALLOW_COLUMNS}) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (id) DO UPDATE SET until = EXCLUDED.until, reason = EXCLUDED.reason"
        ))
        .bind(allow.id.to_string())
        .bind(allow.tenant_id.to_string())
        .bind(user)
        .bind(device)
        .bind(system_time_to_datetime(allow.until))
        .bind(&allow.reason)
        .bind(&allow.created_by)
        .bind(system_time_to_datetime(allow.created_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_capture_allows(&self, tenant: &TenantId) -> anyhow::Result<Vec<CaptureAllow>> {
        let rows = sqlx::query(&format!(
            "SELECT {ALLOW_COLUMNS} FROM capture_allows WHERE tenant_id = $1
             ORDER BY created_at DESC, id"
        ))
        .bind(tenant.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(allow_from_row).collect()
    }

    async fn get_capture_allow(
        &self,
        tenant: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<CaptureAllow>> {
        let row = sqlx::query(&format!(
            "SELECT {ALLOW_COLUMNS} FROM capture_allows WHERE tenant_id = $1 AND id = $2"
        ))
        .bind(tenant.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(allow_from_row).transpose()
    }

    async fn delete_capture_allow(
        &self,
        tenant: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM capture_allows WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.to_string())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn capture_allows_for(
        &self,
        tenant: &TenantId,
        user: &UserId,
        device: &DeviceId,
    ) -> anyhow::Result<Vec<CaptureAllow>> {
        let rows = sqlx::query(&format!(
            "SELECT {ALLOW_COLUMNS} FROM capture_allows
             WHERE tenant_id = $1 AND (user_id = $2 OR device_id = $3)"
        ))
        .bind(tenant.to_string())
        .bind(user.to_string())
        .bind(device.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(allow_from_row).collect()
    }

    async fn put_device_capture(&self, c: &DeviceCapture) -> anyhow::Result<()> {
        sqlx::query(&format!(
            "INSERT INTO device_captures ({CAPTURE_COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
        ))
        .bind(c.id.to_string())
        .bind(c.tenant_id.to_string())
        .bind(c.device_id.to_string())
        .bind(c.user_id.to_string())
        .bind(c.call_id.as_ref().map(ToString::to_string))
        .bind(c.size.min(i64::MAX as u64) as i64)
        .bind(&c.sha256)
        .bind(&c.file_key)
        .bind(&c.subject_keys)
        .bind(system_time_to_datetime(c.uploaded_at))
        .bind(system_time_to_datetime(c.expires_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_device_captures(
        &self,
        tenant: &TenantId,
        device: Option<&DeviceId>,
        user: Option<&UserId>,
    ) -> anyhow::Result<Vec<DeviceCapture>> {
        let rows = sqlx::query(&format!(
            "SELECT {CAPTURE_COLUMNS} FROM device_captures
             WHERE tenant_id = $1
               AND ($2::text IS NULL OR device_id = $2)
               AND ($3::text IS NULL OR user_id = $3)
             ORDER BY uploaded_at DESC, id"
        ))
        .bind(tenant.to_string())
        .bind(device.map(ToString::to_string))
        .bind(user.map(ToString::to_string))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(capture_from_row).collect()
    }

    async fn get_device_capture(
        &self,
        tenant: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<DeviceCapture>> {
        let row = sqlx::query(&format!(
            "SELECT {CAPTURE_COLUMNS} FROM device_captures WHERE tenant_id = $1 AND id = $2"
        ))
        .bind(tenant.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(capture_from_row).transpose()
    }

    async fn delete_device_capture(
        &self,
        tenant: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<DeviceCapture>> {
        let row = sqlx::query(&format!(
            "DELETE FROM device_captures WHERE tenant_id = $1 AND id = $2
             RETURNING {CAPTURE_COLUMNS}"
        ))
        .bind(tenant.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(crate::deletion::held)?;
        row.as_ref().map(capture_from_row).transpose()
    }

    async fn expired_device_captures(
        &self,
        now: SystemTime,
        limit: usize,
    ) -> anyhow::Result<Vec<DeviceCapture>> {
        let rows = sqlx::query(&format!(
            "SELECT {CAPTURE_COLUMNS} FROM device_captures
             WHERE expires_at <= $1
               AND fcp_device_capture_hold(tenant_id, uploaded_at, subject_keys, call_id) IS NULL
             ORDER BY expires_at, id
             LIMIT $2"
        ))
        .bind(system_time_to_datetime(now))
        .bind(limit.clamp(1, 10_000) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(capture_from_row).collect()
    }

    async fn due_capture_files(&self, limit: usize) -> anyhow::Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT file_key FROM device_capture_file_deletions
             WHERE next_attempt_at <= now()
             ORDER BY next_attempt_at, file_key
             LIMIT $1",
        )
        .bind(limit.clamp(1, 10_000) as i64)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn capture_file_removed(&self, key: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM device_capture_file_deletions WHERE file_key = $1")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn capture_file_failed(
        &self,
        key: &str,
        error: &str,
        retry: Duration,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE device_capture_file_deletions
             SET attempts = attempts + 1, last_error = $2, next_attempt_at = $3
             WHERE file_key = $1",
        )
        .bind(key)
        .bind(error)
        .bind(system_time_to_datetime(SystemTime::now() + retry))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
