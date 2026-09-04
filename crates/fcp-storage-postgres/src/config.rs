//! PostgreSQL implementation of the ConfigStore trait.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::config_store::{AuditEntry, ConfigEntry, ConfigStore};
use sqlx::Row;

use crate::PostgresStorage;

/// Convert a sqlx Row to a ConfigEntry.
fn row_to_config_entry(row: sqlx::postgres::PgRow) -> Result<ConfigEntry> {
    Ok(ConfigEntry {
        service: row.try_get("service").context("missing service column")?,
        section: row.try_get("section").context("missing section column")?,
        tenant_id: row
            .try_get("tenant_id")
            .context("missing tenant_id column")?,
        value: row.try_get("value").context("missing value column")?,
        version: row.try_get("version").context("missing version column")?,
        updated_at: row
            .try_get::<DateTime<Utc>, _>("updated_at")
            .context("missing updated_at column")?,
        updated_by: row.try_get("updated_by").ok(),
    })
}

/// Convert a sqlx Row to an AuditEntry.
fn row_to_audit_entry(row: sqlx::postgres::PgRow) -> Result<AuditEntry> {
    Ok(AuditEntry {
        id: row.try_get("id").context("missing id column")?,
        service: row.try_get("service").context("missing service column")?,
        section: row.try_get("section").context("missing section column")?,
        tenant_id: row
            .try_get("tenant_id")
            .context("missing tenant_id column")?,
        old_value: row.try_get("old_value").ok(),
        new_value: row
            .try_get("new_value")
            .context("missing new_value column")?,
        changed_by: row.try_get("changed_by").ok(),
        changed_at: row
            .try_get::<DateTime<Utc>, _>("changed_at")
            .context("missing changed_at column")?,
        version: row.try_get("version").context("missing version column")?,
    })
}

#[async_trait]
impl ConfigStore for PostgresStorage {
    async fn get_config(
        &self,
        service: &str,
        section: &str,
        tenant_id: &str,
    ) -> fcp_core::Result<Option<ConfigEntry>> {
        let row = sqlx::query(
            "SELECT service, section, tenant_id, value, version, updated_at, updated_by
             FROM config_settings
             WHERE service = $1 AND section = $2 AND tenant_id = $3",
        )
        .bind(service)
        .bind(section)
        .bind(tenant_id)
        .fetch_optional(self.reads())
        .await
        .map_err(|e| fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string())))?;

        match row {
            Some(row) => {
                let entry = row_to_config_entry(row).map_err(|e| {
                    fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string()))
                })?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    async fn get_service_config(
        &self,
        service: &str,
        tenant_id: &str,
    ) -> fcp_core::Result<Vec<ConfigEntry>> {
        let rows = sqlx::query(
            "SELECT service, section, tenant_id, value, version, updated_at, updated_by
             FROM config_settings
             WHERE service = $1 AND tenant_id = $2
             ORDER BY section",
        )
        .bind(service)
        .bind(tenant_id)
        .fetch_all(self.reads())
        .await
        .map_err(|e| fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string())))?;

        let entries = rows
            .into_iter()
            .filter_map(|row| row_to_config_entry(row).ok())
            .collect();

        Ok(entries)
    }

    async fn set_config(
        &self,
        service: &str,
        section: &str,
        tenant_id: &str,
        value: serde_json::Value,
        updated_by: Option<&str>,
    ) -> fcp_core::Result<i64> {
        // Use a transaction for atomic upsert + audit log insert
        let mut tx = self.writes().begin().await.map_err(|e| {
            fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string()))
        })?;

        // Get old value for audit log (if exists)
        let old_row = sqlx::query(
            "SELECT value, version FROM config_settings
             WHERE service = $1 AND section = $2 AND tenant_id = $3",
        )
        .bind(service)
        .bind(section)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string())))?;

        let (old_value, new_version) = match &old_row {
            Some(row) => {
                let old_val: serde_json::Value = row.try_get("value").map_err(|e| {
                    fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string()))
                })?;
                let old_ver: i64 = row.try_get("version").map_err(|e| {
                    fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string()))
                })?;
                (Some(old_val), old_ver + 1)
            }
            None => (None, 1),
        };

        // Upsert the config entry
        sqlx::query(
            "INSERT INTO config_settings (service, section, tenant_id, value, version, updated_at, updated_by)
             VALUES ($1, $2, $3, $4, $5, NOW(), $6)
             ON CONFLICT (service, section, tenant_id)
             DO UPDATE SET value = $4, version = $5, updated_at = NOW(), updated_by = $6",
        )
        .bind(service)
        .bind(section)
        .bind(tenant_id)
        .bind(&value)
        .bind(new_version)
        .bind(updated_by)
        .execute(&mut *tx)
        .await
        .map_err(|e| fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string())))?;

        // Insert audit log entry
        sqlx::query(
            "INSERT INTO config_audit_log (service, section, tenant_id, old_value, new_value, changed_by, changed_at, version)
             VALUES ($1, $2, $3, $4, $5, $6, NOW(), $7)",
        )
        .bind(service)
        .bind(section)
        .bind(tenant_id)
        .bind(&old_value)
        .bind(&value)
        .bind(updated_by)
        .bind(new_version)
        .execute(&mut *tx)
        .await
        .map_err(|e| fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string())))?;

        tx.commit().await.map_err(|e| {
            fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string()))
        })?;

        Ok(new_version)
    }

    async fn delete_config(
        &self,
        service: &str,
        section: &str,
        tenant_id: &str,
    ) -> fcp_core::Result<bool> {
        let result = sqlx::query(
            "DELETE FROM config_settings
             WHERE service = $1 AND section = $2 AND tenant_id = $3",
        )
        .bind(service)
        .bind(section)
        .bind(tenant_id)
        .execute(self.writes())
        .await
        .map_err(|e| fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string())))?;

        Ok(result.rows_affected() > 0)
    }

    async fn list_configs(
        &self,
        service: Option<&str>,
        tenant_id: Option<&str>,
    ) -> fcp_core::Result<Vec<ConfigEntry>> {
        let rows = match (service, tenant_id) {
            (Some(svc), Some(tid)) => {
                sqlx::query(
                    "SELECT service, section, tenant_id, value, version, updated_at, updated_by
                     FROM config_settings
                     WHERE service = $1 AND tenant_id = $2
                     ORDER BY service, section",
                )
                .bind(svc)
                .bind(tid)
                .fetch_all(self.reads())
                .await
            }
            (Some(svc), None) => {
                sqlx::query(
                    "SELECT service, section, tenant_id, value, version, updated_at, updated_by
                     FROM config_settings
                     WHERE service = $1
                     ORDER BY service, section",
                )
                .bind(svc)
                .fetch_all(self.reads())
                .await
            }
            (None, Some(tid)) => {
                sqlx::query(
                    "SELECT service, section, tenant_id, value, version, updated_at, updated_by
                     FROM config_settings
                     WHERE tenant_id = $1
                     ORDER BY service, section",
                )
                .bind(tid)
                .fetch_all(self.reads())
                .await
            }
            (None, None) => {
                sqlx::query(
                    "SELECT service, section, tenant_id, value, version, updated_at, updated_by
                     FROM config_settings
                     ORDER BY service, section",
                )
                .fetch_all(self.reads())
                .await
            }
        }
        .map_err(|e| fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string())))?;

        let entries = rows
            .into_iter()
            .filter_map(|row| row_to_config_entry(row).ok())
            .collect();

        Ok(entries)
    }

    async fn get_latest_version(&self, service: &str, tenant_id: &str) -> fcp_core::Result<i64> {
        let row = sqlx::query(
            "SELECT COALESCE(MAX(version), 0) as max_version
             FROM config_settings
             WHERE service = $1 AND tenant_id = $2",
        )
        .bind(service)
        .bind(tenant_id)
        .fetch_one(self.pool())
        .await
        .map_err(|e| fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string())))?;

        let version: i64 = row.try_get("max_version").map_err(|e| {
            fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string()))
        })?;

        Ok(version)
    }

    async fn get_audit_log(
        &self,
        service: &str,
        tenant_id: &str,
        section: Option<&str>,
        limit: i64,
    ) -> fcp_core::Result<Vec<AuditEntry>> {
        let rows = match section {
            Some(sec) => {
                sqlx::query(
                    "SELECT id, service, section, tenant_id, old_value, new_value, changed_by, changed_at, version
                     FROM config_audit_log
                     WHERE service = $1 AND tenant_id = $2 AND section = $3
                     ORDER BY changed_at DESC
                     LIMIT $4",
                )
                .bind(service)
                .bind(tenant_id)
                .bind(sec)
                .bind(limit)
                .fetch_all(self.pool())
                .await
            }
            None => {
                sqlx::query(
                    "SELECT id, service, section, tenant_id, old_value, new_value, changed_by, changed_at, version
                     FROM config_audit_log
                     WHERE service = $1 AND tenant_id = $2
                     ORDER BY changed_at DESC
                     LIMIT $3",
                )
                .bind(service)
                .bind(tenant_id)
                .bind(limit)
                .fetch_all(self.pool())
                .await
            }
        }
        .map_err(|e| fcp_core::Error::Storage(fcp_core::StorageError::Database(e.to_string())))?;

        let entries = rows
            .into_iter()
            .filter_map(|row| row_to_audit_entry(row).ok())
            .collect();

        Ok(entries)
    }
}
