//! fcp-presence-server — Standalone SIP presence server.
//!
//! Deploys presence independently of fcp-call-manager. Tracks user availability
//! via SUBSCRIBE/NOTIFY/PUBLISH, manages watcher subscriptions, and provides
//! a REST API for presence management.

// `fcp_presence::webhook` is deprecated in favour of `fcp-webhooks`; this server
// has not been migrated yet, so it still uses the older in-crate dispatcher.
#![allow(deprecated)]
use anyhow::{Context, Result};
use clap::Parser;
use fcp_events::filter::EventType;
use fcp_events::{EventFilter, EventPublisher, EventSubscriber, InMemoryEventBus};
use fcp_sip::exports::IntegratedUAC;
use fcp_sip::listener::{InboundMessageHandler, ListenerConfig, TransportListener};
use fcp_sip::presence::{PresenceHandler, SipPresenceHandler};
use fcp_sip::transport::ConnectionPool;
use fcp_sip::uac::SipUserAgentClient;
use fcp_sip::{B2buaConfig, CallManager};
use sip_dns::SipResolver;
use sip_observe::{set_transport_metrics, TracingTransportMetrics};
use sip_transaction::TransactionManager;
use sip_transport::TransportKind as SipTransportKind;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

mod config;
mod http_api;
mod xcap_server;

use config::PresenceServerConfig;

/// CLI arguments for fcp-presence-server.
#[derive(Parser)]
#[command(
    name = "fcp-presence",
    version,
    about = "Standalone SIP presence server"
)]
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

    if config.sip.local_ip.is_none() {
        let bind_unspecified = config
            .sip
            .udp_bind
            .parse::<std::net::SocketAddr>()
            .map(|a| a.ip().is_unspecified())
            .unwrap_or(true);
        if bind_unspecified {
            if let Some(detected_ip) = auto_detect_local_ip() {
                config.sip.local_ip = Some(detected_ip);
            }
        }
    }

    config::validate_config(&config)?;

    // Initialize tracing
    init_tracing(&config);

    if !set_transport_metrics(Arc::new(TracingTransportMetrics)) {
        warn!("transport metrics already configured");
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-presence starting"
    );

    // Initialize storage
    let StorageResult {
        dyn_storage,
        config_store,
        history_store,
    } = initialize_storage(&config).await?;

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

    // Create SIP infrastructure for SUBSCRIBE/PUBLISH/NOTIFY.
    let dialog_manager = Arc::new(fcp_sip::exports::DialogManager::new());
    let tcp_pool = Arc::new(ConnectionPool::new());
    let (client_tx, client_rx) = tokio::sync::mpsc::channel(1024);
    tcp_pool.set_inbound_tx(client_tx).await;

    let dispatcher = create_transport_dispatcher(&config, Arc::clone(&tcp_pool)).await?;
    let transaction_manager = Arc::new(TransactionManager::new_with_pool(
        dispatcher.clone(),
        tcp_pool,
    ));
    let uac = create_uac(&config, Arc::clone(&transaction_manager), dispatcher).await?;

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
                        sip_address: config
                            .sip
                            .udp_bind
                            .parse()
                            .unwrap_or_else(|_| "0.0.0.0:5060".parse().unwrap()),
                        sip_tls_address: None,
                        capabilities: vec!["presence".to_string(), "blf".to_string()],
                        active_calls: 0,
                        started_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        draining: false,
                        region: fcp_cluster::resolve_region(config.cluster.region.as_deref()),
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

    let call_manager = Arc::new(CallManager::with_storage_and_events(
        Arc::clone(&dyn_storage),
        Arc::clone(&event_bus) as Arc<dyn EventPublisher>,
    ));
    let local_uri = create_local_uri(&config)?;
    let local_addr = derive_local_addr(&config)?;
    let concrete_presence_handler = Arc::new({
        let handler = SipPresenceHandler::new(
            manager.clone(),
            Arc::clone(&uac),
            config.service.default_tenant_id,
            config.presence.default_subscription_expiry_secs,
        );
        if config.sip.auth_enabled {
            handler.with_digest_auth(
                Arc::clone(&dyn_storage) as Arc<dyn fcp_core::CredentialStore>,
                config.sip.realm.clone(),
            )
        } else {
            handler
        }
    });
    let presence_handler: Arc<dyn fcp_sip::presence::PresenceHandler> =
        Arc::clone(&concrete_presence_handler) as Arc<dyn fcp_sip::presence::PresenceHandler>;
    let handler = Arc::new(
        fcp_sip::handler::SipMessageHandler::new(
            Arc::clone(&transaction_manager),
            Arc::clone(&dialog_manager),
            local_uri,
            None,
            local_addr,
            10000,
            None,
            None,
            call_manager,
            Some(Arc::clone(&uac)),
            Some(Arc::clone(&dyn_storage)),
            None,
            None,
            None,
            B2buaConfig::default(),
            None,
            None,
            None,
            None,
            Some(Arc::clone(&presence_handler)),
            None,
            config.service.node_id.clone(),
        )?
        .with_default_tenant_id(config.service.default_tenant_id),
    );

    let udp_bind: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;
    let tcp_bind: std::net::SocketAddr = config
        .sip
        .tcp_bind
        .parse()
        .context("Invalid TCP bind address")?;
    let ws_bind = config
        .sip
        .ws_bind
        .as_deref()
        .map(|bind| bind.parse().context("Invalid WS bind address"))
        .transpose()?;
    let wss_bind = config
        .sip
        .wss_bind
        .as_deref()
        .map(|bind| bind.parse().context("Invalid WSS bind address"))
        .transpose()?;
    let listener_config = ListenerConfig {
        udp_bind: Some(udp_bind),
        tcp_bind: Some(tcp_bind),
        tls_bind: None,
        tls_cert: config.sip.tls_cert_path.clone(),
        tls_key: config.sip.tls_key_path.clone(),
        tls_client_ca: None,
        tls_client_auth: None,
        ws_bind,
        wss_bind,
    };
    let handler_dyn: Arc<dyn InboundMessageHandler> =
        Arc::clone(&handler) as Arc<dyn InboundMessageHandler>;
    let mut listener = TransportListener::new(listener_config, handler_dyn);
    listener
        .start()
        .await
        .context("Failed to start transport listeners")?;

    let handler_clone = Arc::clone(&handler);
    let mut client_rx = client_rx;
    tokio::spawn(async move {
        while let Some(packet) = client_rx.recv().await {
            let txn_transport = match packet.transport() {
                SipTransportKind::Udp => sip_transaction::TransportKind::Udp,
                SipTransportKind::Tcp => sip_transaction::TransportKind::Tcp,
                SipTransportKind::Tls => sip_transaction::TransportKind::Tls,
                SipTransportKind::Ws => sip_transaction::TransportKind::Ws,
                SipTransportKind::Wss => sip_transaction::TransportKind::Wss,
                SipTransportKind::Sctp => sip_transaction::TransportKind::Sctp,
                SipTransportKind::TlsSctp => sip_transaction::TransportKind::TlsSctp,
            };

            if let Err(e) = handler_clone
                .handle_inbound(
                    txn_transport,
                    packet.peer(),
                    packet.payload().clone(),
                    packet.stream().cloned(),
                    packet.peer_identity().cloned(),
                )
                .await
            {
                tracing::error!("Failed to handle inbound TCP response: {}", e);
            }
        }
    });

    // Redis clustering (if configured)
    #[cfg(feature = "redis")]
    if let Some(ref redis_config) = config.redis {
        setup_redis(&mut manager, redis_config, &config.service.node_id).await?;
    }

    // Start presence analytics consumer using durable shared storage.
    let _analytics_handle = fcp_presence::analytics::spawn_analytics_consumer(
        event_bus.clone() as Arc<dyn fcp_events::EventSubscriber>,
        Arc::clone(&history_store),
    );

    // Start webhook dispatcher (if configured and webhook feature enabled)
    #[cfg(feature = "webhook")]
    let webhook_dispatcher = {
        let dispatcher = if config.webhooks.as_ref().is_some_and(|w| w.enabled) {
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

    // Start background expiry cleanup and NOTIFY retry processing.
    let cleanup_manager = manager.clone();
    let cleanup_interval = config.presence.cleanup_interval_secs;
    let cleanup_presence_handler = Arc::clone(&presence_handler);
    let cleanup_concrete_handler = Arc::clone(&concrete_presence_handler);
    let mut shutdown_rx = manager.shutdown_receiver();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(cleanup_interval));
        let mut retry_interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let expired = cleanup_manager.cleanup_expired_watchers();
                    if !expired.is_empty() {
                        if let Err(e) = cleanup_presence_handler
                            .send_terminated_notifies(&expired, "timeout")
                            .await
                        {
                            warn!(error = %e, "Failed to send terminated NOTIFYs for expired watchers");
                        }
                    }
                    cleanup_manager.cleanup_expired_subscriptions_from_store().await;
                    cleanup_manager.check_publish_expiry_warnings().await;
                    cleanup_manager.cleanup_expired_publishes().await;
                }
                _ = retry_interval.tick() => {
                    cleanup_concrete_handler.process_retry_queue().await;
                }
                _ = shutdown_rx.changed() => {
                    break;
                }
            }
        }
    });

    // Send NOTIFYs whenever presence state changes.
    {
        let event_bus_notify = Arc::clone(&event_bus);
        let notifier_handler = Arc::clone(&presence_handler);
        tokio::spawn(async move {
            run_presence_notifier(event_bus_notify, notifier_handler).await;
        });
    }

    // Start HTTP API
    let app_state = Arc::new(http_api::AppState {
        manager: manager.clone(),
        sip_handler: Some(Arc::clone(&concrete_presence_handler)),
        start_time: Instant::now(),
        api_token: config.http.api_token.clone(),
        resource_list_store: Arc::clone(dyn_storage.resource_list_store()),
        tenant_store: Arc::clone(&dyn_storage) as Arc<dyn fcp_core::storage::TenantStore>,
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
        udp = %config.sip.udp_bind,
        tcp = %config.sip.tcp_bind,
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
    let all_watchers = manager.get_all_watchers_including_winfo();
    if !all_watchers.is_empty() {
        if let Err(e) = concrete_presence_handler
            .send_terminated_notifies(&all_watchers, "probation")
            .await
        {
            warn!(error = %e, "Failed to send shutdown NOTIFYs");
        }
    }
    manager.shutdown().await;
    listener.stop().await?;
    http_handle.abort();
    info!("Shutdown complete");

    Ok(())
}

/// Storage initialization result with both DynStorage and ConfigStore.
struct StorageResult {
    dyn_storage: Arc<fcp_core::storage::DynStorage>,
    config_store: Arc<dyn fcp_core::config_store::ConfigStore>,
    history_store: Arc<dyn fcp_core::storage::PresenceHistoryStore>,
}

/// Initialize PostgreSQL storage backend based on configuration.
async fn initialize_storage(config: &PresenceServerConfig) -> Result<StorageResult> {
    use fcp_storage_postgres::{PostgresConfig, PostgresStorage};

    let pg_config = PostgresConfig {
        connection_string: config.storage.database_url.clone(),
        read_replica_url: config.storage.read_replica_url.clone(),
        max_replica_lag: Duration::from_secs(config.storage.max_replica_lag_secs),
        max_connections: config.storage.max_connections,
        connection_timeout: Duration::from_secs(30),
        encryption: None,
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
        config_store: storage.clone(),
        history_store: storage,
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

/// Create a transport dispatcher with a shared TCP pool.
async fn create_transport_dispatcher(
    config: &PresenceServerConfig,
    tcp_pool: Arc<ConnectionPool>,
) -> Result<Arc<fcp_sip::transport::RealTransportDispatcher>> {
    use fcp_sip::transport::RealTransportDispatcher;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .context("Failed to create UDP socket")?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&bind_addr.into())?;
    let std_socket: std::net::UdpSocket = socket.into();
    let udp_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    let dispatcher = RealTransportDispatcher::new_with_pool(Some(Arc::new(udp_socket)), tcp_pool);
    Ok(Arc::new(dispatcher))
}

/// Create a UAC for outbound NOTIFY delivery.
async fn create_uac(
    config: &PresenceServerConfig,
    transaction_manager: Arc<TransactionManager>,
    dispatcher: Arc<fcp_sip::transport::RealTransportDispatcher>,
) -> Result<Arc<SipUserAgentClient>> {
    let local_uri = create_local_uri(config)?;
    let local_addr = derive_local_addr(config)?;
    let resolver = Arc::new(SipResolver::from_system().context("Failed to create DNS resolver")?);

    let mut builder = IntegratedUAC::builder();
    builder = builder
        .local_uri(local_uri.as_str())?
        .local_addr(&local_addr)?
        .via_advertised_addr(&local_addr)?
        .transaction_manager(transaction_manager)
        .resolver(resolver)
        .dispatcher(dispatcher);

    let integrated_uac = builder.build().context("Failed to build IntegratedUAC")?;
    Ok(Arc::new(SipUserAgentClient::new(Arc::new(integrated_uac))))
}

fn create_local_uri(config: &PresenceServerConfig) -> Result<fcp_sip::exports::SipUri> {
    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    if let Some(ref uri_str) = config.sip.local_uri {
        let uri = fcp_sip::exports::Uri::parse(uri_str)
            .context("Failed to parse configured local_uri")?;
        return uri
            .as_sip()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Configured local_uri is not a SIP URI"));
    }

    let domain = if bind_addr.ip().is_unspecified() {
        config
            .sip
            .local_ip
            .as_ref()
            .filter(|ip| !ip.is_empty())
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind_addr.ip().to_string()
    };

    let uri_str = if bind_addr.port() != 0 && bind_addr.port() != 5060 {
        format!("sip:presence@{}:{}", domain, bind_addr.port())
    } else {
        format!("sip:presence@{}", domain)
    };

    let uri = fcp_sip::exports::Uri::parse(&uri_str).context("Failed to parse local URI")?;
    uri.as_sip()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

fn derive_local_addr(config: &PresenceServerConfig) -> Result<String> {
    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;
    let port = if bind_addr.port() == 0 {
        5060
    } else {
        bind_addr.port()
    };

    let host = if let Some(ref local_ip) = config.sip.local_ip {
        local_ip.clone()
    } else if bind_addr.ip().is_unspecified() {
        auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind_addr.ip().to_string()
    };

    Ok(format!("{}:{}", host, port))
}

fn auto_detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:53").ok()?;
    let addr = socket.local_addr().ok()?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    Some(addr.ip().to_string())
}

async fn run_presence_notifier(
    event_bus: Arc<InMemoryEventBus>,
    presence_handler: Arc<dyn fcp_sip::presence::PresenceHandler>,
) {
    use fcp_events::PlatformEvent;

    let filter = EventFilter::new().types(vec![EventType::Presence]);
    let mut stream = match event_bus.subscribe(filter.clone()).await {
        Ok(stream) => stream,
        Err(e) => {
            tracing::error!(%e, "Failed to subscribe to event bus for presence NOTIFY");
            return;
        }
    };

    loop {
        match stream.next().await {
            Some(event) => match &event {
                PlatformEvent::Presence(fcp_events::PresenceEvent::StatusChanged {
                    aor, ..
                })
                | PlatformEvent::Presence(fcp_events::PresenceEvent::DialogChanged {
                    aor, ..
                }) => {
                    if let Err(e) = presence_handler.notify_watchers(aor).await {
                        warn!(aor = %aor, error = %e, "Failed to notify watchers");
                    }
                }
                _ => {}
            },
            None => {
                warn!("Presence NOTIFY stream ended, resubscribing...");
                tokio::time::sleep(Duration::from_millis(100)).await;
                match event_bus.subscribe(filter.clone()).await {
                    Ok(new_stream) => stream = new_stream,
                    Err(e) => warn!(error = %e, "Failed to resubscribe presence NOTIFY stream"),
                }
            }
        }
    }
}
