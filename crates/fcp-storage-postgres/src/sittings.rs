//! PostgreSQL implementations of [`SittingStore`] — a room's sittings
//! (`conference_sittings`, migration 145) — and [`RegistrantStore`] — a
//! webinar's registrations (`webinar_registrants`, migration 146) —
//! (`docs/WEBINARS.md` §2.6).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::webinar::{
    Registrant, RegistrantStore, Sitting, SittingStore, StreamView, StreamViewSeen,
};
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

    async fn delete_sitting(&self, tenant: &TenantId, id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM conference_sittings WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.to_string())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

const REGISTRANT_COLUMNS: &str = "id, tenant_id, meeting_id, name, email, created_at, revoked_at";

#[derive(Clone)]
pub struct PostgresRegistrantStore {
    pool: Pool<Postgres>,
}

impl PostgresRegistrantStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    fn row_to_registrant(row: sqlx::postgres::PgRow) -> anyhow::Result<Registrant> {
        let tenant: String = row.try_get("tenant_id")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let revoked_at: Option<DateTime<Utc>> = row.try_get("revoked_at")?;
        Ok(Registrant {
            id: row.try_get("id")?,
            tenant_id: TenantId::from_str(&tenant)
                .map_err(|e| anyhow::anyhow!("invalid tenant id {tenant}: {e}"))?,
            meeting_id: row.try_get("meeting_id")?,
            name: row.try_get("name")?,
            email: row.try_get("email")?,
            created_at: from_datetime(created_at),
            revoked_at: revoked_at.map(from_datetime),
        })
    }
}

#[async_trait]
impl RegistrantStore for PostgresRegistrantStore {
    async fn put_registrant(&self, r: &Registrant) -> anyhow::Result<()> {
        sqlx::query(&format!(
            "INSERT INTO webinar_registrants ({REGISTRANT_COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET
               name = EXCLUDED.name,
               email = EXCLUDED.email,
               revoked_at = EXCLUDED.revoked_at"
        ))
        .bind(&r.id)
        .bind(r.tenant_id.to_string())
        .bind(&r.meeting_id)
        .bind(&r.name)
        .bind(r.email.as_deref())
        .bind(to_datetime(r.created_at))
        .bind(r.revoked_at.map(to_datetime))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_registrant(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> anyhow::Result<Option<Registrant>> {
        let row = sqlx::query(&format!(
            "SELECT {REGISTRANT_COLUMNS} FROM webinar_registrants
             WHERE tenant_id = $1 AND id = $2"
        ))
        .bind(tenant.to_string())
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::row_to_registrant).transpose()
    }

    async fn registrants_of_meeting(
        &self,
        tenant: &TenantId,
        meeting_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<(Vec<Registrant>, u64)> {
        // Keyset over (created_at, id), from the registration `after`.
        let rows = sqlx::query(&format!(
            "SELECT {REGISTRANT_COLUMNS} FROM webinar_registrants r
             WHERE tenant_id = $1 AND meeting_id = $2
               AND ($3::TEXT IS NULL OR (created_at, id) > (
                     SELECT created_at, id FROM webinar_registrants
                     WHERE tenant_id = $1 AND id = $3))
             ORDER BY created_at, id LIMIT $4"
        ))
        .bind(tenant.to_string())
        .bind(meeting_id)
        .bind(after)
        .bind(limit.min(i64::MAX as usize) as i64)
        .fetch_all(&self.pool)
        .await?;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM webinar_registrants WHERE tenant_id = $1 AND meeting_id = $2",
        )
        .bind(tenant.to_string())
        .bind(meeting_id)
        .fetch_one(&self.pool)
        .await?;
        let list = rows
            .into_iter()
            .map(Self::row_to_registrant)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok((list, total.max(0) as u64))
    }

    async fn record_stream_view(&self, v: &StreamViewSeen) -> anyhow::Result<()> {
        let at: DateTime<Utc> = v.at.into();
        sqlx::query(
            "INSERT INTO webinar_stream_views
                 (sitting_id, registrant_id, tenant_id, meeting_id, first_at, last_at, watched_secs)
             SELECT $1, $2, $3, $4, $5, $5, $6
              WHERE EXISTS (SELECT 1 FROM webinar_registrants WHERE id = $2 AND tenant_id = $3)
             ON CONFLICT (sitting_id, registrant_id) DO UPDATE SET
                 last_at = GREATEST(webinar_stream_views.last_at, EXCLUDED.last_at),
                 watched_secs = webinar_stream_views.watched_secs + EXCLUDED.watched_secs",
        )
        .bind(&v.sitting_id)
        .bind(&v.registrant_id)
        .bind(v.tenant_id.to_string())
        .bind(&v.meeting_id)
        .bind(at)
        .bind(v.add_secs as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn stream_views_of_sitting(
        &self,
        tenant: &TenantId,
        sitting_id: &str,
    ) -> anyhow::Result<Vec<StreamView>> {
        let rows = sqlx::query(
            "SELECT registrant_id, first_at, last_at, watched_secs FROM webinar_stream_views
              WHERE tenant_id = $1 AND sitting_id = $2 ORDER BY first_at, registrant_id",
        )
        .bind(tenant.to_string())
        .bind(sitting_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok(StreamView {
                    registrant_id: r.try_get("registrant_id")?,
                    first_at: r.try_get::<DateTime<Utc>, _>("first_at")?.into(),
                    last_at: r.try_get::<DateTime<Utc>, _>("last_at")?.into(),
                    watched_secs: r.try_get::<i64, _>("watched_secs")?.max(0) as u64,
                })
            })
            .collect()
    }
}
