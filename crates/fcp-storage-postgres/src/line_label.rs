//! A user's own names for their lines (`docs/DEVICE_LINES.md`).

use std::collections::HashMap;

use async_trait::async_trait;
use fcp_core::line_label::LineLabelStore;
use fcp_core::{TenantId, UserId};
use sqlx::Row;

use crate::PostgresStorage;

#[async_trait]
impl LineLabelStore for PostgresStorage {
    async fn line_labels(&self, user: &UserId) -> anyhow::Result<HashMap<String, String>> {
        let rows = sqlx::query("SELECT line, label FROM user_line_labels WHERE user_id = $1")
            .bind(user.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| Ok((r.try_get("line")?, r.try_get("label")?)))
            .collect()
    }

    async fn set_line_label(
        &self,
        tenant: &TenantId,
        user: &UserId,
        line: &str,
        label: &str,
    ) -> anyhow::Result<()> {
        if label.trim().is_empty() {
            sqlx::query("DELETE FROM user_line_labels WHERE user_id = $1 AND line = $2")
                .bind(user.to_string())
                .bind(line)
                .execute(&self.pool)
                .await?;
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO user_line_labels (user_id, tenant_id, line, label, updated_at)
             VALUES ($1, $2, $3, $4, NOW())
             ON CONFLICT (user_id, line) DO UPDATE SET
                label = EXCLUDED.label, updated_at = EXCLUDED.updated_at",
        )
        .bind(user.to_string())
        .bind(tenant.to_string())
        .bind(line)
        .bind(label.trim())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
