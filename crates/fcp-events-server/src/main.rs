//! FCP Events Server - WebSocket event streaming service
//!
//! This service provides real-time event streaming to clients via WebSocket connections.
//! It subscribes to the FCP event bus and broadcasts events to connected clients based on
//! their subscriptions and tenant isolation.
//!
//! Additionally, it aggregates call state from multiple fcp-call-manager nodes and provides
//! REST endpoints for querying active calls and node status.

mod cli;
mod config;
mod connection;
mod nodes;
mod state;
mod storage;

use anyhow::{Context, Result};
use axum::{
    extract::{ws::WebSocketUpgrade, Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use fcp_core::storage::{ApiTokenStore, CredentialStore};
use fcp_core::TenantId;
use fcp_events::{
    CallEvent, EventPublisher, EventSubscriber, InMemoryEventBus, PlatformEvent, SystemEvent,
};
use fcp_webhooks::{WebhookDispatcher, WebhookDispatcherConfig};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::cli::Cli;
use crate::config::{load_config, EventsConfig};
use crate::connection::{cleanup_task, handle_websocket, ConnectionManager};
use crate::nodes::{NodeRegistry, NodeType};
use crate::state::{CallStateStore, RegistrationStateStore};

/// Application state shared across handlers
#[derive(Clone)]
struct AppState {
    config: EventsConfig,
    auth: EventsAuthState,
    manager: Arc<ConnectionManager>,
    event_bus: Arc<InMemoryEventBus>,
    call_store: Arc<CallStateStore>,
    registration_store: Arc<RegistrationStateStore>,
    node_registry: Arc<NodeRegistry>,
    #[allow(dead_code)]
    webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
    /// This server's region (`[cluster] region` / `FCP_REGION`), if any.
    local_region: Option<String>,
    /// Cross-region event bridge, when peer regions are configured.
    #[cfg(feature = "redis-events")]
    bridge: Option<Arc<fcp_storage_redis::event_bus::EventBridge>>,
    /// Where `/ingest` puts what it is given when the bus is Redis: the
    /// stream every events server in this region consumes and the bridge
    /// forwards, so an event a conference server, IVR or voicemail server
    /// posted here reaches every subscriber and every region
    /// (`docs/DEPLOYMENT_GAPS.md` §1.6). `None` on the in-memory bus,
    /// where `/ingest` delivers to this process alone.
    #[cfg(feature = "redis-events")]
    ingest_bus: Option<Arc<fcp_storage_redis::event_bus::RedisEventBus>>,
}

#[derive(Clone)]
struct EventsAuthState {
    expected_token: Option<String>,
    api_token_store: Option<Arc<dyn ApiTokenStore>>,
    credential_store: Option<Arc<dyn CredentialStore>>,
    session_keys: Option<fcp_core::session::SessionKeys>,
    login_sessions: Option<Arc<dyn fcp_core::session::LoginSessionStore>>,
}

impl EventsAuthState {
    fn auth_enabled(&self) -> bool {
        self.expected_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .is_some()
    }
}

#[derive(Clone, Debug)]
pub(crate) enum AuthPrincipal {
    StaticAdmin,
    ApiToken {
        tenant_id: TenantId,
        permissions: Vec<String>,
    },
    Session {
        _username: String,
        role: String,
        tenant_id: TenantId,
    },
}

impl AuthPrincipal {
    fn has_operator_access(&self) -> bool {
        match self {
            Self::StaticAdmin => true,
            Self::ApiToken { permissions, .. } => {
                permissions.iter().any(|p| p == "admin" || p == "*")
            }
            Self::Session { role, .. } => role != "end_user",
        }
    }

    pub(crate) fn tenant_scope(&self) -> Option<&TenantId> {
        match self {
            Self::StaticAdmin => None,
            Self::ApiToken { tenant_id, .. } | Self::Session { tenant_id, .. } => Some(tenant_id),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Load configuration
    let config_path = cli.config.as_deref();
    let mut config = fcp_core::config::check_stage(load_config(config_path), cli.check_config)?;
    // Secrets from the environment win over the file
    // (`docs/SHIP_AND_OPERATE.md` phase 2).
    if let Some(v) = &cli.auth_token {
        config.auth.token = Some(v.clone());
    }
    if let Some(v) = &cli.session_signing_key {
        config.auth.session_signing_key = Some(v.clone());
    }
    if let Some(v) = &cli.webhook_database_url {
        config.webhook.database_url = v.clone();
    }

    // Apply CLI overrides
    if let Some(log_level) = cli.log_level {
        config.logging.level = log_level;
    }
    if let Some(log_format) = cli.log_format {
        config.logging.format = log_format;
    }
    if let Some(bind_addr) = cli.bind_addr {
        config.websocket.bind_address = bind_addr.parse().context("Invalid bind address format")?;
    }

    // Initialize logging
    init_logging(&config.logging.level, &config.logging.format)?;

    info!(
        service = "fcp-events-server",
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "Starting FCP Events Server"
    );

    fcp_core::config::check_stage(validate_listener_auth(&config), cli.check_config)?;
    if cli.check_config {
        println!(
            "{}",
            fcp_core::config::masked_toml(&config).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }

    // Whether this server delivers emergency notifications: it is asked
    // to, and it has somewhere to deliver to (`docs/E911_BLOCK_2.md` §8).
    let notifications_wanted = config.notifications.enabled
        && (config.notifications.has_channel() || config.webhook.enabled);

    // Initialize PostgreSQL storage for the config store, webhooks and the
    // emergency notifier, which all read `webhook.database_url`.
    let storage_result = if config.webhook.enabled || notifications_wanted {
        // Webhooks on and no store to read them from would be webhooks
        // silently off: refuse to start instead, as every database service
        // does (docs/DEVELOPER_SURFACE.md §3, phase 5).
        Some(
            storage::initialize_storage(&config.webhook.database_url)
                .await
                .context("webhooks or notifications are on, and their store cannot be opened")?,
        )
    } else {
        None
    };
    let config_store: Option<Arc<dyn fcp_core::config_store::ConfigStore>> =
        storage_result.as_ref().map(|r| Arc::clone(&r.config_store));

    // Seed operational config sections into database (DB primary, TOML seed)
    // Bootstrap sections (service, logging) stay TOML-only.
    if let Some(ref cs) = config_store {
        let svc = "events-server";
        let tid = "_system";

        let sections: Vec<(&str, serde_json::Value)> = vec![
            (
                "websocket",
                serde_json::to_value(&config.websocket).unwrap_or_default(),
            ),
            (
                "event_bus",
                serde_json::to_value(&config.event_bus).unwrap_or_default(),
            ),
            (
                "limits",
                serde_json::to_value(&config.limits).unwrap_or_default(),
            ),
            (
                "upstream",
                serde_json::to_value(&config.upstream).unwrap_or_default(),
            ),
            (
                "webhook",
                serde_json::to_value(&config.webhook).unwrap_or_default(),
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
                        "websocket" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.websocket = v;
                            }
                        }
                        "event_bus" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.event_bus = v;
                            }
                        }
                        "limits" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.limits = v;
                            }
                        }
                        "upstream" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.upstream = v;
                            }
                        }
                        "webhook" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.webhook = v;
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
                            warn!(service = svc, section, error = %e, "failed to seed config")
                        }
                    }
                }
                Err(e) => {
                    warn!(service = svc, section, error = %e, "failed to check DB config, using TOML");
                }
            }
        }
        info!("Operational config loading complete (DB primary, TOML seed)");
    }

    validate_listener_auth(&config)?;
    let auth = build_auth_state(&config, storage_result.as_ref());

    // Create event bus. Events that originated in this region go on
    // `event_bus` (WebSocket clients, webhooks, state). Events a bridge
    // brought in from another region go on `remote_event_bus`: WebSocket
    // clients and state see them, webhooks do not — each event's webhooks
    // fire once, in the region it happened in (`docs/MULTI_REGION.md`).
    let event_bus = Arc::new(InMemoryEventBus::new());
    let remote_event_bus = Arc::new(InMemoryEventBus::new());
    let local_region = fcp_core::region::resolve_region(config.cluster.region.as_deref());

    // Initialize cluster membership (if configured)
    #[cfg(feature = "storage-redis")]
    let _cluster_manager = if config.cluster.enabled {
        let redis_url = config
            .cluster
            .redis_url
            .as_ref()
            .or(config.event_bus.redis_url.as_ref());
        if let Some(url) = redis_url {
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
                        http_address: config.websocket.bind_address,
                        sip_address: "0.0.0.0:0".parse().unwrap(),
                        sip_tls_address: None,
                        capabilities: vec!["events".to_string(), "websocket".to_string()],
                        active_calls: 0,
                        started_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        public_url: None,
                        capacity: None,
                        draining: false,
                        region: fcp_cluster::resolve_region(config.cluster.region.as_deref()),
                        video_budget: Vec::new(),
                        registering_trunks: Vec::new(),
                        view: None,
                    };
                    let cm = fcp_cluster::ClusterManager::new(redis, member).with_intervals(
                        config.cluster.heartbeat_interval_secs,
                        config.cluster.heartbeat_ttl_secs,
                    );
                    let _handle = cm.start();
                    info!("Cluster membership started");
                    Some(cm)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to connect to Redis; cluster disabled");
                    None
                }
            }
        } else {
            warn!("cluster.enabled = true but no redis_url configured; cluster disabled");
            None
        }
    } else {
        None
    };

    // Create connection manager
    let manager = Arc::new(ConnectionManager::new(
        config.websocket.heartbeat_interval,
        config.websocket.client_timeout,
        config.limits.max_connections,
        config.limits.max_connections_per_tenant,
    ));

    // Create call state store, registration store, and node registry
    let call_store = Arc::new(CallStateStore::new());
    let registration_store = Arc::new(RegistrationStateStore::new());
    let node_registry = Arc::new(NodeRegistry::new());

    // Create webhook dispatcher if enabled (requires PostgreSQL storage)
    let webhook_dispatcher = match storage_result.as_ref() {
        Some(storage) if config.webhook.enabled => {
            info!("Webhook dispatching enabled, initializing...");

            // Re-use the already-initialized PostgreSQL storage for webhooks
            let webhook_store: Arc<dyn fcp_core::storage::WebhookStore> =
                storage.pg_storage.clone();

            let dispatcher_config = WebhookDispatcherConfig {
                request_timeout: Duration::from_secs(config.webhook.request_timeout_secs),
                max_retries: config.webhook.max_retries,
                base_retry_delay: Duration::from_secs(config.webhook.base_retry_delay_secs),
                max_retry_delay: Duration::from_secs(config.webhook.max_retry_delay_secs),
                max_concurrent: config.webhook.max_concurrent,
                retry_interval: Duration::from_secs(config.webhook.retry_interval_secs),
                allow_private_targets: config.webhook.allow_private_targets,
            };
            if config.webhook.allow_private_targets {
                warn!("[webhook] allow_private_targets: webhooks may reach loopback and private addresses");
            }

            let dispatcher = Arc::new(WebhookDispatcher::new(webhook_store, dispatcher_config));

            // Spawn event consumer that dispatches to webhooks
            fcp_webhooks::spawn_webhook_consumer(
                event_bus.clone() as Arc<dyn EventSubscriber>,
                dispatcher.clone(),
            );

            // Spawn retry processor
            fcp_webhooks::dispatcher::spawn_retry_processor(dispatcher.clone());

            info!("Webhook dispatcher initialized");
            Some(dispatcher)
        }
        _ => {
            info!("Webhook dispatching disabled");
            None
        }
    };

    // On-site notification of emergency calls (Kari's Law,
    // `docs/E911_BLOCK_2.md` §8): the notifier consumes `call.emergency`
    // beside the webhook dispatcher.
    let notifier = match storage_result.as_ref() {
        Some(storage) if notifications_wanted => {
            match build_notifier(
                &config.notifications,
                storage,
                event_bus.clone(),
                webhook_dispatcher.clone(),
            )
            .await
            {
                Ok(notifier) => {
                    let notifier = Arc::new(notifier);
                    fcp_notify::spawn_event_consumer(
                        event_bus.clone() as Arc<dyn EventSubscriber>,
                        Arc::clone(&notifier),
                    );
                    fcp_notify::spawn_retry_processor(
                        Arc::clone(&notifier),
                        Duration::from_secs(config.notifications.retry_interval_secs.max(1)),
                    );
                    info!(
                        channels = ?notifier.kinds().iter().map(|k| k.to_string()).collect::<Vec<_>>(),
                        "Emergency notifier started"
                    );
                    Some(notifier)
                }
                Err(e) => {
                    // A notifier that cannot start must be loud: without it
                    // nobody is told an emergency call was placed.
                    error!(error = %e, "Emergency notifier not started");
                    None
                }
            }
        }
        _ => {
            if notifications_wanted {
                warn!(
                    "Emergency notifications need PostgreSQL storage (webhook.database_url); nobody will be told about emergency calls"
                );
            } else {
                info!(
                    "Emergency notifications not delivered from this server: configure [notifications.smtp], [notifications.sms] or [notifications.call], or enable webhooks"
                );
            }
            None
        }
    };

    // Cross-region event bridge (set below when peer regions are configured).
    #[cfg(feature = "redis-events")]
    let mut event_bridge: Option<Arc<fcp_storage_redis::event_bus::EventBridge>> = None;
    // Where /ingest writes when the bus is Redis (set below).
    #[cfg(feature = "redis-events")]
    let mut ingest_bus: Option<Arc<fcp_storage_redis::event_bus::RedisEventBus>> = None;

    // Start Redis stream consumer if configured
    #[cfg(feature = "redis-events")]
    if config.event_bus.bus_type == "redis" {
        if let Some(ref redis_url) = config.event_bus.redis_url {
            let redis_config = fcp_storage_redis::RedisConfig {
                url: redis_url.clone(),
                key_prefix: config.event_bus.redis_key_prefix.clone(),
            };

            match fcp_storage_redis::RedisStorage::new(redis_config).await {
                Ok(redis_storage) => {
                    let stream_key = format!("{}event_stream", config.event_bus.redis_key_prefix);
                    let consumer = fcp_storage_redis::event_bus::RedisStreamConsumer::new(
                        redis_storage.clone(),
                        stream_key,
                        config.event_bus.consumer_group.clone(),
                        config.event_bus.consumer_name.clone(),
                    );

                    // Ensure consumer group exists
                    if let Err(e) = consumer.ensure_group().await {
                        error!(error = %e, "Failed to create Redis consumer group");
                    } else {
                        let event_bus_redis = event_bus.clone();
                        let remote_bus_redis = remote_event_bus.clone();
                        let call_store_redis = call_store.clone();
                        let registration_store_redis = registration_store.clone();
                        let node_registry_redis = node_registry.clone();

                        tokio::spawn(async move {
                            redis_stream_consumer_task(
                                consumer,
                                event_bus_redis,
                                remote_bus_redis,
                                call_store_redis,
                                registration_store_redis,
                                node_registry_redis,
                            )
                            .await;
                        });
                        info!("Redis stream consumer started; HTTP /ingest writes to the stream");
                    }

                    // What /ingest posts goes on the stream in this
                    // server's region, under the posting node's id.
                    ingest_bus = Some(Arc::new(
                        fcp_storage_redis::event_bus::RedisEventBus::new(
                            redis_storage.clone(),
                            config.service.node_id.clone(),
                        )
                        .with_region(local_region.clone()),
                    ));

                    // Cross-region fan-out: forward this region's events to
                    // every configured peer region.
                    if !config.event_bus.peer_regions.is_empty() {
                        let peers = config
                            .event_bus
                            .peer_regions
                            .iter()
                            .map(|p| fcp_storage_redis::event_bus::PeerRegion {
                                region: p.region.clone(),
                                redis_url: p.redis_url.clone(),
                                key_prefix: p
                                    .key_prefix
                                    .clone()
                                    .unwrap_or_else(|| config.event_bus.redis_key_prefix.clone()),
                            })
                            .collect::<Vec<_>>();
                        let bridge = Arc::new(fcp_storage_redis::event_bus::EventBridge::new(
                            redis_storage.clone(),
                            local_region.clone(),
                            config.event_bus.consumer_name.clone(),
                            peers,
                        ));
                        let _bridge_handles = bridge.spawn();
                        info!(
                            local_region = local_region.as_deref().unwrap_or("-"),
                            peers = ?config.event_bus.peer_regions.iter().map(|p| p.region.as_str()).collect::<Vec<_>>(),
                            "Cross-region event bridge started"
                        );
                        event_bridge = Some(bridge);
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to connect to Redis for event streaming");
                }
            }
        } else {
            warn!("event_bus.bus_type = \"redis\" but redis_url is not set");
        }
    }

    // Create application state
    let state = AppState {
        config: config.clone(),
        auth,
        manager: manager.clone(),
        event_bus: event_bus.clone(),
        call_store: call_store.clone(),
        registration_store: registration_store.clone(),
        node_registry: node_registry.clone(),
        webhook_dispatcher: webhook_dispatcher.clone(),
        local_region: local_region.clone(),
        #[cfg(feature = "redis-events")]
        bridge: event_bridge.clone(),
        #[cfg(feature = "redis-events")]
        ingest_bus: ingest_bus.clone(),
    };

    // Build router with new endpoints
    let protected_routes = Router::new()
        .route("/status", get(status_handler))
        .route("/ws/:tenant_id", get(websocket_handler))
        .route("/ingest", post(ingest_handler))
        .route("/ingest/batch", post(ingest_batch_handler))
        .route("/calls", get(list_calls_handler))
        .route("/calls/:call_id", get(get_call_handler))
        .route("/registrations", get(list_registrations_handler))
        .route("/nodes", get(list_nodes_handler))
        .route("/stats", get(stats_handler))
        .with_state(state.clone());

    // Webhooks are managed on the admin API, tenant by tenant
    // (docs/DEVELOPER_SURFACE.md §3, phase 5); this server delivers them.
    let protected_routes = protected_routes.route_layer(middleware::from_fn_with_state(
        state.clone(),
        require_api_token,
    ));

    // What the notifier has done, for Prometheus (§9). Scraped without a
    // token, like /health.
    let metrics_state = notifier.as_ref().map(|n| n.metrics());
    let app = Router::new()
        .route("/health", get(health_handler))
        .route(
            "/metrics",
            get({
                let bridge_state = state.clone();
                move || {
                    let metrics = metrics_state.clone();
                    let state = bridge_state.clone();
                    async move {
                        let mut body = match metrics {
                            Some(metrics) => metrics.prometheus(),
                            None => String::new(),
                        };
                        // What crosses to each peer region
                        // (docs/MULTI_REGION_FOLLOWUPS.md §2.4).
                        body.push_str(&bridge_prometheus(&state));
                        body
                    }
                }
            }),
        )
        .merge(protected_routes)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // Start background tasks
    let manager_clone = manager.clone();
    tokio::spawn(async move {
        cleanup_task(manager_clone).await;
    });

    let event_bus_clone = event_bus.clone();
    let remote_bus_clone = remote_event_bus.clone();
    let manager_clone = manager.clone();
    let call_store_broadcast = call_store.clone();
    tokio::spawn(async move {
        event_broadcast_task(
            event_bus_clone,
            remote_bus_clone,
            manager_clone,
            call_store_broadcast,
        )
        .await;
    });

    // Spawn stale cleanup task
    let call_store_cleanup = call_store.clone();
    let registration_store_cleanup = registration_store.clone();
    let node_registry_cleanup = node_registry.clone();
    tokio::spawn(async move {
        stale_cleanup_task(
            call_store_cleanup,
            registration_store_cleanup,
            node_registry_cleanup,
        )
        .await;
    });

    // Spawn config poll loop (checks DB every 10s for config changes)
    if let Some(cs) = config_store {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_version: i64 = 0;
            loop {
                interval.tick().await;
                match cs.get_latest_version("events-server", "_system").await {
                    Ok(v) if v > last_version => {
                        info!(
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

    // Start server
    let bind_addr = config.websocket.bind_address;
    info!(bind_address = %bind_addr, "Starting WebSocket server");

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("Failed to bind to {}", bind_addr))?;

    info!(bind_address = %bind_addr, "WebSocket server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("Server error")?;

    // Shutdown cluster manager
    #[cfg(feature = "storage-redis")]
    if let Some(ref cm) = _cluster_manager {
        cm.shutdown().await;
    }

    info!("FCP Events Server shutdown complete");
    Ok(())
}

/// Initialize tracing/logging
fn init_logging(level: &str, format: &str) -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .context("Invalid log level")?;

    match format {
        "json" => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json())
                .init();
        }
        "pretty" => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().pretty())
                .init();
        }
        "compact" => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().compact())
                .init();
        }
        _ => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer())
                .init();
        }
    }

    Ok(())
}

fn validate_listener_auth(config: &EventsConfig) -> Result<()> {
    if !config.websocket.bind_address.ip().is_loopback()
        && config
            .auth
            .token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .is_none()
    {
        anyhow::bail!(
            "Refusing to bind fcp-events-server to {} without auth.token configured",
            config.websocket.bind_address
        );
    }

    Ok(())
}

fn build_auth_state(
    config: &EventsConfig,
    storage_result: Option<&storage::StorageResult>,
) -> EventsAuthState {
    let expected_token = config
        .auth
        .token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned);

    let api_token_store = storage_result.map(|storage| {
        Arc::new(fcp_storage_postgres::api_token::PostgresApiTokenStore::new(
            storage.pg_storage.pool().clone(),
        )) as Arc<dyn ApiTokenStore>
    });
    let credential_store =
        storage_result.map(|storage| storage.pg_storage.clone() as Arc<dyn CredentialStore>);
    let login_sessions = storage_result.map(|storage| {
        Arc::new(
            fcp_storage_postgres::login_session::PostgresLoginSessionStore::new(
                storage.pg_storage.pool().clone(),
            ),
        ) as Arc<dyn fcp_core::session::LoginSessionStore>
    });
    let session_keys = fcp_core::session::SessionKeys::from_config(
        config
            .auth
            .session_signing_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty()),
        &config.auth.previous_session_signing_keys,
        expected_token.as_deref(),
    );

    EventsAuthState {
        expected_token,
        api_token_store,
        credential_store,
        session_keys,
        login_sessions,
    }
}

fn extract_bearer_token(header_value: &str) -> Option<&str> {
    header_value
        .strip_prefix("Bearer ")
        .or_else(|| header_value.strip_prefix("bearer "))
        .map(str::trim)
}

fn extract_token_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(extract_bearer_token)
        .map(ToOwned::to_owned)
}

fn extract_query_token(uri: &axum::http::Uri) -> Option<String> {
    let query = uri.query()?;

    url::form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned())
}

async fn authenticate_presented_token(
    auth: &EventsAuthState,
    token: &str,
) -> Option<AuthPrincipal> {
    if let Some(expected_token) = auth.expected_token.as_deref() {
        if fcp_core::secret::constant_time_eq(token, expected_token) {
            return Some(AuthPrincipal::StaticAdmin);
        }
    }

    if token.starts_with("fcp_") {
        if let Some(store) = auth.api_token_store.as_ref() {
            // Open, not revoked, and its maker still active
            // (docs/DEVELOPER_SURFACE.md §3, phase 1); a personal token
            // reaches only its maker's `/me` routes on the admin API.
            let users = auth.credential_store.as_deref();
            let fcp_core::AuthenticatedApiToken { token, .. } =
                fcp_core::authenticate_api_token(store.as_ref(), users, token).await?;
            if token.scope != fcp_core::ApiTokenScope::Admin {
                return None;
            }

            let store = Arc::clone(store);
            let token_id = token.id;
            tokio::spawn(async move {
                if let Err(error) = store.update_last_used(&token_id).await {
                    debug!(error = %error, "Failed to update API token last_used_at");
                }
            });

            return Some(AuthPrincipal::ApiToken {
                tenant_id: token.tenant_id,
                permissions: token.permissions,
            });
        }
    }

    validate_session_token(auth, token).await
}

async fn authenticate_request_token(
    auth: &EventsAuthState,
    headers: &axum::http::HeaderMap,
    uri: &axum::http::Uri,
) -> Option<AuthPrincipal> {
    let token = extract_token_from_headers(headers).or_else(|| extract_query_token(uri))?;
    authenticate_presented_token(auth, &token).await
}

async fn validate_session_token(auth: &EventsAuthState, token: &str) -> Option<AuthPrincipal> {
    let keys = auth.session_keys.as_ref()?;
    let now = std::time::SystemTime::now();
    // With the users to hand, the shared check: the session live, the user
    // active, the role as it is now (`docs/IDENTITY_AND_DEVICES.md` phase 1).
    if let Some(users) = auth.credential_store.as_ref() {
        let principal = fcp_core::session::authenticate_session(
            token,
            keys,
            users.as_ref(),
            auth.login_sessions.as_deref(),
            now,
        )
        .await?;
        let user = principal.user;
        return Some(AuthPrincipal::Session {
            _username: user.username,
            role: serde_json::to_value(user.role)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
            tenant_id: user.tenant_id,
        });
    }
    // Without them, the token's own word, as before.
    let claims = keys.decode(token, now).ok()?;
    Some(AuthPrincipal::Session {
        _username: claims.username.unwrap_or(claims.sub),
        role: claims.role,
        tenant_id: claims.tenant_id.parse::<TenantId>().ok()?,
    })
}

async fn require_api_token(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    if !state.auth.auth_enabled() {
        request.extensions_mut().insert(AuthPrincipal::StaticAdmin);
        return next.run(request).await;
    }

    if let Some(principal) =
        authenticate_request_token(&state.auth, request.headers(), request.uri()).await
    {
        if !principal.has_operator_access() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Operator access required"})),
            )
                .into_response();
        }

        request.extensions_mut().insert(principal);
        return next.run(request).await;
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "Invalid or missing authentication token"})),
    )
        .into_response()
}

/// Health check endpoint
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Status endpoint with connection statistics
async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let call_stats = state.call_store.get_stats();
    let node_stats = state.node_registry.get_stats();

    let status = serde_json::json!({
        "status": "operational",
        "service": "fcp-events-server",
        "version": env!("CARGO_PKG_VERSION"),
        "node_id": state.config.service.node_id,
        "connections": {
            "total": state.manager.connection_count(),
            "max": state.config.limits.max_connections,
        },
        "calls": {
            "active": call_stats.initiated + call_stats.ringing + call_stats.answered,
            "total_tracked": call_stats.total,
        },
        "nodes": {
            "total": node_stats.total,
            "healthy": node_stats.healthy,
        },
        "region": {
            "local": state.local_region,
            "bridge": bridge_status(&state),
        },
        "config": {
            "heartbeat_interval_secs": state.config.websocket.heartbeat_interval.as_secs(),
            "client_timeout_secs": state.config.websocket.client_timeout.as_secs(),
            "batch_size": state.config.websocket.batch_size,
        }
    });

    Json(status)
}

/// The per-peer bridge counters for `/metrics`; empty without a bridge.
#[cfg(feature = "redis-events")]
fn bridge_prometheus(state: &AppState) -> String {
    let region = state
        .local_region
        .clone()
        .unwrap_or_default()
        .replace('"', "");
    let labels = if region.is_empty() {
        String::new()
    } else {
        format!("region=\"{region}\"")
    };
    fcp_storage_redis::event_bus::peers_prometheus(&bridge_status(state), &labels)
}

#[cfg(not(feature = "redis-events"))]
fn bridge_prometheus(_state: &AppState) -> String {
    String::new()
}

/// Per-peer bridge status for `/status`; empty without a bridge.
#[cfg(feature = "redis-events")]
fn bridge_status(state: &AppState) -> Vec<fcp_storage_redis::event_bus::PeerBridgeStatus> {
    state
        .bridge
        .as_ref()
        .map(|b| b.status())
        .unwrap_or_default()
}

#[cfg(not(feature = "redis-events"))]
fn bridge_status(_state: &AppState) -> Vec<serde_json::Value> {
    Vec::new()
}

/// WebSocket upgrade handler
async fn websocket_handler(
    ws: WebSocketUpgrade,
    Path(tenant_id_str): Path<String>,
    Query(query): Query<WebSocketAuthQuery>,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let tenant_id = match tenant_id_str.parse::<TenantId>() {
        Ok(id) => id,
        Err(e) => {
            error!(tenant_id = %tenant_id_str, error = %e, "Invalid tenant ID");
            return (StatusCode::BAD_REQUEST, "Invalid tenant ID").into_response();
        }
    };

    let mut allow_all_tenants = false;
    if state.auth.auth_enabled() {
        let provided_token = extract_token_from_headers(&headers).or(query.token);
        match provided_token {
            Some(token) => {
                match authenticate_presented_token(&state.auth, &token).await {
                    Some(principal) if principal.has_operator_access() => {
                        if let Some(scope) = principal.tenant_scope() {
                            if scope != &tenant_id {
                                return (
                                    StatusCode::FORBIDDEN,
                                    Json(serde_json::json!({"error": "Tenant scope mismatch"})),
                                )
                                    .into_response();
                            }
                        } else {
                            allow_all_tenants = true;
                        }
                    }
                    Some(_) => {
                        return (
                            StatusCode::FORBIDDEN,
                            Json(serde_json::json!({"error": "Operator access required"})),
                        )
                            .into_response();
                    }
                    None => {
                        return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"error": "Invalid or missing authentication token"})),
                    )
                        .into_response();
                    }
                }
            }
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "Invalid or missing authentication token"})),
                )
                    .into_response();
            }
        }
    }

    info!(tenant_id = %tenant_id, "WebSocket upgrade request");

    // Create event channel for this connection
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Register connection
    let conn_id = match state
        .manager
        .register(tenant_id, allow_all_tenants, event_tx)
    {
        Ok(id) => id,
        Err(e) => {
            error!(tenant_id = %tenant_id, error = %e, "Failed to register connection");
            return (StatusCode::SERVICE_UNAVAILABLE, e).into_response();
        }
    };

    // Upgrade to WebSocket
    ws.on_upgrade(move |socket| {
        handle_websocket(
            socket,
            conn_id,
            tenant_id,
            state.manager,
            event_rx,
            state.config.websocket.batch_size,
            state.config.websocket.batch_timeout,
        )
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
struct WebSocketAuthQuery {
    token: Option<String>,
}

/// The header a posting node names itself in. Without it an ingested
/// event is filed under `ingest`.
const NODE_ID_HEADER: &str = "x-node-id";

fn posting_node(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(NODE_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Ingest events another service posted (a conference server, IVR,
/// voicemail server or the admin). With Redis as the bus they go on the
/// stream under the posting node's id, and this server sees them again
/// through its consumer like any other event — once, and so does every
/// other events server here and every peer region
/// (`docs/DEPLOYMENT_GAPS.md` §1.6). With the in-memory bus they reach
/// this process only. A stream that cannot be written is `503`, so the
/// poster's retry carries the event rather than this server dropping it.
async fn ingest_events(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    events: Vec<PlatformEvent>,
) -> StatusCode {
    #[cfg(feature = "redis-events")]
    if let Some(bus) = &state.ingest_bus {
        let node_id = posting_node(headers).unwrap_or_else(|| "ingest".to_string());
        // Events a site sent up its link name the site
        // (`docs/HYBRID_CONTROL_PLANE.md` §3.3) and fire here as this
        // region's own.
        let site = headers
            .get(fcp_events::ORIGIN_SITE_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        return match bus
            .publish_many_from_site(&node_id, site.as_deref(), events)
            .await
        {
            Ok(()) => StatusCode::ACCEPTED,
            Err(e) => {
                warn!(error = %e, node_id = %node_id, "Ingested events could not be written to the stream");
                StatusCode::SERVICE_UNAVAILABLE
            }
        };
    }

    let node_id = posting_node(headers);
    for event in events {
        process_event_for_state(
            &event,
            &state.call_store,
            &state.registration_store,
            &state.node_registry,
            node_id.as_deref(),
        );
        let _ = state.event_bus.publish(event).await;
    }
    StatusCode::ACCEPTED
}

/// Ingest a single event from another service.
async fn ingest_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(event): Json<PlatformEvent>,
) -> impl IntoResponse {
    ingest_events(&state, &headers, vec![event]).await
}

/// Ingest a batch of events from another service.
async fn ingest_batch_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(events): Json<Vec<PlatformEvent>>,
) -> impl IntoResponse {
    ingest_events(&state, &headers, events).await
}

/// Query parameters for listing calls
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ListCallsQuery {
    /// Filter by node ID
    node_id: Option<String>,
    /// Include ended calls (default: false)
    #[serde(default)]
    include_ended: bool,
}

/// List active calls endpoint
async fn list_calls_handler(
    State(state): State<AppState>,
    Query(query): Query<ListCallsQuery>,
) -> impl IntoResponse {
    let calls = if let Some(ref node_id) = query.node_id {
        state.call_store.get_active_calls_by_node(node_id)
    } else {
        state.call_store.get_active_calls()
    };

    Json(serde_json::json!({
        "calls": calls,
        "count": calls.len(),
    }))
}

/// Get a specific call by ID
async fn get_call_handler(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> impl IntoResponse {
    match state.call_store.get_call(&call_id) {
        Some(call) => Json(serde_json::json!({"call": call})).into_response(),
        None => (StatusCode::NOT_FOUND, "Call not found").into_response(),
    }
}

/// List all active registrations (aggregated from all call-manager nodes)
async fn list_registrations_handler(State(state): State<AppState>) -> impl IntoResponse {
    let registrations = state.registration_store.get_all_registrations();
    Json(serde_json::json!({
        "registrations": registrations,
        "count": registrations.len(),
    }))
}

/// List registered nodes endpoint
async fn list_nodes_handler(State(state): State<AppState>) -> impl IntoResponse {
    let nodes = state.node_registry.get_all_nodes();
    Json(serde_json::json!({
        "nodes": nodes,
        "count": nodes.len(),
    }))
}

/// Aggregated statistics endpoint
async fn stats_handler(State(state): State<AppState>) -> impl IntoResponse {
    let call_stats = state.call_store.get_stats();
    let node_stats = state.node_registry.get_stats();

    Json(serde_json::json!({
        "calls": {
            "total": call_stats.total,
            "active": call_stats.initiated + call_stats.ringing + call_stats.answered,
            "by_state": {
                "initiated": call_stats.initiated,
                "ringing": call_stats.ringing,
                "answered": call_stats.answered,
                "ended": call_stats.ended,
                "failed": call_stats.failed,
            }
        },
        "nodes": {
            "total": node_stats.total,
            "healthy": node_stats.healthy,
            "degraded": node_stats.degraded,
            "offline": node_stats.offline,
            "total_calls": node_stats.total_calls,
        },
        "websocket_connections": state.manager.connection_count(),
    }))
}

/// Process an event for state tracking (call store, registration store, and node registry).
fn process_event_for_state(
    event: &PlatformEvent,
    call_store: &CallStateStore,
    registration_store: &RegistrationStateStore,
    node_registry: &NodeRegistry,
    source_node_id: Option<&str>,
) {
    match event {
        PlatformEvent::Call(call_event) => {
            // Extract node_id and update node registry
            let node_id = extract_node_id_from_call_event(call_event);
            if let Some(ref node_id) = node_id {
                node_registry.register_or_update(node_id, Some(NodeType::CallManager));
            }

            // Update call counts based on event type
            match call_event {
                CallEvent::Initiated {
                    node_id: Some(ref nid),
                    ..
                } => {
                    node_registry.increment_calls(nid);
                }
                CallEvent::Ended {
                    node_id: Some(ref nid),
                    ..
                }
                | CallEvent::Failed {
                    node_id: Some(ref nid),
                    ..
                } => {
                    node_registry.decrement_calls(nid);
                }
                _ => {}
            }

            // Process event in call store
            call_store.process_event(call_event);
        }
        PlatformEvent::Registration(reg_event) => {
            // Process registration event
            registration_store.process_event(reg_event, source_node_id);
            debug!(event = ?reg_event, "Processed registration event");
        }
        PlatformEvent::System(system_event) => {
            match system_event {
                SystemEvent::Started { node_id, .. } => {
                    info!(node_id = %node_id, "Node registered via SystemEvent::Started");
                    node_registry.register_or_update(node_id, Some(NodeType::CallManager));
                }
                SystemEvent::Stopping { node_id, .. } => {
                    info!(node_id = %node_id, "Node stopping");
                    // Node will be marked offline via stale check
                }
                SystemEvent::HealthCheck {
                    node_id, healthy, ..
                } => {
                    debug!(node_id = %node_id, healthy = %healthy, "Health check received");
                    if *healthy {
                        node_registry.register_or_update(node_id, Some(NodeType::CallManager));
                    }
                }
                SystemEvent::ConfigChanged { .. } => {
                    // Config changes are handled by call-manager nodes via cache invalidation
                }
                SystemEvent::NodeJoined { node_id, .. } => {
                    info!(node_id = %node_id, "Node joined cluster");
                    node_registry.register_or_update(node_id, Some(NodeType::CallManager));
                }
                SystemEvent::NodeLeft { node_id, .. } => {
                    info!(node_id = %node_id, "Node left cluster");
                }
                SystemEvent::NodeDraining {
                    node_id,
                    calls,
                    wait_secs,
                    ..
                } => {
                    info!(
                        node_id = %node_id,
                        calls = %calls,
                        wait_secs = %wait_secs,
                        "Node draining"
                    );
                }
                SystemEvent::NodeDrained {
                    node_id,
                    calls_ended,
                    waited_secs,
                    ..
                } => {
                    info!(
                        node_id = %node_id,
                        calls_ended = %calls_ended,
                        waited_secs = %waited_secs,
                        "Node drained"
                    );
                }
                SystemEvent::ClusterViewChanged {
                    node_id,
                    state,
                    summary,
                    ..
                } => {
                    // Said, not acted on (docs/MULTI_REGION_FOLLOWUPS.md §2.3).
                    info!(node_id = %node_id, state = %state, "Node's cluster view changed: {summary}");
                }
                SystemEvent::VersionSkew {
                    node_id,
                    skew,
                    summary,
                    ..
                } => {
                    // Said, not acted on (docs/BACKUP_AND_RESTORE.md §5).
                    info!(node_id = %node_id, skew = %skew, "Cluster releases changed: {summary}");
                }
                SystemEvent::SiteLinkChanged {
                    site_id, name, up, ..
                } => {
                    info!(site_id = %site_id, name = %name, up = %up, "A site's link changed");
                }
            }
        }
        _ => {
            // Other events don't affect state tracking
        }
    }
}

/// The notifier this deployment delivers with: the channels its
/// configuration gives it, and the retry policy it was given
/// (`docs/E911_BLOCK_2.md` §8.3). Credentials sealed with encryption at
/// rest are opened here.
async fn build_notifier(
    config: &config::NotificationsConfig,
    storage: &storage::StorageResult,
    events: Arc<dyn fcp_events::EventBus>,
    webhooks: Option<Arc<fcp_webhooks::dispatcher::WebhookDispatcher>>,
) -> anyhow::Result<fcp_notify::EmergencyNotifier> {
    use fcp_core::encryption::EncryptionStore;

    let store: Arc<dyn fcp_core::e911::EmergencyStore> = Arc::new(
        fcp_storage_postgres::emergency::PostgresEmergencyStore::new(
            storage.pg_storage.pool().clone(),
        ),
    );
    let open = |value: Option<String>| {
        let storage = Arc::clone(&storage.pg_storage);
        async move {
            match value.filter(|v| !v.is_empty()) {
                Some(value) if value.starts_with("enc:") => storage
                    .open_secret(&value)
                    .await
                    .map(Some)
                    .context("Failed to open a sealed notifications credential"),
                other => Ok(other),
            }
        }
    };

    let policy = fcp_notify::RetryPolicy {
        max_attempts: config.max_attempts.max(1),
        base_delay: Duration::from_secs(config.base_retry_delay_secs.max(1)),
        max_delay: Duration::from_secs(config.max_retry_delay_secs.max(1)),
        give_up_after: Duration::from_secs(config.give_up_after_secs.max(1)),
    };
    let mut notifier = fcp_notify::EmergencyNotifier::new(store, policy)
        .with_events(events as Arc<dyn EventPublisher>);

    if let Some(smtp) = &config.smtp {
        let settings = fcp_notify::email::SmtpSettings {
            host: smtp.host.clone(),
            port: smtp.port,
            encryption: smtp.encryption.clone(),
            username: smtp.username.clone(),
            password: open(smtp.password.clone()).await?,
            from: smtp.from.clone(),
            timeout: Duration::from_secs(smtp.timeout_secs.max(1)),
        };
        notifier =
            notifier.with_channel(Arc::new(fcp_notify::email::EmailChannel::new(&settings)?));
    }
    if let Some(sms) = &config.sms {
        let settings = fcp_notify::sms::SmsSettings {
            url: sms.url.clone(),
            token: open(sms.token.clone()).await?,
            from: sms.from.clone(),
            timeout: Duration::from_secs(sms.timeout_secs.max(1)),
        };
        notifier = notifier.with_channel(Arc::new(fcp_notify::sms::SmsChannel::new(settings)?));
    }
    if let Some(call) = &config.call {
        let settings = fcp_notify::call::CallSettings {
            call_manager_url: call.call_manager_url.clone(),
            auth_token: open(call.auth_token.clone()).await?,
            from: call.from.clone(),
            ring_timeout: Duration::from_secs(call.ring_timeout_secs.max(1)),
            repeat: call.repeat.max(1),
            // The call rings, is answered and is spoken to, so it takes
            // longer than the ringing alone.
            timeout: Duration::from_secs(call.ring_timeout_secs.max(1) + 60),
        };
        notifier = notifier.with_channel(Arc::new(fcp_notify::call::CallChannel::new(settings)?));
    }
    if let Some(dispatcher) = webhooks {
        notifier = notifier.with_channel(Arc::new(fcp_notify::webhook::WebhookChannel::new(
            dispatcher,
        )));
    }
    if notifier.kinds().is_empty() {
        anyhow::bail!(
            "notifications.enabled is true but no channel is configured: add [notifications.smtp],              [notifications.sms] or [notifications.call], or enable webhooks"
        );
    }
    Ok(notifier)
}

/// Extract node_id from a CallEvent (if present).
fn extract_node_id_from_call_event(event: &CallEvent) -> Option<String> {
    match event {
        CallEvent::Initiated { node_id, .. }
        | CallEvent::Ringing { node_id, .. }
        | CallEvent::Progress { node_id, .. }
        | CallEvent::Answered { node_id, .. }
        | CallEvent::Ended { node_id, .. }
        | CallEvent::Failed { node_id, .. } => node_id.clone(),
        CallEvent::Emergency { node_id, .. }
        | CallEvent::EmergencyCompleted { node_id, .. }
        | CallEvent::EmergencyNotificationFailed { node_id, .. } => node_id.clone(),
        CallEvent::Parked { .. }
        | CallEvent::Unparked { .. }
        | CallEvent::Held { .. }
        | CallEvent::Resumed { .. }
        | CallEvent::Transcript { .. }
        | CallEvent::PartyJoined { .. }
        | CallEvent::PartyLeft { .. } => None,
    }
}

/// Background task to broadcast events from both event buses (this
/// region's and what bridges brought in from others) to connections.
async fn event_broadcast_task(
    event_bus: Arc<InMemoryEventBus>,
    remote_event_bus: Arc<InMemoryEventBus>,
    manager: Arc<ConnectionManager>,
    call_store: Arc<CallStateStore>,
) {
    use fcp_events::{EventFilter, EventSubscriber};

    // Subscribe to all events on both buses
    let mut local = match event_bus.subscribe(EventFilter::new()).await {
        Ok(stream) => stream,
        Err(e) => {
            error!(error = %e, "Failed to subscribe to event bus");
            return;
        }
    };
    let mut remote = match remote_event_bus.subscribe(EventFilter::new()).await {
        Ok(stream) => stream,
        Err(e) => {
            error!(error = %e, "Failed to subscribe to remote event bus");
            return;
        }
    };

    info!("Event broadcast task started");

    loop {
        let next = tokio::select! {
            e = local.next() => e,
            e = remote.next() => e,
        };
        match next {
            Some(event) => {
                let event_tenant = resolve_event_tenant_id(&event, &call_store);
                // Broadcast to WebSocket connections
                // (State tracking is done in ingest_handler to avoid double-processing)
                manager.publish(event, event_tenant.as_ref());
            }
            None => {
                error!("Event stream ended unexpectedly");
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
    }
}

fn resolve_event_tenant_id(event: &PlatformEvent, call_store: &CallStateStore) -> Option<TenantId> {
    if let Some(tenant_id) = event.tenant_id() {
        return Some(*tenant_id);
    }

    let call_id = event.call_id()?;
    call_store
        .get_call(&call_id.to_string())
        .and_then(|call| call.tenant_id.parse::<TenantId>().ok())
}

/// Background task to clean up stale calls, expired registrations, and check node health.
async fn stale_cleanup_task(
    call_store: Arc<CallStateStore>,
    registration_store: Arc<RegistrationStateStore>,
    node_registry: Arc<NodeRegistry>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));

    info!("Stale cleanup task started (30s interval)");

    loop {
        interval.tick().await;

        // Check for stale nodes
        let offline_nodes = node_registry.check_stale_nodes();
        if offline_nodes > 0 {
            warn!(count = offline_nodes, "Nodes marked offline");
        }

        // Clean up ended calls older than 5 minutes
        let removed_calls = call_store.cleanup_stale(Duration::from_secs(300));
        if removed_calls > 0 {
            debug!(count = removed_calls, "Cleaned up stale ended calls");
        }

        // Clean up expired registrations
        let expired_regs = registration_store.cleanup_expired();
        if expired_regs > 0 {
            debug!(count = expired_regs, "Cleaned up expired registrations");
        }

        // Optionally remove nodes that have been offline for more than 10 minutes
        let removed_nodes = node_registry.remove_stale_nodes(Duration::from_secs(600));
        if removed_nodes > 0 {
            info!(count = removed_nodes, "Removed stale offline nodes");
        }
    }
}

/// Background task that reads events from a Redis Stream (consumer group).
///
/// This replaces the HTTP `/ingest` endpoint for clustered deployments.
/// Events published by any fcp-call-manager node are durably stored in the
/// Redis Stream and consumed here with at-least-once delivery.
///
/// Entries a bridge relayed from another region go to `remote_event_bus`
/// (WebSocket clients and state, no webhooks); everything else to
/// `event_bus`.
#[cfg(feature = "redis-events")]
async fn redis_stream_consumer_task(
    consumer: fcp_storage_redis::event_bus::RedisStreamConsumer,
    event_bus: Arc<InMemoryEventBus>,
    remote_event_bus: Arc<InMemoryEventBus>,
    call_store: Arc<CallStateStore>,
    registration_store: Arc<RegistrationStateStore>,
    node_registry: Arc<NodeRegistry>,
) {
    info!("Redis stream consumer task running");

    loop {
        match consumer.read_entries(100, 2000).await {
            Ok(entries) => {
                if entries.is_empty() {
                    continue;
                }

                let mut ack_ids = Vec::with_capacity(entries.len());

                for entry in entries {
                    let relayed = entry.envelope.relayed;
                    let node_id = entry.envelope.node_id;
                    let event = entry.envelope.event;
                    // Process event for state tracking, under the node
                    // that published it.
                    process_event_for_state(
                        &event,
                        &call_store,
                        &registration_store,
                        &node_registry,
                        Some(&node_id),
                    );

                    // Publish for WebSocket broadcast (and webhooks, if local)
                    let bus = if relayed {
                        &remote_event_bus
                    } else {
                        &event_bus
                    };
                    let _ = bus.publish(event).await;

                    ack_ids.push(entry.id);
                }

                // Acknowledge processed events
                if let Err(e) = consumer.ack(&ack_ids).await {
                    error!(error = %e, "Failed to acknowledge Redis stream events");
                } else {
                    debug!(count = ack_ids.len(), "Acknowledged Redis stream events");
                }
            }
            Err(e) => {
                error!(error = %e, "Redis stream read failed, retrying in 2s...");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// Graceful shutdown signal handler
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C signal");
        }
        _ = terminate => {
            info!("Received SIGTERM signal");
        }
    }

    info!("Initiating graceful shutdown");
}
