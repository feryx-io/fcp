//! PostgreSQL implementation of [`RecordingStore`]: the conference
//! recording registry (migration 053).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::recording::{
    Recording, RecordingConsent, RecordingFilter, RecordingFormat, RecordingStore, RecordingTrigger,
};
use fcp_core::{MeetingId, RecordingId, TenantId};
use sqlx::{Pool, Postgres, Row};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const COLUMNS: &str = "id, tenant_id, meeting_id, room_number, node_id, path, format, \
                       trigger, consent, started_at, ended_at, duration_secs, size_bytes, \
                       participants";

fn to_datetime(st: SystemTime) -> DateTime<Utc> {
    let d = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos()).unwrap_or_default()
}

fn to_system_time(dt: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(dt.timestamp().max(0) as u64)
}

#[derive(Clone)]
pub struct PostgresRecordingStore {
    pool: Pool<Postgres>,
}

impl PostgresRecordingStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    fn row_to_recording(row: sqlx::postgres::PgRow) -> anyhow::Result<Recording> {
        let id: String = row.try_get("id")?;
        let tenant_id: String = row.try_get("tenant_id")?;
        let meeting_id: Option<String> = row.try_get("meeting_id")?;
        let format: String = row.try_get("format")?;
        let trigger: String = row.try_get("trigger")?;
        let consent: String = row.try_get("consent")?;
        let started_at: DateTime<Utc> = row.try_get("started_at")?;
        let ended_at: Option<DateTime<Utc>> = row.try_get("ended_at")?;
        let duration_secs: Option<i64> = row.try_get("duration_secs")?;
        let size_bytes: Option<i64> = row.try_get("size_bytes")?;
        let participants: i32 = row.try_get("participants")?;
        Ok(Recording {
            id: RecordingId::from_str(&id)?,
            tenant_id: TenantId::from_str(&tenant_id)?,
            meeting_id: meeting_id.as_deref().map(MeetingId::from_str).transpose()?,
            room_number: row.try_get("room_number")?,
            node_id: row.try_get("node_id")?,
            path: row.try_get("path")?,
            // A row written by a newer node with a format this one does
            // not know still lists and deletes; it just cannot be played
            // here.
            format: RecordingFormat::parse(&format).unwrap_or_default(),
            trigger: RecordingTrigger::parse(&trigger).unwrap_or_default(),
            consent: RecordingConsent::parse(&consent).unwrap_or_default(),
            started_at: to_system_time(started_at),
            ended_at: ended_at.map(to_system_time),
            duration_secs: duration_secs.map(|d| d.max(0) as u64),
            size_bytes: size_bytes.map(|b| b.max(0) as u64),
            participants: participants.max(0) as u32,
        })
    }
}

#[async_trait]
impl RecordingStore for PostgresRecordingStore {
    async fn create_recording(&self, recording: &Recording) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO conference_recordings (
                id, tenant_id, meeting_id, room_number, node_id, path, format,
                trigger, consent, started_at, ended_at, duration_secs, size_bytes, participants
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             ON CONFLICT (id) DO UPDATE SET
                path = EXCLUDED.path,
                format = EXCLUDED.format,
                consent = EXCLUDED.consent,
                participants = EXCLUDED.participants",
        )
        .bind(recording.id.to_string())
        .bind(recording.tenant_id.to_string())
        .bind(recording.meeting_id.map(|m| m.to_string()))
        .bind(&recording.room_number)
        .bind(&recording.node_id)
        .bind(&recording.path)
        .bind(recording.format.as_str())
        .bind(recording.trigger.as_str())
        .bind(recording.consent.as_str())
        .bind(to_datetime(recording.started_at))
        .bind(recording.ended_at.map(to_datetime))
        .bind(recording.duration_secs.map(|d| d as i64))
        .bind(recording.size_bytes.map(|b| b as i64))
        .bind(recording.participants as i32)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn finish_recording(
        &self,
        id: &RecordingId,
        ended_at: SystemTime,
        duration_secs: u64,
        size_bytes: u64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE conference_recordings
             SET ended_at = $2, duration_secs = $3, size_bytes = $4
             WHERE id = $1",
        )
        .bind(id.to_string())
        .bind(to_datetime(ended_at))
        .bind(duration_secs as i64)
        .bind(size_bytes as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_recording(&self, id: &RecordingId) -> anyhow::Result<Option<Recording>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM conference_recordings WHERE id = $1"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::row_to_recording).transpose()
    }

    async fn list_recordings(
        &self,
        tenant_id: &TenantId,
        filter: &RecordingFilter,
    ) -> anyhow::Result<Vec<Recording>> {
        // The filter is built up positionally so every value stays bound.
        let mut sql = format!("SELECT {COLUMNS} FROM conference_recordings WHERE tenant_id = $1");
        let mut n = 1;
        if filter.room_number.is_some() {
            n += 1;
            sql.push_str(&format!(" AND room_number = ${n}"));
        }
        if filter.meeting_id.is_some() {
            n += 1;
            sql.push_str(&format!(" AND meeting_id = ${n}"));
        }
        if filter.since.is_some() {
            n += 1;
            sql.push_str(&format!(" AND started_at >= ${n}"));
        }
        if filter.until.is_some() {
            n += 1;
            sql.push_str(&format!(" AND started_at < ${n}"));
        }
        match filter.running {
            Some(true) => sql.push_str(" AND ended_at IS NULL"),
            Some(false) => sql.push_str(" AND ended_at IS NOT NULL"),
            None => {}
        }
        sql.push_str(" ORDER BY started_at DESC");
        let limit = filter.limit.clamp(1, 1000) as i64;
        n += 1;
        sql.push_str(&format!(" LIMIT ${n}"));
        n += 1;
        sql.push_str(&format!(" OFFSET ${n}"));

        let mut query = sqlx::query(&sql).bind(tenant_id.to_string());
        if let Some(room) = &filter.room_number {
            query = query.bind(room.clone());
        }
        if let Some(meeting) = &filter.meeting_id {
            query = query.bind(meeting.to_string());
        }
        if let Some(since) = filter.since {
            query = query.bind(to_datetime(since));
        }
        if let Some(until) = filter.until {
            query = query.bind(to_datetime(until));
        }
        query = query.bind(limit).bind(filter.offset as i64);

        let rows = query.fetch_all(&self.pool).await?;
        rows.into_iter().map(Self::row_to_recording).collect()
    }

    async fn delete_recording(&self, id: &RecordingId) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM conference_recordings WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn expired_recordings(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<Vec<Recording>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM conference_recordings
             WHERE tenant_id = $1 AND started_at < $2 AND ended_at IS NOT NULL
             ORDER BY started_at"
        ))
        .bind(tenant_id.to_string())
        .bind(to_datetime(before))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_recording).collect()
    }

    async fn running_recordings(&self, node_id: &str) -> anyhow::Result<Vec<Recording>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM conference_recordings
             WHERE node_id = $1 AND ended_at IS NULL
             ORDER BY started_at"
        ))
        .bind(node_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_recording).collect()
    }

    async fn node_tenants(&self, node_id: &str) -> anyhow::Result<Vec<TenantId>> {
        let rows =
            sqlx::query("SELECT DISTINCT tenant_id FROM conference_recordings WHERE node_id = $1")
                .bind(node_id)
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("tenant_id")?;
                Ok(TenantId::from_str(&id)?)
            })
            .collect()
    }

    async fn recording_at(&self, node_id: &str, path: &str) -> anyhow::Result<Option<Recording>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM conference_recordings
             WHERE node_id = $1 AND path = $2
             ORDER BY started_at DESC
             LIMIT 1"
        ))
        .bind(node_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::row_to_recording).transpose()
    }
}
