//! Redis-backed implementation of the PresenceStore trait.
//!
//! Uses Redis hashes, sets, and sorted sets for presence state storage,
//! enabling multi-node presence clustering.

use anyhow::{Context, Result};
use async_trait::async_trait;
use redis::AsyncCommands;
use tracing::{debug, warn};

use fcp_core::storage::{PresenceStore, StoredAuthPolicy, StoredPresentity, StoredSubscription};
use fcp_core::TenantId;

/// Redis-backed presence store for multi-node deployments.
///
/// Key patterns:
/// - `presence:sub:{subscription_id}` — Hash with subscription fields
/// - `presence:subs_by_aor:{tenant_id}:{aor}` — Set of subscription IDs for a presentity
/// - `presence:subs_by_watcher:{aor}` — Set of subscription IDs for a watcher
/// - `presence:subs_by_tenant:{tenant_id}` — Set of subscription IDs for a tenant
/// - `presence:sub_expiry` — Sorted set (score=epoch) for expiry index
/// - `presence:presentity:{tenant_id}:{aor}` — Hash with presentity fields
/// - `presence:presentities_by_tenant:{tenant_id}` — Set of presentity AORs
/// - `presence:auth:{tenant_id}:{aor}` — Hash with auth policy fields
/// - `presence:auth_by_tenant:{tenant_id}` — Set of AORs with auth policies
pub struct RedisPresenceStore {
    conn: redis::aio::ConnectionManager,
}

impl RedisPresenceStore {
    /// Create a new RedisPresenceStore from a Redis connection URL.
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url).context("Failed to create Redis client")?;
        let conn = client
            .get_connection_manager()
            .await
            .context("Failed to connect to Redis")?;
        Ok(Self { conn })
    }

    /// Create from an existing connection manager.
    pub fn from_connection_manager(conn: redis::aio::ConnectionManager) -> Self {
        Self { conn }
    }

    fn sub_key(id: &str) -> String {
        format!("presence:sub:{id}")
    }

    fn subs_by_aor_key(tenant_id: &TenantId, aor: &str) -> String {
        format!("presence:subs_by_aor:{tenant_id}:{aor}")
    }

    fn subs_by_watcher_key(aor: &str) -> String {
        format!("presence:subs_by_watcher:{aor}")
    }

    fn subs_by_tenant_key(tenant_id: &TenantId) -> String {
        format!("presence:subs_by_tenant:{tenant_id}")
    }

    fn presentity_key(tenant_id: &TenantId, aor: &str) -> String {
        format!("presence:presentity:{tenant_id}:{aor}")
    }

    fn presentities_by_tenant_key(tenant_id: &TenantId) -> String {
        format!("presence:presentities_by_tenant:{tenant_id}")
    }

    fn auth_key(tenant_id: &TenantId, aor: &str) -> String {
        format!("presence:auth:{tenant_id}:{aor}")
    }

    fn auth_by_tenant_key(tenant_id: &TenantId) -> String {
        format!("presence:auth_by_tenant:{tenant_id}")
    }

    const EXPIRY_KEY: &'static str = "presence:sub_expiry";
}

#[async_trait]
impl PresenceStore for RedisPresenceStore {
    async fn save_subscription(&self, sub: StoredSubscription) -> Result<()> {
        let mut conn = self.conn.clone();
        let key = Self::sub_key(&sub.subscription_id);

        let fields: Vec<(String, String)> = vec![
            ("subscription_id".into(), sub.subscription_id.clone()),
            ("watcher_aor".into(), sub.watcher_aor.clone()),
            ("presentity_aor".into(), sub.presentity_aor.clone()),
            ("tenant_id".into(), sub.tenant_id.to_string()),
            ("contact_uri".into(), sub.contact_uri.clone()),
            ("event_package".into(), sub.event_package.clone()),
            ("expires_at_epoch".into(), sub.expires_at_epoch.to_string()),
            (
                "sip_call_id".into(),
                sub.sip_call_id.clone().unwrap_or_default(),
            ),
            (
                "sip_from_tag".into(),
                sub.sip_from_tag.clone().unwrap_or_default(),
            ),
            (
                "sip_to_tag".into(),
                sub.sip_to_tag.clone().unwrap_or_default(),
            ),
        ];

        // Pipeline: HSET + SADD (by_aor, by_watcher, by_tenant) + ZADD (expiry)
        redis::pipe()
            .atomic()
            .cmd("HSET")
            .arg(&key)
            .arg(fields)
            .ignore()
            .cmd("SADD")
            .arg(Self::subs_by_aor_key(&sub.tenant_id, &sub.presentity_aor))
            .arg(&sub.subscription_id)
            .ignore()
            .cmd("SADD")
            .arg(Self::subs_by_watcher_key(&sub.watcher_aor))
            .arg(&sub.subscription_id)
            .ignore()
            .cmd("SADD")
            .arg(Self::subs_by_tenant_key(&sub.tenant_id))
            .arg(&sub.subscription_id)
            .ignore()
            .cmd("ZADD")
            .arg(Self::EXPIRY_KEY)
            .arg(sub.expires_at_epoch as f64)
            .arg(&sub.subscription_id)
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .context("Failed to save subscription to Redis")?;

        debug!(subscription_id = %sub.subscription_id, "Saved subscription to Redis");
        Ok(())
    }

    async fn load_subscriptions(&self, tenant_id: &TenantId) -> Result<Vec<StoredSubscription>> {
        let mut conn = self.conn.clone();

        let sub_ids: Vec<String> = conn
            .smembers(Self::subs_by_tenant_key(tenant_id))
            .await
            .context("Failed to get tenant subscription IDs")?;

        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut subs = Vec::with_capacity(sub_ids.len());
        for id in &sub_ids {
            let fields: std::collections::HashMap<String, String> =
                conn.hgetall(Self::sub_key(id)).await.unwrap_or_default();

            if fields.is_empty() {
                continue;
            }

            let expires: u64 = fields
                .get("expires_at_epoch")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);

            if expires <= now_epoch {
                continue;
            }

            subs.push(stored_sub_from_fields(&fields));
        }

        Ok(subs)
    }

    async fn remove_subscription(&self, subscription_id: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let key = Self::sub_key(subscription_id);

        // Read the sub first to get AOR/watcher/tenant for index cleanup
        let fields: std::collections::HashMap<String, String> =
            conn.hgetall(&key).await.unwrap_or_default();

        if fields.is_empty() {
            return Ok(());
        }

        let presentity_aor = fields.get("presentity_aor").cloned().unwrap_or_default();
        let watcher_aor = fields.get("watcher_aor").cloned().unwrap_or_default();
        let tenant_str = fields.get("tenant_id").cloned().unwrap_or_default();
        let tenant_id: TenantId = tenant_str.parse().unwrap_or_else(|_| TenantId::nil());

        redis::pipe()
            .atomic()
            .cmd("DEL")
            .arg(&key)
            .ignore()
            .cmd("SREM")
            .arg(Self::subs_by_aor_key(&tenant_id, &presentity_aor))
            .arg(subscription_id)
            .ignore()
            .cmd("SREM")
            .arg(Self::subs_by_watcher_key(&watcher_aor))
            .arg(subscription_id)
            .ignore()
            .cmd("SREM")
            .arg(Self::subs_by_tenant_key(&tenant_id))
            .arg(subscription_id)
            .ignore()
            .cmd("ZREM")
            .arg(Self::EXPIRY_KEY)
            .arg(subscription_id)
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .context("Failed to remove subscription from Redis")?;

        debug!(subscription_id = %subscription_id, "Removed subscription from Redis");
        Ok(())
    }

    async fn remove_subscriptions_for_presentity(
        &self,
        tenant_id: &TenantId,
        presentity_aor: &str,
    ) -> Result<()> {
        let mut conn = self.conn.clone();

        let sub_ids: Vec<String> = conn
            .smembers(Self::subs_by_aor_key(tenant_id, presentity_aor))
            .await
            .unwrap_or_default();

        for id in &sub_ids {
            if let Err(e) = self.remove_subscription(id).await {
                warn!(subscription_id = %id, error = %e, "Failed to remove subscription");
            }
        }

        Ok(())
    }

    async fn cleanup_expired_subscriptions(&self) -> Result<usize> {
        let mut conn = self.conn.clone();

        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Get all expired subscription IDs from the sorted set
        let expired_ids: Vec<String> = conn
            .zrangebyscore(Self::EXPIRY_KEY, 0f64, now_epoch as f64)
            .await
            .unwrap_or_default();

        let count = expired_ids.len();
        for id in &expired_ids {
            if let Err(e) = self.remove_subscription(id).await {
                warn!(subscription_id = %id, error = %e, "Failed to remove expired subscription");
            }
        }

        if count > 0 {
            debug!(count, "Cleaned expired subscriptions from Redis");
        }

        Ok(count)
    }

    async fn save_presentity(&self, presentity: StoredPresentity) -> Result<()> {
        let mut conn = self.conn.clone();
        let key = Self::presentity_key(&presentity.tenant_id, &presentity.aor);

        let fields: Vec<(String, String)> = vec![
            ("aor".into(), presentity.aor.clone()),
            ("state".into(), presentity.state.clone()),
            ("note".into(), presentity.note.clone().unwrap_or_default()),
            ("tenant_id".into(), presentity.tenant_id.to_string()),
            (
                "updated_at_epoch".into(),
                presentity.updated_at_epoch.to_string(),
            ),
        ];

        redis::pipe()
            .atomic()
            .cmd("HSET")
            .arg(&key)
            .arg(fields)
            .ignore()
            .cmd("SADD")
            .arg(Self::presentities_by_tenant_key(&presentity.tenant_id))
            .arg(&presentity.aor)
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .context("Failed to save presentity to Redis")?;

        Ok(())
    }

    async fn load_presentities(&self, tenant_id: &TenantId) -> Result<Vec<StoredPresentity>> {
        let mut conn = self.conn.clone();

        let aors: Vec<String> = conn
            .smembers(Self::presentities_by_tenant_key(tenant_id))
            .await
            .unwrap_or_default();

        let mut presentities = Vec::with_capacity(aors.len());
        for aor in &aors {
            let fields: std::collections::HashMap<String, String> = conn
                .hgetall(Self::presentity_key(tenant_id, aor))
                .await
                .unwrap_or_default();

            if fields.is_empty() {
                continue;
            }

            presentities.push(stored_presentity_from_fields(&fields));
        }

        Ok(presentities)
    }

    async fn remove_presentity(&self, tenant_id: &TenantId, aor: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let key = Self::presentity_key(tenant_id, aor);

        let mut pipe = redis::pipe();
        pipe.atomic()
            .cmd("DEL")
            .arg(&key)
            .ignore()
            .cmd("SREM")
            .arg(Self::presentities_by_tenant_key(tenant_id))
            .arg(aor)
            .ignore();

        pipe.query_async::<()>(&mut conn)
            .await
            .context("Failed to remove presentity from Redis")?;

        Ok(())
    }

    async fn save_auth_policy(&self, policy: StoredAuthPolicy) -> Result<()> {
        let mut conn = self.conn.clone();
        let key = Self::auth_key(&policy.tenant_id, &policy.aor);

        let fields: Vec<(String, String)> = vec![
            ("aor".into(), policy.aor.clone()),
            ("default_allow".into(), policy.default_allow.to_string()),
            ("allow_list_json".into(), policy.allow_list_json.clone()),
            ("block_list_json".into(), policy.block_list_json.clone()),
            ("tenant_id".into(), policy.tenant_id.to_string()),
            ("event_allow_json".into(), policy.event_allow_json.clone()),
            ("event_block_json".into(), policy.event_block_json.clone()),
            ("time_windows_json".into(), policy.time_windows_json.clone()),
            (
                "allow_patterns_json".into(),
                policy.allow_patterns_json.clone(),
            ),
            (
                "block_patterns_json".into(),
                policy.block_patterns_json.clone(),
            ),
        ];

        redis::pipe()
            .atomic()
            .cmd("HSET")
            .arg(&key)
            .arg(fields)
            .ignore()
            .cmd("SADD")
            .arg(Self::auth_by_tenant_key(&policy.tenant_id))
            .arg(&policy.aor)
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .context("Failed to save auth policy to Redis")?;

        Ok(())
    }

    async fn load_auth_policies(&self, tenant_id: &TenantId) -> Result<Vec<StoredAuthPolicy>> {
        let mut conn = self.conn.clone();

        let aors: Vec<String> = conn
            .smembers(Self::auth_by_tenant_key(tenant_id))
            .await
            .unwrap_or_default();

        let mut policies = Vec::with_capacity(aors.len());
        for aor in &aors {
            let fields: std::collections::HashMap<String, String> = conn
                .hgetall(Self::auth_key(tenant_id, aor))
                .await
                .unwrap_or_default();

            if fields.is_empty() {
                continue;
            }

            policies.push(stored_auth_from_fields(&fields));
        }

        Ok(policies)
    }

    async fn remove_auth_policy(&self, tenant_id: &TenantId, aor: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let key = Self::auth_key(tenant_id, aor);

        let mut pipe = redis::pipe();
        pipe.atomic()
            .cmd("DEL")
            .arg(&key)
            .ignore()
            .cmd("SREM")
            .arg(Self::auth_by_tenant_key(tenant_id))
            .arg(aor)
            .ignore();

        pipe.query_async::<()>(&mut conn)
            .await
            .context("Failed to remove auth policy from Redis")?;

        Ok(())
    }
}

fn stored_sub_from_fields(
    fields: &std::collections::HashMap<String, String>,
) -> StoredSubscription {
    let empty = String::new();
    StoredSubscription {
        subscription_id: fields.get("subscription_id").unwrap_or(&empty).clone(),
        watcher_aor: fields.get("watcher_aor").unwrap_or(&empty).clone(),
        presentity_aor: fields.get("presentity_aor").unwrap_or(&empty).clone(),
        tenant_id: fields
            .get("tenant_id")
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(TenantId::nil),
        contact_uri: fields.get("contact_uri").unwrap_or(&empty).clone(),
        event_package: fields.get("event_package").unwrap_or(&empty).clone(),
        expires_at_epoch: fields
            .get("expires_at_epoch")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        sip_call_id: fields.get("sip_call_id").filter(|v| !v.is_empty()).cloned(),
        sip_from_tag: fields
            .get("sip_from_tag")
            .filter(|v| !v.is_empty())
            .cloned(),
        sip_to_tag: fields.get("sip_to_tag").filter(|v| !v.is_empty()).cloned(),
    }
}

fn stored_presentity_from_fields(
    fields: &std::collections::HashMap<String, String>,
) -> StoredPresentity {
    let empty = String::new();
    StoredPresentity {
        aor: fields.get("aor").unwrap_or(&empty).clone(),
        state: fields.get("state").unwrap_or(&empty).clone(),
        note: fields.get("note").filter(|v| !v.is_empty()).cloned(),
        tenant_id: fields
            .get("tenant_id")
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(TenantId::nil),
        updated_at_epoch: fields
            .get("updated_at_epoch")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    }
}

fn stored_auth_from_fields(fields: &std::collections::HashMap<String, String>) -> StoredAuthPolicy {
    let empty = String::new();
    StoredAuthPolicy {
        aor: fields.get("aor").unwrap_or(&empty).clone(),
        default_allow: fields
            .get("default_allow")
            .map(|v| v == "true")
            .unwrap_or(false),
        allow_list_json: fields.get("allow_list_json").unwrap_or(&empty).clone(),
        block_list_json: fields.get("block_list_json").unwrap_or(&empty).clone(),
        tenant_id: fields
            .get("tenant_id")
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(TenantId::nil),
        event_allow_json: fields.get("event_allow_json").unwrap_or(&empty).clone(),
        event_block_json: fields.get("event_block_json").unwrap_or(&empty).clone(),
        time_windows_json: fields.get("time_windows_json").unwrap_or(&empty).clone(),
        allow_patterns_json: fields.get("allow_patterns_json").unwrap_or(&empty).clone(),
        block_patterns_json: fields.get("block_patterns_json").unwrap_or(&empty).clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_patterns() {
        let tenant_id = TenantId::nil();
        assert_eq!(
            RedisPresenceStore::sub_key("sub-123"),
            "presence:sub:sub-123"
        );
        assert_eq!(
            RedisPresenceStore::subs_by_aor_key(&tenant_id, "alice@example.com"),
            format!("presence:subs_by_aor:{tenant_id}:alice@example.com")
        );
        assert_eq!(
            RedisPresenceStore::presentity_key(&tenant_id, "alice@example.com"),
            format!("presence:presentity:{tenant_id}:alice@example.com")
        );
    }
}
