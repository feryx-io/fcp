//! PostgreSQL implementation of [`QuestionStore`]: questions asked in a
//! room (`webinar_questions`, migration 142, `docs/WEBINARS.md` §2.5).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::qa::{Question, QuestionStatus, QuestionStore};
use fcp_core::TenantId;
use sqlx::{Pool, Postgres, Row};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

const COLUMNS: &str = "id, tenant_id, meeting_id, room_number, session_id, participant_id, \
                       asker_name, text, status, published, upvotes, answer, answered_live, \
                       asked_at, updated_at, asker_address";

fn to_datetime(st: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(st)
}

fn status_of(s: &str) -> QuestionStatus {
    match s {
        "answered" => QuestionStatus::Answered,
        "dismissed" => QuestionStatus::Dismissed,
        _ => QuestionStatus::Pending,
    }
}

#[derive(Clone)]
pub struct PostgresQuestionStore {
    pool: Pool<Postgres>,
}

impl PostgresQuestionStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    fn row_to_question(row: sqlx::postgres::PgRow) -> anyhow::Result<Question> {
        let tenant: String = row.try_get("tenant_id")?;
        let status: String = row.try_get("status")?;
        let asked_at: DateTime<Utc> = row.try_get("asked_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;
        let upvotes: i32 = row.try_get("upvotes")?;
        Ok(Question {
            id: row.try_get("id")?,
            tenant_id: TenantId::from_str(&tenant)
                .map_err(|e| anyhow::anyhow!("invalid tenant id {tenant}: {e}"))?,
            meeting_id: row.try_get("meeting_id")?,
            room_number: row.try_get("room_number")?,
            session_id: row.try_get("session_id")?,
            participant_id: row.try_get("participant_id")?,
            asker_name: row.try_get("asker_name")?,
            asker_address: row.try_get("asker_address")?,
            text: row.try_get("text")?,
            status: status_of(&status),
            published: row.try_get("published")?,
            upvotes: upvotes.max(0) as u32,
            answer: row.try_get("answer")?,
            answered_live: row.try_get("answered_live")?,
            asked_at: UNIX_EPOCH
                + std::time::Duration::from_millis(asked_at.timestamp_millis().max(0) as u64),
            updated_at: UNIX_EPOCH
                + std::time::Duration::from_millis(updated_at.timestamp_millis().max(0) as u64),
        })
    }
}

#[async_trait]
impl QuestionStore for PostgresQuestionStore {
    async fn put_question(&self, q: &Question) -> anyhow::Result<()> {
        sqlx::query(&format!(
            "INSERT INTO webinar_questions ({COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
             ON CONFLICT (id) DO UPDATE SET
               asker_name = EXCLUDED.asker_name,
               asker_address = EXCLUDED.asker_address,
               text = EXCLUDED.text,
               status = EXCLUDED.status,
               published = EXCLUDED.published,
               upvotes = EXCLUDED.upvotes,
               answer = EXCLUDED.answer,
               answered_live = EXCLUDED.answered_live,
               updated_at = EXCLUDED.updated_at"
        ))
        .bind(&q.id)
        .bind(q.tenant_id.to_string())
        .bind(q.meeting_id.as_deref())
        .bind(&q.room_number)
        .bind(&q.session_id)
        .bind(&q.participant_id)
        .bind(q.asker_name.as_deref())
        .bind(&q.text)
        .bind(q.status.as_str())
        .bind(q.published)
        .bind(q.upvotes.min(i32::MAX as u32) as i32)
        .bind(q.answer.as_deref())
        .bind(q.answered_live)
        .bind(to_datetime(q.asked_at))
        .bind(to_datetime(q.updated_at))
        .bind(q.asker_address.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn questions_of_room(
        &self,
        tenant: &TenantId,
        room_number: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Question>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM webinar_questions
             WHERE tenant_id = $1 AND room_number = $2
             ORDER BY asked_at, id LIMIT $3"
        ))
        .bind(tenant.to_string())
        .bind(room_number)
        .bind(limit.min(i64::MAX as usize) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_question).collect()
    }

    async fn questions_of_session(
        &self,
        tenant: &TenantId,
        session_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Question>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM webinar_questions
             WHERE tenant_id = $1 AND session_id = $2
             ORDER BY asked_at, id LIMIT $3"
        ))
        .bind(tenant.to_string())
        .bind(session_id)
        .bind(limit.min(i64::MAX as usize) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_question).collect()
    }
}
