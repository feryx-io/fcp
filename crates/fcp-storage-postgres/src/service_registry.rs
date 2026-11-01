//! The admin's record of every service it has seen (`docs/SERVICES.md`,
//! phase S2).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::service_registry::{ServiceInstance, ServiceInstanceStore};
use sqlx::Row;
use std::time::SystemTime;

use crate::{datetime_to_system_time, system_time_to_datetime, PostgresStorage};

const COLUMNS: &str =
    "site_id, id, kind, host, version, first_seen, last_seen, up, down_since, stopped, last";

fn row_to_instance(row: &sqlx::postgres::PgRow) -> anyhow::Result<ServiceInstance> {
    let site: String = row.try_get("site_id")?;
    Ok(ServiceInstance {
        id: row.try_get("id")?,
        site_id: (!site.is_empty()).then_some(site),
        kind: row.try_get("kind")?,
        host: row.try_get("host")?,
        version: row.try_get("version")?,
        first_seen: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("first_seen")?),
        last_seen: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("last_seen")?),
        up: row.try_get("up")?,
        down_since: row
            .try_get::<Option<DateTime<Utc>>, _>("down_since")?
            .map(datetime_to_system_time),
        stopped: row.try_get("stopped")?,
        last: row.try_get("last")?,
    })
}

#[async_trait]
impl ServiceInstanceStore for PostgresStorage {
    async fn list_service_instances(&self) -> anyhow::Result<Vec<ServiceInstance>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM service_instances ORDER BY site_id, id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_instance).collect()
    }

    async fn put_service_instance(&self, i: &ServiceInstance) -> anyhow::Result<()> {
        sqlx::query(&format!(
            "INSERT INTO service_instances ({COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (site_id, id) DO UPDATE SET
                kind = EXCLUDED.kind, host = EXCLUDED.host, version = EXCLUDED.version,
                first_seen = EXCLUDED.first_seen, last_seen = EXCLUDED.last_seen,
                up = EXCLUDED.up, down_since = EXCLUDED.down_since,
                stopped = EXCLUDED.stopped, last = EXCLUDED.last"
        ))
        .bind(i.site_id.clone().unwrap_or_default())
        .bind(&i.id)
        .bind(&i.kind)
        .bind(&i.host)
        .bind(&i.version)
        .bind(system_time_to_datetime(i.first_seen))
        .bind(system_time_to_datetime(i.last_seen))
        .bind(i.up)
        .bind(i.down_since.map(system_time_to_datetime))
        .bind(i.stopped)
        .bind(&i.last)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_service_instance(
        &self,
        site_id: Option<&str>,
        id: &str,
    ) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM service_instances WHERE site_id = $1 AND id = $2")
            .bind(site_id.unwrap_or_default())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn forget_service_instances_down_before(
        &self,
        before: SystemTime,
    ) -> anyhow::Result<u64> {
        let done = sqlx::query("DELETE FROM service_instances WHERE NOT up AND down_since < $1")
            .bind(system_time_to_datetime(before))
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected())
    }
}
