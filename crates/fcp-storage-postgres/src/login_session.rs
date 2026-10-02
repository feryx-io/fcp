//! Sign-in sessions in PostgreSQL (`docs/IDENTITY_AND_DEVICES.md` phase 1).
//!
//! Read and written on the primary: a session revoked must be refused at
//! once, which a replica a second behind would not do.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::session::{LoginSession, LoginSessionStore, RefreshOutcome, REFRESH_REUSE_GRACE};
use fcp_core::{LoginSessionId, UserId};
use sqlx::{Pool, Postgres, Row};
use std::time::SystemTime;

const COLUMNS: &str = "id, tenant_id, user_id, method, created_at, refreshed_at, expires_at, \
     revoked_at, revoked_reason, ip, user_agent, refresh_hash, previous_refresh_hash, \
     idp_provider_id, idp_sid";

fn at(t: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(t)
}

fn store_error(e: sqlx::Error) -> fcp_core::Error {
    fcp_core::Error::Storage(fcp_core::error::StorageError::Database(e.to_string()))
}

/// PostgreSQL-backed sign-in sessions.
#[derive(Clone)]
pub struct PostgresLoginSessionStore {
    pool: Pool<Postgres>,
}

impl PostgresLoginSessionStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

fn parse(row: &sqlx::postgres::PgRow) -> fcp_core::Result<LoginSession> {
    let bad = |what: &str| {
        fcp_core::Error::Storage(fcp_core::error::StorageError::Database(format!(
            "a login session's {what} does not parse"
        )))
    };
    Ok(LoginSession {
        id: row.get::<String, _>("id").parse().map_err(|_| bad("id"))?,
        tenant_id: row
            .get::<String, _>("tenant_id")
            .parse()
            .map_err(|_| bad("tenant"))?,
        user_id: row
            .get::<String, _>("user_id")
            .parse()
            .map_err(|_| bad("user"))?,
        method: row.get("method"),
        created_at: row.get::<DateTime<Utc>, _>("created_at").into(),
        refreshed_at: row.get::<DateTime<Utc>, _>("refreshed_at").into(),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at").into(),
        revoked_at: row
            .get::<Option<DateTime<Utc>>, _>("revoked_at")
            .map(Into::into),
        revoked_reason: row.get("revoked_reason"),
        ip: row.get("ip"),
        user_agent: row.get("user_agent"),
        refresh_hash: row.get("refresh_hash"),
        previous_refresh_hash: row.get("previous_refresh_hash"),
        idp: row
            .get::<Option<String>, _>("idp_provider_id")
            .map(|provider_id| fcp_core::session::IdpSession {
                provider_id,
                sid: row.get("idp_sid"),
            }),
    })
}

#[async_trait]
impl LoginSessionStore for PostgresLoginSessionStore {
    async fn create_login_session(&self, s: &LoginSession) -> fcp_core::Result<()> {
        sqlx::query(
            "INSERT INTO login_sessions (id, tenant_id, user_id, method, created_at,
                 refreshed_at, expires_at, ip, user_agent, refresh_hash, idp_provider_id, idp_sid)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(s.id.to_string())
        .bind(s.tenant_id.to_string())
        .bind(s.user_id.to_string())
        .bind(&s.method)
        .bind(at(s.created_at))
        .bind(at(s.refreshed_at))
        .bind(at(s.expires_at))
        .bind(&s.ip)
        .bind(&s.user_agent)
        .bind(&s.refresh_hash)
        .bind(s.idp.as_ref().map(|i| i.provider_id.clone()))
        .bind(s.idp.as_ref().and_then(|i| i.sid.clone()))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn get_login_session(
        &self,
        id: &LoginSessionId,
    ) -> fcp_core::Result<Option<LoginSession>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM login_sessions WHERE id = $1"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        row.as_ref().map(parse).transpose()
    }

    async fn rotate_refresh(
        &self,
        id: &LoginSessionId,
        presented_hash: &str,
        next_hash: &str,
        now: SystemTime,
    ) -> fcp_core::Result<RefreshOutcome> {
        // The current secret: turned over in one statement, so two
        // refreshes cannot both win.
        let rotated = sqlx::query(
            "UPDATE login_sessions
             SET previous_refresh_hash = refresh_hash, refresh_hash = $3, refreshed_at = $4
             WHERE id = $1 AND refresh_hash = $2 AND revoked_at IS NULL AND expires_at > $4",
        )
        .bind(id.to_string())
        .bind(presented_hash)
        .bind(next_hash)
        .bind(at(now))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        if rotated.rows_affected() == 1 {
            return Ok(RefreshOutcome::Rotated);
        }
        let Some(session) = self.get_login_session(id).await? else {
            return Ok(RefreshOutcome::Unknown);
        };
        if !session.is_live(now) || session.previous_refresh_hash.as_deref() != Some(presented_hash)
        {
            return Ok(RefreshOutcome::Unknown);
        }
        let since = now.duration_since(session.refreshed_at).unwrap_or_default();
        if since <= REFRESH_REUSE_GRACE {
            return Ok(RefreshOutcome::RecentlyRotated);
        }
        // The secret before the current one, presented after the grace:
        // someone else has it. The session ends for both.
        self.revoke_login_session(id, "refresh secret reused", now)
            .await?;
        Ok(RefreshOutcome::Reused)
    }

    async fn revoke_login_session(
        &self,
        id: &LoginSessionId,
        reason: &str,
        now: SystemTime,
    ) -> fcp_core::Result<bool> {
        let done = sqlx::query(
            "UPDATE login_sessions SET revoked_at = $2, revoked_reason = $3
             WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(id.to_string())
        .bind(at(now))
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(done.rows_affected() == 1)
    }

    async fn revoke_user_sessions(
        &self,
        user_id: &UserId,
        keep: Option<&LoginSessionId>,
        reason: &str,
        now: SystemTime,
    ) -> fcp_core::Result<u64> {
        let done = sqlx::query(
            "UPDATE login_sessions SET revoked_at = $3, revoked_reason = $4
             WHERE user_id = $1 AND revoked_at IS NULL AND ($2::TEXT IS NULL OR id <> $2)",
        )
        .bind(user_id.to_string())
        .bind(keep.map(|k| k.to_string()))
        .bind(at(now))
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(done.rows_affected())
    }

    async fn revoke_idp_sessions(
        &self,
        provider_id: &str,
        user_id: Option<&UserId>,
        sid: Option<&str>,
        reason: &str,
        now: SystemTime,
    ) -> fcp_core::Result<u64> {
        if user_id.is_none() && sid.is_none() {
            return Ok(0);
        }
        let done = sqlx::query(
            "UPDATE login_sessions SET revoked_at = $4, revoked_reason = $5
             WHERE idp_provider_id = $1 AND revoked_at IS NULL
               AND ($2::TEXT IS NULL OR user_id = $2)
               AND ($3::TEXT IS NULL OR idp_sid = $3)",
        )
        .bind(provider_id)
        .bind(user_id.map(|u| u.to_string()))
        .bind(sid)
        .bind(at(now))
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(done.rows_affected())
    }

    async fn list_user_sessions(&self, user_id: &UserId) -> fcp_core::Result<Vec<LoginSession>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM login_sessions
             WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > NOW()
             ORDER BY created_at DESC"
        ))
        .bind(user_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(parse).collect()
    }

    async fn sweep_ended_sessions(&self, keep: std::time::Duration) -> fcp_core::Result<u64> {
        let before = at(SystemTime::now() - keep);
        let done = sqlx::query(
            "DELETE FROM login_sessions
             WHERE expires_at < $1 OR (revoked_at IS NOT NULL AND revoked_at < $1)",
        )
        .bind(before)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(done.rows_affected())
    }
}
