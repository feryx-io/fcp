//! PostgreSQL implementation of [`EmergencyStore`] (E911 configuration and
//! the emergency-call audit trail). See migration 050.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::e911::{
    DispatchableLocation, EmergencyCallRecord, EmergencyConfig, EmergencyKind, EmergencyStore,
};
use fcp_core::TenantId;
use sqlx::{Pool, Postgres, Row};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn to_datetime(st: SystemTime) -> DateTime<Utc> {
    let d = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos()).unwrap_or_default()
}

fn to_system_time(dt: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(dt.timestamp().max(0) as u64)
}

#[derive(Clone)]
pub struct PostgresEmergencyStore {
    pool: Pool<Postgres>,
}

impl PostgresEmergencyStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    fn row_to_record(row: sqlx::postgres::PgRow) -> anyhow::Result<EmergencyCallRecord> {
        let tenant: String = row.try_get("tenant_id")?;
        let kind: String = row.try_get("kind")?;
        let location: Option<serde_json::Value> = row.try_get("location")?;
        let placed_at: DateTime<Utc> = row.try_get("placed_at")?;
        Ok(EmergencyCallRecord {
            id: row.try_get("id")?,
            tenant_id: TenantId::from_str(&tenant)?,
            call_id: row.try_get("call_id")?,
            sip_call_id: row.try_get("sip_call_id")?,
            caller_uri: row.try_get("caller_uri")?,
            caller_username: row.try_get("caller_username")?,
            dialed_number: row.try_get("dialed_number")?,
            kind: if kind == "test" {
                EmergencyKind::Test
            } else {
                EmergencyKind::Live
            },
            callback_number: row.try_get("callback_number")?,
            location: location
                .map(serde_json::from_value::<DispatchableLocation>)
                .transpose()?,
            trunk_name: row.try_get("trunk_name")?,
            outcome: row.try_get("outcome")?,
            placed_at: to_system_time(placed_at),
            notified: row.try_get("notified")?,
        })
    }
}

#[async_trait]
impl EmergencyStore for PostgresEmergencyStore {
    async fn get_emergency_config(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Option<EmergencyConfig>> {
        let row = sqlx::query(
            "SELECT tenant_id, emergency_numbers, test_numbers, dial_prefixes, notify_contacts,
                    require_dispatchable_location, updated_at
             FROM emergency_configs WHERE tenant_id = $1",
        )
        .bind(tenant_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;
        Ok(Some(EmergencyConfig {
            tenant_id: *tenant_id,
            emergency_numbers: row.try_get("emergency_numbers")?,
            test_numbers: row.try_get("test_numbers")?,
            dial_prefixes: row.try_get("dial_prefixes")?,
            notify_contacts: row.try_get("notify_contacts")?,
            require_dispatchable_location: row.try_get("require_dispatchable_location")?,
            updated_at: to_system_time(updated_at),
        }))
    }

    async fn put_emergency_config(&self, config: &EmergencyConfig) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO emergency_configs
                (tenant_id, emergency_numbers, test_numbers, dial_prefixes, notify_contacts,
                 require_dispatchable_location, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (tenant_id) DO UPDATE SET
                emergency_numbers = EXCLUDED.emergency_numbers,
                test_numbers = EXCLUDED.test_numbers,
                dial_prefixes = EXCLUDED.dial_prefixes,
                notify_contacts = EXCLUDED.notify_contacts,
                require_dispatchable_location = EXCLUDED.require_dispatchable_location,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(config.tenant_id.to_string())
        .bind(&config.emergency_numbers)
        .bind(&config.test_numbers)
        .bind(&config.dial_prefixes)
        .bind(&config.notify_contacts)
        .bind(config.require_dispatchable_location)
        .bind(to_datetime(config.updated_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert_emergency_call(&self, record: &EmergencyCallRecord) -> anyhow::Result<i64> {
        let location = record
            .location
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO emergency_calls
                (tenant_id, call_id, sip_call_id, caller_uri, caller_username, dialed_number,
                 kind, callback_number, location, location_source, trunk_name, outcome,
                 placed_at, notified)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             RETURNING id",
        )
        .bind(record.tenant_id.to_string())
        .bind(&record.call_id)
        .bind(&record.sip_call_id)
        .bind(&record.caller_uri)
        .bind(&record.caller_username)
        .bind(&record.dialed_number)
        .bind(record.kind.to_string())
        .bind(&record.callback_number)
        .bind(location)
        .bind(record.location.as_ref().map(|l| l.source.to_string()))
        .bind(&record.trunk_name)
        .bind(&record.outcome)
        .bind(to_datetime(record.placed_at))
        .bind(record.notified)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    async fn list_emergency_calls(
        &self,
        tenant_id: &TenantId,
        limit: usize,
    ) -> anyhow::Result<Vec<EmergencyCallRecord>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, call_id, sip_call_id, caller_uri, caller_username,
                    dialed_number, kind, callback_number, location, trunk_name, outcome,
                    placed_at, notified
             FROM emergency_calls WHERE tenant_id = $1
             ORDER BY placed_at DESC, id DESC LIMIT $2",
        )
        .bind(tenant_id.to_string())
        .bind(limit.clamp(1, 1000) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_record).collect()
    }

    async fn mark_emergency_call_notified(&self, id: i64) -> anyhow::Result<()> {
        sqlx::query("UPDATE emergency_calls SET notified = TRUE WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
