//! Identity providers, users' identities at them and sign-ins in flight in
//! PostgreSQL (`docs/IDENTITY_AND_DEVICES.md` phase 3). A sign-in and its
//! one-time code are each taken by one `DELETE … RETURNING`, so a replayed
//! callback or code finds nothing.

use crate::PostgresStorage;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::identity::{
    IdentityProvider, IdentityProviderId, IdentityStore, ProviderKind, SsoHandoff, SsoLogin,
    UserIdentity, SSO_HANDOFF_TTL, SSO_LOGIN_TTL,
};
use fcp_core::{TenantId, UserId};
use sqlx::Row;
use std::time::SystemTime;

fn store_error(e: impl std::fmt::Display) -> fcp_core::Error {
    fcp_core::Error::Storage(fcp_core::error::StorageError::Database(e.to_string()))
}

fn at(t: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(t)
}

const PROVIDER_COLUMNS: &str =
    "id, tenant_id, name, kind, enabled, settings_json, client_secret, created_at, updated_at";
const IDENTITY_COLUMNS: &str =
    "provider_id, subject, user_id, tenant_id, email, created_at, last_sign_in_at";

impl PostgresStorage {
    async fn provider_from(
        &self,
        row: &sqlx::postgres::PgRow,
    ) -> fcp_core::Result<IdentityProvider> {
        let settings: serde_json::Value = row.get("settings_json");
        Ok(IdentityProvider {
            id: row.get::<String, _>("id").parse().map_err(store_error)?,
            tenant_id: row
                .get::<String, _>("tenant_id")
                .parse()
                .map_err(store_error)?,
            name: row.get("name"),
            kind: ProviderKind::parse(&row.get::<String, _>("kind")).unwrap_or_default(),
            enabled: row.get("enabled"),
            settings: serde_json::from_value(settings).map_err(store_error)?,
            client_secret: self
                .open_opt(row.get("client_secret"))
                .await
                .map_err(store_error)?,
            created_at: row.get::<DateTime<Utc>, _>("created_at").into(),
            updated_at: row.get::<DateTime<Utc>, _>("updated_at").into(),
        })
    }
}

fn identity_from(row: &sqlx::postgres::PgRow) -> fcp_core::Result<UserIdentity> {
    Ok(UserIdentity {
        provider_id: row
            .get::<String, _>("provider_id")
            .parse()
            .map_err(store_error)?,
        subject: row.get("subject"),
        user_id: row
            .get::<String, _>("user_id")
            .parse()
            .map_err(store_error)?,
        tenant_id: row
            .get::<String, _>("tenant_id")
            .parse()
            .map_err(store_error)?,
        email: row.get("email"),
        created_at: row.get::<DateTime<Utc>, _>("created_at").into(),
        last_sign_in_at: row
            .get::<Option<DateTime<Utc>>, _>("last_sign_in_at")
            .map(Into::into),
    })
}

#[async_trait]
impl IdentityStore for PostgresStorage {
    async fn list_identity_providers(
        &self,
        tenant_id: &TenantId,
    ) -> fcp_core::Result<Vec<IdentityProvider>> {
        let rows = sqlx::query(&format!(
            "SELECT {PROVIDER_COLUMNS} FROM identity_providers WHERE tenant_id = $1 ORDER BY name, id"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            out.push(self.provider_from(row).await?);
        }
        Ok(out)
    }

    async fn get_identity_provider(
        &self,
        id: &IdentityProviderId,
    ) -> fcp_core::Result<Option<IdentityProvider>> {
        let row = sqlx::query(&format!(
            "SELECT {PROVIDER_COLUMNS} FROM identity_providers WHERE id = $1"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        match row {
            Some(row) => Ok(Some(self.provider_from(&row).await?)),
            None => Ok(None),
        }
    }

    async fn put_identity_provider(&self, p: &IdentityProvider) -> fcp_core::Result<()> {
        let secret = self
            .seal_opt(p.client_secret.as_deref())
            .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO identity_providers (id, tenant_id, name, kind, enabled, settings_json,
                 client_secret, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name, kind = EXCLUDED.kind, enabled = EXCLUDED.enabled,
                 settings_json = EXCLUDED.settings_json,
                 client_secret = EXCLUDED.client_secret, updated_at = EXCLUDED.updated_at",
        )
        .bind(p.id.to_string())
        .bind(p.tenant_id.to_string())
        .bind(&p.name)
        .bind(p.kind.as_str())
        .bind(p.enabled)
        .bind(serde_json::to_value(&p.settings).map_err(store_error)?)
        .bind(secret)
        .bind(at(p.created_at))
        .bind(at(p.updated_at))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn delete_identity_provider(&self, id: &IdentityProviderId) -> fcp_core::Result<bool> {
        let done = sqlx::query("DELETE FROM identity_providers WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(done.rows_affected() == 1)
    }

    async fn get_user_identity(
        &self,
        provider_id: &IdentityProviderId,
        subject: &str,
    ) -> fcp_core::Result<Option<UserIdentity>> {
        let row = sqlx::query(&format!(
            "SELECT {IDENTITY_COLUMNS} FROM user_identities WHERE provider_id = $1 AND subject = $2"
        ))
        .bind(provider_id.to_string())
        .bind(subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        row.as_ref().map(identity_from).transpose()
    }

    async fn put_user_identity(&self, i: &UserIdentity) -> fcp_core::Result<()> {
        sqlx::query(
            "INSERT INTO user_identities (provider_id, subject, user_id, tenant_id, email,
                 created_at, last_sign_in_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (provider_id, subject) DO UPDATE SET
                 email = EXCLUDED.email, last_sign_in_at = EXCLUDED.last_sign_in_at",
        )
        .bind(i.provider_id.to_string())
        .bind(&i.subject)
        .bind(i.user_id.to_string())
        .bind(i.tenant_id.to_string())
        .bind(&i.email)
        .bind(at(i.created_at))
        .bind(i.last_sign_in_at.map(at))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn list_user_identities(&self, user_id: &UserId) -> fcp_core::Result<Vec<UserIdentity>> {
        let rows = sqlx::query(&format!(
            "SELECT {IDENTITY_COLUMNS} FROM user_identities WHERE user_id = $1 ORDER BY created_at"
        ))
        .bind(user_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(identity_from).collect()
    }

    async fn list_tenant_identities(
        &self,
        tenant_id: &TenantId,
    ) -> fcp_core::Result<Vec<UserIdentity>> {
        let rows = sqlx::query(&format!(
            "SELECT {IDENTITY_COLUMNS} FROM user_identities WHERE tenant_id = $1
             ORDER BY provider_id, subject"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(identity_from).collect()
    }

    async fn delete_user_identity(
        &self,
        provider_id: &IdentityProviderId,
        subject: &str,
    ) -> fcp_core::Result<bool> {
        let done =
            sqlx::query("DELETE FROM user_identities WHERE provider_id = $1 AND subject = $2")
                .bind(provider_id.to_string())
                .bind(subject)
                .execute(&self.pool)
                .await
                .map_err(store_error)?;
        Ok(done.rows_affected() == 1)
    }

    async fn create_sso_login(&self, l: &SsoLogin) -> fcp_core::Result<()> {
        // Sign-ins nobody finished are cleared as new ones begin.
        sqlx::query("DELETE FROM sso_logins WHERE created_at < $1")
            .bind(at(SystemTime::now() - SSO_LOGIN_TTL))
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO sso_logins (state, provider_id, tenant_id, nonce, code_verifier, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&l.state)
        .bind(l.provider_id.to_string())
        .bind(l.tenant_id.to_string())
        .bind(&l.nonce)
        .bind(&l.code_verifier)
        .bind(at(l.created_at))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn take_sso_login(
        &self,
        state: &str,
        now: SystemTime,
    ) -> fcp_core::Result<Option<SsoLogin>> {
        let row = sqlx::query(
            "DELETE FROM sso_logins WHERE state = $1
             RETURNING state, provider_id, tenant_id, nonce, code_verifier, created_at",
        )
        .bind(state)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        let Some(row) = row else { return Ok(None) };
        let login = SsoLogin {
            state: row.get("state"),
            provider_id: row
                .get::<String, _>("provider_id")
                .parse()
                .map_err(store_error)?,
            tenant_id: row
                .get::<String, _>("tenant_id")
                .parse()
                .map_err(store_error)?,
            nonce: row.get("nonce"),
            code_verifier: row.get("code_verifier"),
            created_at: row.get::<DateTime<Utc>, _>("created_at").into(),
        };
        let fresh = now.duration_since(login.created_at).unwrap_or_default() <= SSO_LOGIN_TTL;
        Ok(fresh.then_some(login))
    }

    async fn put_sso_handoff(&self, h: &SsoHandoff, now: SystemTime) -> fcp_core::Result<()> {
        sqlx::query("DELETE FROM sso_handoffs WHERE created_at < $1")
            .bind(at(now - SSO_HANDOFF_TTL))
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO sso_handoffs (code_hash, user_id, provider_id, idp_sid, created_at,
                 idp_logout_hint)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&h.code_hash)
        .bind(h.user_id.to_string())
        .bind(h.provider_id.to_string())
        .bind(&h.idp_sid)
        .bind(at(now))
        .bind(&h.idp_logout_hint)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn remember_saml_assertion(
        &self,
        provider_id: &IdentityProviderId,
        assertion_id: &str,
        until: SystemTime,
    ) -> fcp_core::Result<bool> {
        sqlx::query("DELETE FROM saml_assertions WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        let done = sqlx::query(
            "INSERT INTO saml_assertions (provider_id, assertion_id, expires_at)
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(provider_id.to_string())
        .bind(assertion_id)
        .bind(at(until))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(done.rows_affected() == 1)
    }

    async fn take_sso_handoff(
        &self,
        code_hash: &str,
        now: SystemTime,
    ) -> fcp_core::Result<Option<SsoHandoff>> {
        let row = sqlx::query(
            "DELETE FROM sso_handoffs WHERE code_hash = $1 AND created_at >= $2
             RETURNING code_hash, user_id, provider_id, idp_sid, idp_logout_hint",
        )
        .bind(code_hash)
        .bind(at(now - SSO_HANDOFF_TTL))
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(SsoHandoff {
            code_hash: row.get("code_hash"),
            user_id: row
                .get::<String, _>("user_id")
                .parse()
                .map_err(store_error)?,
            provider_id: row
                .get::<String, _>("provider_id")
                .parse()
                .map_err(store_error)?,
            idp_sid: row.get("idp_sid"),
            idp_logout_hint: row.get("idp_logout_hint"),
        }))
    }
}
