//! Tenants' speech settings (`docs/TENANT_SPEECH.md`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::speech::{SpeechSettingsStore, TenantSpeech};
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
}
