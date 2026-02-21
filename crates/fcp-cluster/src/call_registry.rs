//! Call ownership registry — tracks which node owns which active call.
//!
//! Key design:
//! ```text
//! Key:    {prefix}cluster:call:{call_id}
//! Value:  node_id
//! TTL:    3600s (max call duration)
//! ```

use fcp_storage_redis::RedisStorage;
use redis::AsyncCommands;
use tracing::debug;

/// Tracks which node owns which call in Redis.
#[derive(Clone)]
pub struct CallRegistry {
    storage: RedisStorage,
    node_id: String,
    call_ttl_secs: u64,
}

impl CallRegistry {
    /// Create a new call registry.
    pub fn new(storage: RedisStorage, node_id: String) -> Self {
        Self {
            storage,
            node_id,
            call_ttl_secs: 3600,
        }
    }

    /// Create with custom TTL.
    pub fn with_ttl(storage: RedisStorage, node_id: String, ttl_secs: u64) -> Self {
        Self {
            storage,
            node_id,
            call_ttl_secs: ttl_secs,
        }
    }

    fn call_key(&self, call_id: &str) -> String {
        self.storage.key(&format!("cluster:call:{}", call_id))
    }

    /// Register a call as owned by this node.
    pub async fn register_call(&self, call_id: &str) -> anyhow::Result<()> {
        let key = self.call_key(call_id);
        let mut conn = self.storage.connection();

        let (): () = redis::pipe()
            .atomic()
            .set(&key, &self.node_id)
            .expire(&key, self.call_ttl_secs as i64)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to register call: {}", e))?;

        debug!(call_id = %call_id, node_id = %self.node_id, "Registered call ownership");
        Ok(())
    }

    /// Remove call ownership (call terminated).
    pub async fn unregister_call(&self, call_id: &str) -> anyhow::Result<()> {
        let key = self.call_key(call_id);
        let mut conn = self.storage.connection();

        let _: Option<String> = conn
            .del(&key)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to unregister call: {}", e))?;

        debug!(call_id = %call_id, "Unregistered call ownership");
        Ok(())
    }

    /// Find which node owns a given call.
    pub async fn get_call_owner(&self, call_id: &str) -> anyhow::Result<Option<String>> {
        let key = self.call_key(call_id);
        let mut conn = self.storage.connection();

        let owner: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get call owner: {}", e))?;

        Ok(owner)
    }

    /// Check if this node owns a given call.
    pub async fn is_local_call(&self, call_id: &str) -> anyhow::Result<bool> {
        match self.get_call_owner(call_id).await? {
            Some(owner) => Ok(owner == self.node_id),
            None => Ok(false),
        }
    }

    /// Refresh the TTL on a call ownership key.
    ///
    /// Should be called periodically for long-running calls to prevent the
    /// ownership key from expiring while the call is still active.
    pub async fn refresh_call(&self, call_id: &str) -> anyhow::Result<()> {
        let key = self.call_key(call_id);
        let mut conn = self.storage.connection();

        let _: bool = conn
            .expire(&key, self.call_ttl_secs as i64)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to refresh call TTL: {}", e))?;

        debug!(call_id = %call_id, ttl_secs = self.call_ttl_secs, "Refreshed call ownership TTL");
        Ok(())
    }
}
