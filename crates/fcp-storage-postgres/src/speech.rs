//! Tenants' speech settings (`docs/TENANT_SPEECH.md`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::speech::{
    CapLevel, CapUnit, SiteSpeechCap, SpeechAccount, SpeechAccountKind, SpeechAccountStore,
    SpeechCap, SpeechCapKind, SpeechSettingsStore, SpeechUsage, SpeechUsageStore, TenantSpeech,
};
use fcp_core::TenantId;
use sqlx::Row;
use std::collections::BTreeMap;

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

    async fn speech_prices(&self) -> anyhow::Result<BTreeMap<String, f64>> {
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT prices FROM platform_speech WHERE id")
                .fetch_optional(self.reads())
                .await?;
        Ok(row
            .and_then(|(v,)| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    async fn put_speech_prices(&self, prices: &BTreeMap<String, f64>) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO platform_speech (id, prices, updated_at) VALUES (TRUE, $1, now())
             ON CONFLICT (id) DO UPDATE SET prices = EXCLUDED.prices, updated_at = now()",
        )
        .bind(serde_json::to_value(prices)?)
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn speech_minute_prices(&self) -> anyhow::Result<BTreeMap<String, f64>> {
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT minute_prices FROM platform_speech WHERE id")
                .fetch_optional(self.reads())
                .await?;
        Ok(row
            .and_then(|(v,)| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    async fn put_speech_minute_prices(&self, prices: &BTreeMap<String, f64>) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO platform_speech (id, minute_prices, updated_at) VALUES (TRUE, $1, now())
             ON CONFLICT (id) DO UPDATE SET minute_prices = EXCLUDED.minute_prices,
                                            updated_at = now()",
        )
        .bind(serde_json::to_value(prices)?)
        .execute(self.writes())
        .await?;
        Ok(())
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    price_per_million: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    price_per_minute: Option<f64>,
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
            price_per_million: settings.price_per_million,
            price_per_minute: settings.price_per_minute,
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
            price_per_million: a.price_per_million,
            price_per_minute: a.price_per_minute,
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
        let mut tx = self.writes().begin().await?;
        let done = sqlx::query("DELETE FROM speech_accounts WHERE tenant_id = $1 AND id = $2")
            .bind(tenant_id.to_string())
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        // Its caps go with it (docs/SPEECH_CAPS.md).
        sqlx::query("DELETE FROM speech_caps WHERE tenant_id = $1 AND provider = $2")
            .bind(tenant_id.to_string())
            .bind(format!("{}{id}", fcp_core::speech::ACCOUNT_PREFIX))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(done.rows_affected() > 0)
    }
}

#[async_trait]
impl SpeechUsageStore for PostgresStorage {
    async fn add_speech_usage(&self, usage: &[SpeechUsage]) -> anyhow::Result<()> {
        if usage.is_empty() {
            return Ok(());
        }
        let mut tx = self.writes().begin().await?;
        for u in usage {
            sqlx::query(
                "INSERT INTO speech_usage
                     (tenant_id, provider, hour, requests, characters, synthesized_characters,
                      recognized_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (tenant_id, hour, provider, site_id) DO UPDATE SET
                     requests = speech_usage.requests + EXCLUDED.requests,
                     characters = speech_usage.characters + EXCLUDED.characters,
                     synthesized_characters =
                         speech_usage.synthesized_characters + EXCLUDED.synthesized_characters,
                     recognized_ms = speech_usage.recognized_ms + EXCLUDED.recognized_ms",
            )
            .bind(u.tenant_id.to_string())
            .bind(&u.provider)
            .bind(system_time_to_datetime(u.hour))
            .bind(u.requests as i64)
            .bind(u.characters as i64)
            .bind(u.synthesized_characters as i64)
            .bind(u.recognized_ms.min(i64::MAX as u64) as i64)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn speech_usage(
        &self,
        tenant_id: &TenantId,
        from: std::time::SystemTime,
        to: std::time::SystemTime,
    ) -> anyhow::Result<Vec<SpeechUsage>> {
        let rows = sqlx::query(
            "SELECT provider, hour, requests, characters, synthesized_characters, recognized_ms,
                    site_id
               FROM speech_usage
              WHERE tenant_id = $1 AND hour >= $2 AND hour < $3
              ORDER BY hour, provider, site_id",
        )
        .bind(tenant_id.to_string())
        .bind(system_time_to_datetime(from))
        .bind(system_time_to_datetime(to))
        .fetch_all(self.reads())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(SpeechUsage {
                    tenant_id: *tenant_id,
                    provider: r.try_get("provider")?,
                    hour: datetime_to_system_time(r.try_get::<DateTime<Utc>, _>("hour")?),
                    requests: r.try_get::<i64, _>("requests")?.max(0) as u64,
                    characters: r.try_get::<i64, _>("characters")?.max(0) as u64,
                    synthesized_characters: r.try_get::<i64, _>("synthesized_characters")?.max(0)
                        as u64,
                    recognized_ms: r.try_get::<i64, _>("recognized_ms")?.max(0) as u64,
                    site_id: Some(r.try_get::<String, _>("site_id")?).filter(|s| !s.is_empty()),
                })
            })
            .collect()
    }

    async fn speech_usage_after(
        &self,
        after: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, SpeechUsage)>> {
        let rows = sqlx::query(
            "SELECT tenant_id, provider, hour, requests, characters, synthesized_characters,
                    recognized_ms, ship_seq
               FROM speech_usage
              WHERE site_id = '' AND ship_seq > $1
              ORDER BY ship_seq
              LIMIT $2",
        )
        .bind(after)
        .bind(limit.min(10_000) as i64)
        .fetch_all(self.writes())
        .await?;
        rows.iter()
            .map(|r| {
                Ok((
                    r.try_get::<i64, _>("ship_seq")?,
                    SpeechUsage {
                        tenant_id: r.try_get::<String, _>("tenant_id")?.parse()?,
                        provider: r.try_get("provider")?,
                        hour: datetime_to_system_time(r.try_get::<DateTime<Utc>, _>("hour")?),
                        requests: r.try_get::<i64, _>("requests")?.max(0) as u64,
                        characters: r.try_get::<i64, _>("characters")?.max(0) as u64,
                        synthesized_characters: r
                            .try_get::<i64, _>("synthesized_characters")?
                            .max(0) as u64,
                        recognized_ms: r.try_get::<i64, _>("recognized_ms")?.max(0) as u64,
                        site_id: None,
                    },
                ))
            })
            .collect()
    }

    async fn put_site_speech_usage(
        &self,
        site_id: &str,
        usage: &[SpeechUsage],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!site_id.is_empty(), "a site's usage names the site");
        let mut tx = self.writes().begin().await?;
        for u in usage {
            sqlx::query(
                "INSERT INTO speech_usage
                     (tenant_id, provider, hour, site_id, requests, characters, synthesized_characters,
                      recognized_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (tenant_id, hour, provider, site_id) DO UPDATE SET
                     requests = EXCLUDED.requests,
                     characters = EXCLUDED.characters,
                     synthesized_characters = EXCLUDED.synthesized_characters,
                     recognized_ms = EXCLUDED.recognized_ms",
            )
            .bind(u.tenant_id.to_string())
            .bind(&u.provider)
            .bind(system_time_to_datetime(u.hour))
            .bind(site_id)
            .bind(u.requests as i64)
            .bind(u.characters as i64)
            .bind(u.synthesized_characters as i64)
            .bind(u.recognized_ms.min(i64::MAX as u64) as i64)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn all_speech_caps(&self) -> anyhow::Result<Vec<SpeechCap>> {
        // At a site, the cloud's caps as the link carried them, as the
        // platform's on the tenant (docs/SPEECH_CAPS.md C3).
        let rows = sqlx::query(
            "SELECT tenant_id, provider, kind, unit, monthly_characters FROM speech_caps
             UNION ALL
             SELECT tenant_id, provider, 'platform', unit, cap FROM speech_cap_carried
              ORDER BY 1 NULLS FIRST, 2, 3, 4",
        )
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(cap_row).collect()
    }

    async fn speech_caps(&self, tenant_id: Option<&TenantId>) -> anyhow::Result<Vec<SpeechCap>> {
        let rows = sqlx::query(
            "SELECT tenant_id, provider, kind, unit, monthly_characters FROM speech_caps
              WHERE tenant_id IS NOT DISTINCT FROM $1
             UNION ALL
             SELECT tenant_id, provider, 'platform', unit, cap FROM speech_cap_carried
              WHERE tenant_id = $1
              ORDER BY 2, 3, 4",
        )
        .bind(tenant_id.map(|t| t.to_string()))
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(cap_row).collect()
    }

    async fn put_speech_cap(&self, cap: &SpeechCap) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO speech_caps (tenant_id, provider, kind, monthly_characters, unit)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT ((COALESCE(tenant_id, '')), provider, kind, unit) DO UPDATE SET
                 monthly_characters = EXCLUDED.monthly_characters,
                 updated_at = NOW()",
        )
        .bind(cap.tenant_id.map(|t| t.to_string()))
        .bind(&cap.provider)
        .bind(cap.kind.as_str())
        .bind(
            cap.monthly_characters
                .map(|n| n.min(i64::MAX as u64) as i64),
        )
        .bind(cap.unit.as_str())
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_speech_cap(
        &self,
        tenant_id: Option<&TenantId>,
        provider: &str,
        kind: SpeechCapKind,
        unit: CapUnit,
    ) -> anyhow::Result<bool> {
        let done = sqlx::query(
            "DELETE FROM speech_caps
              WHERE tenant_id IS NOT DISTINCT FROM $1 AND provider = $2 AND kind = $3
                AND unit = $4",
        )
        .bind(tenant_id.map(|t| t.to_string()))
        .bind(provider)
        .bind(kind.as_str())
        .bind(unit.as_str())
        .execute(self.writes())
        .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn note_speech_cap(
        &self,
        tenant_id: &TenantId,
        provider: &str,
        unit: CapUnit,
        month: std::time::SystemTime,
        level: CapLevel,
        cap: u64,
        used: u64,
    ) -> anyhow::Result<bool> {
        if level == CapLevel::Ok {
            return Ok(false);
        }
        let done = sqlx::query(
            "INSERT INTO speech_cap_notices (tenant_id, provider, month, level, cap, used, unit)
             VALUES ($1, $2, ($3::timestamptz AT TIME ZONE 'UTC')::date, $4, $5, $6, $7)
             ON CONFLICT DO NOTHING",
        )
        .bind(tenant_id.to_string())
        .bind(provider)
        .bind(system_time_to_datetime(month))
        .bind(level.as_str())
        .bind(cap.min(i64::MAX as u64) as i64)
        .bind(used.min(i64::MAX as u64) as i64)
        .bind(unit.as_str())
        .execute(self.writes())
        .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn used_by_source(
        &self,
        from: std::time::SystemTime,
        unit: CapUnit,
    ) -> anyhow::Result<Vec<(TenantId, String, String, u64)>> {
        let column = match unit {
            CapUnit::Characters => "synthesized_characters",
            CapUnit::Minutes => "recognized_ms",
        };
        let rows = sqlx::query(&format!(
            "SELECT tenant_id, provider, site_id, SUM({column})::BIGINT AS n
               FROM speech_usage
              WHERE hour >= $1 AND {column} > 0
              GROUP BY tenant_id, provider, site_id"
        ))
        .bind(system_time_to_datetime(from))
        .fetch_all(self.reads())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let Ok(tenant) = r.try_get::<String, _>("tenant_id")?.parse::<TenantId>() else {
                continue;
            };
            out.push((
                tenant,
                r.try_get("provider")?,
                r.try_get("site_id")?,
                r.try_get::<Option<i64>, _>("n")?.unwrap_or(0).max(0) as u64,
            ));
        }
        Ok(out)
    }

    async fn put_site_speech_cap(&self, cap: &SiteSpeechCap) -> anyhow::Result<()> {
        // Only a change is written, so the journal carries changes alone.
        sqlx::query(
            "INSERT INTO site_speech_caps (id, tenant_id, provider, month, cap, used, unit)
             VALUES ($1, $2, $3, ($4::timestamptz AT TIME ZONE 'UTC')::date, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET
                 month = EXCLUDED.month, cap = EXCLUDED.cap, used = EXCLUDED.used,
                 updated_at = NOW()
             WHERE (site_speech_caps.month, site_speech_caps.cap, site_speech_caps.used)
                 IS DISTINCT FROM (EXCLUDED.month, EXCLUDED.cap, EXCLUDED.used)",
        )
        .bind(cap.row_id())
        .bind(cap.tenant_id.to_string())
        .bind(&cap.provider)
        .bind(system_time_to_datetime(cap.month))
        .bind(cap.cap.min(i64::MAX as u64) as i64)
        .bind(serde_json::to_value(&cap.used)?)
        .bind(cap.unit.as_str())
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn site_speech_caps(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<SiteSpeechCap>> {
        let rows = sqlx::query(
            "SELECT tenant_id, provider, unit, month, cap, used FROM site_speech_caps
              WHERE tenant_id = $1 ORDER BY provider, unit",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.reads())
        .await?;
        rows.iter().map(Self::site_cap_row).collect()
    }

    async fn delete_site_speech_cap(
        &self,
        tenant_id: &TenantId,
        provider: &str,
        unit: CapUnit,
    ) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM site_speech_caps WHERE id = $1")
            .bind(SiteSpeechCap::row_id_of(tenant_id, provider, unit))
            .execute(self.writes())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn put_carried_speech_cap(
        &self,
        tenant_id: &TenantId,
        provider: &str,
        unit: CapUnit,
        month: std::time::SystemTime,
        cap: u64,
        used_elsewhere: u64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO speech_cap_carried (tenant_id, provider, month, cap, used_elsewhere, unit)
             VALUES ($1, $2, ($3::timestamptz AT TIME ZONE 'UTC')::date, $4, $5, $6)
             ON CONFLICT (tenant_id, provider, unit) DO UPDATE SET
                 month = EXCLUDED.month, cap = EXCLUDED.cap,
                 used_elsewhere = EXCLUDED.used_elsewhere, updated_at = NOW()",
        )
        .bind(tenant_id.to_string())
        .bind(provider)
        .bind(system_time_to_datetime(month))
        .bind(cap.min(i64::MAX as u64) as i64)
        .bind(used_elsewhere.min(i64::MAX as u64) as i64)
        .bind(unit.as_str())
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn delete_carried_speech_cap(
        &self,
        tenant_id: &TenantId,
        provider: &str,
        unit: CapUnit,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "DELETE FROM speech_cap_carried WHERE tenant_id = $1 AND provider = $2 AND unit = $3",
        )
        .bind(tenant_id.to_string())
        .bind(provider)
        .bind(unit.as_str())
        .execute(self.writes())
        .await?;
        Ok(())
    }

    async fn used_since(
        &self,
        from: std::time::SystemTime,
    ) -> anyhow::Result<Vec<(TenantId, String, CapUnit, u64)>> {
        // At a site, what the cloud says was used elsewhere this month.
        let rows = sqlx::query(
            "SELECT tenant_id, provider, unit, SUM(n)::BIGINT AS n FROM (
                 SELECT tenant_id, provider, 'characters' AS unit, synthesized_characters AS n
                   FROM speech_usage WHERE hour >= $1 AND synthesized_characters > 0
                 UNION ALL
                 SELECT tenant_id, provider, 'minutes', recognized_ms
                   FROM speech_usage WHERE hour >= $1 AND recognized_ms > 0
                 UNION ALL
                 SELECT tenant_id, provider, unit, used_elsewhere
                   FROM speech_cap_carried
                  WHERE month = ($1::timestamptz AT TIME ZONE 'UTC')::date
             ) u
              GROUP BY tenant_id, provider, unit",
        )
        .bind(system_time_to_datetime(from))
        .fetch_all(self.reads())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let tenant: String = r.try_get("tenant_id")?;
            let Ok(tenant) = tenant.parse::<TenantId>() else {
                continue;
            };
            let unit: String = r.try_get("unit")?;
            out.push((
                tenant,
                r.try_get("provider")?,
                CapUnit::parse(&unit).unwrap_or_default(),
                r.try_get::<Option<i64>, _>("n")?.unwrap_or(0).max(0) as u64,
            ));
        }
        Ok(out)
    }
}

impl PostgresStorage {
    fn site_cap_row(r: &sqlx::postgres::PgRow) -> anyhow::Result<SiteSpeechCap> {
        let month: chrono::NaiveDate = r.try_get("month")?;
        Ok(SiteSpeechCap {
            tenant_id: r.try_get::<String, _>("tenant_id")?.parse()?,
            provider: r.try_get("provider")?,
            unit: CapUnit::parse(&r.try_get::<String, _>("unit")?).unwrap_or_default(),
            month: datetime_to_system_time(
                month.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc(),
            ),
            cap: r.try_get::<i64, _>("cap")?.max(0) as u64,
            used: serde_json::from_value(r.try_get("used")?).unwrap_or_default(),
        })
    }
}

fn cap_row(r: &sqlx::postgres::PgRow) -> anyhow::Result<SpeechCap> {
    let tenant: Option<String> = r.try_get("tenant_id")?;
    let kind: String = r.try_get("kind")?;
    Ok(SpeechCap {
        tenant_id: tenant.map(|t| t.parse()).transpose()?,
        provider: r.try_get("provider")?,
        kind: SpeechCapKind::parse(&kind)
            .ok_or_else(|| anyhow::anyhow!("speech cap kind {kind:?}"))?,
        unit: CapUnit::parse(&r.try_get::<String, _>("unit")?).unwrap_or_default(),
        monthly_characters: r
            .try_get::<Option<i64>, _>("monthly_characters")?
            .map(|n| n.max(0) as u64),
    })
}
