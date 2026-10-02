//! A user's second factor in PostgreSQL (`docs/IDENTITY_AND_DEVICES.md`
//! phase 2b): the secret sealed at rest, the step and the recovery codes
//! spent in single statements, so two sign-ins cannot both use one.

use crate::PostgresStorage;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::totp::{TotpStore, UserTotp};
use fcp_core::UserId;
use sqlx::Row;
use std::time::SystemTime;

fn store_error(e: impl std::fmt::Display) -> fcp_core::Error {
    fcp_core::Error::Storage(fcp_core::error::StorageError::Database(e.to_string()))
}

#[async_trait]
impl TotpStore for PostgresStorage {
    async fn get_user_totp(&self, user_id: &UserId) -> fcp_core::Result<Option<UserTotp>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, secret, confirmed_at, last_step, recovery_hashes, created_at
             FROM user_totp WHERE id = $1",
        )
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let secret = self
            .open_opt(Some(row.get::<String, _>("secret")))
            .await
            .map_err(store_error)?
            .unwrap_or_default();
        Ok(Some(UserTotp {
            user_id: *user_id,
            tenant_id: row
                .get::<String, _>("tenant_id")
                .parse()
                .map_err(store_error)?,
            secret,
            confirmed_at: row
                .get::<Option<DateTime<Utc>>, _>("confirmed_at")
                .map(Into::into),
            last_step: row.get::<Option<i64>, _>("last_step").map(|s| s as u64),
            recovery_hashes: row.get("recovery_hashes"),
            created_at: row.get::<DateTime<Utc>, _>("created_at").into(),
        }))
    }

    async fn put_user_totp(&self, totp: &UserTotp) -> fcp_core::Result<()> {
        let secret = self
            .seal_opt(Some(&totp.secret))
            .map_err(store_error)?
            .unwrap_or_default();
        sqlx::query(
            "INSERT INTO user_totp (id, tenant_id, secret, confirmed_at, last_step,
                 recovery_hashes, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET
                 secret = EXCLUDED.secret, confirmed_at = EXCLUDED.confirmed_at,
                 last_step = EXCLUDED.last_step, recovery_hashes = EXCLUDED.recovery_hashes,
                 created_at = EXCLUDED.created_at",
        )
        .bind(totp.user_id.to_string())
        .bind(totp.tenant_id.to_string())
        .bind(secret)
        .bind(totp.confirmed_at.map(DateTime::<Utc>::from))
        .bind(totp.last_step.map(|s| s as i64))
        .bind(&totp.recovery_hashes)
        .bind(DateTime::<Utc>::from(totp.created_at))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn confirm_user_totp(
        &self,
        user_id: &UserId,
        recovery_hashes: &[String],
        step: u64,
        at: SystemTime,
    ) -> fcp_core::Result<()> {
        sqlx::query(
            "UPDATE user_totp SET confirmed_at = $2, recovery_hashes = $3, last_step = $4
             WHERE id = $1",
        )
        .bind(user_id.to_string())
        .bind(DateTime::<Utc>::from(at))
        .bind(recovery_hashes)
        .bind(step as i64)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn use_totp_step(&self, user_id: &UserId, step: u64) -> fcp_core::Result<bool> {
        let done = sqlx::query(
            "UPDATE user_totp SET last_step = $2
             WHERE id = $1 AND (last_step IS NULL OR last_step < $2)",
        )
        .bind(user_id.to_string())
        .bind(step as i64)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(done.rows_affected() == 1)
    }

    async fn use_recovery_code(&self, user_id: &UserId, hash: &str) -> fcp_core::Result<bool> {
        let done = sqlx::query(
            "UPDATE user_totp SET recovery_hashes = array_remove(recovery_hashes, $2)
             WHERE id = $1 AND $2 = ANY(recovery_hashes)",
        )
        .bind(user_id.to_string())
        .bind(hash)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(done.rows_affected() == 1)
    }

    async fn set_recovery_codes(
        &self,
        user_id: &UserId,
        recovery_hashes: &[String],
    ) -> fcp_core::Result<()> {
        sqlx::query("UPDATE user_totp SET recovery_hashes = $2 WHERE id = $1")
            .bind(user_id.to_string())
            .bind(recovery_hashes)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    async fn delete_user_totp(&self, user_id: &UserId) -> fcp_core::Result<()> {
        sqlx::query("DELETE FROM user_totp WHERE id = $1")
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }
}
