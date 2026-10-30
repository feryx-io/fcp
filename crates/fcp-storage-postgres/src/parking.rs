//! Tenants' parking lots (`docs/PARKING.md`).

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::parking::{ParkingLot, ParkingLotStore};
use fcp_core::{CallTarget, TenantId};
use sqlx::Row;

use crate::{datetime_to_system_time, system_time_to_datetime, PostgresStorage};

const LOT: &str = "id, tenant_id, name, first_slot, last_slot, park_number, timeout_secs, \
                   fallback, music, location_id, enabled, created_at, updated_at";

fn row_to_lot(row: &sqlx::postgres::PgRow) -> anyhow::Result<ParkingLot> {
    let fallback: Option<String> = row.try_get("fallback")?;
    Ok(ParkingLot {
        id: row.try_get("id")?,
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        name: row.try_get("name")?,
        first: row.try_get("first_slot")?,
        last: row.try_get("last_slot")?,
        park_number: row.try_get("park_number")?,
        timeout_secs: row.try_get::<i32, _>("timeout_secs")?.max(0) as u32,
        fallback: fallback
            .as_deref()
            .map(CallTarget::parse)
            .transpose()
            .map_err(|e| anyhow::anyhow!("a lot's fallback: {e}"))?,
        music: row.try_get("music")?,
        location_id: row.try_get("location_id")?,
        enabled: row.try_get("enabled")?,
        created_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("created_at")?),
        updated_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("updated_at")?),
    })
}

#[async_trait]
impl ParkingLotStore for PostgresStorage {
    async fn list_parking_lots(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<ParkingLot>> {
        let rows = sqlx::query(&format!(
            "SELECT {LOT} FROM parking_lots WHERE tenant_id = $1
             ORDER BY char_length(first_slot), first_slot, id"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_lot).collect()
    }

    async fn get_parking_lot(
        &self,
        tenant_id: &TenantId,
        id: &str,
    ) -> anyhow::Result<Option<ParkingLot>> {
        let row = sqlx::query(&format!(
            "SELECT {LOT} FROM parking_lots WHERE tenant_id = $1 AND id = $2"
        ))
        .bind(tenant_id.to_string())
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(row_to_lot).transpose()
    }

    async fn put_parking_lot(&self, lot: &ParkingLot) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO parking_lots (id, tenant_id, name, first_slot, last_slot, park_number,
                 timeout_secs, fallback, music, location_id, enabled, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name, first_slot = EXCLUDED.first_slot,
                last_slot = EXCLUDED.last_slot, park_number = EXCLUDED.park_number,
                timeout_secs = EXCLUDED.timeout_secs, fallback = EXCLUDED.fallback,
                music = EXCLUDED.music, location_id = EXCLUDED.location_id,
                enabled = EXCLUDED.enabled, updated_at = EXCLUDED.updated_at
             WHERE parking_lots.tenant_id = EXCLUDED.tenant_id",
        )
        .bind(&lot.id)
        .bind(lot.tenant_id.to_string())
        .bind(&lot.name)
        .bind(&lot.first)
        .bind(&lot.last)
        .bind(&lot.park_number)
        .bind(lot.timeout_secs as i32)
        .bind(lot.fallback.as_ref().map(CallTarget::as_string))
        .bind(&lot.music)
        .bind(&lot.location_id)
        .bind(lot.enabled)
        .bind(system_time_to_datetime(lot.created_at))
        .bind(system_time_to_datetime(lot.updated_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_parking_lot(&self, tenant_id: &TenantId, id: &str) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM parking_lots WHERE tenant_id = $1 AND id = $2")
            .bind(tenant_id.to_string())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
