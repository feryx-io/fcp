//! Redis-backed call parking store for cross-node park/pickup.
//!
//! Key design (`docs/PARKING.md` §2):
//! ```text
//! Key:    {prefix}park:{tenant}:{slot}
//! Value:  JSON { call_id, node_id, parker_extension, tenant_id, slot, lot_id, parked_at }
//! TTL:    the lot's timeout and a margin, set when the slot is claimed
//! ```
//!
//! A slot is claimed with `SET NX`: one call holds it, whichever node
//! parked it. Keyed by orbit alone, two tenants shared every slot and two
//! nodes parking at once both wrote the same key.

use crate::RedisStorage;
use anyhow::Result;
use async_trait::async_trait;
use fcp_core::storage::{ClusterParkedCall, ClusterParkingStore};
use fcp_core::TenantId;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// A parked call entry stored in Redis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisParkedCall {
    /// Internal call ID of the parked call.
    pub call_id: String,
    /// Node that owns/parked this call.
    pub node_id: String,
    /// Extension of the user who parked the call.
    pub parker_extension: String,
    /// Tenant ID.
    pub tenant_id: String,
    #[serde(default)]
    pub slot: String,
    #[serde(default)]
    pub lot_id: String,
    /// Unix timestamp when the call was parked.
    pub parked_at: u64,
}

/// Redis-backed parking store.
#[derive(Clone)]
pub struct RedisParkingStore {
    storage: RedisStorage,
}

impl RedisParkingStore {
    /// Create a new parking store.
    pub fn new(storage: RedisStorage) -> Self {
        Self { storage }
    }

    fn park_key(&self, tenant_id: &TenantId, slot: &str) -> String {
        self.storage.key(&format!("park:{tenant_id}:{slot}"))
    }

    /// Claim the slot for `parked_call` unless a call holds it; whether it
    /// was claimed.
    pub async fn claim(
        &self,
        tenant_id: &TenantId,
        slot: &str,
        parked_call: &RedisParkedCall,
        ttl_secs: u64,
    ) -> Result<bool> {
        let key = self.park_key(tenant_id, slot);
        let mut conn = self.storage.connection();
        let json = serde_json::to_string(parked_call)?;
        let set: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg(&json)
            .arg("NX")
            .arg("EX")
            .arg(ttl_secs.max(1))
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("Redis park SET NX failed: {e}"))?;
        let claimed = set.is_some();
        debug!(slot, call_id = %parked_call.call_id, claimed, ttl_secs, "Claimed parking slot in Redis");
        Ok(claimed)
    }

    /// Retrieve (peek) a parked call without removing it.
    pub async fn get(&self, tenant_id: &TenantId, slot: &str) -> Result<Option<RedisParkedCall>> {
        let key = self.park_key(tenant_id, slot);
        let mut conn = self.storage.connection();
        let json: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| anyhow::anyhow!("Redis park GET failed: {}", e))?;
        json.map(|j| serde_json::from_str(&j).map_err(Into::into))
            .transpose()
    }

    /// The tenant's parked calls, by slot: what a site's link reports to
    /// the cloud (`docs/PARKING.md` phase 4).
    pub async fn list(&self, tenant_id: &TenantId) -> Result<Vec<RedisParkedCall>> {
        let mut conn = self.storage.connection();
        let pattern = self.storage.key(&format!("park:{tenant_id}:*"));
        let mut found = Vec::new();
        let mut cursor = 0u64;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await
                .map_err(|e| anyhow::anyhow!("Redis SCAN failed: {e}"))?;
            for key in keys {
                let json: Option<String> = conn
                    .get(&key)
                    .await
                    .map_err(|e| anyhow::anyhow!("Redis park GET failed: {e}"))?;
                if let Some(parked) = json.and_then(|j| serde_json::from_str(&j).ok()) {
                    found.push(parked);
                }
            }
            if next == 0 {
                break;
            }
            cursor = next;
        }
        found.sort_by(|a: &RedisParkedCall, b: &RedisParkedCall| {
            (a.slot.len(), &a.slot).cmp(&(b.slot.len(), &b.slot))
        });
        Ok(found)
    }

    /// Unpark (remove) a call from a slot. Returns the parked call if it existed.
    ///
    /// Uses a Lua script for true atomicity — only one caller can retrieve the
    /// value even when multiple nodes race to unpark the same slot.
    pub async fn unpark(
        &self,
        tenant_id: &TenantId,
        slot: &str,
    ) -> Result<Option<RedisParkedCall>> {
        let key = self.park_key(tenant_id, slot);
        let mut conn = self.storage.connection();
        let script = redis::Script::new(
            r#"
            local val = redis.call('GET', KEYS[1])
            if val then
                redis.call('DEL', KEYS[1])
            end
            return val
            "#,
        );
        let json: Option<String> = script
            .key(&key)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("Redis unpark failed: {}", e))?;
        match json {
            Some(j) => {
                let parked: RedisParkedCall = serde_json::from_str(&j)?;
                debug!(slot, call_id = %parked.call_id, "Unparked call from Redis");
                Ok(Some(parked))
            }
            None => Ok(None),
        }
    }
}

impl RedisParkedCall {
    /// Convert from the fcp-core cluster type.
    pub fn from_cluster(info: &ClusterParkedCall) -> Self {
        Self {
            call_id: info.call_id.clone(),
            node_id: info.node_id.clone(),
            parker_extension: info.parker_extension.clone(),
            tenant_id: info.tenant_id.clone(),
            slot: info.slot.clone(),
            lot_id: info.lot_id.clone(),
            parked_at: info.parked_at_unix,
        }
    }

    /// Convert to the fcp-core cluster type.
    pub fn to_cluster(&self) -> ClusterParkedCall {
        ClusterParkedCall {
            call_id: self.call_id.clone(),
            node_id: self.node_id.clone(),
            parker_extension: self.parker_extension.clone(),
            tenant_id: self.tenant_id.clone(),
            slot: self.slot.clone(),
            lot_id: self.lot_id.clone(),
            parked_at_unix: self.parked_at,
        }
    }
}

#[async_trait]
impl ClusterParkingStore for RedisParkingStore {
    async fn cluster_park(
        &self,
        tenant_id: &TenantId,
        slot: &str,
        info: ClusterParkedCall,
        ttl_secs: u64,
    ) -> Result<bool> {
        self.claim(
            tenant_id,
            slot,
            &RedisParkedCall::from_cluster(&info),
            ttl_secs,
        )
        .await
    }

    async fn cluster_lookup(
        &self,
        tenant_id: &TenantId,
        slot: &str,
    ) -> Result<Option<ClusterParkedCall>> {
        Ok(self.get(tenant_id, slot).await?.map(|r| r.to_cluster()))
    }

    async fn cluster_unpark(
        &self,
        tenant_id: &TenantId,
        slot: &str,
    ) -> Result<Option<ClusterParkedCall>> {
        Ok(self.unpark(tenant_id, slot).await?.map(|r| r.to_cluster()))
    }
}
