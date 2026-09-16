//! PostgreSQL implementation of [`EmergencyStore`] (E911 configuration and
//! the emergency-call audit trail). See migration 050.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::e911::notify::{
    EmergencyNotice, EmergencyNotification, NotificationStatus, NotifyContact,
};
use fcp_core::e911::{
    DeclaredLocation, DispatchableLocation, EmergencyCallRecord, EmergencyCompletion,
    EmergencyConfig, EmergencyKind, EmergencyNetwork, EmergencyStore, NetworkApplies,
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

    fn row_to_notification(row: sqlx::postgres::PgRow) -> anyhow::Result<EmergencyNotification> {
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let delivered_at: Option<DateTime<Utc>> = row.try_get("delivered_at")?;
        let next_retry_at: Option<DateTime<Utc>> = row.try_get("next_retry_at")?;
        let contact: NotifyContact = serde_json::from_value(row.try_get("contact")?)?;
        let notice: EmergencyNotice = serde_json::from_value(row.try_get("notice")?)?;
        Ok(EmergencyNotification {
            id: row.try_get("id")?,
            tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
            call_record_id: row.try_get("call_record_id")?,
            call_id: row.try_get("call_id")?,
            contact,
            attempt: row.try_get("attempt")?,
            status: NotificationStatus::from_str(&row.try_get::<String, _>("status")?)
                .map_err(|e| anyhow::anyhow!(e))?,
            error: row.try_get("error")?,
            delivered_at: delivered_at.map(to_system_time),
            next_retry_at: next_retry_at.map(to_system_time),
            notice,
            created_at: to_system_time(created_at),
        })
    }

    fn row_to_network(row: sqlx::postgres::PgRow) -> anyhow::Result<EmergencyNetwork> {
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;
        Ok(EmergencyNetwork {
            id: fcp_core::EmergencyNetworkId::from_str(&row.try_get::<String, _>("id")?)?,
            tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
            cidr: row.try_get::<String, _>("cidr")?.parse()?,
            location_id: fcp_core::LocationId::from_str(&row.try_get::<String, _>("location_id")?)?,
            name: row.try_get("name")?,
            priority: row.try_get("priority")?,
            applies_to: row
                .try_get::<String, _>("applies_to")?
                .parse::<NetworkApplies>()
                .unwrap_or_default(),
            created_at: to_system_time(created_at),
            updated_at: to_system_time(updated_at),
        })
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
            sip_status: row
                .try_get::<Option<i32>, _>("sip_status")?
                .map(|s| s as u16),
            answered_at: row
                .try_get::<Option<DateTime<Utc>>, _>("answered_at")?
                .map(to_system_time),
            ended_at: row
                .try_get::<Option<DateTime<Utc>>, _>("ended_at")?
                .map(to_system_time),
            delivery: row.try_get("delivery")?,
            location_structured: row.try_get("location_structured")?,
            geolocation_error: row.try_get("geolocation_error")?,
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
            "SELECT tenant_id, emergency_numbers, test_numbers, dial_prefixes, contacts,
                    notify_contacts, require_dispatchable_location, updated_at,
                    trust_device_locations, declared_location_ttl_secs
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
            contacts: {
                let typed: Vec<NotifyContact> = row
                    .try_get::<serde_json::Value, _>("contacts")
                    .ok()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                if typed.is_empty() {
                    // A tenant whose contacts are still the free text of
                    // block 1 (migration 071 reads most of them; this
                    // reads the rest the same way, by shape).
                    row.try_get::<Vec<String>, _>("notify_contacts")
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|c| NotifyContact::parse(c))
                        .collect()
                } else {
                    typed
                }
            },
            require_dispatchable_location: row.try_get("require_dispatchable_location")?,
            trust_device_locations: row
                .try_get::<Vec<String>, _>("trust_device_locations")
                .unwrap_or_default()
                .iter()
                .filter_map(|t| serde_json::from_value(serde_json::Value::String(t.clone())).ok())
                .collect(),
            declared_location_ttl: std::time::Duration::from_secs(
                row.try_get::<i64, _>("declared_location_ttl_secs")
                    .unwrap_or(86_400)
                    .max(0) as u64,
            ),
            updated_at: to_system_time(updated_at),
        }))
    }

    async fn put_emergency_config(&self, config: &EmergencyConfig) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO emergency_configs
                (tenant_id, emergency_numbers, test_numbers, dial_prefixes, notify_contacts,
                 require_dispatchable_location, updated_at, trust_device_locations,
                 declared_location_ttl_secs, contacts)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (tenant_id) DO UPDATE SET
                emergency_numbers = EXCLUDED.emergency_numbers,
                test_numbers = EXCLUDED.test_numbers,
                dial_prefixes = EXCLUDED.dial_prefixes,
                notify_contacts = EXCLUDED.notify_contacts,
                contacts = EXCLUDED.contacts,
                require_dispatchable_location = EXCLUDED.require_dispatchable_location,
                trust_device_locations = EXCLUDED.trust_device_locations,
                declared_location_ttl_secs = EXCLUDED.declared_location_ttl_secs,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(config.tenant_id.to_string())
        .bind(&config.emergency_numbers)
        .bind(&config.test_numbers)
        .bind(&config.dial_prefixes)
        // The plain targets stay in notify_contacts for anything reading
        // the column that was there before block 2; `contacts` is the truth.
        .bind(config.contact_targets())
        .bind(config.require_dispatchable_location)
        .bind(to_datetime(config.updated_at))
        .bind(
            config
                .trust_device_locations
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>(),
        )
        .bind(config.declared_location_ttl.as_secs() as i64)
        .bind(serde_json::to_value(&config.contacts)?)
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
                 placed_at, notified, delivery, location_structured)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
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
        .bind(&record.delivery)
        .bind(record.location_structured)
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
                    placed_at, notified, sip_status, answered_at, ended_at, delivery,
                    location_structured, geolocation_error
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

    async fn list_emergency_networks(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Vec<EmergencyNetwork>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, cidr, location_id, name, priority, applies_to,
                    created_at, updated_at
             FROM emergency_networks WHERE tenant_id = $1
             ORDER BY priority, cidr",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_network).collect()
    }

    async fn get_emergency_network(
        &self,
        id: &fcp_core::EmergencyNetworkId,
    ) -> anyhow::Result<Option<EmergencyNetwork>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, cidr, location_id, name, priority, applies_to,
                    created_at, updated_at
             FROM emergency_networks WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::row_to_network).transpose()
    }

    async fn put_emergency_network(&self, network: &EmergencyNetwork) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO emergency_networks
                (id, tenant_id, cidr, location_id, name, priority, applies_to,
                 created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO UPDATE SET
                cidr = EXCLUDED.cidr,
                location_id = EXCLUDED.location_id,
                name = EXCLUDED.name,
                priority = EXCLUDED.priority,
                applies_to = EXCLUDED.applies_to,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(network.id.to_string())
        .bind(network.tenant_id.to_string())
        .bind(network.cidr.to_string())
        .bind(network.location_id.to_string())
        .bind(&network.name)
        .bind(network.priority)
        .bind(network.applies_to.to_string())
        .bind(to_datetime(network.created_at))
        .bind(to_datetime(network.updated_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_emergency_network(
        &self,
        id: &fcp_core::EmergencyNetworkId,
    ) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM emergency_networks WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_declared_location(
        &self,
        tenant_id: &TenantId,
        user_id: &fcp_core::UserId,
    ) -> anyhow::Result<Option<DeclaredLocation>> {
        let row = sqlx::query(
            "SELECT tenant_id, user_id, address, declared_at, expires_at
             FROM declared_locations WHERE tenant_id = $1 AND user_id = $2",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let declared_at: DateTime<Utc> = row.try_get("declared_at")?;
        let expires_at: DateTime<Utc> = row.try_get("expires_at")?;
        Ok(Some(DeclaredLocation {
            tenant_id: TenantId::from_str(&row.try_get::<String, _>("tenant_id")?)?,
            user_id: fcp_core::UserId::from_str(&row.try_get::<String, _>("user_id")?)?,
            address: serde_json::from_value(row.try_get("address")?)?,
            declared_at: to_system_time(declared_at),
            expires_at: to_system_time(expires_at),
        }))
    }

    async fn put_declared_location(&self, location: &DeclaredLocation) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO declared_locations
                (tenant_id, user_id, address, declared_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (tenant_id, user_id) DO UPDATE SET
                address = EXCLUDED.address,
                declared_at = EXCLUDED.declared_at,
                expires_at = EXCLUDED.expires_at",
        )
        .bind(location.tenant_id.to_string())
        .bind(location.user_id.to_string())
        .bind(serde_json::to_value(&location.address)?)
        .bind(to_datetime(location.declared_at))
        .bind(to_datetime(location.expires_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_declared_location(
        &self,
        tenant_id: &TenantId,
        user_id: &fcp_core::UserId,
    ) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM declared_locations WHERE tenant_id = $1 AND user_id = $2")
            .bind(tenant_id.to_string())
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn complete_emergency_call(
        &self,
        id: i64,
        completion: &EmergencyCompletion,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE emergency_calls
             SET trunk_name = COALESCE($2, trunk_name), outcome = $3, sip_status = $4,
                 answered_at = $5, ended_at = $6, delivery = $7,
                 geolocation_error = COALESCE($8, geolocation_error)
             WHERE id = $1",
        )
        .bind(id)
        .bind(&completion.trunk_name)
        .bind(&completion.outcome)
        .bind(completion.sip_status.map(i32::from))
        .bind(completion.answered_at.map(to_datetime))
        .bind(to_datetime(completion.ended_at))
        .bind(&completion.delivery)
        .bind(&completion.geolocation_error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── On-site notification (§8) ─────────────────────────────────────

    async fn get_emergency_call_by_call_id(
        &self,
        tenant_id: &TenantId,
        call_id: &str,
    ) -> anyhow::Result<Option<EmergencyCallRecord>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, call_id, sip_call_id, caller_uri, caller_username,
                    dialed_number, kind, callback_number, location, trunk_name, outcome,
                    placed_at, notified, sip_status, answered_at, ended_at, delivery,
                    location_structured, geolocation_error
             FROM emergency_calls WHERE tenant_id = $1 AND call_id = $2
             ORDER BY id DESC LIMIT 1",
        )
        .bind(tenant_id.to_string())
        .bind(call_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::row_to_record).transpose()
    }

    async fn insert_emergency_notification(
        &self,
        notification: &EmergencyNotification,
    ) -> anyhow::Result<i64> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO emergency_notifications
                (tenant_id, call_record_id, call_id, kind, target, contact, attempt,
                 status, error, delivered_at, next_retry_at, test, notice, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             RETURNING id",
        )
        .bind(notification.tenant_id.to_string())
        .bind(notification.call_record_id)
        .bind(&notification.call_id)
        .bind(notification.contact.kind.to_string())
        .bind(notification.contact.display_target())
        .bind(serde_json::to_value(&notification.contact)?)
        .bind(notification.attempt)
        .bind(notification.status.to_string())
        .bind(&notification.error)
        .bind(notification.delivered_at.map(to_datetime))
        .bind(notification.next_retry_at.map(to_datetime))
        .bind(notification.is_test())
        .bind(serde_json::to_value(&notification.notice)?)
        .bind(to_datetime(notification.created_at))
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    async fn list_emergency_notifications(
        &self,
        tenant_id: &TenantId,
        call_id: &str,
    ) -> anyhow::Result<Vec<EmergencyNotification>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, call_record_id, call_id, kind, target, contact, attempt,
                    status, error, delivered_at, next_retry_at, test, notice, created_at
             FROM emergency_notifications
             WHERE tenant_id = $1 AND call_id = $2
             ORDER BY created_at, id",
        )
        .bind(tenant_id.to_string())
        .bind(call_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_notification).collect()
    }

    async fn due_emergency_notifications(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<EmergencyNotification>> {
        // Claim what is due in one statement, so two notifiers never take
        // the same attempt.
        let rows = sqlx::query(
            "UPDATE emergency_notifications SET next_retry_at = NULL
             WHERE id IN (
                 SELECT id FROM emergency_notifications
                 WHERE next_retry_at IS NOT NULL AND next_retry_at <= NOW()
                 ORDER BY next_retry_at
                 LIMIT $1
                 FOR UPDATE SKIP LOCKED
             )
             RETURNING id, tenant_id, call_record_id, call_id, kind, target, contact, attempt,
                    status, error, delivered_at, next_retry_at, test, notice, created_at",
        )
        .bind(limit.clamp(1, 1000) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_notification).collect()
    }

    async fn clear_notification_retry(&self, id: i64) -> anyhow::Result<()> {
        sqlx::query("UPDATE emergency_notifications SET next_retry_at = NULL WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn last_emergency_notifications(
        &self,
        tenant_id: &TenantId,
        test_only: bool,
    ) -> anyhow::Result<Vec<EmergencyNotification>> {
        let rows = sqlx::query(
            "SELECT DISTINCT ON (kind, target) id, tenant_id, call_record_id, call_id, kind, target, contact, attempt,
                    status, error, delivered_at, next_retry_at, test, notice, created_at
             FROM emergency_notifications
             WHERE tenant_id = $1 AND delivered_at IS NOT NULL AND ($2 = FALSE OR test)
             ORDER BY kind, target, delivered_at DESC",
        )
        .bind(tenant_id.to_string())
        .bind(test_only)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_notification).collect()
    }
}
