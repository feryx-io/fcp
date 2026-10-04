//! Provisioning keys and the fetch log (`docs/PROVISIONING.md`).

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::provisioning::{
    hash_key, DeviceFetch, PendingDevice, ProvisioningKey, ProvisioningStore, PENDING_KEPT,
};
use fcp_core::{DeviceId, TenantId};
use sqlx::Row;

use crate::{datetime_to_system_time, system_time_to_datetime, PostgresStorage};

/// The fetches kept per tenant.
const FETCHES_KEPT: i64 = 500;

#[async_trait]
impl ProvisioningStore for PostgresStorage {
    async fn get_provisioning_key(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Option<ProvisioningKey>> {
        let row = sqlx::query("SELECT key_sealed, created_at FROM provisioning_keys WHERE id = $1")
            .bind(tenant_id.to_string())
            .fetch_optional(self.reads())
            .await?;
        let Some(row) = row else { return Ok(None) };
        let key = self
            .open_opt(Some(row.try_get::<String, _>("key_sealed")?))
            .await?
            .unwrap_or_default();
        Ok(Some(ProvisioningKey {
            tenant_id: *tenant_id,
            key,
            created_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("created_at")?),
        }))
    }

    async fn put_provisioning_key(&self, key: &ProvisioningKey) -> anyhow::Result<()> {
        let sealed = self.seal_opt(Some(&key.key))?.unwrap_or_default();
        sqlx::query(
            "INSERT INTO provisioning_keys (id, key_hash, key_sealed, created_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO UPDATE SET
                 key_hash = EXCLUDED.key_hash,
                 key_sealed = EXCLUDED.key_sealed,
                 created_at = EXCLUDED.created_at",
        )
        .bind(key.tenant_id.to_string())
        .bind(hash_key(&key.key))
        .bind(sealed)
        .bind(system_time_to_datetime(key.created_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn tenant_for_provisioning_key(&self, key: &str) -> anyhow::Result<Option<TenantId>> {
        let id: Option<String> =
            sqlx::query_scalar("SELECT id FROM provisioning_keys WHERE key_hash = $1")
                .bind(hash_key(key))
                .fetch_optional(self.reads())
                .await?;
        Ok(id.and_then(|id| TenantId::from_str(&id).ok()))
    }

    async fn record_fetch(&self, fetch: &DeviceFetch) -> anyhow::Result<()> {
        let tenant = fetch.tenant_id.to_string();
        sqlx::query(
            "INSERT INTO device_fetches
                 (tenant_id, device_id, mac_address, file, at, address, user_agent, outcome)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&tenant)
        .bind(fetch.device_id.map(|d| d.to_string()))
        .bind(&fetch.mac_address)
        .bind(&fetch.file)
        .bind(system_time_to_datetime(fetch.at))
        .bind(&fetch.address)
        .bind(&fetch.user_agent)
        .bind(&fetch.outcome)
        .execute(self.writes())
        .await?;
        // The newest few hundred are what anyone reads.
        sqlx::query(
            "DELETE FROM device_fetches
             WHERE tenant_id = $1 AND id < (
                 SELECT min(id) FROM (
                     SELECT id FROM device_fetches WHERE tenant_id = $1
                     ORDER BY id DESC LIMIT $2) newest)",
        )
        .bind(&tenant)
        .bind(FETCHES_KEPT)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn device_fetches(
        &self,
        tenant_id: &TenantId,
        device_id: &DeviceId,
        limit: usize,
    ) -> anyhow::Result<Vec<DeviceFetch>> {
        let rows = sqlx::query(
            "SELECT device_id, mac_address, file, at, address, user_agent, outcome
             FROM device_fetches WHERE tenant_id = $1 AND device_id = $2
             ORDER BY id DESC LIMIT $3",
        )
        .bind(tenant_id.to_string())
        .bind(device_id.to_string())
        .bind(limit.clamp(1, FETCHES_KEPT as usize) as i64)
        .fetch_all(self.reads())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(DeviceFetch {
                    tenant_id: *tenant_id,
                    device_id: row
                        .try_get::<Option<String>, _>("device_id")?
                        .and_then(|d| d.parse().ok()),
                    mac_address: row.try_get("mac_address")?,
                    file: row.try_get("file")?,
                    at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("at")?),
                    address: row.try_get("address")?,
                    user_agent: row.try_get("user_agent")?,
                    outcome: row.try_get("outcome")?,
                })
            })
            .collect()
    }

    async fn note_pending_device(&self, p: &PendingDevice) -> anyhow::Result<()> {
        let tenant = p.tenant_id.to_string();
        sqlx::query(
            "INSERT INTO pending_devices
                 (tenant_id, mac_address, first_seen, last_seen, address, user_agent,
                  vendor, model, firmware, fetches)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1)
             ON CONFLICT (tenant_id, mac_address) DO UPDATE SET
                 last_seen = EXCLUDED.last_seen,
                 address = EXCLUDED.address,
                 user_agent = EXCLUDED.user_agent,
                 vendor = COALESCE(EXCLUDED.vendor, pending_devices.vendor),
                 model = COALESCE(EXCLUDED.model, pending_devices.model),
                 firmware = COALESCE(EXCLUDED.firmware, pending_devices.firmware),
                 fetches = pending_devices.fetches + 1",
        )
        .bind(&tenant)
        .bind(&p.mac_address)
        .bind(system_time_to_datetime(p.first_seen))
        .bind(system_time_to_datetime(p.last_seen))
        .bind(&p.address)
        .bind(&p.user_agent)
        .bind(p.vendor.map(|v| v.to_string()))
        .bind(&p.model)
        .bind(&p.firmware)
        .execute(self.writes())
        .await?;
        sqlx::query(
            "DELETE FROM pending_devices
             WHERE tenant_id = $1 AND mac_address NOT IN (
                 SELECT mac_address FROM pending_devices WHERE tenant_id = $1
                 ORDER BY last_seen DESC LIMIT $2)",
        )
        .bind(&tenant)
        .bind(PENDING_KEPT as i64)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn list_pending_devices(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Vec<PendingDevice>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            pending_columns!(),
            " FROM pending_devices WHERE tenant_id = $1 ORDER BY last_seen DESC"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.into_iter()
            .map(|r| row_to_pending(tenant_id, r))
            .collect()
    }

    async fn get_pending_device(
        &self,
        tenant_id: &TenantId,
        mac_address: &str,
    ) -> anyhow::Result<Option<PendingDevice>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            pending_columns!(),
            " FROM pending_devices WHERE tenant_id = $1 AND mac_address = $2"
        ))
        .bind(tenant_id.to_string())
        .bind(mac_address)
        .fetch_optional(self.writes())
        .await?;
        row.map(|r| row_to_pending(tenant_id, r)).transpose()
    }

    async fn remove_pending_device(
        &self,
        tenant_id: &TenantId,
        mac_address: &str,
    ) -> anyhow::Result<bool> {
        let done =
            sqlx::query("DELETE FROM pending_devices WHERE tenant_id = $1 AND mac_address = $2")
                .bind(tenant_id.to_string())
                .bind(mac_address)
                .execute(self.writes())
                .await?;
        Ok(done.rows_affected() > 0)
    }
}

macro_rules! pending_columns {
    () => {
        "mac_address, first_seen, last_seen, address, user_agent, vendor, model, firmware, fetches"
    };
}
use pending_columns;

fn row_to_pending(
    tenant_id: &TenantId,
    row: sqlx::postgres::PgRow,
) -> anyhow::Result<PendingDevice> {
    Ok(PendingDevice {
        tenant_id: *tenant_id,
        mac_address: row.try_get("mac_address")?,
        first_seen: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("first_seen")?),
        last_seen: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("last_seen")?),
        address: row.try_get("address")?,
        user_agent: row.try_get("user_agent")?,
        vendor: row
            .try_get::<Option<String>, _>("vendor")?
            .and_then(|v| v.parse().ok()),
        model: row.try_get("model")?,
        firmware: row.try_get("firmware")?,
        fetches: row.try_get::<i32, _>("fetches")?.max(0) as u32,
    })
}
