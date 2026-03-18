//! fcp-call-manager — Combined SIP registrar and B2BUA call handler.
//!
//! Supports two operating modes:
//! - **Call Manager mode** (default): SIP registrar + B2BUA for user devices
//! - **Session Manager mode**: SIP trunk termination + call routing (no registrar)
//!
//! Mode is determined by configuration: when `registrar.enabled = false` and
//! trunks are configured, the binary operates as a session manager.
//!
//! This binary wires up existing fcp-sip components (SipMessageHandler,
//! TransportListener, FcpRegistrar) with storage, config, and an HTTP API.

use anyhow::{Context, Result};
use clap::Parser;
use fcp_core::storage::{DeviceStore, ExtensionStore, LineBindingStore, PhoneNumberStore};
use fcp_events::{EventFilter, EventPublisher, EventSubscriber, InMemoryEventBus};
use fcp_events::filter::EventType;
use fcp_media::{BackendType, MediaConfig};
use fcp_routing::{RoutingService, YamlRoutingStore};
use fcp_sbc_core::{CpsChecker, NoCpsLimit, TrunkCredentialProvider, TrunkHealthMonitor, TrunkRegistrar, TrunkRouter};
use fcp_sip::flow_handler::FlowCallHandler;
use fcp_sip::prelude::*;
use fcp_voicemail::{VoicemailConfig, VoicemailService};
use fcp_sip::service::{DialogService, SubscriptionService, TransactionService};
use fcp_sip::transport::ConnectionPool;
use sip_observe::{set_transport_metrics, TracingTransportMetrics};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

mod cli;
mod config;
mod health;
mod queue_audio;
mod queue_handler;
mod storage;

use cli::Cli;
use config::CallManagerConfig;
use fcp_sip::presence::{extract_aor, SipPresenceHandler};

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI arguments
    let cli = Cli::parse();

    // 2. Load configuration
    let mut config = config::load_config(cli.config.as_deref())?;

    // Apply CLI overrides
    if let Some(ref level) = cli.log_level {
        config.logging.level = level.clone();
    }
    if let Some(ref format) = cli.log_format {
        config.logging.format = format.clone();
    }
    if let Some(ref bind) = cli.sip_bind {
        config.sip.udp_bind = bind.clone();
        config.sip.tcp_bind = bind.clone();
    }
    if let Some(ref bind) = cli.http_bind {
        config.http.bind_address = bind
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid HTTP bind address '{}': {}", bind, e))?;
    }
    // Only apply public_ip override if it's non-empty
    // (clap treats empty env vars as Some("") instead of None)
    if let Some(ref ip) = cli.public_ip {
        if !ip.is_empty() {
            config.sip.local_ip = Some(ip.clone());
            config.media.public_ip = Some(ip.clone());
        }
    }
    // Apply TLS/SIPS CLI overrides
    if let Some(ref sips_bind) = cli.sips_bind {
        config.sip.sips_bind = sips_bind.clone();
    }
    if let Some(ref tls_cert) = cli.tls_cert {
        config.sip.tls_cert_path = Some(tls_cert.display().to_string());
    }
    if let Some(ref tls_key) = cli.tls_key {
        config.sip.tls_key_path = Some(tls_key.display().to_string());
    }

    // Auto-detect local IP if not explicitly configured and bind is 0.0.0.0
    // This prevents using unroutable addresses (0.0.0.0, 127.0.0.1) in SIP
    // messages when running inside Docker or binding to all interfaces.
    if config.sip.local_ip.is_none() {
        let bind_unspecified = config
            .sip
            .udp_bind
            .parse::<std::net::SocketAddr>()
            .map(|a| a.ip().is_unspecified())
            .unwrap_or(true);
        if bind_unspecified {
            if let Some(detected_ip) = auto_detect_local_ip() {
                config.sip.local_ip = Some(detected_ip.clone());
                if config.media.public_ip.is_none() {
                    config.media.public_ip = Some(detected_ip);
                }
            }
        }
    }

    // 3. Initialize tracing
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-call-manager starting"
    );

    if let Some(ref ip) = config.sip.local_ip {
        info!(
            local_ip = %ip,
            source = if cli.public_ip.as_ref().map_or(false, |p| !p.is_empty()) { "PUBLIC_IP env" } else { "auto-detected" },
            "Using local IP for SIP signaling and media"
        );
    }

    // Set up observability
    if !set_transport_metrics(Arc::new(TracingTransportMetrics)) {
        warn!("transport metrics already configured");
    }

    // 4. Initialize storage
    let storage_result = storage::initialize_storage(&config.storage).await?;
    let config_store = storage_result.config_store;
    let pg_flow_store = storage_result.flow_store;
    #[allow(unused_mut)]
    let mut dyn_storage = storage_result.dyn_storage;

    // 4b. Initialize Redis + split backend (if configured)
    #[cfg(feature = "storage-redis")]
    let redis_storage = {
        let rs = storage::initialize_redis(&config.storage, &config.cluster).await?;

        // Register Prometheus metrics for Redis operations
        if rs.is_some() {
            fcp_storage_redis::metrics::register_metrics();
        }

        // Swap registration store to Redis if configured
        if config.storage.registration_backend.as_deref() == Some("redis") {
            if let Some(ref redis) = rs {
                dyn_storage = storage::apply_redis_registration_store(dyn_storage, redis);
            } else {
                warn!("registration_backend = \"redis\" but redis_url is not set; using primary backend");
            }
        }

        rs
    };

    // 4b2. Wrap hot-path stores in DashMap caches for fast lookups
    let cache_ttl = Duration::from_secs(60);
    let (cached_storage, cached_creds, cached_exts, cached_devs) =
        storage::apply_cached_stores(dyn_storage, cache_ttl);
    let dyn_storage = cached_storage;

    // 4b3. Create event bus early (needed by cluster membership and registrar)
    let event_bus = Arc::new(InMemoryEventBus::new());

    // 4c. Initialize cluster membership (if configured)
    #[cfg(feature = "storage-redis")]
    let _cluster_manager = if config.cluster.enabled {
        if let Some(ref redis) = redis_storage {
            let member = fcp_cluster::ClusterMember {
                node_id: config.service.node_id.clone(),
                node_type: config.service.service_type.clone(),
                http_address: config.http.bind_address,
                sip_address: config.sip.udp_bind.parse().unwrap_or_else(|_| "0.0.0.0:5060".parse().unwrap()),
                capabilities: vec!["registrar".to_string(), "b2bua".to_string()],
                active_calls: 0,
                started_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            };

            let cm = fcp_cluster::ClusterManager::new(redis.clone(), member)
                .with_intervals(config.cluster.heartbeat_interval_secs, config.cluster.heartbeat_ttl_secs)
                .with_event_publisher(Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>);
            let _handle = cm.start();
            info!("Cluster membership started");

            let call_registry = fcp_cluster::CallRegistry::with_ttl(
                redis.clone(),
                config.service.node_id.clone(),
                config.cluster.call_ttl_secs,
            );
            Some((cm, call_registry))
        } else {
            warn!("cluster.enabled = true but redis_url is not set; cluster disabled");
            None
        }
    } else {
        None
    };

    // 4d. Create cluster parking store (if Redis is available and parking is enabled)
    #[cfg(feature = "storage-redis")]
    let cluster_parking: Option<Arc<dyn fcp_core::ClusterParkingStore>> =
        if let Some(ref redis) = redis_storage {
            let parking_timeout = config.pbx.parking.timeout_seconds;
            let store = fcp_storage_redis::parking::RedisParkingStore::new(
                redis.clone(),
                parking_timeout,
            );
            info!("Cluster parking store initialized (Redis)");
            Some(Arc::new(store) as Arc<dyn fcp_core::ClusterParkingStore>)
        } else {
            None
        };

    #[cfg(not(feature = "storage-redis"))]
    let cluster_parking: Option<Arc<dyn fcp_core::ClusterParkingStore>> = None;

    // 4e. Seed operational config sections into database (DB primary, TOML seed)
    // Bootstrap sections (service, sip, storage, http, logging, cluster) stay TOML-only.
    {
        let cs = config_store.as_ref();
        let svc = "call-manager";
        let tid = "_system";

        let sections: Vec<(&str, serde_json::Value)> = vec![
            ("registrar", serde_json::to_value(&config.registrar).unwrap_or_default()),
            ("calls", serde_json::to_value(&config.calls).unwrap_or_default()),
            ("media", serde_json::to_value(&config.media).unwrap_or_default()),
            ("routing", serde_json::to_value(&config.routing).unwrap_or_default()),
            ("events", serde_json::to_value(&config.events).unwrap_or_default()),
            ("b2bua", serde_json::to_value(&config.b2bua).unwrap_or_default()),
            ("pbx", serde_json::to_value(&config.pbx).unwrap_or_default()),
            ("queue", serde_json::to_value(&config.queue).unwrap_or_default()),
            ("presence", serde_json::to_value(&config.presence).unwrap_or_default()),
        ];

        for (section, toml_val) in sections {
            // Check if DB has this section; if not, seed it
            match cs.get_config(svc, section, tid).await {
                Ok(Some(entry)) => {
                    // DB has it — use DB version to override local config
                    match section {
                        "registrar" => if let Ok(v) = serde_json::from_value(entry.value) { config.registrar = v; },
                        "calls" => if let Ok(v) = serde_json::from_value(entry.value) { config.calls = v; },
                        "media" => if let Ok(v) = serde_json::from_value(entry.value) { config.media = v; },
                        "routing" => if let Ok(v) = serde_json::from_value(entry.value) { config.routing = v; },
                        "events" => if let Ok(v) = serde_json::from_value(entry.value) { config.events = v; },
                        "b2bua" => if let Ok(v) = serde_json::from_value(entry.value) { config.b2bua = v; },
                        "pbx" => if let Ok(v) = serde_json::from_value(entry.value) { config.pbx = v; },
                        "queue" => if let Ok(v) = serde_json::from_value(entry.value) { config.queue = v; },
                        "presence" => if let Ok(v) = serde_json::from_value(entry.value) { config.presence = v; },
                        _ => {}
                    }
                    info!(service = svc, section, version = entry.version, "loaded config from database");
                }
                Ok(None) => {
                    // Seed from TOML
                    match cs.set_config(svc, section, tid, toml_val, Some("seed")).await {
                        Ok(version) => info!(service = svc, section, version, "seeded config into database"),
                        Err(e) => warn!(service = svc, section, error = %e, "failed to seed config"),
                    }
                }
                Err(e) => {
                    warn!(service = svc, section, error = %e, "failed to check DB config, using TOML");
                }
            }
        }
        // Fix stale PBX config: if DB had pbx.enabled=false from pre-v0.23.0 seed,
        // correct it so feature codes (*98 etc.) work.
        if !config.pbx.enabled {
            config.pbx.enabled = true;
            let _ = cs
                .set_config(
                    svc,
                    "pbx",
                    tid,
                    serde_json::to_value(&config.pbx).unwrap_or_default(),
                    Some("fix-pbx-default"),
                )
                .await;
            info!("PBX enabled flag corrected in database");
        }

        info!("Operational config loading complete (DB primary, TOML seed)");
    }

    // 5. Create SIP components
    let dialog_manager = DialogManager::new();
    let subscription_manager = SubscriptionManager::new();

    // 6. Create shared TCP connection pool
    let tcp_pool = Arc::new(ConnectionPool::new());

    // Set up TCP response routing
    let (client_tx, client_rx) = tokio::sync::mpsc::channel(1024);
    tcp_pool.set_inbound_tx(client_tx).await;

    // 7. Create transport dispatcher
    let dispatcher = create_transport_dispatcher(&config, Arc::clone(&tcp_pool)).await?;

    // 8. Create transaction manager with shared pool
    let transaction_manager = TransactionManager::new_with_pool(dispatcher.clone(), tcp_pool);

    // 9. Create services
    let transaction_service = TransactionService::new(transaction_manager);
    let dialog_service = DialogService::new(dialog_manager);
    let _subscription_service = SubscriptionService::new(subscription_manager);

    let transaction_manager_ref = Arc::clone(transaction_service.manager());
    let dialog_manager_ref = Arc::clone(dialog_service.manager());

    // 10. Build credential provider from trunks (if any) so UAC can handle 401/407 challenges
    let trunk_cred_provider: Option<Arc<TrunkCredentialProvider>> = if !config.trunks.is_empty() {
        Some(Arc::new(TrunkCredentialProvider::from_trunks(&config.trunks)))
    } else {
        None
    };

    // 11. Create UAC for outbound calls (B-leg)
    let dispatcher_for_flow = dispatcher.clone();
    let uac = create_uac(
        &config,
        Arc::clone(&transaction_manager_ref),
        dispatcher,
        trunk_cred_provider,
    )
    .await?;

    // 12. If Redis event bus is configured, bridge local ↔ cluster events
    #[cfg(feature = "storage-redis")]
    let redis_event_publisher: Option<Arc<dyn EventPublisher>> = if config.events.use_redis {
        if let Some(ref redis) = redis_storage {
            let redis_bus = fcp_storage_redis::event_bus::RedisEventBus::new(
                redis.clone(),
                config.service.node_id.clone(),
            );

            // Start subscriber: listens for remote events on Redis Pub/Sub,
            // filters self-published, and re-publishes to local InMemoryEventBus.
            let _sub_handle = redis_bus.start_subscriber(Arc::clone(&event_bus));

            info!("Redis event bus initialized (Pub/Sub + Streams)");
            Some(Arc::new(redis_bus) as Arc<dyn EventPublisher>)
        } else {
            warn!("events.use_redis = true but redis_url is not set; falling back to in-memory");
            None
        }
    } else {
        None
    };

    // Choose the event publisher: Redis if available, otherwise in-memory
    #[cfg(feature = "storage-redis")]
    let event_publisher: Arc<dyn EventPublisher> = redis_event_publisher
        .unwrap_or_else(|| Arc::clone(&event_bus) as Arc<dyn EventPublisher>);

    #[cfg(not(feature = "storage-redis"))]
    let event_publisher: Arc<dyn EventPublisher> = Arc::clone(&event_bus) as Arc<dyn EventPublisher>;

    info!("Event bus initialized");

    // 13. Create FcpRegistrar (conditional — disabled in session-manager mode)
    let registrar = if config.registrar.enabled {
        Some(create_registrar(
            &config,
            &dyn_storage,
            Arc::clone(&event_publisher),
        ))
    } else {
        info!("Registrar disabled (session-manager mode)");
        None
    };

    // 13b. Trunk infrastructure (session-manager mode)
    let trunk_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>;
    let outbound_route_provider: Option<Arc<dyn fcp_sip::OutboundRouteProvider>> =
        if !config.trunks.is_empty() {
            let health = Arc::new(TrunkHealthMonitor::new(&config.trunks));
            let cps: Arc<dyn CpsChecker> = Arc::new(NoCpsLimit);
            let router = Arc::new(TrunkRouter::new(
                config.trunks.clone(),
                Arc::clone(&health),
                cps,
            ));

            // Spawn health probes
            let health_clone = Arc::clone(&health);
            let trunks_clone = config.trunks.clone();
            let uac_clone = Arc::clone(&uac);
            tokio::spawn(fcp_sbc_core::run_health_probes(
                health_clone,
                trunks_clone,
                uac_clone,
            ));

            // Spawn trunk registration task
            let trunk_registrar = Arc::new(TrunkRegistrar::new(&config.trunks));
            if trunk_registrar.has_registerable_trunks() {
                let (tx, rx) = tokio::sync::watch::channel(false);
                let reg = Arc::clone(&trunk_registrar);
                let reg_health = Arc::clone(&health);
                let reg_uac = Arc::clone(&uac);
                tokio::spawn(fcp_sbc_core::run_trunk_registration(
                    reg, reg_health, reg_uac, rx,
                ));
                trunk_shutdown_tx = Some(tx);
            } else {
                trunk_shutdown_tx = None;
            }

            info!(count = config.trunks.len(), "Trunk routing enabled");
            Some(router as Arc<dyn fcp_sip::OutboundRouteProvider>)
        } else {
            trunk_shutdown_tx = None;
            None
        };

    // 13. Create routing service (if configured)
    let routing_service = if let Some(ref rules_file) = config.routing.rules_file {
        match YamlRoutingStore::new(PathBuf::from(rules_file), config.routing.hot_reload).await {
            Ok(store) => {
                info!(rules_file = %rules_file, hot_reload = config.routing.hot_reload, "Routing service initialized");
                Some(Arc::new(RoutingService::new(Arc::new(store))))
            }
            Err(e) => {
                warn!(%e, rules_file = %rules_file, "Failed to load routing rules, routing disabled");
                None
            }
        }
    } else {
        info!("No routing rules file configured, routing disabled");
        None
    };

    // 14. Create media manager
    let media_manager = create_media_manager(&config).await?;

    // 15. Create CallManager for B2BUA functionality (with event publishing)
    let mut call_manager_inner = fcp_sip::CallManager::with_storage_and_events(
        Arc::clone(&dyn_storage),
        Arc::clone(&event_publisher),
    );
    call_manager_inner.set_node_id(&config.service.node_id);
    let call_manager = Arc::new(call_manager_inner);

    // 15a. Wire cluster active-call counter to CallManager
    #[cfg(feature = "storage-redis")]
    if let Some((ref cm, _)) = _cluster_manager {
        let counter = cm.active_calls_counter();
        let call_mgr = Arc::clone(&call_manager);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                counter.store(
                    call_mgr.active_call_count(),
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
        });
        info!("Cluster active-call counter wired to CallManager");
    }

    // 16. Create PBX handler (if enabled)
    let pbx_handler: Option<Arc<fcp_pbx::PbxHandler>> = if config.pbx.enabled {
        let handler = Arc::new(fcp_pbx::PbxHandler::new(config.pbx.clone()));
        info!(
            hunt_groups = config.pbx.hunt_groups.len(),
            pickup_groups = config.pbx.pickup_groups.len(),
            parking = config.pbx.parking.enabled,
            cmc = config.pbx.cmc.enabled,
            fac = config.pbx.fac.enabled,
            "PBX features enabled"
        );
        Some(handler)
    } else {
        None
    };

    // 17. Create SipMessageHandler (wires all components together)
    let local_uri = create_local_uri(&config)?;
    let media_profile = if config.calls.enable_sdp {
        Some(MediaProfileBuilder::audio_only())
    } else {
        None
    };
    let local_addr = config
        .sip
        .local_ip
        .clone()
        .or_else(|| config.media.public_ip.clone())
        .unwrap_or_else(|| derive_local_ip(&config.sip.udp_bind));

    // 17a. Create QueueManager and QueueCallHandler (if queue system is enabled)
    let queue_manager: Option<Arc<fcp_queue::QueueManager>> = if config.queue.enabled {
        let tenant_id = fcp_core::TenantId::nil(); // Default tenant for single-node
        let mut qm = fcp_queue::QueueManager::new(
            config.queue.clone(),
            Some(Arc::clone(&event_publisher)),
            tenant_id.clone(),
        );

        // Set up queue persistence and restore agents
        let queue_store: Arc<dyn fcp_core::storage::QueueStore> =
            Arc::clone(&dyn_storage) as _;
        qm.set_queue_store(Arc::clone(&queue_store));

        let qm = Arc::new(qm);

        // Restore agent state from storage
        match qm.restore_agents(queue_store.as_ref(), &tenant_id).await {
            Ok(count) if count > 0 => {
                info!(restored = count, "Restored agent state from storage");
            }
            Ok(_) => {}
            Err(e) => {
                warn!(error = %e, "Failed to restore agent state from storage");
            }
        }

        Some(qm)
    } else {
        None
    };

    // Start queue stats publisher (for wallboard WebSocket consumers)
    if let Some(ref qm) = queue_manager {
        if config.queue.stats_publish_interval_secs > 0 {
            let tenant_id = fcp_core::TenantId::nil();
            qm.start_stats_publisher(config.queue.stats_publish_interval_secs, tenant_id);
            info!(
                interval_secs = config.queue.stats_publish_interval_secs,
                "Queue stats publisher started"
            );
        }
    }

    let queue_call_handler: Option<Arc<queue_handler::QueueCallHandler>> =
        if let Some(ref qm) = queue_manager {
            let port_pool_config = forge_rtp::PortPoolConfig::new(
                config.media.rtp_port_start,
                config.media.rtp_port_end,
            )
            .context("Invalid RTP port pool configuration for queue handler")?;
            let queue_port_pool = Arc::new(forge_rtp::PortPool::new(port_pool_config));
            let handler = queue_handler::QueueCallHandler::new(
                Arc::clone(qm),
                config.queue.clone(),
                local_uri.as_str().to_string(),
                local_addr.clone(),
                queue_port_pool,
                Arc::clone(&event_publisher),
                Arc::clone(&uac),
                Arc::clone(&dyn_storage),
            );
            info!(
                queues = config.queue.queues.len(),
                "Queue/ACD handler enabled"
            );
            Some(Arc::new(handler))
        } else {
            None
        };
    let queue_handler: Option<Arc<dyn fcp_sip::queue::QueueHandler>> =
        queue_call_handler.as_ref().map(|h| Arc::clone(h) as Arc<dyn fcp_sip::queue::QueueHandler>);

    // 17b. Create PresenceManager (if enabled)
    let presence_manager: Option<fcp_presence::PresenceManager> = if config.presence.enabled {
        let mut pm = fcp_presence::PresenceManager::new(
            config.presence.clone(),
            Some(Arc::clone(&event_publisher)),
        );
        info!("Presence service enabled (in-memory)");

        // Wire up Redis Pub/Sub for cross-node presence propagation
        #[cfg(feature = "storage-redis")]
        if let Some(ref _redis) = redis_storage {
            if let Some(ref redis_url) = config.storage.redis_url {
                match fcp_presence::redis_pubsub::RedisPresencePubSub::new(
                    redis_url,
                    Some(config.service.node_id.clone()),
                ) {
                    Ok(pubsub) => {
                        info!(
                            node_id = %pubsub.node_id(),
                            "Presence Redis Pub/Sub initialized"
                        );
                        pm.set_pubsub(std::sync::Arc::new(pubsub));
                        match pm.run_pubsub_listener().await {
                            Ok(_handle) => info!("Presence Redis Pub/Sub listener started"),
                            Err(e) => warn!(error = %e, "Failed to start presence Pub/Sub listener"),
                        }
                    }
                    Err(e) => warn!(error = %e, "Failed to initialize presence Redis Pub/Sub"),
                }
            }
        }

        // Wire presence into queue distribution so DND/Busy agents are skipped
        if let Some(ref qm) = queue_manager {
            let checker = Arc::new(fcp_sip::presence_checker::PresenceCheckerAdapter::new(pm.clone()));
            qm.set_presence_checker(checker);
            info!("Queue agent selection will respect presence DND state");
        }

        Some(pm)
    } else {
        None
    };

    // 17c. Create SIP presence handler (if enabled)
    let concrete_presence_handler: Option<Arc<SipPresenceHandler>> =
        if let Some(ref pm) = presence_manager {
            let pres_tenant = fcp_core::TenantId::nil();
            let handler = SipPresenceHandler::new(
                pm.clone(),
                Arc::clone(&uac),
                pres_tenant,
                config.presence.default_subscription_expiry_secs,
            );
            Some(Arc::new(handler))
        } else {
            None
        };
    let presence_handler: Option<Arc<dyn fcp_sip::presence::PresenceHandler>> =
        concrete_presence_handler
            .as_ref()
            .map(|h| Arc::clone(h) as Arc<dyn fcp_sip::presence::PresenceHandler>);

    // 17c. Create VoicemailService for *98 voicemail access
    let voicemail_service = Arc::new(VoicemailService::new(
        Arc::clone(&dyn_storage) as Arc<dyn fcp_core::storage::VoicemailStore>,
        Arc::clone(&media_manager),
        VoicemailConfig::default(),
    ));

    // 17d. Create FlowCallHandler for IVR flows (*98 voicemail access, future IVR features)
    let flow_store: Arc<dyn fcp_flow::FlowStore> = pg_flow_store;
    let contact_uri = local_uri.clone();
    let mut flow_handler = FlowCallHandler::new(
        Arc::clone(&flow_store),
        Some(Arc::clone(&event_publisher)),
        Some(Arc::clone(&media_manager)),
    )
    .with_call_manager(Arc::clone(&call_manager))
    .with_transaction_manager(Arc::clone(&transaction_manager_ref))
    .with_dialog_manager(Arc::clone(&dialog_manager_ref))
    .with_uris(local_uri.clone(), contact_uri)
    .with_media_profile(
        MediaProfileBuilder::audio_only(),
        local_addr.clone(),
        config.media.rtp_port_start,
    )
    .with_voicemail_service(Arc::clone(&voicemail_service))
    .with_dispatcher(dispatcher_for_flow as Arc<dyn sip_transaction::TransportDispatcher>);

    // Wire registrar for Dial node extension resolution (direct to device, bypassing B2BUA loop)
    if let Some(ref reg) = registrar {
        flow_handler = flow_handler.with_registrar(Arc::clone(reg));
    }

    // Wire storage for extension → phone number → device → registrar resolution chain
    flow_handler = flow_handler.with_storage(Arc::clone(&dyn_storage));

    // Wire shared TTS service to FlowEngine for Gather Say prompts and Say node pre-synthesis
    if let Some(tts) = media_manager.tts_service() {
        info!("Shared TTS service wired to FlowEngine for Gather/Say nodes");
        flow_handler = flow_handler.with_tts_service(Arc::clone(tts));
    }

    // 17e. Create PromptService for system prompt resolution (WAV file → TTS fallback)
    let flow_handler = if let Some(prompt_store) = dyn_storage.prompt_store() {
        let default_tenant_id = fcp_core::TenantId::from_uuid(uuid::Uuid::nil());
        let prompt_service = Arc::new(fcp_core::PromptService::new(
            Arc::clone(prompt_store),
            default_tenant_id.clone(),
            "en-US",
        ));

        // Seed system prompts at startup
        let system_prompts = vec![
            ("vm_enter_pin", "PIN entry prompt", "Enter your PIN followed by pound."),
            ("vm_incorrect_pin", "Wrong PIN", "Incorrect PIN. Please try again."),
            ("vm_auth_failed", "Auth failure", "Authentication failed. Goodbye."),
            ("vm_post_message_menu", "Message controls", "Press 1 to replay. Press 2 to save. Press 3 to delete. Press 4 for next message. Press star for main menu."),
            ("vm_message_saved", "Confirmation", "Message saved."),
            ("vm_message_deleted", "Confirmation", "Message deleted."),
            ("vm_end_of_messages", "End of list", "End of messages."),
            ("vm_goodbye", "Goodbye", "Goodbye."),
        ];

        let mut seeded = 0u32;
        for (name, description, tts_text) in &system_prompts {
            // Only create the prompt record if it doesn't already exist —
            // re-seeding would overwrite the audio_path set by TTS pre-generation.
            if let Ok(Some(_existing)) = prompt_store
                .get_prompt_by_name_and_language(&default_tenant_id, name, "en-US")
                .await
            {
                continue;
            }
            let mut prompt = fcp_core::Prompt::new_tts(
                default_tenant_id.clone(),
                *name,
                fcp_core::PromptCategory::System,
                "en-US",
                *tts_text,
            );
            prompt.description = Some(description.to_string());
            prompt.is_system = true;
            if let Err(e) = prompt_store.put_prompt(prompt).await {
                warn!(name = name, error = %e, "Failed to seed system prompt");
            }
            seeded += 1;
        }
        if seeded > 0 {
            info!(seeded, "Seeded new system prompts");
        }

        // Pre-generate TTS audio for system prompts that don't have audio files yet
        if let Some(tts) = media_manager.tts_service() {
            let prompts_dir = PathBuf::from(&config.media.prompts_dir);
            if let Err(e) = pre_generate_prompt_audio(
                &prompt_service,
                &default_tenant_id,
                tts,
                config.media.tts_voice.as_deref(),
                &prompts_dir,
                &system_prompts,
            ).await {
                warn!(error = %e, "Failed to pre-generate prompt audio (TTS will be used at call-time)");
            }
        }

        flow_handler.with_prompt_service(prompt_service)
    } else {
        warn!("No prompt store configured — IVR will use TTS fallback only");
        flow_handler
    };

    // Wire flow trace store for persistent execution tracing
    let flow_handler = if let Some(trace_store) = dyn_storage.flow_trace_store() {
        info!("Flow execution tracing enabled (PostgreSQL-backed)");
        flow_handler.with_flow_trace_store(Arc::clone(trace_store))
    } else {
        flow_handler
    };

    let flow_handler = Arc::new(flow_handler);

    let mut handler = SipMessageHandler::new(
        Arc::clone(&transaction_manager_ref),
        Arc::clone(&dialog_manager_ref),
        local_uri,
        media_profile,
        local_addr,
        config.media.rtp_port_start,
        registrar.as_ref().map(Arc::clone),
        routing_service, // routing service from config
        Arc::clone(&call_manager),
        Some(Arc::clone(&uac)),
        Some(Arc::clone(&dyn_storage)),
        Some(Arc::clone(&media_manager)),
        Some(fcp_core::config::MediaConfig {

            enabled: true,
            rtp_port_start: config.media.rtp_port_start,
            rtp_port_end: config.media.rtp_port_end,
            public_ip: config.media.public_ip.clone(),
            ..Default::default()
        }),
        Some(Arc::clone(&flow_handler)),
        config.b2bua.clone(),
        outbound_route_provider,
        pbx_handler.clone(),
        None, // No conference handler
        queue_handler,
        presence_handler.clone(),
        cluster_parking.clone(),
        config.service.node_id.clone(),
    )?;

    // Set up SIP trace channel for raw message capture to PostgreSQL
    let sip_trace_store: Option<Arc<dyn fcp_core::SipTraceStore>> =
        if let Some(ref store) = dyn_storage.sip_trace_store() {
            Some(Arc::clone(store))
        } else {
            None
        };
    let sip_trace_event_bus = Arc::clone(&event_bus) as Arc<dyn fcp_events::EventBus>;
    if sip_trace_store.is_some() {
        let (sip_trace_tx, mut sip_trace_rx) =
            tokio::sync::mpsc::channel::<fcp_core::SipTraceMessage>(2000);
        handler.set_sip_trace_tx(sip_trace_tx);

        let trace_store = sip_trace_store.clone().unwrap();
        let trace_event_bus = sip_trace_event_bus.clone();
        tokio::spawn(async move {
            let mut batch: Vec<fcp_core::SipTraceMessage> = Vec::with_capacity(50);
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    msg = sip_trace_rx.recv() => {
                        match msg {
                            Some(trace_msg) => {
                                // Publish real-time event
                                let event = fcp_events::types::PlatformEvent::Sip(
                                    fcp_events::types::SipTraceEvent::MessageCaptured {
                                        call_id: trace_msg.call_id.clone(),
                                        sip_call_id: trace_msg.sip_call_id.clone(),
                                        direction: trace_msg.direction.clone(),
                                        method_or_status: trace_msg.method_or_status.clone(),
                                        from_uri: trace_msg.from_uri.clone(),
                                        to_uri: trace_msg.to_uri.clone(),
                                        source_addr: trace_msg.source_addr.clone(),
                                        dest_addr: trace_msg.dest_addr.clone(),
                                        transport: trace_msg.transport.clone(),
                                        cseq: trace_msg.cseq.clone(),
                                        raw_message: trace_msg.raw_message.clone(),
                                        timestamp: trace_msg.timestamp,
                                        node_id: trace_msg.node_id.clone(),
                                    },
                                );
                                let _ = trace_event_bus.publish(event).await;

                                batch.push(trace_msg);
                                if batch.len() >= 50 {
                                    if let Err(e) = trace_store.insert_sip_traces_batch(&batch).await {
                                        tracing::warn!(error = %e, "Failed to insert SIP trace batch");
                                    }
                                    batch.clear();
                                }
                            }
                            None => {
                                // Channel closed — flush remaining
                                if !batch.is_empty() {
                                    let _ = trace_store.insert_sip_traces_batch(&batch).await;
                                }
                                break;
                            }
                        }
                    }
                    _ = interval.tick() => {
                        // Flush batch on timer
                        if !batch.is_empty() {
                            if let Err(e) = trace_store.insert_sip_traces_batch(&batch).await {
                                tracing::warn!(error = %e, "Failed to insert SIP trace batch");
                            }
                            batch.clear();
                        }
                    }
                }
            }
        });
        info!("SIP message tracing enabled — writing to PostgreSQL");
    }

    let handler = Arc::new(handler);

    // 16. Create and start TransportListener
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

    // Parse TLS bind address and configure TLS if cert and key are provided
    #[cfg(feature = "tls")]
    let (tls_bind, tls_cert, tls_key) = {
        let tls_configured =
            config.sip.tls_cert_path.is_some() && config.sip.tls_key_path.is_some();
        if tls_configured {
            let sips_bind: std::net::SocketAddr = config
                .sip
                .sips_bind
                .parse()
                .context("Invalid SIPS bind address")?;
            info!(
                sips_bind = %sips_bind,
                cert = ?config.sip.tls_cert_path,
                key = ?config.sip.tls_key_path,
                "TLS/SIPS configuration enabled"
            );
            (
                Some(sips_bind),
                config.sip.tls_cert_path.clone(),
                config.sip.tls_key_path.clone(),
            )
        } else {
            (None, None, None)
        }
    };

    let listener_config = ListenerConfig {
        udp_bind: Some(udp_bind),
        tcp_bind: Some(tcp_bind),
        #[cfg(feature = "tls")]
        tls_bind,
        #[cfg(feature = "tls")]
        tls_cert,
        #[cfg(feature = "tls")]
        tls_key,
    };

    let handler_dyn: Arc<dyn InboundMessageHandler> =
        Arc::clone(&handler) as Arc<dyn InboundMessageHandler>;
    let mut listener = TransportListener::new(listener_config, handler_dyn);
    listener
        .start()
        .await
        .context("Failed to start transport listeners")?;

    // 17. Spawn TCP response routing task
    let handler_clone = Arc::clone(&handler);
    let mut client_rx = client_rx;
    tokio::spawn(async move {
        while let Some(packet) = client_rx.recv().await {
            let txn_transport = match packet.transport() {
                SipTransportKind::Udp => SipTransactionTransportKind::Udp,
                SipTransportKind::Tcp => SipTransactionTransportKind::Tcp,
                SipTransportKind::Tls => SipTransactionTransportKind::Tls,
                SipTransportKind::Ws => SipTransactionTransportKind::Ws,
                SipTransportKind::Wss => SipTransactionTransportKind::Wss,
                SipTransportKind::Sctp => SipTransactionTransportKind::Sctp,
                SipTransportKind::TlsSctp => SipTransactionTransportKind::TlsSctp,
            };

            if let Err(e) = handler_clone
                .handle_inbound(
                    txn_transport,
                    packet.peer(),
                    packet.payload().clone(),
                    packet.stream().cloned(),
                )
                .await
            {
                tracing::error!("Failed to handle outbound TCP response: {}", e);
            }
        }
    });

    // 18. Spawn HTTP health/status/calls server
    let app_state = Arc::new(health::AppState {
        call_manager: Arc::clone(&call_manager),
        storage: Arc::clone(&dyn_storage),
        media_manager: Arc::clone(&media_manager),
        queue_manager: queue_manager.clone(),
        queue_handler: queue_call_handler.clone(),
        presence_manager: presence_manager.clone(),
        event_bus: Arc::clone(&event_bus),
        start_time: Instant::now(),
        config_path: cli.config.clone(),
        api_token: config.http.api_token.clone(),
        pbx_handler: pbx_handler.clone(),
        cluster_parking: cluster_parking.clone(),
    });
    let http_bind = config.http.bind_address;
    tokio::spawn(async move {
        if let Err(e) = health::run_http_server(http_bind, app_state).await {
            tracing::error!(%e, "HTTP API server exited");
        }
    });

    // 19. Spawn registration cleanup task (only when registrar is enabled)
    if config.registrar.enabled {
        let cleanup_storage: Arc<dyn fcp_core::RegistrationStore> = Arc::clone(&dyn_storage) as _;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                match cleanup_storage.cleanup_expired().await {
                    Ok(count) => {
                        if count > 0 {
                            info!(count, "Cleaned up expired registrations");
                        }
                    }
                    Err(e) => {
                        warn!(%e, "Failed to cleanup expired registrations");
                    }
                }
            }
        });
    }

    // 20. Spawn parking timeout monitor (if PBX parking is enabled)
    if let Some(ref pbx) = pbx_handler {
        if pbx.parking_manager().is_enabled() {
            let pbx_clone = Arc::clone(pbx);
            let handler_clone_park = Arc::clone(&handler);
            let call_manager_clone_park = Arc::clone(&call_manager);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
                loop {
                    interval.tick().await;
                    let expired = pbx_clone.parking_manager().expired_calls();
                    for parked in expired {
                        info!(
                            orbit = parked.orbit,
                            call_id = %parked.call_id,
                            parker = %parked.parker_extension,
                            "Parked call expired, routing back to parker"
                        );
                        if let Err(e) = handler_clone_park
                            .route_expired_parked_call(&parked, &call_manager_clone_park)
                            .await
                        {
                            tracing::error!(
                                call_id = %parked.call_id,
                                error = %e,
                                "Failed to route expired parked call"
                            );
                            // Terminate the call on failure
                            let _ = call_manager_clone_park.terminate_call(
                                &parked.call_id,
                                Some(format!("Park expiry routing failed: {}", e)),
                            );
                        }
                    }
                }
            });
            info!("Parking timeout monitor started (5s interval)");
        }
    }

    // 21. Spawn event forwarding task (if configured AND Redis is not active)
    // When Redis event bus is enabled, events flow via Pub/Sub + Streams.
    // HTTP forwarding is disabled to prevent duplicate event delivery.
    let skip_http_forwarding = config.events.use_redis;
    if let Some(ref events_url) = config.events.server_url {
        if skip_http_forwarding {
            info!("HTTP event forwarding disabled (Redis event bus is active)");
        } else {
            let events_url = events_url.clone();
            let event_bus_clone = Arc::clone(&event_bus);
            tokio::spawn(async move {
                if let Err(e) = run_event_forwarder(event_bus_clone, &events_url).await {
                    tracing::error!(%e, "Event forwarder task failed");
                }
            });
            info!(events_url = %config.events.server_url.as_deref().unwrap_or(""), "Event forwarding enabled");
        }
    }

    // 21b. Spawn webhook dispatcher tasks (one per configured webhook)
    for webhook in &config.events.webhooks {
        let event_bus_clone = Arc::clone(&event_bus);
        let webhook_clone = webhook.clone();
        tokio::spawn(async move {
            if let Err(e) = run_webhook_dispatcher(event_bus_clone, webhook_clone).await {
                tracing::error!(%e, "Webhook dispatcher task failed");
            }
        });
    }
    if !config.events.webhooks.is_empty() {
        info!(count = config.events.webhooks.len(), "Webhook dispatchers started");
    }

    // 21c. Spawn dedicated queue webhook dispatcher (if configured in queue section)
    if let Some(ref url) = config.queue.webhook_url {
        let wh = config::WebhookConfig {
            url: url.clone(),
            event_types: vec!["queue".to_string()],
            secret: None,
        };
        let bus = Arc::clone(&event_bus);
        tokio::spawn(async move {
            if let Err(e) = run_webhook_dispatcher(bus, wh).await {
                tracing::error!(%e, "Queue webhook dispatcher failed");
            }
        });
        info!(url = %url, "Queue webhook dispatcher started");
    }

    // 21d. Spawn presence event listener (if presence is enabled)
    if let Some(ref pm) = presence_manager {
        let pm_clone = pm.clone();
        let event_bus_clone = Arc::clone(&event_bus);
        let tenant_id = fcp_core::TenantId::nil();
        let storage_for_presence = Arc::clone(&dyn_storage);
        tokio::spawn(async move {
            run_presence_listener(event_bus_clone, pm_clone, tenant_id, storage_for_presence).await;
        });
        info!("Presence event listener started");

        // Spawn watcher + PUBLISH + store cleanup task (with shutdown signal)
        let pm_cleanup = pm.clone();
        let ph_cleanup = presence_handler.as_ref().map(Arc::clone);
        let concrete_ph_cleanup = concrete_presence_handler.as_ref().map(Arc::clone);
        let mut cleanup_shutdown_rx = pm.shutdown_receiver();
        tokio::spawn(async move {
            let interval_dur = pm_cleanup.cleanup_interval();
            let mut interval = tokio::time::interval(interval_dur);
            // Shorter interval for retry queue processing (10s)
            let mut retry_interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let removed = pm_cleanup.cleanup_expired_watchers();
                        if !removed.is_empty() {
                            info!(count = removed.len(), "Cleaned up expired presence watchers");
                            // Send terminated NOTIFY to expired watchers
                            if let Some(ref ph) = ph_cleanup {
                                if let Err(e) = ph.send_terminated_notifies(&removed, "timeout").await {
                                    warn!(error = %e, "Failed to send terminated NOTIFYs for expired watchers");
                                }
                            }
                        }
                        // Check for PUBLISH expiry warnings before cleanup
                        pm_cleanup.check_publish_expiry_warnings().await;
                        // Clean up expired PUBLISH ESC entries (RFC 3903)
                        pm_cleanup.cleanup_expired_publishes().await;
                        // Clean up expired subscriptions from persistent store
                        pm_cleanup.cleanup_expired_subscriptions_from_store().await;
                    }
                    _ = retry_interval.tick() => {
                        // Process NOTIFY retry queue
                        if let Some(ref ph) = concrete_ph_cleanup {
                            ph.process_retry_queue().await;
                        }
                    }
                    _ = cleanup_shutdown_rx.changed() => {
                        info!("Presence cleanup task received shutdown signal");
                        break;
                    }
                }
            }
        });

        // Spawn presence NOTIFY dispatcher — sends NOTIFY to watchers on state changes
        if let Some(ref ph) = presence_handler {
            let ph_clone = Arc::clone(ph);
            let event_bus_notify = Arc::clone(&event_bus);
            tokio::spawn(async move {
                run_presence_notifier(event_bus_notify, ph_clone).await;
            });
            info!("Presence NOTIFY dispatcher started");
        }
    }

    // 21e. Spawn ConfigChanged event listener for cache invalidation
    {
        let cached_creds = Arc::clone(&cached_creds);
        let cached_exts = Arc::clone(&cached_exts);
        let cached_devs = Arc::clone(&cached_devs);
        let event_bus_cache = Arc::clone(&event_bus);
        tokio::spawn(async move {
            run_cache_invalidation_listener(event_bus_cache, cached_creds, cached_exts, cached_devs)
                .await;
        });
        info!("Cache invalidation listener started");
    }

    // 22. Publish SystemEvent::Started to announce this node
    {
        let started_event = fcp_events::PlatformEvent::System(fcp_events::SystemEvent::Started {
            node_id: config.service.node_id.clone(),
            timestamp: std::time::SystemTime::now(),
        });
        if let Err(e) = event_publisher.publish(started_event).await {
            warn!(%e, "Failed to publish SystemEvent::Started");
        } else {
            info!(node_id = %config.service.node_id, "Published SystemEvent::Started");
        }
    }

    // Log operating mode
    if !config.trunks.is_empty() && !config.registrar.enabled {
        info!(trunks = config.trunks.len(), "Operating in session-manager mode");
    } else if !config.trunks.is_empty() {
        info!(trunks = config.trunks.len(), "Operating in call-manager mode with trunks");
    } else {
        info!("Operating in call-manager mode");
    }

    let tls_enabled = config.sip.tls_cert_path.is_some() && config.sip.tls_key_path.is_some();
    info!(
        udp = %config.sip.udp_bind,
        tcp = %config.sip.tcp_bind,
        sips = if tls_enabled { config.sip.sips_bind.as_str() } else { "disabled" },
        http = %config.http.bind_address,
        registrar = config.registrar.enabled,
        auth = config.registrar.auth_enabled,
        realm = %config.registrar.realm,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        trunks = config.trunks.len(),
        routing = config.routing.rules_file.is_some(),
        events_forwarding = config.events.server_url.is_some(),
        webhooks = config.events.webhooks.len(),
        pbx = config.pbx.enabled,
        "fcp-call-manager ready — listening for SIP messages"
    );

    // 20b. Spawn config poll loop (checks DB every 10s for config changes)
    {
        let cs = Arc::clone(&config_store);
        let registrar_clone = registrar.clone();
        let handler_for_poll = Arc::clone(&handler);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_version: i64 = 0;
            loop {
                interval.tick().await;
                match cs.get_latest_version("call-manager", "_system").await {
                    Ok(v) if v > last_version => {
                        info!(old_version = last_version, new_version = v, "config version changed in database");
                        last_version = v;

                        // Re-read registrar config from DB and apply runtime changes
                        if let Some(ref reg) = registrar_clone {
                            match cs
                                .get_config("call-manager", "registrar", "_system")
                                .await
                            {
                                Ok(Some(entry)) => {
                                    if let Ok(reg_cfg) = serde_json::from_value::<
                                        config::RegistrarSection,
                                    >(
                                        entry.value
                                    ) {
                                        let current = reg.is_auth_enabled();
                                        if reg_cfg.auth_enabled != current {
                                            if reg_cfg.auth_enabled {
                                                reg.enable_auth();
                                            } else {
                                                reg.disable_auth();
                                            }
                                        }
                                    }
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    tracing::debug!(
                                        error = %e,
                                        "config poll: failed to read registrar config"
                                    );
                                }
                            }
                        }

                        // Re-read B2BUA config and update voicemail_server_uri at runtime
                        match cs.get_config("call-manager", "b2bua", "_system").await {
                            Ok(Some(entry)) => {
                                if let Ok(b2bua_cfg) = serde_json::from_value::<
                                    fcp_b2bua_core::B2buaConfig,
                                >(entry.value) {
                                    if let Ok(mut guard) = handler_for_poll.voicemail_server_uri_override().write() {
                                        *guard = b2bua_cfg.voicemail_server_uri;
                                        tracing::debug!("config poll: updated voicemail_server_uri override");
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::debug!(
                                    error = %e,
                                    "config poll: failed to read b2bua config"
                                );
                            }
                        }
                    }
                    Ok(_) => {} // no change
                    Err(e) => {
                        tracing::debug!(error = %e, "config poll: failed to check version");
                    }
                }
            }
        });
    }

    // 21. Wait for shutdown signal (with SIGHUP for config reload)
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to register SIGTERM handler");
    let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        .expect("Failed to register SIGHUP handler");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
                break;
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown...");
                break;
            }
            _ = sighup.recv() => {
                info!("Received SIGHUP, reloading queue configuration...");
                match config::load_config(cli.config.as_deref()) {
                    Ok(new_config) => {
                        if let Some(ref qm) = queue_manager {
                            let new_queues = qm.reload_config(new_config.queue.clone());
                            if let Some(ref qh) = queue_call_handler {
                                for name in new_queues {
                                    info!(queue = %name, "Spawning distribution loop for new queue");
                                    qh.spawn_distribution_loop_for(name);
                                }
                            }
                            info!("Queue configuration reloaded successfully");
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to reload configuration");
                    }
                }
            }
        }
    }

    // 21. Graceful shutdown
    // Signal cluster manager to shutdown (removes heartbeat key)
    #[cfg(feature = "storage-redis")]
    if let Some((cm, _)) = &_cluster_manager {
        cm.shutdown().await;
    }

    // Signal presence manager to shutdown (persists state, signals cleanup task)
    if let Some(ref pm) = presence_manager {
        pm.shutdown().await;
    }

    // Send terminated NOTIFYs to all presence watchers including winfo (RFC 3265 §3.2.4, reason=probation)
    if let (Some(ref pm), Some(ref ph)) = (&presence_manager, &presence_handler) {
        let all_watchers = pm.get_all_watchers_including_winfo();
        if !all_watchers.is_empty() {
            info!(count = all_watchers.len(), "Sending terminated NOTIFYs to presence watchers...");
            if let Err(e) = ph.send_terminated_notifies(&all_watchers, "probation").await {
                warn!(error = %e, "Failed to send some shutdown NOTIFYs");
            }
        }

        // Drain retry queue entries
        let retry_count = pm.retry_queue_len();
        if retry_count > 0 {
            info!(count = retry_count, "Draining NOTIFY retry queue on shutdown");
        }
    }

    // Signal trunk registration task to de-register (if running)
    if let Some(tx) = trunk_shutdown_tx {
        let _ = tx.send(true);
        // Wait briefly for de-registration attempts
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
    listener.stop().await?;
    info!("Shutdown complete");
    Ok(())
}

/// Create the FcpRegistrar from configuration and storage.
fn create_registrar(
    config: &CallManagerConfig,
    storage: &Arc<fcp_core::DynStorage>,
    event_publisher: Arc<dyn fcp_events::EventPublisher>,
) -> Arc<fcp_sip::registrar::FcpRegistrar> {
    let registration_store: Arc<dyn fcp_core::RegistrationStore> = Arc::clone(storage) as _;
    // Always provide credential store so auth can be toggled at runtime
    let credential_store: Option<Arc<dyn fcp_core::CredentialStore>> =
        Some(Arc::clone(storage) as _);
    let realm = Some(config.registrar.realm.clone());

    let registrar = fcp_sip::registrar::FcpRegistrar::with_events(
        registration_store,
        credential_store,
        realm,
        event_publisher,
        config.registrar.auth_enabled,
    );

    // Add domain aliases if configured
    let registrar = if !config.registrar.domain_aliases.is_empty() {
        info!(
            aliases = ?config.registrar.domain_aliases,
            "Domain aliasing enabled for registration lookups"
        );
        registrar.with_domain_aliases(config.registrar.domain_aliases.clone())
    } else {
        registrar
    };

    Arc::new(registrar)
}

/// Create a transport dispatcher with shared TCP pool.
async fn create_transport_dispatcher(
    config: &CallManagerConfig,
    tcp_pool: Arc<ConnectionPool>,
) -> Result<Arc<fcp_sip::transport::RealTransportDispatcher>> {
    use fcp_sip::transport::RealTransportDispatcher;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    // Create UDP socket with SO_REUSEADDR
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

    let mut dispatcher =
        RealTransportDispatcher::new_with_pool(Some(Arc::new(udp_socket)), tcp_pool);

    // Add TLS client config if TLS is enabled (for outbound SIPS connections)
    // We check if TLS cert is configured, meaning TLS server is enabled
    if config.sip.tls_cert_path.is_some() {
        match RealTransportDispatcher::load_tls_client_config() {
            Ok(tls_config) => {
                info!("TLS client configuration loaded for outbound SIPS connections");
                dispatcher = dispatcher.with_tls_config(tls_config);
            }
            Err(e) => {
                warn!("Failed to load TLS client config: {}. Outbound TLS will not work.", e);
            }
        }
    }

    Ok(Arc::new(dispatcher))
}

/// Create a UAC for outbound calls (B-leg).
///
/// If `credential_provider` is provided (from trunk config), the UAC will
/// automatically handle 401/407 digest challenges from upstream trunks.
async fn create_uac(
    config: &CallManagerConfig,
    transaction_manager: Arc<TransactionManager>,
    dispatcher: Arc<fcp_sip::transport::RealTransportDispatcher>,
    credential_provider: Option<Arc<TrunkCredentialProvider>>,
) -> Result<Arc<SipUserAgentClient>> {
    let local_uri = create_local_uri(config)?;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    let local_addr = if let Some(ref local_ip) = config.sip.local_ip {
        let port = if bind_addr.port() == 0 {
            5060
        } else {
            bind_addr.port()
        };
        format!("{}:{}", local_ip, port)
    } else {
        let port = if bind_addr.port() == 0 {
            5060
        } else {
            bind_addr.port()
        };
        format!("{}:{}", bind_addr.ip(), port)
    };

    let resolver = Arc::new(SipResolver::from_system().context("Failed to create DNS resolver")?);

    let mut builder = IntegratedUAC::builder();
    builder = builder
        .local_uri(local_uri.as_str())
        .local_addr(&local_addr)?
        .via_advertised_addr(&local_addr)?
        .transaction_manager(transaction_manager)
        .resolver(resolver)
        .dispatcher(dispatcher);

    if let Some(cred_provider) = credential_provider {
        builder = builder.credential_provider(cred_provider);
    }

    let integrated_uac = builder.build().context("Failed to build IntegratedUAC")?;

    Ok(Arc::new(SipUserAgentClient::new(Arc::new(integrated_uac))))
}

/// Create a local SIP URI from configuration.
fn create_local_uri(config: &CallManagerConfig) -> Result<SipUri> {
    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    // Use explicit local_uri if configured
    if let Some(ref uri_str) = config.sip.local_uri {
        let uri = Uri::parse(uri_str).context("Failed to parse configured local_uri")?;
        return uri
            .as_sip()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Configured local_uri is not a SIP URI"));
    }

    // Build from local_ip or bind address
    let domain = if bind_addr.ip().is_unspecified() {
        config
            .sip
            .local_ip
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind_addr.ip().to_string()
    };

    let uri_str = if bind_addr.port() != 0 && bind_addr.port() != 5060 {
        format!("sip:call-manager@{}:{}", domain, bind_addr.port())
    } else {
        format!("sip:call-manager@{}", domain)
    };

    let uri = Uri::parse(&uri_str).context("Failed to parse local URI")?;
    uri.as_sip()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

/// Create a media manager from configuration.
async fn create_media_manager(config: &CallManagerConfig) -> Result<Arc<fcp_media::MediaManager>> {
    // Create a single shared TTS service for the entire system
    let tts_service = if let Some(ref engine) = config.media.tts_engine {
        let tts_config = fcp_media::TtsConfig {
            provider: fcp_media::TtsProviderConfig::Local {
                engine: engine.clone(),
                executable_path: config.media.tts_python.as_ref().map(std::path::PathBuf::from),
            },
            cache_dir: std::path::PathBuf::from(&config.media.tts_cache_dir),
            cache_max_size_mb: 500,
            default_language: "en-US".to_string(),
            default_voice: config.media.tts_voice.clone(),
        };
        match fcp_media::TtsService::new(tts_config).await {
            Ok(svc) => {
                let svc = Arc::new(svc);
                info!(provider = %svc.provider_name(), "Shared TTS service created");
                Some(svc)
            }
            Err(e) => {
                warn!(error = %e, "Failed to create TTS service — TTS will be unavailable");
                None
            }
        }
    } else {
        None
    };

    let media_config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: config.media.rtp_port_start,
        rtp_port_end: config.media.rtp_port_end,
        public_ip: config.media.public_ip.clone(),
        tts: None, // TTS config not needed — injecting shared service directly
        tts_service,
        ..Default::default()
    };

    let manager = fcp_media::MediaManager::new(media_config)
        .await
        .context("Failed to create media manager")?;

    info!(
        backend = manager.backend_name(),
        tts_engine = ?config.media.tts_engine,
        rtp_ports = format!(
            "{}-{}",
            config.media.rtp_port_start, config.media.rtp_port_end
        ),
        "Media manager created"
    );

    Ok(Arc::new(manager))
}

/// Pre-generate TTS audio for system prompts that don't have audio files yet.
///
/// Uses the shared TtsService to synthesize audio for each prompt, saves
/// the WAV files to `prompts_dir`, and updates the prompt DB records with the
/// file path so that `PromptService::resolve()` can serve them without TTS.
async fn pre_generate_prompt_audio(
    prompt_service: &Arc<fcp_core::PromptService>,
    tenant_id: &fcp_core::TenantId,
    tts_service: &Arc<fcp_media::TtsService>,
    tts_voice: Option<&str>,
    prompts_dir: &std::path::Path,
    system_prompts: &[(&str, &str, &str)],
) -> Result<()> {
    // Check which prompts already have audio
    let mut need_generation = Vec::new();
    for (name, _desc, tts_text) in system_prompts {
        if let Ok(Some(_path)) = prompt_service.resolve(name, Some("en-US")).await {
            // Already has a valid audio file
            continue;
        }
        need_generation.push((*name, *tts_text));
    }

    if need_generation.is_empty() {
        info!("All system prompts already have audio files");
        return Ok(());
    }

    info!(
        count = need_generation.len(),
        "Pre-generating TTS audio for system prompts"
    );

    // Ensure prompts directory exists
    let tenant_dir = prompts_dir.join(tenant_id.to_string());
    tokio::fs::create_dir_all(&tenant_dir)
        .await
        .context("Failed to create prompts directory")?;

    let mut generated = 0u32;
    for (name, tts_text) in &need_generation {
        match tts_service.synthesize(tts_text, Some("en-US"), tts_voice).await {
            Ok(cached_path) => {
                // Copy from TTS cache to permanent prompts directory
                let dest_path = tenant_dir.join(format!("{}.wav", name));
                if let Err(e) = tokio::fs::copy(&cached_path, &dest_path).await {
                    warn!(name = name, error = %e, "Failed to copy TTS audio to prompts dir");
                    continue;
                }

                let file_size = tokio::fs::metadata(&dest_path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0);

                // Update the prompt record with the audio path
                if let Err(e) = prompt_service
                    .cache_tts_audio(name, "en-US", dest_path.clone(), file_size)
                    .await
                {
                    warn!(name = name, error = %e, "Failed to update prompt record with audio path");
                    continue;
                }

                info!(name = name, path = %dest_path.display(), "Generated prompt audio");
                generated += 1;
            }
            Err(e) => {
                warn!(name = name, error = %e, "Failed to synthesize prompt audio");
            }
        }
    }

    info!(generated = generated, total = need_generation.len(), "Prompt audio pre-generation complete");
    Ok(())
}

/// Derive local IP from bind address string.
///
/// If the bind address is unspecified (0.0.0.0), falls back to auto-detection.
fn derive_local_ip(bind_addr: &str) -> String {
    if let Ok(addr) = bind_addr.parse::<std::net::SocketAddr>() {
        if !addr.ip().is_unspecified() {
            return addr.ip().to_string();
        }
    }
    auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Auto-detect the local LAN IP by asking the OS which interface routes to the internet.
///
/// Opens a UDP socket to a well-known address (no actual traffic is sent) and checks
/// which local address the OS selected. This is the standard technique for determining
/// the machine's routable IP address.
fn auto_detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // connect() on a UDP socket doesn't send anything — it just sets the
    // default destination so the OS selects the appropriate source address.
    socket.connect("8.8.8.8:53").ok()?;
    let addr = socket.local_addr().ok()?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    Some(addr.ip().to_string())
}

/// Initialize tracing subscriber based on configuration.
fn init_tracing(config: &CallManagerConfig) {
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
            // "pretty" or anything else
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .with_env_filter(env_filter)
                .init();
        }
    }
}

/// Run a webhook dispatcher task for a single webhook endpoint.
///
/// Subscribes to the event bus with an optional event-type filter and POSTs
/// matching events as JSON to the configured URL.
async fn run_webhook_dispatcher(
    event_bus: Arc<InMemoryEventBus>,
    webhook: config::WebhookConfig,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("Failed to create HTTP client for webhook dispatcher")?;

    // Build event filter from webhook config
    let filter = if webhook.event_types.is_empty() {
        EventFilter::new()
    } else {
        let types: Vec<EventType> = webhook
            .event_types
            .iter()
            .filter_map(|t| match t.to_lowercase().as_str() {
                "call" => Some(EventType::Call),
                "media" => Some(EventType::Media),
                "queue" => Some(EventType::Queue),
                "recording" => Some(EventType::Recording),
                "system" => Some(EventType::System),
                "user" => Some(EventType::User),
                "registration" => Some(EventType::Registration),
                "voicemail" => Some(EventType::Voicemail),
                "conference" => Some(EventType::Conference),
                "ivr" => Some(EventType::Ivr),
                "flow" => Some(EventType::Flow),
                other => {
                    warn!(event_type = %other, "Unknown webhook event type, ignoring");
                    None
                }
            })
            .collect();
        EventFilter::new().types(types)
    };

    let mut stream = event_bus
        .subscribe(filter.clone())
        .await
        .context("Failed to subscribe to event bus for webhook")?;

    let url = webhook.url.clone();
    let secret = webhook.secret.clone();
    info!(
        url = %url,
        event_types = ?webhook.event_types,
        has_secret = secret.is_some(),
        "Webhook dispatcher started"
    );

    loop {
        match stream.next().await {
            Some(event) => {
                let client = client.clone();
                let url = url.clone();
                let secret = secret.clone();
                tokio::spawn(async move {
                    let json_body = match serde_json::to_vec(&event) {
                        Ok(body) => body,
                        Err(e) => {
                            tracing::warn!(%e, "Failed to serialize event for webhook");
                            return;
                        }
                    };

                    let mut request = client
                        .post(&url)
                        .header("Content-Type", "application/json");

                    // Add HMAC signature header if secret is configured
                    if let Some(ref secret) = secret {
                        use std::fmt::Write;
                        // Simple HMAC-like signature: hex(SHA256(secret + body))
                        // In production, use a proper HMAC crate; for now use a
                        // deterministic hash so webhooks can verify authenticity.
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        std::hash::Hash::hash(secret.as_bytes(), &mut hasher);
                        std::hash::Hash::hash(&json_body, &mut hasher);
                        let hash = std::hash::Hasher::finish(&hasher);
                        let mut sig = String::with_capacity(16);
                        let _ = write!(sig, "{:016x}", hash);
                        request = request.header("X-Webhook-Signature", sig);
                    }

                    match request.body(json_body).send().await {
                        Ok(resp) => {
                            if !resp.status().is_success() {
                                tracing::warn!(
                                    url = %url,
                                    status = %resp.status(),
                                    "Webhook received non-2xx response"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(url = %url, %e, "Webhook delivery failed");
                        }
                    }
                });
            }
            None => {
                warn!(url = %url, "Webhook event stream ended, resubscribing...");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                match event_bus.subscribe(filter.clone()).await {
                    Ok(new_stream) => {
                        stream = new_stream;
                        info!(url = %url, "Resubscribed to event bus for webhook");
                    }
                    Err(e) => {
                        warn!(url = %url, %e, "Failed to resubscribe for webhook");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

/// Run the event forwarder task that subscribes to local events and POSTs them to fcp-events-server.
async fn run_event_forwarder(event_bus: Arc<InMemoryEventBus>, events_url: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("Failed to create HTTP client for event forwarding")?;

    // Subscribe to all events
    let mut stream = event_bus
        .subscribe(EventFilter::new())
        .await
        .context("Failed to subscribe to event bus")?;

    info!(url = %events_url, "Event forwarder started");

    loop {
        match stream.next().await {
            Some(event) => {
                tracing::debug!(?event, "Forwarding event to events server");
                // Fire-and-forget POST to events server
                let client = client.clone();
                let url = events_url.to_string();
                tokio::spawn(async move {
                    match client.post(&url).json(&event).send().await {
                        Ok(resp) => {
                            if resp.status().is_success() {
                                tracing::debug!(status = %resp.status(), "Event forwarded");
                            } else {
                                tracing::warn!(
                                    status = %resp.status(),
                                    "Event forward received non-2xx response"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(%e, "Failed to forward event to events server");
                        }
                    }
                });
            }
            None => {
                warn!("Event stream ended unexpectedly, resubscribing...");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                // Try to resubscribe
                match event_bus.subscribe(EventFilter::new()).await {
                    Ok(new_stream) => {
                        stream = new_stream;
                        info!("Resubscribed to event bus");
                    }
                    Err(e) => {
                        warn!(%e, "Failed to resubscribe to event bus");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

/// Resolve a conference participant_id to an AOR.
///
/// The participant_id may be a SIP URI, extension number, or user@domain.
/// We try extract_aor first, then fall back to searching known presences
/// by matching the user part against existing AORs.
fn resolve_conference_participant_aor(
    participant_id: &str,
    presence: &fcp_presence::PresenceManager,
) -> Option<String> {
    // Try direct extraction if it looks like a SIP URI
    if let Some(aor) = extract_aor(participant_id) {
        return Some(aor);
    }
    // Fall back: participant_id is likely just an extension number (e.g., "5001").
    // Search known presences for a matching user part.
    for info in presence.get_all_presences() {
        let user_part = info.aor.split('@').next().unwrap_or("");
        if user_part == participant_id {
            return Some(info.aor.clone());
        }
    }
    None
}

/// Resolve a voicemail mailbox_id to an AOR.
///
/// Mailbox IDs are typically the extension number (e.g., "5001").
/// Searches known presences for a matching user part.
fn resolve_mailbox_aor(
    mailbox_id: &str,
    presence: &fcp_presence::PresenceManager,
) -> Option<String> {
    // Search known presences for a matching user part
    for info in presence.get_all_presences() {
        let user_part = info.aor.split('@').next().unwrap_or("");
        if user_part == mailbox_id {
            return Some(info.aor.clone());
        }
    }
    None
}

/// Resolve an extension AOR (e.g., "1234@host") to device AOR(s) (e.g., "polycom@host").
///
/// Looks up the extension by number, resolves its target to device(s),
/// and returns the device SIP URIs as AORs. Returns an empty vec if
/// the AOR is not an extension or resolution fails.
async fn resolve_extension_to_device_aors(
    aor: &str,
    tenant_id: &fcp_core::TenantId,
    storage: &fcp_core::DynStorage,
) -> Vec<String> {
    let (ext_number, _host) = match aor.split_once('@') {
        Some((user, host)) => (user, host),
        None => {
            tracing::debug!(aor = %aor, "resolve_extension_to_device_aors: no '@' in AOR");
            return Vec::new();
        }
    };
    // Only try if it looks like a numeric extension
    if !ext_number.chars().all(|c| c.is_ascii_digit()) || ext_number.is_empty() {
        tracing::debug!(aor = %aor, ext_number = %ext_number, "resolve_extension_to_device_aors: not a numeric extension");
        return Vec::new();
    }
    let extension = match storage.get_extension_by_number(tenant_id, ext_number).await {
        Ok(Some(ext)) => ext,
        Ok(None) => {
            tracing::warn!(aor = %aor, ext_number = %ext_number, "resolve_extension_to_device_aors: extension not found in storage");
            return Vec::new();
        }
        Err(e) => {
            tracing::warn!(aor = %aor, ext_number = %ext_number, error = %e, "resolve_extension_to_device_aors: storage error");
            return Vec::new();
        }
    };
    tracing::info!(
        aor = %aor,
        ext_number = %ext_number,
        target = %extension.target,
        "resolve_extension_to_device_aors: found extension"
    );
    let mut device_aors = Vec::new();
    match &extension.target {
        fcp_core::ExtensionTarget::Device(device_id) => {
            match storage.get_device(device_id).await {
                Ok(Some(device)) => {
                    match &device.sip_uri {
                        Some(sip_uri) => {
                            match fcp_sip::presence::extract_aor(sip_uri) {
                                Some(dev_aor) => {
                                    tracing::info!(ext = %ext_number, device_id = %device_id, sip_uri = %sip_uri, dev_aor = %dev_aor, "Resolved Device target to AOR");
                                    device_aors.push(dev_aor);
                                }
                                None => tracing::warn!(ext = %ext_number, device_id = %device_id, sip_uri = %sip_uri, "extract_aor returned None for device sip_uri"),
                            }
                        }
                        None => tracing::warn!(ext = %ext_number, device_id = %device_id, "Device has no sip_uri"),
                    }
                }
                Ok(None) => tracing::warn!(ext = %ext_number, device_id = %device_id, "Device not found in storage"),
                Err(e) => tracing::warn!(ext = %ext_number, device_id = %device_id, error = %e, "Failed to get device"),
            }
        }
        fcp_core::ExtensionTarget::PhoneNumber(phone_number_id) => {
            match storage.get_phone_number(phone_number_id).await {
                Ok(Some(phone_number)) => {
                    match storage.get_line_binding_by_phone_number(&phone_number.id).await {
                        Ok(Some(line_binding)) => {
                            let enabled = line_binding.enabled_devices();
                            tracing::info!(ext = %ext_number, phone_number_id = %phone_number_id, enabled_devices = enabled.len(), "PhoneNumber target: resolving devices");
                            for device_binding in enabled {
                                if let Ok(Some(device)) = storage.get_device(&device_binding.device_id).await {
                                    if let Some(sip_uri) = &device.sip_uri {
                                        if let Some(dev_aor) = fcp_sip::presence::extract_aor(sip_uri) {
                                            tracing::info!(ext = %ext_number, device_id = %device_binding.device_id, dev_aor = %dev_aor, "Resolved PhoneNumber device to AOR");
                                            device_aors.push(dev_aor);
                                        }
                                    } else {
                                        tracing::warn!(ext = %ext_number, device_id = %device_binding.device_id, "PhoneNumber device has no sip_uri");
                                    }
                                }
                            }
                        }
                        Ok(None) => tracing::warn!(ext = %ext_number, phone_number_id = %phone_number_id, "No line binding for phone number"),
                        Err(e) => tracing::warn!(ext = %ext_number, phone_number_id = %phone_number_id, error = %e, "Failed to get line binding"),
                    }
                }
                Ok(None) => tracing::warn!(ext = %ext_number, phone_number_id = %phone_number_id, "Phone number not found"),
                Err(e) => tracing::warn!(ext = %ext_number, phone_number_id = %phone_number_id, error = %e, "Failed to get phone number"),
            }
        }
        fcp_core::ExtensionTarget::User(user_id) => {
            match storage.list_devices_by_user(user_id).await {
                Ok(devices) => {
                    tracing::info!(ext = %ext_number, user_id = %user_id, device_count = devices.len(), "User target: resolving devices");
                    for device in &devices {
                        if let Some(sip_uri) = &device.sip_uri {
                            if let Some(dev_aor) = fcp_sip::presence::extract_aor(sip_uri) {
                                tracing::info!(ext = %ext_number, device_id = %device.id, dev_aor = %dev_aor, "Resolved User device to AOR");
                                device_aors.push(dev_aor);
                            }
                        } else {
                            tracing::warn!(ext = %ext_number, device_id = %device.id, "User device has no sip_uri");
                        }
                    }
                }
                Err(e) => tracing::warn!(ext = %ext_number, user_id = %user_id, error = %e, "Failed to list user devices"),
            }
        }
        other => {
            tracing::debug!(ext = %ext_number, target = %other, "Extension target type not resolvable to device AORs");
        }
    }
    if device_aors.is_empty() {
        tracing::warn!(aor = %aor, ext_number = %ext_number, "resolve_extension_to_device_aors: resolved 0 device AORs");
    } else {
        tracing::info!(aor = %aor, ext_number = %ext_number, ?device_aors, "resolve_extension_to_device_aors: resolved device AORs");
    }
    device_aors
}

/// Resolve a phone-number AOR (e.g. "15185551234@host") to the extension AOR(s)
/// that target that phone number.  This allows presence to be mirrored from the
/// phone number identity to the extension identity (and its device AOR(s)).
async fn resolve_phone_number_to_extension_aors(
    aor: &str,
    tenant_id: &fcp_core::TenantId,
    storage: &fcp_core::DynStorage,
) -> Vec<String> {
    let (user_part, host) = match aor.split_once('@') {
        Some((user, host)) => (user, host),
        None => return Vec::new(),
    };

    // Must look like a phone number: all digits, 10+ chars (E.164 without +)
    if user_part.len() < 10 || !user_part.chars().all(|c| c.is_ascii_digit()) {
        return Vec::new();
    }

    // Try to find the phone number in storage by E.164 (with and without leading 1)
    let e164_candidates: Vec<String> = if user_part.starts_with('1') && user_part.len() == 11 {
        vec![
            format!("+{}", user_part),
            format!("+{}", &user_part[1..]),
        ]
    } else {
        vec![
            format!("+{}", user_part),
            format!("+1{}", user_part),
        ]
    };

    let mut phone_number_id = None;
    for candidate in &e164_candidates {
        if let Ok(Some(pn)) = storage.get_phone_number_by_e164(candidate).await {
            phone_number_id = Some(pn.id);
            tracing::debug!(aor = %aor, e164 = %candidate, "Matched phone number in storage");
            break;
        }
    }

    let pn_id = match phone_number_id {
        Some(id) => id,
        None => {
            tracing::debug!(aor = %aor, "No phone number found in storage for AOR");
            return Vec::new();
        }
    };

    // Find extensions that target this phone number
    let extensions = match storage.list_extensions(tenant_id).await {
        Ok(exts) => exts,
        Err(e) => {
            tracing::warn!(aor = %aor, error = %e, "Failed to list extensions for phone number resolution");
            return Vec::new();
        }
    };

    let mut ext_aors = Vec::new();
    for ext in &extensions {
        if !ext.enabled {
            continue;
        }
        if let fcp_core::ExtensionTarget::PhoneNumber(ref target_pn_id) = ext.target {
            if *target_pn_id == pn_id {
                let ext_aor = format!("{}@{}", ext.number, host);
                tracing::info!(
                    phone_aor = %aor,
                    ext_number = %ext.number,
                    ext_aor = %ext_aor,
                    "Resolved phone number to extension AOR"
                );
                ext_aors.push(ext_aor);
            }
        }
    }

    ext_aors
}

/// Resolve a device AOR (e.g. "polycom@192.168.1.225") to the extension AOR(s)
/// that target this device.  This allows presence to be mirrored when a device
/// is called directly by its SIP username rather than by extension or phone number.
async fn resolve_device_aor_to_extension_aors(
    aor: &str,
    tenant_id: &fcp_core::TenantId,
    storage: &fcp_core::DynStorage,
) -> Vec<String> {
    let (user_part, host) = match aor.split_once('@') {
        Some((user, host)) => (user, host),
        None => return Vec::new(),
    };

    // Skip if it looks like an extension (all digits, short) or a phone number (all digits, 10+)
    if user_part.chars().all(|c| c.is_ascii_digit()) {
        return Vec::new();
    }

    // Reconstruct the SIP URI and look up the device
    let sip_uri = format!("sip:{}@{}", user_part, host);
    let device = match storage.get_device_by_sip_uri(&sip_uri).await {
        Ok(Some(dev)) => dev,
        Ok(None) => {
            tracing::debug!(aor = %aor, sip_uri = %sip_uri, "No device found for SIP URI");
            return Vec::new();
        }
        Err(e) => {
            tracing::warn!(aor = %aor, error = %e, "Failed to look up device by SIP URI");
            return Vec::new();
        }
    };

    // Find extensions that target this device
    let extensions = match storage.list_extensions(tenant_id).await {
        Ok(exts) => exts,
        Err(e) => {
            tracing::warn!(aor = %aor, error = %e, "Failed to list extensions for device resolution");
            return Vec::new();
        }
    };

    let mut ext_aors = Vec::new();
    for ext in &extensions {
        if !ext.enabled {
            continue;
        }
        match &ext.target {
            fcp_core::ExtensionTarget::Device(ref dev_id) if *dev_id == device.id => {
                let ext_aor = format!("{}@{}", ext.number, host);
                tracing::info!(
                    device_aor = %aor,
                    ext_number = %ext.number,
                    ext_aor = %ext_aor,
                    "Resolved device AOR to extension"
                );
                ext_aors.push(ext_aor);
            }
            fcp_core::ExtensionTarget::User(ref user_id) => {
                // Check if this device belongs to the user
                if device.owner_user_id.as_ref() == Some(user_id) {
                    let ext_aor = format!("{}@{}", ext.number, host);
                    tracing::info!(
                        device_aor = %aor,
                        ext_number = %ext.number,
                        ext_aor = %ext_aor,
                        "Resolved device AOR to extension (via user)"
                    );
                    ext_aors.push(ext_aor);
                }
            }
            fcp_core::ExtensionTarget::PhoneNumber(ref pn_id) => {
                // Check if the device is bound to this phone number via line binding
                if let Ok(Some(lb)) = storage.get_line_binding_by_phone_number(pn_id).await {
                    if lb.enabled_devices().iter().any(|db| db.device_id == device.id) {
                        let ext_aor = format!("{}@{}", ext.number, host);
                        tracing::info!(
                            device_aor = %aor,
                            ext_number = %ext.number,
                            ext_aor = %ext_aor,
                            "Resolved device AOR to extension (via phone number line binding)"
                        );
                        ext_aors.push(ext_aor);
                    }
                }
            }
            _ => {}
        }
    }

    ext_aors
}

/// Subscribe to call and registration events, update presence accordingly.
async fn run_presence_listener(
    event_bus: Arc<InMemoryEventBus>,
    presence: fcp_presence::PresenceManager,
    tenant_id: fcp_core::TenantId,
    storage: Arc<fcp_core::DynStorage>,
) {
    use fcp_events::PlatformEvent;

    let filter = EventFilter::new().types(vec![
        EventType::Call,
        EventType::Registration,
        EventType::Conference,
        EventType::Voicemail,
    ]);

    let mut stream = match event_bus.subscribe(filter.clone()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%e, "Failed to subscribe to event bus for presence");
            return;
        }
    };

    tracing::info!("Presence event listener running");

    loop {
        match stream.next().await {
            Some(event) => {
                match &event {
                    // Call answered → update dialog state to Confirmed, transition Ringing→OnThePhone
                    PlatformEvent::Call(fcp_events::CallEvent::Answered { call_id, .. }) => {
                        let call_id_str = call_id.to_string();
                        // Find AORs with active calls for this call_id
                        let affected_aors = presence.get_aors_with_active_call(&call_id_str);
                        tracing::info!(%call_id, ?affected_aors, "Call answered — updating presence for affected AORs");
                        for aor in &affected_aors {
                            presence.update_call_state(
                                aor,
                                &call_id_str,
                                fcp_presence::DialogState::Confirmed,
                            );
                            fcp_presence::events::dialog_changed(
                                presence.event_publisher(),
                                aor,
                                &call_id_str,
                                "confirmed",
                                &tenant_id,
                            ).await;
                            // Transition Ringing → OnThePhone for the callee
                            if let Some(info) = presence.get_presence(aor) {
                                if info.state == fcp_presence::PresenceState::Ringing {
                                    let update = fcp_presence::PresenceUpdate {
                                        aor: aor.clone(),
                                        state: fcp_presence::PresenceState::OnThePhone,
                                        note: Some("On the phone".to_string()),
                                    };
                                    let _ = presence.update_presence(update, &tenant_id).await;
                                    tracing::info!(%call_id, aor = %aor, "Transitioned Ringing → OnThePhone on answer");
                                }
                            }
                        }
                    }

                    // Call initiated (inbound) → set Ringing for callee, track active call
                    PlatformEvent::Call(fcp_events::CallEvent::Initiated { from, to, call_id, .. }) => {
                        let call_id_str = call_id.to_string();
                        tracing::info!(%call_id, from = %from, to = %to, "Presence: CallEvent::Initiated received");

                        if let Some(callee_aor) = extract_aor(to) {
                            // Set Ringing for the extension AOR
                            let update = fcp_presence::PresenceUpdate {
                                aor: callee_aor.clone(),
                                state: fcp_presence::PresenceState::Ringing,
                                note: Some("Ringing".to_string()),
                            };
                            let _ = presence.update_presence(update, &tenant_id).await;

                            // Track active call for callee (Recipient, Early state)
                            let caller_uri = from.clone();
                            presence.add_active_call(&callee_aor, fcp_presence::ActiveCallInfo {
                                call_id: call_id_str.clone(),
                                remote_uri: caller_uri.clone(),
                                remote_display_name: None,
                                direction: fcp_presence::CallDirection::Recipient,
                                dialog_state: fcp_presence::DialogState::Early,
                            });
                            fcp_presence::events::dialog_changed(
                                presence.event_publisher(),
                                &callee_aor,
                                &call_id_str,
                                "early",
                                &tenant_id,
                            ).await;

                            // Also set Ringing for device AOR(s) that the extension resolves to
                            let device_aors = resolve_extension_to_device_aors(&callee_aor, &tenant_id, &storage).await;
                            for dev_aor in &device_aors {
                                let update = fcp_presence::PresenceUpdate {
                                    aor: dev_aor.clone(),
                                    state: fcp_presence::PresenceState::Ringing,
                                    note: Some("Ringing".to_string()),
                                };
                                let _ = presence.update_presence(update, &tenant_id).await;
                                presence.add_active_call(dev_aor, fcp_presence::ActiveCallInfo {
                                    call_id: call_id_str.clone(),
                                    remote_uri: caller_uri.clone(),
                                    remote_display_name: None,
                                    direction: fcp_presence::CallDirection::Recipient,
                                    dialog_state: fcp_presence::DialogState::Early,
                                });
                                fcp_presence::events::dialog_changed(
                                    presence.event_publisher(),
                                    dev_aor,
                                    &call_id_str,
                                    "early",
                                    &tenant_id,
                                ).await;
                                tracing::info!(ext_aor = %callee_aor, dev_aor = %dev_aor, "Mirrored Ringing to device AOR");
                            }

                            // If the callee AOR is a phone number, also mirror presence
                            // to the extension(s) that target this phone number.
                            let ext_aors = resolve_phone_number_to_extension_aors(&callee_aor, &tenant_id, &storage).await;
                            for ext_aor in &ext_aors {
                                let update = fcp_presence::PresenceUpdate {
                                    aor: ext_aor.clone(),
                                    state: fcp_presence::PresenceState::Ringing,
                                    note: Some("Ringing".to_string()),
                                };
                                let _ = presence.update_presence(update, &tenant_id).await;
                                presence.add_active_call(ext_aor, fcp_presence::ActiveCallInfo {
                                    call_id: call_id_str.clone(),
                                    remote_uri: caller_uri.clone(),
                                    remote_display_name: None,
                                    direction: fcp_presence::CallDirection::Recipient,
                                    dialog_state: fcp_presence::DialogState::Early,
                                });
                                fcp_presence::events::dialog_changed(
                                    presence.event_publisher(),
                                    ext_aor,
                                    &call_id_str,
                                    "early",
                                    &tenant_id,
                                ).await;
                                tracing::info!(phone_aor = %callee_aor, ext_aor = %ext_aor, "Mirrored Ringing from phone number to extension AOR");

                                // Also mirror to the extension's device AOR(s)
                                let ext_device_aors = resolve_extension_to_device_aors(ext_aor, &tenant_id, &storage).await;
                                for ext_dev_aor in &ext_device_aors {
                                    let update = fcp_presence::PresenceUpdate {
                                        aor: ext_dev_aor.clone(),
                                        state: fcp_presence::PresenceState::Ringing,
                                        note: Some("Ringing".to_string()),
                                    };
                                    let _ = presence.update_presence(update, &tenant_id).await;
                                    presence.add_active_call(ext_dev_aor, fcp_presence::ActiveCallInfo {
                                        call_id: call_id_str.clone(),
                                        remote_uri: caller_uri.clone(),
                                        remote_display_name: None,
                                        direction: fcp_presence::CallDirection::Recipient,
                                        dialog_state: fcp_presence::DialogState::Early,
                                    });
                                    fcp_presence::events::dialog_changed(
                                        presence.event_publisher(),
                                        ext_dev_aor,
                                        &call_id_str,
                                        "early",
                                        &tenant_id,
                                    ).await;
                                    tracing::info!(phone_aor = %callee_aor, ext_aor = %ext_aor, dev_aor = %ext_dev_aor, "Mirrored Ringing from phone number to extension device AOR");
                                }
                            }

                            // If the callee AOR is a device (non-numeric user part), resolve
                            // it to the extension(s) that target this device.
                            let dev_ext_aors = resolve_device_aor_to_extension_aors(&callee_aor, &tenant_id, &storage).await;
                            for ext_aor in &dev_ext_aors {
                                let update = fcp_presence::PresenceUpdate {
                                    aor: ext_aor.clone(),
                                    state: fcp_presence::PresenceState::Ringing,
                                    note: Some("Ringing".to_string()),
                                };
                                let _ = presence.update_presence(update, &tenant_id).await;
                                presence.add_active_call(ext_aor, fcp_presence::ActiveCallInfo {
                                    call_id: call_id_str.clone(),
                                    remote_uri: caller_uri.clone(),
                                    remote_display_name: None,
                                    direction: fcp_presence::CallDirection::Recipient,
                                    dialog_state: fcp_presence::DialogState::Early,
                                });
                                fcp_presence::events::dialog_changed(
                                    presence.event_publisher(),
                                    ext_aor,
                                    &call_id_str,
                                    "early",
                                    &tenant_id,
                                ).await;
                                tracing::info!(device_aor = %callee_aor, ext_aor = %ext_aor, "Mirrored Ringing from device AOR to extension");
                            }
                        }
                        if let Some(caller_aor) = extract_aor(from) {
                            presence.set_on_the_phone(&caller_aor, &tenant_id).await;

                            // Track active call for caller (Initiator, Confirmed state)
                            let callee_uri = to.clone();
                            presence.add_active_call(&caller_aor, fcp_presence::ActiveCallInfo {
                                call_id: call_id_str.clone(),
                                remote_uri: callee_uri,
                                remote_display_name: None,
                                direction: fcp_presence::CallDirection::Initiator,
                                dialog_state: fcp_presence::DialogState::Confirmed,
                            });
                            fcp_presence::events::dialog_changed(
                                presence.event_publisher(),
                                &caller_aor,
                                &call_id_str,
                                "confirmed",
                                &tenant_id,
                            ).await;
                        }
                    }

                    // Call ended or failed → remove active call tracking, revert presence
                    PlatformEvent::Call(fcp_events::CallEvent::Ended { call_id, .. })
                    | PlatformEvent::Call(fcp_events::CallEvent::Failed { call_id, .. }) => {
                        let call_id_str = call_id.to_string();
                        // Collect AORs affected before removing the call.
                        // Use active_calls directly (not get_all_presences) so we
                        // find AORs even when no presentity entry exists.
                        let affected_aors = presence.get_aors_with_active_call(&call_id_str);
                        tracing::info!(%call_id, ?affected_aors, "Presence: CallEvent::Ended/Failed — cleaning up");
                        presence.remove_active_call_by_id(&call_id_str);
                        for aor in &affected_aors {
                            fcp_presence::events::dialog_changed(
                                presence.event_publisher(),
                                aor,
                                &call_id_str,
                                "terminated",
                                &tenant_id,
                            ).await;
                            // If this AOR has no remaining active calls, revert
                            // presence to Open (only if in a call-related state).
                            if presence.get_active_calls(aor).is_empty() {
                                // Bypass auto_phone_state — the system set the
                                // state on Initiated, so the system must clear it.
                                let is_call_state = presence.get_presence(aor)
                                    .map(|p| matches!(p.state,
                                        fcp_presence::PresenceState::OnThePhone
                                        | fcp_presence::PresenceState::Ringing))
                                    .unwrap_or(false);
                                if is_call_state {
                                    let update = fcp_presence::PresenceUpdate {
                                        aor: aor.clone(),
                                        state: fcp_presence::PresenceState::Open,
                                        note: None,
                                    };
                                    let _ = presence.update_presence(update, &tenant_id).await;
                                }
                            }
                        }
                        tracing::debug!(%call_id, "Call ended/failed — active call removed from dialog tracking");
                    }

                    // Registration → set Open
                    PlatformEvent::Registration(fcp_events::RegistrationEvent::Registered { aor, .. }) => {
                        if let Some(aor) = extract_aor(aor) {
                            let update = fcp_presence::PresenceUpdate {
                                aor: aor.clone(),
                                state: fcp_presence::PresenceState::Open,
                                note: None,
                            };
                            let _ = presence.update_presence(update, &tenant_id).await;

                            // Also set Open for extension(s) this device belongs to
                            let ext_aors = resolve_device_aor_to_extension_aors(&aor, &tenant_id, &storage).await;
                            for ext_aor in &ext_aors {
                                let update = fcp_presence::PresenceUpdate {
                                    aor: ext_aor.clone(),
                                    state: fcp_presence::PresenceState::Open,
                                    note: None,
                                };
                                let _ = presence.update_presence(update, &tenant_id).await;
                                tracing::info!(device_aor = %aor, ext_aor = %ext_aor, "Mirrored registration Open to extension");
                            }
                        }
                    }

                    // Unregistration → set Closed
                    PlatformEvent::Registration(fcp_events::RegistrationEvent::Unregistered { aor, .. }) => {
                        if let Some(aor) = extract_aor(aor) {
                            let update = fcp_presence::PresenceUpdate {
                                aor: aor.clone(),
                                state: fcp_presence::PresenceState::Closed,
                                note: None,
                            };
                            let _ = presence.update_presence(update, &tenant_id).await;

                            // Also set Closed for extension(s) this device belongs to
                            let ext_aors = resolve_device_aor_to_extension_aors(&aor, &tenant_id, &storage).await;
                            for ext_aor in &ext_aors {
                                let update = fcp_presence::PresenceUpdate {
                                    aor: ext_aor.clone(),
                                    state: fcp_presence::PresenceState::Closed,
                                    note: None,
                                };
                                let _ = presence.update_presence(update, &tenant_id).await;
                                tracing::info!(device_aor = %aor, ext_aor = %ext_aor, "Mirrored unregistration Closed to extension");
                            }
                        }
                    }

                    // Conference participant joined → set Busy (in conference)
                    PlatformEvent::Conference(fcp_events::ConferenceEvent::ParticipantJoined {
                        participant_id,
                        tenant_id: event_tenant_id,
                        ..
                    }) => {
                        let aor = resolve_conference_participant_aor(participant_id, &presence);
                        if let Some(aor) = aor {
                            let update = fcp_presence::PresenceUpdate {
                                aor,
                                state: fcp_presence::PresenceState::Busy,
                                note: Some("In conference".to_string()),
                            };
                            let _ = presence.update_presence(update, event_tenant_id).await;
                        }
                    }

                    // Conference participant left → revert to Open if no other active calls
                    PlatformEvent::Conference(fcp_events::ConferenceEvent::ParticipantLeft {
                        participant_id,
                        tenant_id: event_tenant_id,
                        ..
                    }) => {
                        let aor = resolve_conference_participant_aor(participant_id, &presence);
                        if let Some(aor) = aor {
                            if presence.get_active_calls(&aor).is_empty() {
                                let update = fcp_presence::PresenceUpdate {
                                    aor,
                                    state: fcp_presence::PresenceState::Open,
                                    note: None,
                                };
                                let _ = presence.update_presence(update, event_tenant_id).await;
                            }
                        }
                    }

                    // MWI updated → annotate presence note with voicemail count
                    PlatformEvent::Voicemail(fcp_events::VoicemailEvent::MwiUpdated {
                        mailbox_id,
                        new_messages,
                        ..
                    }) => {
                        if *new_messages > 0 {
                            let aor = resolve_mailbox_aor(mailbox_id, &presence);
                            if let Some(aor) = aor {
                                if let Some(info) = presence.get_presence(&aor) {
                                    let note = format!("{} new voicemail(s)", new_messages);
                                    let update = fcp_presence::PresenceUpdate {
                                        aor,
                                        state: info.state,
                                        note: Some(note),
                                    };
                                    let _ = presence.update_presence(update, &tenant_id).await;
                                }
                            }
                        }
                    }

                    _ => {}
                }
            }
            None => {
                warn!("Presence event stream ended, resubscribing...");
                tokio::time::sleep(Duration::from_millis(100)).await;
                match event_bus.subscribe(filter.clone()).await {
                    Ok(new_stream) => {
                        stream = new_stream;
                        info!("Presence event listener resubscribed");
                    }
                    Err(e) => {
                        warn!(%e, "Failed to resubscribe presence event listener");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

/// Subscribe to ConfigChanged events and evict matching cache entries.
///
/// This ensures that when an admin updates a user, extension, or device via the
/// API, all nodes' in-memory caches are invalidated so the next lookup fetches
/// fresh data from the database.
async fn run_cache_invalidation_listener(
    event_bus: Arc<InMemoryEventBus>,
    cached_creds: Arc<fcp_core::CachedCredentialStore>,
    cached_exts: Arc<fcp_core::CachedExtensionStore>,
    cached_devs: Arc<fcp_core::CachedDeviceStore>,
) {
    let filter = EventFilter::new().types(vec![EventType::System]);

    let mut stream = match event_bus.subscribe(filter.clone()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%e, "Failed to subscribe to event bus for cache invalidation");
            return;
        }
    };

    tracing::info!("Cache invalidation listener running");

    loop {
        match stream.next().await {
            Some(event) => {
                if let fcp_events::PlatformEvent::System(
                    fcp_events::SystemEvent::ConfigChanged {
                        entity_type,
                        entity_id,
                        ..
                    },
                ) = &event
                {
                    match entity_type.as_str() {
                        "user" => {
                            // Evict all credential cache entries — we don't have the
                            // username, only the user_id, so clear the whole cache.
                            cached_creds.evict_all();
                            tracing::debug!(
                                entity_id = %entity_id,
                                "Evicted credential cache (user config changed)"
                            );
                        }
                        "extension" => {
                            cached_exts.evict_all();
                            tracing::debug!(
                                entity_id = %entity_id,
                                "Evicted extension cache (extension config changed)"
                            );
                        }
                        "device" => {
                            cached_devs.evict_all();
                            tracing::debug!(
                                entity_id = %entity_id,
                                "Evicted device cache (device config changed)"
                            );
                        }
                        _ => {}
                    }
                }
            }
            None => {
                warn!("Cache invalidation stream ended, resubscribing...");
                tokio::time::sleep(Duration::from_millis(100)).await;
                match event_bus.subscribe(filter.clone()).await {
                    Ok(new_stream) => {
                        stream = new_stream;
                        info!("Cache invalidation listener resubscribed");
                    }
                    Err(e) => {
                        warn!(%e, "Failed to resubscribe cache invalidation listener");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

/// Subscribe to presence StatusChanged events and send NOTIFY to all watchers.
async fn run_presence_notifier(
    event_bus: Arc<InMemoryEventBus>,
    presence_handler: Arc<dyn fcp_sip::presence::PresenceHandler>,
) {
    use fcp_events::PlatformEvent;

    let filter = EventFilter::new().types(vec![EventType::Presence]);

    let mut stream = match event_bus.subscribe(filter.clone()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%e, "Failed to subscribe to event bus for presence NOTIFY");
            return;
        }
    };

    tracing::info!("Presence NOTIFY dispatcher running");

    loop {
        match stream.next().await {
            Some(event) => {
                match &event {
                    PlatformEvent::Presence(fcp_events::PresenceEvent::StatusChanged { aor, .. }) => {
                        if let Err(e) = presence_handler.notify_watchers(aor).await {
                            warn!(aor = %aor, error = %e, "Failed to notify watchers");
                        }
                    }
                    PlatformEvent::Presence(fcp_events::PresenceEvent::DialogChanged { aor, .. }) => {
                        if let Err(e) = presence_handler.notify_watchers(aor).await {
                            warn!(aor = %aor, error = %e, "Failed to notify dialog watchers");
                        }
                    }
                    _ => {}
                }
            }
            None => {
                warn!("Presence NOTIFY stream ended, resubscribing...");
                tokio::time::sleep(Duration::from_millis(100)).await;
                match event_bus.subscribe(filter.clone()).await {
                    Ok(new_stream) => {
                        stream = new_stream;
                        info!("Presence NOTIFY dispatcher resubscribed");
                    }
                    Err(e) => {
                        warn!(%e, "Failed to resubscribe presence NOTIFY dispatcher");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}
