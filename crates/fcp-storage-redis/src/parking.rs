//! Redis-backed call parking store for cross-node park/pickup.
//!
//! Key design:
//! ```text
//! Key:    {prefix}park:{orbit}
//! Type:   Hash { call_id, node_id, parker_extension, tenant_id, parked_at }
//! TTL:    parking timeout (default 120s)
//! ```

use crate::RedisStorage;
use anyhow::Result;
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
    /// Unix timestamp when the call was parked.
    pub parked_at: u64,
}

/// Redis-backed parking store.
#[derive(Clone)]
pub struct RedisParkingStore {
    storage: RedisStorage,
    default_timeout_secs: u64,
}

impl RedisParkingStore {
    /// Create a new parking store.
    pub fn new(storage: RedisStorage, default_timeout_secs: u64) -> Self {
        Self {
            storage,
            default_timeout_secs,
        }
    }

    fn park_key(&self, orbit: u16) -> String {
        self.storage.key(&format!("park:{}", orbit))
    }

    /// Park a call in a specific orbit.
    pub async fn park(
        &self,
        orbit: u16,
        parked_call: RedisParkedCall,
        timeout_secs: Option<u64>,
    ) -> Result<()> {
        let key = self.park_key(orbit);
        let mut conn = self.storage.connection();
        let ttl = timeout_secs.unwrap_or(self.default_timeout_secs);

        let json = serde_json::to_string(&parked_call)?;

        let (): () = redis::pipe()
            .atomic()
            .set(&key, &json)
            .expire(&key, ttl as i64)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("Redis park SET failed: {}", e))?;

        debug!(orbit = orbit, call_id = %parked_call.call_id, ttl_secs = ttl, "Parked call in Redis");
        Ok(())
    }

    /// Retrieve (peek) a parked call without removing it.
    pub async fn get(&self, orbit: u16) -> Result<Option<RedisParkedCall>> {
        let key = self.park_key(orbit);
        let mut conn = self.storage.connection();

        let json: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| anyhow::anyhow!("Redis park GET failed: {}", e))?;

        match json {
            Some(j) => {
                let parked: RedisParkedCall = serde_json::from_str(&j)?;
                Ok(Some(parked))
            }
            None => Ok(None),
        }
    }

    /// Unpark (remove) a call from an orbit. Returns the parked call if it existed.
    pub async fn unpark(&self, orbit: u16) -> Result<Option<RedisParkedCall>> {
        let key = self.park_key(orbit);
        let mut conn = self.storage.connection();

        // GET + DEL in a pipeline for atomicity
        let (json, _): (Option<String>, ()) = redis::pipe()
            .get(&key)
            .del(&key)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("Redis unpark GET+DEL failed: {}", e))?;

        match json {
            Some(j) => {
                let parked: RedisParkedCall = serde_json::from_str(&j)?;
                debug!(orbit = orbit, call_id = %parked.call_id, "Unparked call from Redis");
                Ok(Some(parked))
            }
            None => Ok(None),
        }
    }
}
