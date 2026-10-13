//! Sign-in sessions in PostgreSQL (`docs/IDENTITY_AND_DEVICES.md` phase 1).
//!
//! Read and written on the primary: a session revoked must be refused at
//! once, which a replica a second behind would not do.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::session::{
    AppAuthorization, AppClient, LoginSession, LoginSessionStore, RefreshOutcome,
    REFRESH_REUSE_GRACE,
};
use fcp_core::{LoginSessionId, TenantId, UserId};
use sqlx::{Pool, Postgres, Row};
use std::time::SystemTime;

const COLUMNS: &str = "id, tenant_id, user_id, method, created_at, refreshed_at, expires_at, \
     revoked_at, revoked_reason, ip, user_agent, refresh_hash, previous_refresh_hash, \
     idp_provider_id, idp_sid, idp_logout_hint, app_name, app_install_id, app_platform, app_version, \
     device_id";

const AUTHORIZATION_COLUMNS: &str = "id, client_id, redirect_uri, code_challenge, state, \
     created_at, expires_at, user_id, tenant_id, code_hash, code_expires_at";

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
                logout_hint: row.get("idp_logout_hint"),
            }),
        app: match (
            row.get::<Option<String>, _>("app_name"),
            row.get::<Option<String>, _>("app_install_id"),
        ) {
            (Some(name), Some(install_id)) => Some(AppClient {
                name,
                install_id,
                platform: row.get("app_platform"),
                version: row.get("app_version"),
            }),
            _ => None,
        },
        device_id: row
            .get::<Option<String>, _>("device_id")
            .map(|d| d.parse().map_err(|_| bad("device")))
            .transpose()?,
    })
}

fn parse_authorization(row: &sqlx::postgres::PgRow) -> fcp_core::Result<AppAuthorization> {
    let bad = |what: &str| {
        fcp_core::Error::Storage(fcp_core::error::StorageError::Database(format!(
            "an app authorization's {what} does not parse"
        )))
    };
    Ok(AppAuthorization {
        id: row.get("id"),
        client_id: row.get("client_id"),
        redirect_uri: row.get("redirect_uri"),
        code_challenge: row.get("code_challenge"),
        state: row.get("state"),
        created_at: row.get::<DateTime<Utc>, _>("created_at").into(),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at").into(),
        user_id: row
            .get::<Option<String>, _>("user_id")
            .map(|u| u.parse().map_err(|_| bad("user")))
            .transpose()?,
        tenant_id: row
            .get::<Option<String>, _>("tenant_id")
            .map(|t| t.parse::<TenantId>().map_err(|_| bad("tenant")))
            .transpose()?,
        code_hash: row.get("code_hash"),
        code_expires_at: row
            .get::<Option<DateTime<Utc>>, _>("code_expires_at")
            .map(Into::into),
    })
}

#[async_trait]
impl LoginSessionStore for PostgresLoginSessionStore {
    async fn create_login_session(&self, s: &LoginSession) -> fcp_core::Result<()> {
        sqlx::query(
            "INSERT INTO login_sessions (id, tenant_id, user_id, method, created_at,
                 refreshed_at, expires_at, ip, user_agent, refresh_hash, idp_provider_id, idp_sid,
                 app_name, app_install_id, app_platform, app_version, device_id, idp_logout_hint)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                 $18)",
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
        .bind(s.app.as_ref().map(|a| a.name.clone()))
        .bind(s.app.as_ref().map(|a| a.install_id.clone()))
        .bind(s.app.as_ref().and_then(|a| a.platform.clone()))
        .bind(s.app.as_ref().and_then(|a| a.version.clone()))
        .bind(s.device_id.map(|d| d.to_string()))
        .bind(s.idp.as_ref().and_then(|i| i.logout_hint.clone()))
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

    async fn extend_login_session(
        &self,
        id: &LoginSessionId,
        expires_at: SystemTime,
    ) -> fcp_core::Result<()> {
        sqlx::query(
            "UPDATE login_sessions SET expires_at = GREATEST(expires_at, $2)
             WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(id.to_string())
        .bind(at(expires_at))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn app_sessions_of_install(
        &self,
        user_id: &UserId,
        install_id: &str,
    ) -> fcp_core::Result<Vec<LoginSession>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM login_sessions
             WHERE user_id = $1 AND app_install_id = $2
               AND revoked_at IS NULL AND expires_at > NOW()"
        ))
        .bind(user_id.to_string())
        .bind(install_id)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(parse).collect()
    }

    async fn app_install_device(
        &self,
        user_id: &UserId,
        install_id: &str,
    ) -> fcp_core::Result<Option<fcp_core::DeviceId>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT device_id FROM app_installs WHERE user_id = $1 AND install_id = $2",
        )
        .bind(user_id.to_string())
        .bind(install_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(row.and_then(|(d,)| d.parse().ok()))
    }

    async fn set_app_install_device(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        install_id: &str,
        device_id: &fcp_core::DeviceId,
    ) -> fcp_core::Result<()> {
        sqlx::query(
            "INSERT INTO app_installs (user_id, install_id, tenant_id, device_id)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, install_id) DO UPDATE SET device_id = EXCLUDED.device_id",
        )
        .bind(user_id.to_string())
        .bind(install_id)
        .bind(tenant_id.to_string())
        .bind(device_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn app_installs_of_user(
        &self,
        user_id: &UserId,
    ) -> fcp_core::Result<Vec<(fcp_core::DeviceId, String)>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT device_id, install_id FROM app_installs WHERE user_id = $1")
                .bind(user_id.to_string())
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?;
        Ok(rows
            .into_iter()
            .filter_map(|(d, i)| d.parse().ok().map(|d| (d, i)))
            .collect())
    }

    async fn revoke_device_sessions(
        &self,
        device_id: &fcp_core::DeviceId,
        reason: &str,
        now: SystemTime,
    ) -> fcp_core::Result<u64> {
        let done = sqlx::query(
            "UPDATE login_sessions SET revoked_at = $2, revoked_reason = $3
             WHERE device_id = $1 AND revoked_at IS NULL",
        )
        .bind(device_id.to_string())
        .bind(at(now))
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(done.rows_affected())
    }

    async fn create_app_authorization(&self, a: &AppAuthorization) -> fcp_core::Result<()> {
        sqlx::query(
            "INSERT INTO app_authorizations
                 (id, client_id, redirect_uri, code_challenge, state, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&a.id)
        .bind(&a.client_id)
        .bind(&a.redirect_uri)
        .bind(&a.code_challenge)
        .bind(&a.state)
        .bind(at(a.created_at))
        .bind(at(a.expires_at))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn get_app_authorization(&self, id: &str) -> fcp_core::Result<Option<AppAuthorization>> {
        let row = sqlx::query(&format!(
            "SELECT {AUTHORIZATION_COLUMNS} FROM app_authorizations WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        row.as_ref().map(parse_authorization).transpose()
    }

    async fn grant_app_authorization(
        &self,
        id: &str,
        user_id: &UserId,
        tenant_id: &TenantId,
        code_hash: &str,
        code_expires_at: SystemTime,
        now: SystemTime,
    ) -> fcp_core::Result<bool> {
        let done = sqlx::query(
            "UPDATE app_authorizations
             SET user_id = $2, tenant_id = $3, code_hash = $4, code_expires_at = $5
             WHERE id = $1 AND user_id IS NULL AND expires_at > $6",
        )
        .bind(id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(code_hash)
        .bind(at(code_expires_at))
        .bind(at(now))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(done.rows_affected() == 1)
    }

    async fn redeem_app_code(
        &self,
        code_hash: &str,
        now: SystemTime,
    ) -> fcp_core::Result<Option<AppAuthorization>> {
        let row = sqlx::query(&format!(
            "UPDATE app_authorizations SET redeemed_at = $2
             WHERE code_hash = $1 AND redeemed_at IS NULL AND code_expires_at > $2
             RETURNING {AUTHORIZATION_COLUMNS}"
        ))
        .bind(code_hash)
        .bind(at(now))
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        row.as_ref().map(parse_authorization).transpose()
    }

    async fn sweep_ended_sessions(&self, keep: std::time::Duration) -> fcp_core::Result<u64> {
        let before = at(SystemTime::now() - keep);
        // App sign-ins nobody finished, or long finished.
        sqlx::query("DELETE FROM app_authorizations WHERE expires_at < $1")
            .bind(before)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
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
