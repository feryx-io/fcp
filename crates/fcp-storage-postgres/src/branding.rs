//! Tenants' brands and their assets (`docs/BRANDING.md`).

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::branding::{BrandAsset, BrandStore, TenantBrand};
use fcp_core::TenantId;
use sqlx::Row;

use crate::{datetime_to_system_time, system_time_to_datetime, PostgresStorage};

const ASSET_COLS: &str = "tenant_id, sha256, mime, size, width, height, file_name, uploaded_at";

fn row_to_asset(row: &sqlx::postgres::PgRow) -> anyhow::Result<BrandAsset> {
    Ok(BrandAsset {
        tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
        sha256: row.try_get("sha256")?,
        mime: row.try_get("mime")?,
        size: row.try_get::<i64, _>("size")? as u64,
        width: row.try_get::<Option<i32>, _>("width")?.map(|w| w as u32),
        height: row.try_get::<Option<i32>, _>("height")?.map(|h| h as u32),
        file_name: row.try_get("file_name")?,
        uploaded_at: datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("uploaded_at")?),
    })
}

#[async_trait]
impl BrandStore for PostgresStorage {
    async fn get_brand(&self, tenant_id: &TenantId) -> anyhow::Result<Option<TenantBrand>> {
        let row = sqlx::query("SELECT brand, updated_at FROM tenant_brands WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .fetch_optional(self.reads())
            .await?;
        row.map(|row| {
            let mut brand: TenantBrand =
                serde_json::from_value(row.try_get::<serde_json::Value, _>("brand")?)?;
            brand.tenant_id = *tenant_id;
            brand.updated_at =
                datetime_to_system_time(row.try_get::<DateTime<Utc>, _>("updated_at")?);
            Ok(brand)
        })
        .transpose()
    }

    async fn put_brand(&self, brand: &TenantBrand) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tenant_brands (tenant_id, brand, updated_at) VALUES ($1, $2, $3)
             ON CONFLICT (tenant_id) DO UPDATE SET brand = EXCLUDED.brand,
                                                   updated_at = EXCLUDED.updated_at",
        )
        .bind(brand.tenant_id.to_string())
        .bind(serde_json::to_value(brand)?)
        .bind(system_time_to_datetime(brand.updated_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_brand(&self, tenant_id: &TenantId) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM tenant_brands WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .execute(self.writes())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn put_brand_asset(&self, asset: &BrandAsset, bytes: &[u8]) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO brand_assets
                 (tenant_id, sha256, mime, size, width, height, file_name, bytes, uploaded_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (tenant_id, sha256) DO NOTHING",
        )
        .bind(asset.tenant_id.to_string())
        .bind(&asset.sha256)
        .bind(&asset.mime)
        .bind(asset.size as i64)
        .bind(asset.width.map(|w| w as i32))
        .bind(asset.height.map(|h| h as i32))
        .bind(&asset.file_name)
        .bind(bytes)
        .bind(system_time_to_datetime(asset.uploaded_at))
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn list_brand_assets(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<BrandAsset>> {
        let rows = sqlx::query(&format!(
            "SELECT {ASSET_COLS} FROM brand_assets WHERE tenant_id = $1
             ORDER BY uploaded_at, sha256"
        ))
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(row_to_asset).collect()
    }

    async fn get_brand_asset(
        &self,
        tenant_id: &TenantId,
        sha256: &str,
    ) -> anyhow::Result<Option<BrandAsset>> {
        let row = sqlx::query(&format!(
            "SELECT {ASSET_COLS} FROM brand_assets WHERE tenant_id = $1 AND sha256 = $2"
        ))
        .bind(tenant_id.to_string())
        .bind(sha256)
        .fetch_optional(self.reads())
        .await?;
        row.as_ref().map(row_to_asset).transpose()
    }

    async fn brand_asset_bytes(
        &self,
        sha256: &str,
    ) -> anyhow::Result<Option<(BrandAsset, Vec<u8>)>> {
        let row = sqlx::query(&format!(
            "SELECT {ASSET_COLS}, bytes FROM brand_assets WHERE sha256 = $1 LIMIT 1"
        ))
        .bind(sha256)
        .fetch_optional(self.reads())
        .await?;
        row.map(|row| Ok((row_to_asset(&row)?, row.try_get::<Vec<u8>, _>("bytes")?)))
            .transpose()
    }

    async fn delete_brand_asset(&self, tenant_id: &TenantId, sha256: &str) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM brand_assets WHERE tenant_id = $1 AND sha256 = $2")
            .bind(tenant_id.to_string())
            .bind(sha256)
            .execute(self.writes())
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
