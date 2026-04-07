//! fcp-presence-server — Standalone SIP presence server.
//!
//! Deploys presence independently of fcp-call-manager. Tracks user availability
//! via SUBSCRIBE/NOTIFY/PUBLISH, manages watcher subscriptions, and provides
//! a REST API for presence management.

use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::info;

mod config;
mod http_api;
mod xcap_server;

use config::PresenceServerConfig;

/// CLI arguments for fcp-presence-server.
#[derive(Parser)]
#[command(name = "fcp-presence", about = "Standalone SIP presence server")]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,

    /// Override log level (e.g., debug, info, warn, error).
    #[arg(long)]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load configuration
    let mut config = config::load_config(cli.config.as_deref())?;

    if let Some(ref level) = cli.log_level {
        config.logging.level = level.clone();
    }

    config::validate_config(&config)?;

    // Initialize tracing
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-presence starting"
    );

    // Initialize storage
    let storage_result = initialize_storage(&config).await?;
    let config_store = storage_result.config_store;
    let dyn_storage = storage_result.dyn_storage;

    // Seed operational config sections into database (DB primary, TOML seed)
    // Bootstrap sections (service, storage, http, logging) stay TOML-only.
    {
        let cs = config_store.as_ref();
        let svc = "presence";
        let tid = "_system";

        let sections: Vec<(&str, serde_json::Value)> = vec![
            (
                "presence",
                serde_json::to_value(&config.presence).unwrap_or_default(),
            ),
            (
                "redis",
                serde_json::to_value(&config.redis).unwrap_or_default(),
            ),
            (
                "xcap",
                serde_json::to_value(&config.xcap).unwrap_or_default(),
            ),
            (
                "webhook",
                serde_json::to_value(&config.webhooks).unwrap_or_default(),
            ),
            (
                "cluster",
                serde_json::to_value(&config.cluster).unwrap_or_default(),
            ),
        ];

        for (section, toml_val) in sections {
            match cs.get_config(svc, section, tid).await {
                Ok(Some(entry)) => {
                    match section {
                        "presence" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.presence = v;
                            }
                        }
                        "redis" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.redis = v;
                            }
                        }
                        "xcap" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.xcap = v;
                            }
                        }
                        "webhook" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.webhooks = v;
                            }
                        }
                        "cluster" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.cluster = v;
                            }
                        }
                        _ => {}
                    }
                    info!(
                        service = svc,
                        section,
                        version = entry.version,
                        "loaded config from database"
                    );
                }
                Ok(None) => {
                    match cs
                        .set_config(svc, section, tid, toml_val, Some("seed"))
                        .await
                    {
                        Ok(version) => info!(
                            service = svc,
                            section, version, "seeded config into database"
                        ),
                        Err(e) => {
                            tracing::warn!(service = svc, section, error = %e, "failed to seed config")
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(service = svc, section, error = %e, "failed to check DB config, using TOML");
                }
            }
        }
        info!("Operational config loading complete (DB primary, TOML seed)");
    }

    // Create PresenceConfig from our config
    let presence_config = fcp_presence::PresenceConfig {
        max_presentities: config.presence.max_presentities,
        default_subscription_expiry_secs: config.presence.default_subscription_expiry_secs,
        cleanup_interval_secs: config.presence.cleanup_interval_secs,
        ..Default::default()
    };

    // Create event bus for presence events + analytics
    let event_bus = Arc::new(fcp_events::InMemoryEventBus::new());

    // Initialize cluster membership (if configured)
    #[cfg(feature = "storage-redis")]
    let _cluster_manager = if config.cluster.enabled {
        if let Some(ref url) = config.storage.redis_url {
            match fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
                url: url.clone(),
                ..Default::default()
            })
            .await
            {
                Ok(redis) => {
                    let member = fcp_cluster::ClusterMember {
                        node_id: config.service.node_id.clone(),
                        node_type: config.service.service_type.clone(),
                        http_address: config.http.bind_address,
                        sip_address: "0.0.0.0:0".parse().unwrap(),
                        capabilities: vec!["presence".to_string(), "blf".to_string()],
                        active_calls: 0,
                        started_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                    };
                    let cm = fcp_cluster::ClusterManager::new(redis, member)
                        .with_intervals(
                            config.cluster.heartbeat_interval_secs,
                            config.cluster.heartbeat_ttl_secs,
                        )
                        .with_event_publisher(
                            event_bus.clone() as Arc<dyn fcp_events::EventPublisher>
                        );
                    let _handle = cm.start();
                    info!("Cluster membership started");
                    Some(cm)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to connect to Redis; cluster disabled");
                    None
                }
            }
        } else {
            tracing::warn!("cluster.enabled = true but redis_url is not set; cluster disabled");
            None
        }
    } else {
        None
    };

    // Create PresenceManager
    let mut manager = fcp_presence::PresenceManager::new(
        presence_config,
        Some(event_bus.clone() as Arc<dyn fcp_events::EventPublisher>),
    );

    // Set up resource list store
    manager.set_resource_list_store(Arc::clone(dyn_storage.resource_list_store()));

    // Redis clustering (if configured)
    #[cfg(feature = "redis")]
    if let Some(ref redis_config) = config.redis {
        setup_redis(&mut manager, redis_config, &config.service.node_id).await?;
    }

    // Start presence analytics consumer
    let history_store: Arc<dyn fcp_core::storage::PresenceHistoryStore> = Arc::new(
        fcp_presence::analytics::InMemoryPresenceHistory::new(100_000),
    );
    let _analytics_handle = fcp_presence::analytics::spawn_analytics_consumer(
        event_bus.clone() as Arc<dyn fcp_events::EventSubscriber>,
        Arc::clone(&history_store),
    );

    // Start webhook dispatcher (if configured and webhook feature enabled)
    #[cfg(feature = "webhook")]
    let webhook_dispatcher = {
        let dispatcher = if config.webhooks.as_ref().map_or(false, |w| w.enabled) {
            let wh = config.webhooks.as_ref().unwrap();
            let d = Arc::new(fcp_presence::webhook::WebhookDispatcher::new(
                wh.max_retries,
                wh.base_delay_ms,
                wh.max_delay_ms,
            ));
            let _handle = fcp_presence::webhook::spawn_webhook_consumer(
                event_bus.clone() as Arc<dyn fcp_events::EventSubscriber>,
                Arc::clone(&d),
            );
            info!("Webhook dispatcher started");
            Some(d)
        } else {
            None
        };
        dispatcher
    };
    #[cfg(not(feature = "webhook"))]
    let _webhook_dispatcher: Option<()> = None;

    // Start background expiry cleanup
    let cleanup_manager = manager.clone();
    let cleanup_interval = config.presence.cleanup_interval_secs;
    let mut shutdown_rx = manager.shutdown_receiver();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(cleanup_interval));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    cleanup_manager.cleanup_expired_subscriptions_from_store().await;
                    cleanup_manager.cleanup_expired_watchers();
                    cleanup_manager.check_publish_expiry_warnings().await;
                    cleanup_manager.cleanup_expired_publishes().await;
                }
                _ = shutdown_rx.changed() => {
                    break;
                }
            }
        }
    });

    // Start HTTP API
    let app_state = Arc::new(http_api::AppState {
        manager: manager.clone(),
        sip_handler: None, // SIP transport setup is a follow-up task
        start_time: Instant::now(),
        api_token: config.http.api_token.clone(),
        resource_list_store: Arc::clone(dyn_storage.resource_list_store()),
        default_tenant_id: config.service.default_tenant_id,
        history_store: Some(Arc::clone(&history_store)),
        #[cfg(feature = "webhook")]
        webhook_dispatcher,
    });

    // Determine XCAP root path from config
    let xcap_root = config
        .xcap
        .as_ref()
        .filter(|x| x.enabled)
        .map(|x| x.root_path.clone())
        .unwrap_or_else(|| "/xcap-root".to_string());

    let http_bind = config.http.bind_address;
    let router = http_api::router(app_state, Some(&xcap_root));

    let http_handle = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(http_bind)
            .await
            .expect("Failed to bind HTTP listener");
        info!(bind = %http_bind, "HTTP API listening");
        axum::serve(listener, router).await.ok();
    });

    // Log startup summary
    info!(
        http = %config.http.bind_address,
        storage = "postgres",
        redis = config.redis.is_some(),
        "fcp-presence ready"
    );

    // Spawn config poll loop (checks DB every 10s for config changes)
    {
        let cs = Arc::clone(&config_store);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_version: i64 = 0;
            loop {
                interval.tick().await;
                match cs.get_latest_version("presence", "_system").await {
                    Ok(v) if v > last_version => {
                        tracing::info!(
                            old_version = last_version,
                            new_version = v,
                            "config version changed in database"
                        );
                        last_version = v;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!(error = %e, "config poll: failed to check version");
                    }
                }
            }
        });
    }

    // Wait for shutdown signal
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("Failed to register SIGTERM handler")?;

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received SIGINT, shutting down...");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, shutting down...");
        }
    }

    // Shutdown cluster manager
    #[cfg(feature = "storage-redis")]
    if let Some(ref cm) = _cluster_manager {
        cm.shutdown().await;
    }

    // Graceful shutdown
    manager.shutdown().await;
    http_handle.abort();
    info!("Shutdown complete");

    Ok(())
}

/// Storage initialization result with both DynStorage and ConfigStore.
struct StorageResult {
    dyn_storage: Arc<fcp_core::storage::DynStorage>,
    config_store: Arc<dyn fcp_core::config_store::ConfigStore>,
}

/// Initialize PostgreSQL storage backend based on configuration.
async fn initialize_storage(config: &PresenceServerConfig) -> Result<StorageResult> {
    use fcp_storage_postgres::{PostgresConfig, PostgresStorage};

    let pg_config = PostgresConfig {
        connection_string: config.storage.database_url.clone(),
        max_connections: config.storage.max_connections,
        connection_timeout: Duration::from_secs(30),
    };

    let storage = PostgresStorage::new(pg_config)
        .await
        .context("Failed to initialize PostgreSQL storage")?;

    let storage = Arc::new(storage);

    info!("PostgreSQL storage initialized successfully");
    Ok(StorageResult {
        dyn_storage: Arc::new(fcp_core::storage::DynStorage::from_all_in_one(
            storage.clone(),
        )),
        config_store: storage,
    })
}

/// Set up Redis Pub/Sub for cross-node clustering.
#[cfg(feature = "redis")]
async fn setup_redis(
    manager: &mut fcp_presence::PresenceManager,
    redis_config: &config::RedisSection,
    node_id: &str,
) -> Result<()> {
    use fcp_presence::redis_pubsub::RedisPresencePubSub;

    let pubsub = RedisPresencePubSub::new(
        &redis_config.url,
        Some(
            redis_config
                .node_id
                .clone()
                .unwrap_or_else(|| node_id.to_string()),
        ),
    )?;
    info!(
        node_id = %pubsub.node_id(),
        redis_url = %redis_config.url,
        "Redis Pub/Sub initialized"
    );
    manager.set_pubsub(Arc::new(pubsub));

    // Start the pubsub listener
    let handle = manager.run_pubsub_listener().await?;
    info!("Redis Pub/Sub listener started");

    // Detach the handle (it runs until shutdown)
    drop(handle);

    Ok(())
}

fn init_tracing(config: &PresenceServerConfig) {
    use tracing_subscriber::EnvFilter;

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.logging.level));

    match config.logging.format.as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .json()
                .with_env_filter(env_filter)
                .init();
        }
        "compact" => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .compact()
                .with_env_filter(env_filter)
                .init();
        }
        _ => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .with_env_filter(env_filter)
                .init();
        }
    }
}
