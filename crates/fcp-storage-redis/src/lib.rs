//! Redis storage backend for FCP.
//!
//! Provides Redis-backed implementations of FCP storage traits, primarily:
//! - `RegistrationStore` — ephemeral SIP registrations with native TTL expiry
//! - `RedisEventBus` — distributed event bus via Pub/Sub + Streams
//! - Parking store — Redis-backed call parking for cross-node pickup

#[cfg(feature = "cdr")]
pub mod cdr_stream;
pub mod circuit_breaker;
pub mod registration;

#[cfg(feature = "event-bus")]
pub mod event_bus;

#[cfg(feature = "metrics")]
pub mod metrics;

pub mod parking;

use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::info;

pub use circuit_breaker::{CircuitState, RedisCircuitBreaker};

/// Shared Redis connection manager used by all Redis-backed stores.
#[derive(Clone)]
pub struct RedisStorage {
    client: redis::Client,
    conn: redis::aio::ConnectionManager,
    key_prefix: String,
    circuit_breaker: Option<Arc<RedisCircuitBreaker>>,
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

/// How a connection comes back after it is lost.
///
/// The library's defaults hand its `factor` of 100 to the backoff as the
/// *multiplier*, from a one-second floor, so a lost connection is retried
/// after 1 s and then every 60 s — and every command on it waits on the
/// retry. A two-second blip in the link to Redis stalled a node's
/// membership, events and parking for minutes
/// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3 found it). Retries are a second
/// apart at most, and an attempt that hears nothing gives up after two
/// seconds rather than never.
///
/// No response timeout: `XREADGROUP … BLOCK` rides on these connections
/// and waits longer than any sensible one.
fn connection_config() -> redis::aio::ConnectionManagerConfig {
    redis::aio::ConnectionManagerConfig::new()
        .set_max_delay(1_000)
        .set_connection_timeout(std::time::Duration::from_secs(2))
}

impl RedisStorage {
    /// Create a new Redis storage connection.
    pub async fn new(config: RedisConfig) -> Result<Self> {
        let client = redis::Client::open(config.url.as_str())
            .with_context(|| format!("Failed to create Redis client for {}", config.url))?;

        let conn =
            redis::aio::ConnectionManager::new_with_config(client.clone(), connection_config())
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
            circuit_breaker: None,
        })
    }

    /// Attach a circuit breaker to this storage instance (builder pattern).
    pub fn with_circuit_breaker(mut self, threshold: u32, cooldown: std::time::Duration) -> Self {
        self.circuit_breaker = Some(Arc::new(RedisCircuitBreaker::new(threshold, cooldown)));
        self
    }

    /// Get a reference to the circuit breaker, if configured.
    pub fn circuit_breaker(&self) -> Option<&Arc<RedisCircuitBreaker>> {
        self.circuit_breaker.as_ref()
    }

    /// Execute an operation through the circuit breaker.
    ///
    /// If no circuit breaker is configured, the operation runs directly.
    /// When the breaker is open, returns an error immediately without
    /// executing the operation.
    pub async fn execute<F, Fut, T>(&self, op: &str, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        if let Some(ref cb) = self.circuit_breaker {
            if !cb.allow_request() {
                anyhow::bail!("Redis circuit breaker open (operation: {})", op);
            }

            match f().await {
                Ok(val) => {
                    cb.record_success();
                    Ok(val)
                }
                Err(e) => {
                    cb.record_failure();
                    Err(e)
                }
            }
        } else {
            f().await
        }
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

    /// The same Redis and key prefix on a **separate** connection. The
    /// shared connection is multiplexed, so a blocking command on it
    /// (`XREADGROUP … BLOCK`) holds up every other command behind it;
    /// consumers that block should read on a connection of their own.
    pub async fn detached(&self) -> Result<Self> {
        let conn = redis::aio::ConnectionManager::new_with_config(
            self.client.clone(),
            connection_config(),
        )
        .await
        .context("Failed to open a detached Redis connection")?;
        Ok(Self {
            client: self.client.clone(),
            conn,
            key_prefix: self.key_prefix.clone(),
            circuit_breaker: self.circuit_breaker.clone(),
        })
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
