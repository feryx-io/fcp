//! PostgreSQL implementation of [`ChatStore`]: chat in a room
//! (`conference_chat`, migration 143, `docs/WEBINARS.md` §2.5).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::chat::{ChatAudience, ChatRecord, ChatStore};
use fcp_core::TenantId;
use sqlx::{Pool, Postgres, Row};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

const COLUMNS: &str = "id, tenant_id, meeting_id, room_number, session_id, participant_id, \
                       sender_name, sender_address, audience, text, sent_at, deleted_at, \
                       deleted_by";

fn to_datetime(st: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(st)
}

fn to_system(at: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_millis(at.timestamp_millis().max(0) as u64)
}

#[derive(Clone)]
pub struct PostgresChatStore {
    pool: Pool<Postgres>,
}

impl PostgresChatStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    fn row_to_record(row: sqlx::postgres::PgRow) -> anyhow::Result<ChatRecord> {
        let tenant: String = row.try_get("tenant_id")?;
        let audience: String = row.try_get("audience")?;
        let sent_at: DateTime<Utc> = row.try_get("sent_at")?;
        let deleted_at: Option<DateTime<Utc>> = row.try_get("deleted_at")?;
        Ok(ChatRecord {
            id: row.try_get("id")?,
            tenant_id: TenantId::from_str(&tenant)
                .map_err(|e| anyhow::anyhow!("invalid tenant id {tenant}: {e}"))?,
            meeting_id: row.try_get("meeting_id")?,
            room_number: row.try_get("room_number")?,
            session_id: row.try_get("session_id")?,
            participant_id: row.try_get("participant_id")?,
            sender_name: row.try_get("sender_name")?,
            sender_address: row.try_get("sender_address")?,
            audience: ChatAudience::parse(&audience).unwrap_or_default(),
            text: row.try_get("text")?,
            sent_at: to_system(sent_at),
            deleted_at: deleted_at.map(to_system),
            deleted_by: row.try_get("deleted_by")?,
        })
    }
}

#[async_trait]
impl ChatStore for PostgresChatStore {
    async fn put_chat(&self, m: &ChatRecord) -> anyhow::Result<()> {
        sqlx::query(&format!(
            "INSERT INTO conference_chat ({COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (id) DO UPDATE SET
               deleted_at = EXCLUDED.deleted_at,
               deleted_by = EXCLUDED.deleted_by"
        ))
        .bind(&m.id)
        .bind(m.tenant_id.to_string())
        .bind(m.meeting_id.as_deref())
        .bind(&m.room_number)
        .bind(&m.session_id)
        .bind(&m.participant_id)
        .bind(m.sender_name.as_deref())
        .bind(m.sender_address.as_deref())
        .bind(m.audience.as_str())
        .bind(&m.text)
        .bind(to_datetime(m.sent_at))
        .bind(m.deleted_at.map(to_datetime))
        .bind(m.deleted_by.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn chat_of_room(
        &self,
        tenant: &TenantId,
        room_number: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ChatRecord>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM conference_chat
             WHERE tenant_id = $1 AND room_number = $2
             ORDER BY sent_at, id LIMIT $3"
        ))
        .bind(tenant.to_string())
        .bind(room_number)
        .bind(limit.min(i64::MAX as usize) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_record).collect()
    }
}
