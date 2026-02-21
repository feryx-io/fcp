//! Redis storage backend for FCP.
//!
//! Provides Redis-backed implementations of FCP storage traits, primarily:
//! - `RegistrationStore` — ephemeral SIP registrations with native TTL expiry
//! - `RedisEventBus` — distributed event bus via Pub/Sub + Streams
//! - Parking store — Redis-backed call parking for cross-node pickup

pub mod registration;

#[cfg(feature = "event-bus")]
pub mod event_bus;

pub mod parking;

use anyhow::{Context, Result};
use tracing::info;

/// Shared Redis connection manager used by all Redis-backed stores.
#[derive(Clone)]
pub struct RedisStorage {
    client: redis::Client,
    conn: redis::aio::ConnectionManager,
    key_prefix: String,
}

/// Configuration for connecting to Redis.
#[derive(Debug, Clone)]
pub struct RedisConfig {
    /// Redis connection URL (e.g. "redis://redis:6379")
    pub url: String,
    /// Key prefix for all Redis keys (default: "fcp:")
    pub key_prefix: String,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://127.0.0.1:6379".to_string(),
            key_prefix: "fcp:".to_string(),
        }
    }
}

impl RedisStorage {
    /// Create a new Redis storage connection.
    pub async fn new(config: RedisConfig) -> Result<Self> {
        let client = redis::Client::open(config.url.as_str())
            .with_context(|| format!("Failed to create Redis client for {}", config.url))?;

        let conn = redis::aio::ConnectionManager::new(client.clone())
            .await
            .with_context(|| format!("Failed to connect to Redis at {}", config.url))?;

        // Validate connectivity with a PING before returning
        let mut test_conn = conn.clone();
        let _: String = redis::cmd("PING")
            .query_async(&mut test_conn)
            .await
            .with_context(|| format!("Redis health check failed at {}", config.url))?;

        info!(url = %config.url, prefix = %config.key_prefix, "Redis storage connected");

        Ok(Self {
            client,
            conn,
            key_prefix: config.key_prefix,
        })
    }

    /// Check Redis connectivity by issuing a PING command.
    pub async fn health_check(&self) -> Result<()> {
        let mut conn = self.conn.clone();
        let pong: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .context("Redis health check failed")?;
        if pong != "PONG" {
            anyhow::bail!("Redis PING returned unexpected response: {}", pong);
        }
        Ok(())
    }

    /// Get a clone of the connection manager.
    pub fn connection(&self) -> redis::aio::ConnectionManager {
        self.conn.clone()
    }

    /// Get the underlying Redis client (for creating PubSub connections).
    pub fn client(&self) -> &redis::Client {
        &self.client
    }

    /// Get the key prefix.
    pub fn key_prefix(&self) -> &str {
        &self.key_prefix
    }

    /// Build a full Redis key with the configured prefix.
    pub fn key(&self, suffix: &str) -> String {
        format!("{}{}", self.key_prefix, suffix)
    }
}
