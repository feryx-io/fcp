//! Redis-backed `RegistrationStore` implementation.
//!
//! Uses a Hash-per-AOR design:
//! ```text
//! Key:    {prefix}reg:{aor}       e.g. "fcp:reg:sip:alice@example.com"
//! Type:   Hash
//! Fields: {contact_uri} => JSON(Registration)
//! TTL:    max(expires_at) across all contacts
//! ```
//!
//! On `get_registrations`: HGETALL + client-side expired-contact filtering.
//! On `put_registration`: pipeline HSET + EXPIRE with max TTL.
//! `cleanup_expired` is a no-op — Redis TTL handles key-level expiry.

use crate::RedisStorage;
use anyhow::Result;
use async_trait::async_trait;
use fcp_core::storage::{Registration, RegistrationStore};
use redis::AsyncCommands;
use std::time::SystemTime;
use tracing::{debug, warn};

/// Redis-backed registration store.
#[derive(Clone)]
pub struct RedisRegistrationStore {
    storage: RedisStorage,
}

impl RedisRegistrationStore {
    /// Create a new Redis registration store.
    pub fn new(storage: RedisStorage) -> Self {
        Self { storage }
    }

    /// Build the Redis key for an AOR.
    fn reg_key(&self, aor: &str) -> String {
        self.storage.key(&format!("reg:{}", aor))
    }
}

#[async_trait]
impl RegistrationStore for RedisRegistrationStore {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        let key = self.reg_key(aor);
        let mut conn = self.storage.connection();

        let entries: std::collections::HashMap<String, String> =
            conn.hgetall(&key).await.map_err(|e| {
                anyhow::anyhow!("Redis HGETALL failed for {}: {}", key, e)
            })?;

        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let now = SystemTime::now();
        let mut registrations = Vec::with_capacity(entries.len());

        for (_contact, json) in &entries {
            match serde_json::from_str::<Registration>(json) {
                Ok(reg) => {
                    // Filter out expired contacts client-side
                    if reg.expires_at > now {
                        registrations.push(reg);
                    }
                }
                Err(e) => {
                    warn!(key = %key, error = %e, "Failed to deserialize registration from Redis");
                }
            }
        }

        debug!(aor = %aor, count = registrations.len(), "Retrieved registrations from Redis");
        Ok(registrations)
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        let key = self.reg_key(&registration.aor);
        let mut conn = self.storage.connection();

        let json = serde_json::to_string(&registration)?;

        // Calculate TTL from expires_at
        let ttl_secs = registration
            .expires_at
            .duration_since(SystemTime::now())
            .unwrap_or_default()
            .as_secs();

        if ttl_secs == 0 {
            // Already expired, remove instead of adding
            let _: () = conn
                .hdel(&key, &registration.contact)
                .await
                .map_err(|e| anyhow::anyhow!("Redis HDEL failed: {}", e))?;
            return Ok(());
        }

        // Pipeline: HSET + update EXPIRE to the max TTL across all contacts
        let (): () = redis::pipe()
            .atomic()
            .hset(&key, &registration.contact, &json)
            .expire(&key, ttl_secs as i64)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("Redis pipeline (HSET+EXPIRE) failed: {}", e))?;

        debug!(
            aor = %registration.aor,
            contact = %registration.contact,
            ttl_secs = ttl_secs,
            "Stored registration in Redis"
        );

        Ok(())
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        let key = self.reg_key(aor);
        let mut conn = self.storage.connection();

        let _: () = conn
            .hdel(&key, contact)
            .await
            .map_err(|e| anyhow::anyhow!("Redis HDEL failed: {}", e))?;

        // If the hash is now empty, delete the key
        let remaining: usize = conn
            .hlen(&key)
            .await
            .map_err(|e| anyhow::anyhow!("Redis HLEN failed: {}", e))?;

        if remaining == 0 {
            let _: () = conn
                .del(&key)
                .await
                .map_err(|e| anyhow::anyhow!("Redis DEL failed: {}", e))?;
        }

        debug!(aor = %aor, contact = %contact, "Deleted registration from Redis");
        Ok(())
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        let key = self.reg_key(aor);
        let mut conn = self.storage.connection();

        let _: () = conn
            .del(&key)
            .await
            .map_err(|e| anyhow::anyhow!("Redis DEL failed: {}", e))?;

        debug!(aor = %aor, "Deleted all registrations from Redis");
        Ok(())
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        // No-op: Redis TTL handles key-level expiry automatically.
        // Individual expired contacts within a still-alive hash are
        // filtered out client-side in get_registrations().
        Ok(0)
    }

    async fn registration_count(&self) -> Result<usize> {
        let mut conn = self.storage.connection();
        let pattern = self.storage.key("reg:*");

        // Use SCAN to count registration keys
        let mut count = 0usize;
        let mut cursor = 0u64;
        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await
                .map_err(|e| anyhow::anyhow!("Redis SCAN failed: {}", e))?;

            // For each key, count the number of hash fields (contacts)
            for key in &keys {
                let hlen: usize = conn
                    .hlen(key)
                    .await
                    .map_err(|e| anyhow::anyhow!("Redis HLEN failed: {}", e))?;
                count += hlen;
            }

            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }

        Ok(count)
    }

    async fn get_all_registrations(&self) -> Result<Vec<Registration>> {
        let mut conn = self.storage.connection();
        let pattern = self.storage.key("reg:*");
        let now = SystemTime::now();

        let mut all_registrations = Vec::new();
        let mut cursor = 0u64;

        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await
                .map_err(|e| anyhow::anyhow!("Redis SCAN failed: {}", e))?;

            for key in &keys {
                let entries: std::collections::HashMap<String, String> =
                    conn.hgetall(key).await.map_err(|e| {
                        anyhow::anyhow!("Redis HGETALL failed for {}: {}", key, e)
                    })?;

                for (_contact, json) in &entries {
                    match serde_json::from_str::<Registration>(json) {
                        Ok(reg) if reg.expires_at > now => {
                            all_registrations.push(reg);
                        }
                        Ok(_) => {} // expired
                        Err(e) => {
                            warn!(key = %key, error = %e, "Failed to deserialize registration");
                        }
                    }
                }
            }

            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }

        Ok(all_registrations)
    }
}

#[cfg(test)]
mod tests {
    use fcp_core::storage::Registration;
    use std::time::{Duration, SystemTime};

    fn make_registration(aor: &str, contact: &str, ttl_secs: u64) -> Registration {
        Registration {
            aor: aor.to_string(),
            contact: contact.to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(ttl_secs),
            call_id: "test-call-id".to_string(),
            cseq: 1,
            user_agent: Some("FCP Test".to_string()),
            q: Some(1.0),
            registered_at: SystemTime::now(),
            path: None,
            flow_token: None,
        }
    }

    #[test]
    fn test_registration_serialization() {
        let reg = make_registration("sip:alice@example.com", "sip:alice@192.168.1.100:5060", 3600);
        let json = serde_json::to_string(&reg).unwrap();
        let deserialized: Registration = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.aor, reg.aor);
        assert_eq!(deserialized.contact, reg.contact);
    }
}
