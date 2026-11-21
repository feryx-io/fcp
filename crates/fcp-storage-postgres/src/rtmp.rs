//! Webinars' RTMP destinations (`docs/WEBINARS.md` W7): the ingest URL and
//! the stream key, sealed as every secret column is.

use crate::PostgresStorage;
use chrono::{DateTime, Utc};
use fcp_core::webinar::{RtmpDestination, RtmpDestinationStore};
use fcp_core::TenantId;
use sqlx::Row;

#[async_trait::async_trait]
impl RtmpDestinationStore for PostgresStorage {
    async fn get_rtmp_destination(
        &self,
        tenant: &TenantId,
        meeting_id: &str,
    ) -> anyhow::Result<Option<RtmpDestination>> {
        let Some(row) = sqlx::query(
            "SELECT url, key_sealed, updated_at FROM webinar_rtmp_destinations
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.to_string())
        .bind(meeting_id)
        .fetch_optional(self.writes())
        .await?
        else {
            return Ok(None);
        };
        let key = self
            .open_opt(Some(row.try_get::<String, _>("key_sealed")?))
            .await?
            .unwrap_or_default();
        Ok(Some(RtmpDestination {
            tenant_id: *tenant,
            meeting_id: meeting_id.to_string(),
            url: row.try_get("url")?,
            key,
            updated_at: row.try_get::<DateTime<Utc>, _>("updated_at")?.into(),
        }))
    }

    async fn put_rtmp_destination(&self, d: &RtmpDestination) -> anyhow::Result<()> {
        let sealed = self.seal_opt(Some(&d.key))?.unwrap_or_default();
        let at: DateTime<Utc> = d.updated_at.into();
        sqlx::query(
            "INSERT INTO webinar_rtmp_destinations (id, tenant_id, url, key_sealed, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO UPDATE SET
                 url = EXCLUDED.url,
                 key_sealed = EXCLUDED.key_sealed,
                 updated_at = EXCLUDED.updated_at
             WHERE webinar_rtmp_destinations.tenant_id = EXCLUDED.tenant_id",
        )
        .bind(&d.meeting_id)
        .bind(d.tenant_id.to_string())
        .bind(&d.url)
        .bind(sealed)
        .bind(at)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_rtmp_destination(
        &self,
        tenant: &TenantId,
        meeting_id: &str,
    ) -> anyhow::Result<bool> {
        let done =
            sqlx::query("DELETE FROM webinar_rtmp_destinations WHERE tenant_id = $1 AND id = $2")
                .bind(tenant.to_string())
                .bind(meeting_id)
                .execute(self.writes())
                .await?;
        Ok(done.rows_affected() > 0)
    }
}
