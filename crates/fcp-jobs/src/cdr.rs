//! The CDR processor role (`docs/CDR_PIPELINE.md` §4.3): consume the CDR
//! stream the nodes ship to, and normalize, enrich, rate, store, settle and
//! announce each record. Enabled with `[cdr] enabled = true`; any number of
//! `fcp-jobs` replicas join the same consumer group.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fcp_cdr::consumer::{Consumer, ConsumerConfig, RecordSource};
use fcp_cdr::{DirectionEnricher, Processor};
use fcp_core::storage::{CdrStore, PhoneNumberStore};
use fcp_events::EventPublisher;
use fcp_storage_redis::cdr_stream::RedisCdrStream;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::config::JobsConfig;

/// The `[cdr]` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CdrConfig {
    /// Run the processor in this `fcp-jobs`.
    #[serde(default)]
    pub enabled: bool,

    /// The system of record the records are stored in.
    #[serde(default = "default_database_url")]
    pub database_url: String,

    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// The nodes' Redis key prefix; the stream is `<prefix>cdrs`.
    #[serde(default = "default_key_prefix")]
    pub key_prefix: String,

    /// Records per batch, and how long a read waits for the first.
    #[serde(default = "default_batch")]
    pub batch: usize,
    #[serde(default = "default_batch_wait_ms")]
    pub batch_wait_ms: u64,

    /// An entry a consumer has held this long without acknowledging is
    /// taken over: that consumer is gone.
    #[serde(default = "default_claim_idle_secs")]
    pub claim_idle_secs: u64,

    /// Acknowledged entries are kept this long for replay before the
    /// stream is trimmed.
    #[serde(default = "default_retain_hours")]
    pub retain_hours: u64,

    /// How long a tenant's rate plan and numbers are cached.
    #[serde(default = "default_cache_secs")]
    pub cache_secs: u64,

    /// Region stamped on the events this processor publishes; falls back
    /// to `FCP_REGION`.
    #[serde(default)]
    pub region: Option<String>,
}

fn default_database_url() -> String {
    "postgresql://localhost/fcp".to_string()
}
fn default_max_connections() -> u32 {
    10
}
fn default_key_prefix() -> String {
    "fcp:".to_string()
}
fn default_batch() -> usize {
    500
}
fn default_batch_wait_ms() -> u64 {
    200
}
fn default_claim_idle_secs() -> u64 {
    60
}
fn default_retain_hours() -> u64 {
    24
}
fn default_cache_secs() -> u64 {
    60
}

impl Default for CdrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            database_url: default_database_url(),
            max_connections: default_max_connections(),
            key_prefix: default_key_prefix(),
            batch: default_batch(),
            batch_wait_ms: default_batch_wait_ms(),
            claim_idle_secs: default_claim_idle_secs(),
            retain_hours: default_retain_hours(),
            cache_secs: default_cache_secs(),
            region: None,
        }
    }
}

/// The running role.
pub struct CdrRole {
    consumer: tokio::sync::Mutex<Option<Consumer>>,
    pub source: Arc<RedisCdrStream>,
    pub processor: Arc<Processor>,
}

impl CdrRole {
    /// Connect to the database and Redis, and start consuming.
    pub async fn start(config: &JobsConfig) -> Result<Self> {
        let cdr = &config.cdr;
        let pg = fcp_storage_postgres::PostgresStorage::new(fcp_storage_postgres::PostgresConfig {
            connection_string: cdr.database_url.clone(),
            max_connections: cdr.max_connections,
            connection_timeout: Duration::from_secs(30),
            encryption: None,
            read_replica_url: None,
            max_replica_lag: Duration::from_secs(5),
        })
        .await
        .context("Failed to connect the CDR processor to its database")?;
        let pg = Arc::new(pg);

        let redis = fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: config.redis.url.clone(),
            key_prefix: cdr.key_prefix.clone(),
        })
        .await
        .context("Failed to connect the CDR processor to Redis")?;

        let region = cdr
            .region
            .clone()
            .or_else(|| std::env::var("FCP_REGION").ok());
        let events: Arc<dyn EventPublisher> = Arc::new(
            fcp_storage_redis::event_bus::RedisEventBus::new(
                redis.clone(),
                config.service.node_id.clone(),
            )
            .with_region(region),
        );

        let cache = Duration::from_secs(cdr.cache_secs);
        let billing = fcp_billing::BillingEngine::new(Arc::clone(&pg))
            .with_events(Arc::clone(&events))
            .with_plan_cache(cache);
        let store: Arc<dyn CdrStore> = pg.clone();
        // The record table's upkeep, beside fcp-admin's: partitions and
        // rollups, one host at a time under the store's locks.
        fcp_cdr::spawn_maintenance(Arc::clone(&store), fcp_cdr::MaintenanceConfig::default());
        let numbers: Arc<dyn PhoneNumberStore> = pg.clone();
        let processor = Arc::new(
            Processor::new(store, Some(Arc::new(billing)))
                .with_enricher(Arc::new(DirectionEnricher::new(numbers, cache)))
                .with_events(events),
        );

        let source = Arc::new(RedisCdrStream::new(
            redis,
            config.service.node_id.clone(),
            Duration::from_secs(cdr.retain_hours * 3600),
        ));
        let consumer = Consumer::spawn(
            Arc::clone(&source) as Arc<dyn RecordSource>,
            Arc::clone(&processor),
            ConsumerConfig {
                batch: cdr.batch.max(1),
                batch_wait: Duration::from_millis(cdr.batch_wait_ms.max(1)),
                claim_idle: Duration::from_secs(cdr.claim_idle_secs.max(1)),
                ..ConsumerConfig::default()
            },
        );
        info!(
            stream = %source.stream_key(),
            consumer = %config.service.node_id,
            batch = cdr.batch,
            "CDR processor started"
        );
        Ok(Self {
            consumer: tokio::sync::Mutex::new(Some(consumer)),
            source,
            processor,
        })
    }

    /// Finish the batch in hand and stop. Idempotent.
    pub async fn stop(&self) {
        if let Some(consumer) = self.consumer.lock().await.take() {
            consumer.stop().await;
        }
    }

    /// The role's part of `/status`.
    pub async fn status(&self) -> serde_json::Value {
        let source = match self.source.status().await {
            Ok(s) => serde_json::to_value(s).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        };
        serde_json::json!({
            "stream": self.source.stream_key(),
            "source": source,
            "processor": self.processor.stats().snapshot(),
        })
    }

    /// The role's part of `/metrics`.
    pub async fn metrics(&self, node_id: &str) -> String {
        let labels = format!("node=\"{}\"", node_id.replace('"', ""));
        let mut out = self.processor.stats().snapshot().prometheus(&labels);
        if let Ok(status) = self.source.status().await {
            for (name, help, value) in [
                (
                    "cdr_stream_length",
                    "Entries in the CDR stream.",
                    status.length,
                ),
                (
                    "cdr_stream_pending",
                    "Entries delivered and not yet acknowledged.",
                    status.pending,
                ),
                (
                    "cdr_stream_dead_letters",
                    "Entries kept because they will never be stored.",
                    status.dead_letters,
                ),
            ] {
                fcp_cdr::stats::family(&mut out, name, "gauge", help);
                out.push_str(&format!("{name}{{{labels}}} {value}\n"));
            }
        }
        out
    }
}
