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
    let dyn_storage = initialize_storage(&config).await?;

    // Create PresenceConfig from our config
    let presence_config = fcp_presence::PresenceConfig {
        max_presentities: config.presence.max_presentities,
        default_subscription_expiry_secs: config.presence.default_subscription_expiry_secs,
        cleanup_interval_secs: config.presence.cleanup_interval_secs,
        ..Default::default()
    };

    // Create event bus for presence events + analytics
    let event_bus = Arc::new(fcp_events::InMemoryEventBus::new());

    // Create PresenceManager
    let mut manager = fcp_presence::PresenceManager::new(
        presence_config,
        Some(event_bus.clone() as Arc<dyn fcp_events::EventPublisher>),
        Some(Arc::clone(dyn_storage.presence_store())),
    );

    // Set up resource list store
    manager.set_resource_list_store(Arc::clone(dyn_storage.resource_list_store()));

    // Redis clustering (if configured)
    #[cfg(feature = "redis")]
    if let Some(ref redis_config) = config.redis {
        setup_redis(&mut manager, redis_config, &config.service.node_id).await?;
    }

    // Start presence analytics consumer
    let history_store: Arc<dyn fcp_core::storage::PresenceHistoryStore> =
        Arc::new(fcp_presence::analytics::InMemoryPresenceHistory::new(100_000));
    let _analytics_handle = fcp_presence::analytics::spawn_analytics_consumer(
        event_bus.clone() as Arc<dyn fcp_events::EventSubscriber>,
        Arc::clone(&history_store),
    );

    // Start webhook dispatcher (if configured and webhook feature enabled)
    #[cfg(feature = "webhook")]
    let webhook_dispatcher = {
        let dispatcher = if config
            .webhooks
            .as_ref()
            .map_or(false, |w| w.enabled)
        {
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
        resource_list_store: Arc::clone(dyn_storage.resource_list_store()),
        default_tenant_id: fcp_core::TenantId::default(),
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
        storage = %config.storage.backend,
        redis = config.redis.is_some(),
        "fcp-presence ready"
    );

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

    // Graceful shutdown
    manager.shutdown().await;
    http_handle.abort();
    info!("Shutdown complete");

    Ok(())
}

/// Initialize storage backend based on configuration.
async fn initialize_storage(
    config: &PresenceServerConfig,
) -> Result<Arc<fcp_core::storage::DynStorage>> {
    match config.storage.backend.as_str() {
        #[cfg(feature = "storage-sqlite")]
        "sqlite" => {
            // Ensure data directory exists
            if let Some(path) = config
                .storage
                .connection_string
                .strip_prefix("sqlite://")
            {
                if let Some(parent) = std::path::Path::new(path).parent() {
                    std::fs::create_dir_all(parent).ok();
                }
            }

            let sqlite_config =
                fcp_storage_sqlite::SqliteConfig::new(&config.storage.connection_string);
            let storage = fcp_storage_sqlite::SqliteStorage::new(sqlite_config)
                .await
                .context("Failed to initialize SQLite storage")?;
            info!(backend = "sqlite", "Storage initialized");
            Ok(Arc::new(fcp_core::storage::DynStorage::from_all_in_one(
                Arc::new(storage),
            )))
        }

        #[cfg(feature = "storage-postgres")]
        "postgres" => {
            let pg_config =
                fcp_storage_postgres::PostgresConfig::new(&config.storage.connection_string);
            let storage = fcp_storage_postgres::PostgresStorage::new(pg_config)
                .await
                .context("Failed to initialize PostgreSQL storage")?;
            info!(backend = "postgres", "Storage initialized");
            Ok(Arc::new(fcp_core::storage::DynStorage::from_all_in_one(
                Arc::new(storage),
            )))
        }

        #[cfg(feature = "storage-mysql")]
        "mysql" => {
            let mysql_config =
                fcp_storage_mysql::MysqlConfig::new(&config.storage.connection_string);
            let storage = fcp_storage_mysql::MysqlStorage::new(mysql_config)
                .await
                .context("Failed to initialize MySQL storage")?;
            info!(backend = "mysql", "Storage initialized");
            Ok(Arc::new(fcp_core::storage::DynStorage::from_all_in_one(
                Arc::new(storage),
            )))
        }

        #[cfg(feature = "storage-mongodb")]
        "mongodb" => {
            let mongo_config =
                fcp_storage_mongodb::MongodbConfig::new(&config.storage.connection_string);
            let storage = fcp_storage_mongodb::MongodbStorage::new(mongo_config)
                .await
                .context("Failed to initialize MongoDB storage")?;
            info!(backend = "mongodb", "Storage initialized");
            Ok(Arc::new(fcp_core::storage::DynStorage::from_all_in_one(
                Arc::new(storage),
            )))
        }

        other => anyhow::bail!("Unsupported storage backend: {other}"),
    }
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
        Some(redis_config.node_id.clone().unwrap_or_else(|| node_id.to_string())),
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
