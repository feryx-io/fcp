//! PostgreSQL implementation of [`SittingStore`]: a room's sittings
//! (`conference_sittings`, migration 145, `docs/WEBINARS.md` §2.6).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::webinar::{Sitting, SittingStore};
use fcp_core::TenantId;
use sqlx::{Pool, Postgres, Row};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const COLUMNS: &str =
    "id, tenant_id, meeting_id, room_number, kind, started_at, live_at, ended_at, ended_by";

fn to_datetime(st: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(st)
}

fn from_datetime(dt: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(dt.timestamp_millis().max(0) as u64)
}

#[derive(Clone)]
pub struct PostgresSittingStore {
    pool: Pool<Postgres>,
}

impl PostgresSittingStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    fn row_to_sitting(row: sqlx::postgres::PgRow) -> anyhow::Result<Sitting> {
        let tenant: String = row.try_get("tenant_id")?;
        let started_at: DateTime<Utc> = row.try_get("started_at")?;
        let live_at: Option<DateTime<Utc>> = row.try_get("live_at")?;
        let ended_at: Option<DateTime<Utc>> = row.try_get("ended_at")?;
        Ok(Sitting {
            id: row.try_get("id")?,
            tenant_id: TenantId::from_str(&tenant)
                .map_err(|e| anyhow::anyhow!("invalid tenant id {tenant}: {e}"))?,
            meeting_id: row.try_get("meeting_id")?,
            room_number: row.try_get("room_number")?,
            kind: row.try_get("kind")?,
            started_at: from_datetime(started_at),
            live_at: live_at.map(from_datetime),
            ended_at: ended_at.map(from_datetime),
            ended_by: row.try_get("ended_by")?,
        })
    }
}

#[async_trait]
impl SittingStore for PostgresSittingStore {
    async fn put_sitting(&self, s: &Sitting) -> anyhow::Result<()> {
        // A sitting only moves forward: what a later write does not know
        // (one racing an earlier one) is kept.
        sqlx::query(&format!(
            "INSERT INTO conference_sittings ({COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO UPDATE SET
               live_at = COALESCE(EXCLUDED.live_at, conference_sittings.live_at),
               ended_at = COALESCE(EXCLUDED.ended_at, conference_sittings.ended_at),
               ended_by = COALESCE(EXCLUDED.ended_by, conference_sittings.ended_by)"
        ))
        .bind(&s.id)
        .bind(s.tenant_id.to_string())
        .bind(s.meeting_id.as_deref())
        .bind(&s.room_number)
        .bind(&s.kind)
        .bind(to_datetime(s.started_at))
        .bind(s.live_at.map(to_datetime))
        .bind(s.ended_at.map(to_datetime))
        .bind(s.ended_by.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn sittings_of_meeting(
        &self,
        tenant: &TenantId,
        meeting_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Sitting>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM conference_sittings
             WHERE tenant_id = $1 AND meeting_id = $2
             ORDER BY started_at DESC, id LIMIT $3"
        ))
        .bind(tenant.to_string())
        .bind(meeting_id)
        .bind(limit.min(i64::MAX as usize) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_sitting).collect()
    }

    async fn get_sitting(&self, tenant: &TenantId, id: &str) -> anyhow::Result<Option<Sitting>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM conference_sittings WHERE tenant_id = $1 AND id = $2"
        ))
        .bind(tenant.to_string())
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::row_to_sitting).transpose()
    }
}
