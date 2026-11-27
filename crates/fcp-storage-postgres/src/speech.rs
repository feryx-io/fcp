//! Tenants' speech settings (`docs/TENANT_SPEECH.md`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::speech::{
    SpeechAccount, SpeechAccountKind, SpeechAccountStore, SpeechSettingsStore, TenantSpeech,
};
use fcp_core::TenantId;
use sqlx::Row;

use crate::{datetime_to_system_time, system_time_to_datetime, PostgresStorage};

#[async_trait]
impl SpeechSettingsStore for PostgresStorage {
    async fn get_tenant_speech(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Option<TenantSpeech>> {
        let row =
            sqlx::query("SELECT settings, updated_at FROM tenant_speech WHERE tenant_id = $1")
                .bind(tenant_id.to_string())
                .fetch_optional(self.reads())
                .await?;
        row.map(|row| {
            let mut speech: TenantSpeech =
                serde_json::from_value(row.try_get::<serde_json::Value, _>("settings")?)?;
            speech.tenant_id = *tenant_id;
            speech.updated_at =
                datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("updated_at")?);
            Ok(speech)
        })
        .transpose()
    }

    async fn put_tenant_speech(&self, speech: &TenantSpeech) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tenant_speech (tenant_id, settings, updated_at) VALUES ($1, $2, $3)
             ON CONFLICT (tenant_id) DO UPDATE SET settings = EXCLUDED.settings,
                                                   updated_at = EXCLUDED.updated_at",
        )
        .bind(speech.tenant_id.to_string())
        .bind(serde_json::to_value(speech)?)
        .bind(system_time_to_datetime(speech.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_tenant_speech(&self, tenant_id: &TenantId) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM tenant_speech WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .execute(self.writes())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn held_back_providers(&self) -> anyhow::Result<Vec<String>> {
        let row: Option<(Vec<String>,)> =
            sqlx::query_as("SELECT held_back FROM platform_speech WHERE id")
                .fetch_optional(self.reads())
                .await?;
        Ok(row.map(|(h,)| h).unwrap_or_default())
    }

    async fn put_held_back_providers(&self, providers: &[String]) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO platform_speech (id, held_back, updated_at) VALUES (TRUE, $1, now())
             ON CONFLICT (id) DO UPDATE SET held_back = EXCLUDED.held_back, updated_at = now()",
        )
        .bind(providers)
        .execute(self.writes())
        .await?;
        Ok(())
    }
}

const ACCOUNT_COLUMNS: &str = "id, tenant_id, name, kind, settings, secret, created_at, updated_at";

/// What an account keeps beside its secret.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct AccountSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access_key_id: Option<String>,
}

impl PostgresStorage {
    async fn account_from(&self, row: &sqlx::postgres::PgRow) -> anyhow::Result<SpeechAccount> {
        let settings: AccountSettings =
            serde_json::from_value(row.try_get::<serde_json::Value, _>("settings")?)
                .unwrap_or_default();
        let kind: String = row.try_get("kind")?;
        Ok(SpeechAccount {
            id: row.try_get::<String, _>("id")?.parse()?,
            tenant_id: row.try_get::<String, _>("tenant_id")?.parse()?,
            name: row.try_get("name")?,
            kind: SpeechAccountKind::parse(&kind)
                .ok_or_else(|| anyhow::anyhow!("unknown speech account kind {kind:?}"))?,
            region: settings.region,
            model: settings.model,
            endpoint: settings.endpoint,
            access_key_id: settings.access_key_id,
            secret: self.open_opt(row.try_get("secret")?).await?,
            created_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("created_at")?),
            updated_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("updated_at")?),
        })
    }
}

#[async_trait]
impl SpeechAccountStore for PostgresStorage {
    async fn list_speech_accounts(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Vec<SpeechAccount>> {
        let rows = sqlx::query(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM speech_accounts WHERE tenant_id = $1 ORDER BY name"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            out.push(self.account_from(row).await?);
        }
        Ok(out)
    }

    async fn get_speech_account(
        &self,
        tenant_id: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<SpeechAccount>> {
        let row = sqlx::query(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM speech_accounts WHERE tenant_id = $1 AND id = $2"
        ))
        .bind(tenant_id.to_string())
        .bind(id.to_string())
        .fetch_optional(self.reads())
        .await?;
        match row {
            Some(row) => Ok(Some(self.account_from(&row).await?)),
            None => Ok(None),
        }
    }

    async fn put_speech_account(&self, a: &SpeechAccount) -> anyhow::Result<()> {
        let settings = AccountSettings {
            region: a.region.clone(),
            model: a.model.clone(),
            endpoint: a.endpoint.clone(),
            access_key_id: a.access_key_id.clone(),
        };
        let secret = self.seal_opt(a.secret.as_deref())?;
        sqlx::query(
            "INSERT INTO speech_accounts (id, tenant_id, name, kind, settings, secret, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name, kind = EXCLUDED.kind, settings = EXCLUDED.settings,
                 secret = EXCLUDED.secret, updated_at = EXCLUDED.updated_at
             WHERE speech_accounts.tenant_id = EXCLUDED.tenant_id",
        )
        .bind(a.id.to_string())
        .bind(a.tenant_id.to_string())
        .bind(&a.name)
        .bind(a.kind.as_str())
        .bind(serde_json::to_value(&settings)?)
        .bind(secret)
        .bind(system_time_to_datetime(a.created_at))
        .bind(system_time_to_datetime(a.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_speech_account(
        &self,
        tenant_id: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM speech_accounts WHERE tenant_id = $1 AND id = $2")
            .bind(tenant_id.to_string())
            .bind(id.to_string())
            .execute(self.writes())
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
