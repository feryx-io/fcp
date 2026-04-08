//! PostgreSQL implementation of the API token store.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::storage::{ApiToken, ApiTokenStore};
use fcp_core::{ApiTokenId, TenantId};
use sqlx::{Pool, Postgres, Row};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn system_time_to_datetime(st: SystemTime) -> DateTime<Utc> {
    let duration = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    DateTime::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos()).unwrap_or_default()
}

fn datetime_to_system_time(dt: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH
        + Duration::from_secs(dt.timestamp() as u64)
        + Duration::from_nanos(dt.timestamp_subsec_nanos() as u64)
}

/// PostgreSQL-backed API token store.
#[derive(Clone)]
pub struct PostgresApiTokenStore {
    pool: Pool<Postgres>,
}

impl PostgresApiTokenStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ApiTokenStore for PostgresApiTokenStore {
    async fn create_api_token(&self, token: ApiToken) -> anyhow::Result<()> {
        let permissions_json = serde_json::to_value(&token.permissions)?;
        let expires_at = token.expires_at.map(system_time_to_datetime);

        sqlx::query(
            "INSERT INTO api_tokens (id, name, token_hash, prefix, user_id, tenant_id, permissions, expires_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(token.id.to_string())
        .bind(&token.name)
        .bind(&token.token_hash)
        .bind(&token.prefix)
        .bind(&token.user_id)
        .bind(token.tenant_id.to_string())
        .bind(permissions_json)
        .bind(expires_at)
        .bind(system_time_to_datetime(token.created_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_api_tokens(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<ApiToken>> {
        let rows = sqlx::query(
            "SELECT id, name, token_hash, prefix, user_id, tenant_id, permissions, expires_at, created_at, last_used_at
             FROM api_tokens
             WHERE tenant_id = $1
             ORDER BY created_at DESC",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(row_to_api_token).collect())
    }

    async fn get_api_token(&self, id: &ApiTokenId) -> anyhow::Result<Option<ApiToken>> {
        let row = sqlx::query(
            "SELECT id, name, token_hash, prefix, user_id, tenant_id, permissions, expires_at, created_at, last_used_at
             FROM api_tokens
             WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.as_ref().map(row_to_api_token))
    }

    async fn get_api_token_by_hash(&self, token_hash: &str) -> anyhow::Result<Option<ApiToken>> {
        let row = sqlx::query(
            "SELECT id, name, token_hash, prefix, user_id, tenant_id, permissions, expires_at, created_at, last_used_at
             FROM api_tokens
             WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.as_ref().map(row_to_api_token))
    }

    async fn delete_api_token(&self, id: &ApiTokenId) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM api_tokens WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_last_used(&self, id: &ApiTokenId) -> anyhow::Result<()> {
        sqlx::query("UPDATE api_tokens SET last_used_at = NOW() WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn row_to_api_token(row: &sqlx::postgres::PgRow) -> ApiToken {
    let created_at: DateTime<Utc> = row.get("created_at");
    let expires_at: Option<DateTime<Utc>> = row.get("expires_at");
    let last_used_at: Option<DateTime<Utc>> = row.get("last_used_at");
    let permissions_json: serde_json::Value = row.get("permissions");
    let permissions: Vec<String> = serde_json::from_value(permissions_json).unwrap_or_default();

    let id_str: String = row.get("id");
    let tenant_str: String = row.get("tenant_id");

    ApiToken {
        id: id_str.parse().unwrap_or_else(|_| ApiTokenId::new()),
        name: row.get("name"),
        token_hash: row.get("token_hash"),
        prefix: row.get("prefix"),
        user_id: row.get("user_id"),
        tenant_id: tenant_str.parse().unwrap_or_else(|_| TenantId::nil()),
        permissions,
        expires_at: expires_at.map(datetime_to_system_time),
        created_at: datetime_to_system_time(created_at),
        last_used_at: last_used_at.map(datetime_to_system_time),
    }
}
