//! fcp-voicemail-server — Standalone voicemail server.
//!
//! Accepts SIP INVITEs for configured DIDs/extensions, plays greetings,
//! records messages, and manages MWI. Uses the same FlowCallHandler wiring
//! pattern as fcp-ivr with programmatic Answer → Voicemail → Hangup flows.
//!
//! Key difference from fcp-ivr:
//! - Creates a VoicemailService and passes it to FlowCallHandler
//! - Generates programmatic flow definitions per voicemail route (no JSON flows)
//! - No registrar, no LCR routing

use anyhow::{Context, Result};
use clap::Parser;
use fcp_core::storage::{LineBindingStore, PhoneNumberStore, VoicemailStore};
use fcp_core::{storage::ExtensionStore, CredentialStore, TenantStore};
use fcp_events::{EventFilter, EventSubscriber, InMemoryEventBus};
use fcp_media::{BackendType, MediaConfig};
use fcp_sbc_core::{
    CpsChecker, NoCpsLimit, TrunkCredentialProvider, TrunkHealthMonitor, TrunkRegistrar,
    TrunkRouter,
};
use fcp_sip::flow_handler::FlowCallHandler;
use fcp_sip::prelude::*;
use fcp_sip::service::{DialogService, SubscriptionService, TransactionService};
use fcp_sip::transport::ConnectionPool;
use fcp_voicemail::{VoicemailConfig, VoicemailService};
use sip_observe::{set_transport_metrics, TracingTransportMetrics};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

mod cli;
mod config;
mod drain;
mod health;
mod storage;

use cli::Cli;
use config::VoicemailServerConfig;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI arguments
    let cli = Cli::parse();

    // 2. Load configuration
    let mut config = fcp_core::config::check_stage(
        config::load_config(cli.config.as_deref()),
        cli.check_config,
    )?;
    // Secrets from the environment win over the file
    // (`docs/SHIP_AND_OPERATE.md` phase 2).
    if let Some(v) = &cli.database_url {
        config.storage.database_url = v.clone();
    }
    if let Some(v) = &cli.api_token {
        config.http.api_token = Some(v.clone());
    }
    if let Some(v) = &cli.events_auth_token {
        config.events.auth_token = Some(v.clone());
    }

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
    if let Some(ref ip) = cli.public_ip {
        if !ip.is_empty() {
            config.sip.local_ip = Some(ip.clone());
            config.media.public_ip = Some(ip.clone());
        }
    }

    // Auto-detect local IP if not explicitly configured and bind is 0.0.0.0
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
    fcp_core::config::check_stage(config::validate_config(&config), cli.check_config)?;
    if cli.check_config {
        println!(
            "{}",
            fcp_core::config::masked_toml(&config).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }
    init_tracing(&config);
    // On a cloud VM whose public address is NAT at the edge, services on
    // this host cannot reach one another by it (`fcp_sip::self_reach`).
    if let Some(ip) = config.media.public_ip.as_deref() {
        fcp_sip::self_reach::warn_if_unreachable(ip, "fcp-voicemail-server").await;
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-voicemail starting"
    );

    if let Some(ref ip) = config.sip.local_ip {
        info!(
            local_ip = %ip,
            source = if cli.public_ip.as_ref().is_some_and(|p| !p.is_empty()) { "PUBLIC_IP env" } else { "auto-detected" },
            "Using local IP for SIP signaling and media"
        );
    }

    // Set up observability
    if !set_transport_metrics(Arc::new(TracingTransportMetrics)) {
        warn!("transport metrics already configured");
    }

    // 4. Initialize storage
    let storage_result = storage::initialize_storage(&config.storage).await?;
    let replica_health = storage_result.replica.clone();
    let config_store = storage_result.config_store;
    let dyn_storage = storage_result.dyn_storage;
    let flow_store_db = storage_result.flow_store;

    // 4b. Seed operational config sections into database (DB primary, TOML seed)
    // Bootstrap sections (service, sip, storage, http, logging) stay TOML-only.
    {
        let cs = config_store.as_ref();
        let svc = "voicemail";
        let tid = "_system";

        let sections: Vec<(&str, serde_json::Value)> = vec![
            (
                "media",
                serde_json::to_value(&config.media).unwrap_or_default(),
            ),
            (
                "events",
                serde_json::to_value(&config.events).unwrap_or_default(),
            ),
            (
                "voicemail",
                serde_json::to_value(&config.voicemail).unwrap_or_default(),
            ),
            (
                "b2bua",
                serde_json::to_value(&config.b2bua).unwrap_or_default(),
            ),
            (
                "cluster",
                serde_json::to_value(&config.cluster).unwrap_or_default(),
            ),
        ];

        for (section, toml_val) in sections {
            match fcp_core::reconcile_with_seed(cs, svc, section, tid, &toml_val).await {
                Ok(Some(entry)) => {
                    match section {
                        "media" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.media = v;
                            }
                        }
                        "events" => {
                            if let Ok(mut v) =
                                serde_json::from_value::<config::EventsSection>(entry.value)
                            {
                                // Preserve auth_token from TOML — it should not be overridden by DB config
                                if v.auth_token.is_none() {
                                    v.auth_token = config.events.auth_token.clone();
                                }
                                config.events = v;
                            }
                        }
                        "voicemail" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.voicemail = v;
                            }
                        }
                        "b2bua" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.b2bua = v;
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

    // 5. Create SIP components
    let dialog_manager = DialogManager::new();
    let subscription_manager = SubscriptionManager::new();

    // 6. Create shared TCP connection pool
    let tcp_pool = Arc::new(ConnectionPool::new());

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

    // 10. Build credential provider from trunks (if any)
    let trunk_cred_provider: Option<Arc<TrunkCredentialProvider>> = if !config.trunks.is_empty() {
        Some(Arc::new(TrunkCredentialProvider::from_trunks(
            &config.trunks,
        )))
    } else {
        None
    };

    // 11. Create UAC for outbound calls
    let uac = create_uac(
        &config,
        Arc::clone(&transaction_manager_ref),
        dispatcher,
        trunk_cred_provider,
    )
    .await?;

    // 12. Create event bus
    let event_bus = Arc::new(InMemoryEventBus::new());
    info!("Event bus initialized (in-memory)");

    // Initialize Redis storage (if configured)
    #[cfg(feature = "storage-redis")]
    let redis_storage = if let Some(ref url) = config.storage.redis_url {
        match fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: url.clone(),
            ..Default::default()
        })
        .await
        {
            Ok(rs) => Some(rs),
            Err(e) => {
                warn!(error = %e, "Failed to connect to Redis; cluster disabled");
                None
            }
        }
    } else {
        None
    };

    // Start cluster membership (if configured)
    #[cfg(feature = "storage-redis")]
    let _cluster_manager = if config.cluster.enabled {
        if let Some(ref redis) = redis_storage {
            let member = fcp_cluster::ClusterMember {
                node_id: config.service.node_id.clone(),
                node_type: config.service.service_type.clone(),
                http_address: fcp_cluster::advertised_address(
                    config.http.bind_address,
                    config.sip.local_ip.as_deref(),
                ),
                sip_address: fcp_cluster::advertised_address(
                    config
                        .sip
                        .udp_bind
                        .parse()
                        .unwrap_or_else(|_| "0.0.0.0:5060".parse().unwrap()),
                    config.sip.local_ip.as_deref(),
                ),
                sip_tls_address: None,
                capabilities: vec!["voicemail".to_string(), "mwi".to_string()],
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
            let cm =
                fcp_cluster::ClusterManager::new(redis.clone(), member)
                    .with_intervals(
                        config.cluster.heartbeat_interval_secs,
                        config.cluster.heartbeat_ttl_secs,
                    )
                    .with_event_publisher(
                        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>
                    );
            let _handle = cm.start();
            info!("Cluster membership started");
            Some(Arc::new(cm))
        } else {
            warn!("cluster.enabled = true but redis_url is not set; cluster disabled");
            None
        }
    } else {
        None
    };

    // 13. Trunk infrastructure (optional)
    //
    // The node routes by its own `[[trunks]]` — the platform's — and by every
    // tenant's, read from the database and kept current as they change
    // (`docs/TENANT_TRUNKS.md` §4). Either may be empty at startup and filled
    // later, so the provider always exists.
    let trunk_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>;
    let outbound_route_provider: Option<Arc<dyn fcp_sip::OutboundRouteProvider>> = {
        let trunk_set = Arc::new(fcp_sbc_core::TrunkSet::platform_only(config.trunks.clone()));
        let health = Arc::new(TrunkHealthMonitor::new(&config.trunks));
        let cps: Arc<dyn CpsChecker> = Arc::new(NoCpsLimit);
        // This node never releases a call it counted on a trunk, so it
        // counts none rather than filling the trunk's channels for good.
        let router = Arc::new(
            TrunkRouter::with_trunk_set(Arc::clone(&trunk_set), Arc::clone(&health), cps)
                .without_call_counting(),
        );

        // Read the tenants' trunks now, and again whenever they change
        fcp_sbc_core::spawn_trunk_refresh(
            Arc::clone(&trunk_set),
            Arc::clone(&dyn_storage),
            Arc::clone(&health),
            Some(Arc::clone(&event_bus) as Arc<dyn fcp_events::EventSubscriber>),
            fcp_sbc_core::trunk_refresh::REFRESH_INTERVAL,
        )
        .await;

        // Probe every trunk the node routes by, the tenants' included
        tokio::spawn(fcp_sbc_core::run_health_probes(
            Arc::clone(&health),
            Arc::clone(&trunk_set),
            Arc::clone(&uac),
            fcp_sbc_core::HealthAnnouncer::new(
                Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
                config.service.node_id.clone(),
            ),
        ));

        // Keep the REGISTER bindings of every registering trunk this node
        // routes by — its own and the tenants' — one node to a trunk
        // (`docs/TENANT_TRUNKS.md` §8). The task runs whatever the node
        // starts with: a tenant's trunk arrives after startup.
        let trunk_registrar = Arc::new(TrunkRegistrar::new());
        {
            let (tx, rx) = tokio::sync::watch::channel(false);
            let ownership: Arc<dyn fcp_sbc_core::TrunkOwnership> = match _cluster_manager.as_ref() {
                Some(cm) => Arc::new(fcp_sbc_core::ClusterOwnership::new(Arc::clone(cm))),
                None => Arc::new(fcp_sbc_core::AloneOnThisNode),
            };
            tokio::spawn(fcp_sbc_core::run_trunk_registration(
                fcp_sbc_core::TrunkRegistrationContext {
                    registrar: Arc::clone(&trunk_registrar),
                    trunks: Arc::clone(&trunk_set),
                    health: Arc::clone(&health),
                    uac: Arc::clone(&uac),
                    ownership,
                    events: Some(Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>),
                    node_id: config.service.node_id.clone(),
                    registers_link: false,
                },
                rx,
            ));
            trunk_shutdown_tx = Some(tx);
        }

        info!(
            platform_trunks = config.trunks.len(),
            tenants_with_trunks = trunk_set.tenant_count(),
            "Trunk routing enabled"
        );
        Some(router as Arc<dyn fcp_sip::OutboundRouteProvider>)
    };

    // 14. Create media manager
    let media_manager = create_media_manager(&config).await?;

    // 15. Create CallManager for B2BUA call tracking
    let call_manager = Arc::new(fcp_sip::CallManager::with_storage_and_events(
        Arc::clone(&dyn_storage),
        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
    ));

    // Its directories, made if they are missing: the recorder refuses a
    // directory that does not exist, so a message was never kept where
    // nothing had made them (a fresh volume — D8).
    for dir in [
        &config.voicemail.recordings_dir,
        &config.voicemail.greetings_dir,
    ] {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("cannot make the voicemail directory {dir}"))?;
    }

    // 16. Create VoicemailService
    let vm_config = VoicemailConfig {
        recordings_dir: PathBuf::from(&config.voicemail.recordings_dir),
        greetings_dir: PathBuf::from(&config.voicemail.greetings_dir),
        max_message_duration: config.voicemail.max_message_duration,
        max_messages_per_mailbox: config.voicemail.max_messages_per_mailbox,
        play_beep: config.voicemail.play_beep,
        default_greeting_path: config
            .voicemail
            .default_greeting_path
            .as_ref()
            .map(PathBuf::from),
        beep_audio_path: config.voicemail.beep_audio_path.as_ref().map(PathBuf::from),
        message_dirs: Vec::new(),
    };

    let voicemail_service = {
        let service = VoicemailService::new(
            Arc::clone(&dyn_storage) as Arc<dyn fcp_core::storage::VoicemailStore>,
            Arc::clone(&media_manager),
            vm_config,
        );
        Arc::new(match media_manager.stt_service() {
            Some(stt) => service.with_stt(Arc::clone(stt)),
            None => service,
        })
    };

    info!("VoicemailService created");

    // 17. Create flow store and FlowCallHandler with voicemail service wired in
    // Each mailbox's flow is built here and held in memory, never written:
    // the stored ones belonged to no tenant, so no one could see or edit
    // them (`docs/DEEP_TESTING.md` D8). A mailbox's own call handling is a
    // flow of its tenant's, run by the call manager (its `flow_id`).
    let flow_store: Arc<dyn fcp_flow::FlowStore> =
        Arc::new(fcp_flow::BuiltinFlows::new(Arc::clone(&flow_store_db)));

    let local_uri = create_local_uri(&config)?;
    let contact_uri = local_uri.clone();
    let media_profile = MediaProfileBuilder::audio_only();
    let local_addr = config
        .sip
        .local_ip
        .clone()
        .or_else(|| config.media.public_ip.clone())
        .unwrap_or_else(|| derive_local_ip(&config.sip.udp_bind));

    let flow_handler = FlowCallHandler::new(
        Arc::clone(&flow_store),
        Some(Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>),
        Some(Arc::clone(&media_manager)),
    )
    .with_call_manager(Arc::clone(&call_manager))
    .with_transaction_manager(Arc::clone(&transaction_manager_ref))
    .with_dialog_manager(Arc::clone(&dialog_manager_ref))
    .with_uris(local_uri.clone(), contact_uri)
    .with_media_profile(
        media_profile,
        local_addr.clone(),
        config.media.rtp_port_start,
    )
    .with_voicemail_service(Arc::clone(&voicemail_service));

    let flow_handler = Arc::new(flow_handler);

    // 18. Create programmatic voicemail flows for each route
    for route in &config.voicemail.routes {
        let flow_id = format!("vm-flow-{}", route.mailbox_id);
        let flow_def = build_voicemail_flow(
            &flow_id,
            &route.mailbox_id,
            &route.did,
            config.voicemail.max_message_duration,
            config.voicemail.play_beep,
        );

        // Only insert if the flow doesn't already exist (preserves user edits)
        if flow_store.get_flow(&flow_id).await?.is_none() {
            flow_store.insert_flow(flow_def).await.with_context(|| {
                format!("Failed to save voicemail flow for route {}", route.did)
            })?;
        }

        flow_handler
            .add_route(route.did.clone(), flow_id.clone())
            .await;

        info!(
            did = %route.did,
            mailbox_id = %route.mailbox_id,
            flow_id = %flow_id,
            "Voicemail route registered"
        );
    }

    // Register default mailbox route if configured
    if let Some(ref default_mailbox_id) = config.voicemail.default_mailbox_id {
        let flow_id = format!("vm-flow-{}", default_mailbox_id);
        let flow_def = build_voicemail_flow(
            &flow_id,
            default_mailbox_id,
            "default",
            config.voicemail.max_message_duration,
            config.voicemail.play_beep,
        );

        if flow_store.get_flow(&flow_id).await?.is_none() {
            flow_store.insert_flow(flow_def).await?;
        }
        flow_handler
            .add_route("_default".to_string(), flow_id)
            .await;

        info!(
            default_mailbox_id = %default_mailbox_id,
            "Default voicemail route registered"
        );
    }

    // Every tenant's mailboxes, routed by every name a call for one may
    // arrive under (`register_mailbox_routes`). Only the default tenant's
    // were once, so another tenant's mailboxes answered nothing after a
    // restart.
    {
        let tenants: Vec<fcp_core::TenantId> = match dyn_storage.list_tenants().await {
            Ok(tenants) => tenants.into_iter().map(|t| t.id).collect(),
            Err(e) => {
                warn!(error = %e, "Failed to list tenants; routing the default tenant's mailboxes only");
                vec![config.service.default_tenant_id]
            }
        };
        let mut auto_routes = 0u32;
        for tenant_id in &tenants {
            let extensions = match dyn_storage.list_extensions(tenant_id).await {
                Ok(exts) => exts,
                Err(e) => {
                    warn!(tenant_id = %tenant_id, error = %e, "Failed to list extensions for voicemail alias resolution");
                    Vec::new()
                }
            };
            let mailboxes = match dyn_storage.list_mailboxes(tenant_id).await {
                Ok(mailboxes) => mailboxes,
                Err(e) => {
                    warn!(tenant_id = %tenant_id, error = %e, "Failed to list mailboxes for auto-discovery");
                    continue;
                }
            };
            for mb in mailboxes.iter().filter(|mb| mb.enabled) {
                let flow_id = format!("vm-auto-{}", mb.id);
                let flow_def = build_voicemail_flow(
                    &flow_id,
                    &mb.id,
                    &mb.extension,
                    config.voicemail.max_message_duration,
                    config.voicemail.play_beep,
                );
                if let Err(e) = flow_store.hold_flow(flow_def).await {
                    warn!(mailbox_id = %mb.id, error = %e, "Failed to build the mailbox's flow");
                    continue;
                }
                register_mailbox_routes(mb, &flow_id, &flow_handler, &dyn_storage, &extensions)
                    .await;
                auto_routes += 1;
            }
        }
        info!(
            auto_routes,
            tenants = tenants.len(),
            "Auto-discovered mailbox routes from database"
        );
    }

    // Register voicemail access extension if configured
    if let Some(ref access_ext) = config.voicemail.access_extension {
        let flow_id = "vm-access-flow";
        let flow_def = build_voicemail_access_flow(flow_id);
        flow_store
            .hold_flow(flow_def)
            .await
            .context("Failed to build the voicemail access flow")?;
        flow_handler
            .add_route(access_ext.clone(), flow_id.to_string())
            .await;
        info!(
            extension = %access_ext,
            flow_id = %flow_id,
            "Voicemail access route registered"
        );
    }

    info!(
        voicemail_routes = config.voicemail.routes.len(),
        default_mailbox = config
            .voicemail
            .default_mailbox_id
            .as_deref()
            .unwrap_or("none"),
        access_extension = config
            .voicemail
            .access_extension
            .as_deref()
            .unwrap_or("none"),
        "Voicemail flow routes loaded"
    );

    // 18b. What this node accepts (`docs/CAPACITY_AND_SURVIVABILITY.md`
    // §5). Every limit is optional: a voicemail server that writes no
    // `[admission]` section keeps none, as it always did.
    let admission = Arc::new(fcp_sbc_core::AdmissionController::new(
        config.admission.resolve(),
    ));

    // 19. Create SipMessageHandler — flow_handler = Some(...)
    let handler = Arc::new(
        SipMessageHandler::new(
            Arc::clone(&transaction_manager_ref),
            Arc::clone(&dialog_manager_ref),
            local_uri,
            Some(MediaProfileBuilder::audio_only()),
            local_addr,
            config.media.rtp_port_start,
            None, // registrar = None
            None, // routing_service = None
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
            None,          // No PBX handler
            None,          // No conference handler
            None,          // No queue handler
            None,          // presence_handler
            None,          // cluster_parking
            String::new(), // node_id
        )?
        .with_default_tenant_id(config.service.default_tenant_id)
        .with_admission(Some(
            Arc::clone(&admission) as Arc<dyn fcp_sip::admission::NodeAdmission>
        )),
    );

    // 19b. How this node leaves service
    // (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): the same sequence whether
    // it starts at `POST /drain` or at SIGTERM, with the messages already
    // spoken saved before the node stops.
    #[cfg(feature = "storage-redis")]
    let announcer = fcp_sbc_core::ClusterAnnouncer::maybe(_cluster_manager.clone());
    #[cfg(not(feature = "storage-redis"))]
    let announcer: Option<Arc<dyn fcp_sip::drain::DrainAnnouncer>> = None;
    let drain = Arc::new(fcp_sip::drain::Drain::new(
        Arc::clone(&admission) as Arc<dyn fcp_sip::admission::NodeAdmission>,
        Arc::new(drain::CallersLeavingMessages::new(
            fcp_sip::drain::NodeCalls::new(Arc::clone(&handler), Arc::clone(&call_manager))
                .with_announcer(announcer),
            Arc::clone(&voicemail_service),
        )) as Arc<dyn fcp_sip::drain::DrainTarget>,
        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
        config.service.node_id.clone(),
        config.service.drain_wait_secs,
    ));

    // 19c. Say this process is there, member or not (`docs/SERVICES.md`).
    #[cfg(feature = "storage-redis")]
    let beacon = config.storage.redis_url.as_ref().map(|url| {
        let (calls, leaving) = (Arc::clone(&drain), Arc::clone(&drain));
        let mut beacon = fcp_cluster::ServiceBeacon::new(
            fcp_cluster::ServiceIdentity::new(&config.service.node_id, "voicemail")
                .region(fcp_cluster::resolve_region(
                    config.cluster.region.as_deref(),
                ))
                .member(config.cluster.enabled)
                .http(fcp_cluster::advertised_address(
                    config.http.bind_address,
                    config.sip.local_ip.as_deref(),
                ))
                .sip(&config.sip.udp_bind),
        )
        .check(fcp_cluster::Check::tenants(
            Arc::clone(&dyn_storage) as Arc<dyn fcp_core::storage::TenantStore>
        ))
        .load(move || {
            let drain = Arc::clone(&calls);
            async move {
                std::collections::BTreeMap::from([(
                    "calls".to_string(),
                    drain.carrying().await as u64,
                )])
            }
        })
        .draining(move || leaving.is_draining());
        if let Some(events) = &config.events.server_url {
            beacon = beacon.check(fcp_cluster::Check::health("events", events));
        }
        let handle = beacon.start(fcp_storage_redis::RedisConfig {
            url: url.clone(),
            ..Default::default()
        });
        drain.set_health(handle.health());
        handle
    });

    // 20. Create and start TransportListener
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
        #[cfg(feature = "tls")]
        tls_bind: None,
        #[cfg(feature = "tls")]
        tls_cert: config.sip.tls_cert_path.clone(),
        #[cfg(feature = "tls")]
        tls_key: config.sip.tls_key_path.clone(),
        #[cfg(feature = "tls")]
        tls_client_ca: None,
        #[cfg(feature = "tls")]
        tls_client_auth: None,
        ws_bind,
        wss_bind,
    };

    // What one source may send, and the peers that carry many parties'
    // traffic and are exempt (`docs/DEEP_TESTING.md` D2c).
    fcp_sip::ingress::set_limit(config.sip.max_messages_per_source)?;
    let mut exempt = fcp_sip::ingress::Exemptions::new().networks(
        "[sip] trusted_sources",
        fcp_sip::emergency::TrustedEdges::parse(&config.sip.trusted_sources)
            .context("[sip] trusted_sources")?,
    );
    if let Some(cm) = &_cluster_manager {
        exempt = exempt.check("cluster members", cm.sip_peer_check());
    }
    exempt.install();

    let handler_dyn: Arc<dyn InboundMessageHandler> =
        Arc::clone(&handler) as Arc<dyn InboundMessageHandler>;
    let mut listener = TransportListener::new(listener_config, handler_dyn);
    listener
        .start()
        .await
        .context("Failed to start transport listeners")?;

    // 21. Spawn TCP response routing task
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
                    packet.peer_identity().cloned(),
                )
                .await
            {
                tracing::error!("Failed to handle inbound TCP response: {}", e);
            }
        }
    });

    // 22. Spawn HTTP server (voicemail health routes)
    let vm_state = Arc::new(health::VoicemailState {
        replica: replica_health.clone(),
        call_manager: Arc::clone(&call_manager),
        flow_handler: Arc::clone(&flow_handler),
        dyn_storage: Arc::clone(&dyn_storage),
        start_time: Instant::now(),
        api_token: config.http.api_token.clone(),
        drain: Arc::clone(&drain),
    });
    let http_bind = config.http.bind_address;
    tokio::spawn(async move {
        if let Err(e) = health::run_http_server(http_bind, vm_state).await {
            tracing::error!(%e, "HTTP API server exited");
        }
    });

    // 23. Spawn event forwarding task (if configured)
    if let Some(ref events_url) = config.events.server_url {
        let events_url = events_url.clone();
        let events_auth_token = config.events.auth_token.clone();
        let events_node_id = config.service.node_id.clone();
        let event_bus_clone = Arc::clone(&event_bus);
        tokio::spawn(async move {
            if let Err(e) = run_event_forwarder(
                event_bus_clone,
                &events_url,
                events_auth_token,
                events_node_id,
            )
            .await
            {
                tracing::error!(%e, "Event forwarder task failed");
            }
        });
        info!(events_url = %config.events.server_url.as_deref().unwrap_or(""), "Event forwarding enabled");
    }

    // 23b. Spawn event subscriber (receives mailbox lifecycle events from admin API)
    if let Some(ref events_url) = config.events.server_url {
        let ws_url = events_url
            .trim_end_matches("/ingest")
            .trim_end_matches('/')
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        let tenant_id = config.service.default_tenant_id.to_string();
        let ws_url = events_ws_url(&ws_url, &tenant_id);
        let sub_auth_token = config.events.auth_token.clone();

        let sub_flow_store = Arc::clone(&flow_store);
        let sub_flow_handler = Arc::clone(&flow_handler);
        let sub_storage = Arc::clone(&dyn_storage);
        let sub_max_duration = config.voicemail.max_message_duration;
        let sub_play_beep = config.voicemail.play_beep;
        tokio::spawn(async move {
            run_events_subscriber(
                &ws_url,
                sub_auth_token,
                sub_flow_store,
                sub_flow_handler,
                sub_storage,
                config.service.default_tenant_id,
                sub_max_duration,
                sub_play_beep,
            )
            .await;
        });
        info!("Events subscriber started (listening for mailbox lifecycle events)");
    }

    // 24. Log startup summary
    info!(
        udp = %config.sip.udp_bind,
        tcp = %config.sip.tcp_bind,
        http = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        voicemail_routes = config.voicemail.routes.len(),
        default_mailbox = config.voicemail.default_mailbox_id.as_deref().unwrap_or("none"),
        trunks = config.trunks.len(),
        events_forwarding = config.events.server_url.is_some(),
        "fcp-voicemail ready — listening for SIP INVITE messages"
    );

    // 24b. Spawn config poll loop (checks DB every 10s for config changes)
    {
        let cs = Arc::clone(&config_store);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_version: i64 = 0;
            loop {
                interval.tick().await;
                match cs.get_latest_version("voicemail", "_system").await {
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

    // 25. Wait for shutdown signal
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to register SIGTERM handler");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, initiating graceful shutdown...");
        }
    }

    // 26. Graceful shutdown

    // Leave service before stopping (`docs/CAPACITY_AND_SURVIVABILITY.md`
    // §6): take no new calls, give the callers leaving a message until
    // `drain_wait_secs` to finish, then hang up what is left — saving each
    // message spoken so far. With `drain_wait_secs = 0` — the default —
    // this is what it always was.
    if config.service.drain_wait_secs > 0 || drain.is_draining() {
        fcp_sip::drain::drain_for_shutdown(&drain).await;
    } else {
        drain.start().await;
    }

    // Shutdown cluster manager
    #[cfg(feature = "storage-redis")]
    if let Some(ref cm) = _cluster_manager {
        cm.shutdown().await;
    }
    #[cfg(feature = "storage-redis")]
    if let Some(beacon) = &beacon {
        beacon.shutdown().await;
    }

    if let Some(tx) = trunk_shutdown_tx {
        let _ = tx.send(true);
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
    listener.stop().await?;
    info!("Shutdown complete");
    Ok(())
}

/// Build a programmatic voicemail flow: Answer → Voicemail → Hangup
fn build_voicemail_flow(
    flow_id: &str,
    mailbox_id: &str,
    extension: &str,
    max_duration: u32,
    play_beep: bool,
) -> fcp_flow::FlowDefinition {
    use fcp_flow::{FlowDefinition, FlowNode, NodeType};

    let mut nodes = HashMap::new();

    nodes.insert(
        "answer".to_string(),
        FlowNode {
            id: "answer".to_string(),
            name: Some("Answer call".to_string()),
            node_type: NodeType::Answer {
                next: "voicemail".to_string(),
                on_error: None,
            },
        },
    );

    nodes.insert(
        "voicemail".to_string(),
        FlowNode {
            id: "voicemail".to_string(),
            name: Some(format!("Record voicemail for {}", extension)),
            node_type: NodeType::Voicemail {
                mailbox: mailbox_id.to_string(),
                is_busy: false,
                max_duration,
                play_beep,
                next_on_success: "hangup".to_string(),
                next_on_error: Some("hangup".to_string()),
            },
        },
    );

    nodes.insert(
        "hangup".to_string(),
        FlowNode {
            id: "hangup".to_string(),
            name: Some("Hang up".to_string()),
            node_type: NodeType::Hangup { reason: None },
        },
    );

    FlowDefinition {
        id: flow_id.to_string(),
        name: format!("Voicemail: {}", extension),
        description: Some(format!("Auto-generated voicemail flow for {}", extension)),
        version: 1,
        initial_node: "answer".to_string(),
        nodes,
        variables: HashMap::new(),
        tenant_id: None,
        enabled: true,
        created_at: None,
        updated_at: None,
    }
}

/// Build a programmatic voicemail access flow: Answer → VoicemailAccess → Hangup
///
/// The VoicemailAccess node handles the full IVR: PIN entry, message listing,
/// playback, save/delete — all within a single flow node.
fn build_voicemail_access_flow(flow_id: &str) -> fcp_flow::FlowDefinition {
    use fcp_flow::{FlowDefinition, FlowNode, NodeType};

    let mut nodes = HashMap::new();

    nodes.insert(
        "answer".to_string(),
        FlowNode {
            id: "answer".to_string(),
            name: Some("Answer call".to_string()),
            node_type: NodeType::Answer {
                next: "vm_access".to_string(),
                on_error: None,
            },
        },
    );

    nodes.insert(
        "vm_access".to_string(),
        FlowNode {
            id: "vm_access".to_string(),
            name: Some("Voicemail access IVR".to_string()),
            node_type: NodeType::VoicemailAccess {
                mailbox: "{{caller_extension}}".to_string(),
                max_pin_attempts: 3,
                next: "hangup".to_string(),
                on_auth_failure: Some("hangup".to_string()),
            },
        },
    );

    nodes.insert(
        "hangup".to_string(),
        FlowNode {
            id: "hangup".to_string(),
            name: Some("Hang up".to_string()),
            node_type: NodeType::Hangup { reason: None },
        },
    );

    FlowDefinition {
        id: flow_id.to_string(),
        name: "Voicemail Access".to_string(),
        description: Some("Auto-generated voicemail access IVR flow".to_string()),
        version: 1,
        initial_node: "answer".to_string(),
        nodes,
        variables: HashMap::new(),
        tenant_id: None,
        enabled: true,
        created_at: None,
        updated_at: None,
    }
}

/// Create a transport dispatcher with shared TCP pool.
async fn create_transport_dispatcher(
    config: &VoicemailServerConfig,
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

/// Create a UAC for outbound calls.
async fn create_uac(
    config: &VoicemailServerConfig,
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
        .local_uri(local_uri.as_str())?
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
fn create_local_uri(config: &VoicemailServerConfig) -> Result<SipUri> {
    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    if let Some(ref uri_str) = config.sip.local_uri {
        let uri = Uri::parse(uri_str).context("Failed to parse configured local_uri")?;
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
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind_addr.ip().to_string()
    };

    let uri_str = if bind_addr.port() != 0 && bind_addr.port() != 5060 {
        format!("sip:voicemail@{}:{}", domain, bind_addr.port())
    } else {
        format!("sip:voicemail@{}", domain)
    };

    let uri = Uri::parse(&uri_str).context("Failed to parse local URI")?;
    uri.as_sip()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

/// Create a media manager from configuration.
async fn create_media_manager(
    config: &VoicemailServerConfig,
) -> Result<Arc<fcp_media::MediaManager>> {
    let media_config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: config.media.rtp_port_start,
        rtp_port_end: config.media.rtp_port_end,
        public_ip: config.media.public_ip.clone(),
        recording_dir: config.voicemail.recordings_dir.clone(),
        stt: config.stt.clone(),
        ..Default::default()
    };

    let manager = fcp_media::MediaManager::new(media_config)
        .await
        .context("Failed to create media manager")?;

    info!(
        backend = manager.backend_name(),
        rtp_ports = format!(
            "{}-{}",
            config.media.rtp_port_start, config.media.rtp_port_end
        ),
        "Media manager created"
    );

    Ok(Arc::new(manager))
}

/// Derive local IP from bind address string.
fn derive_local_ip(bind_addr: &str) -> String {
    if let Ok(addr) = bind_addr.parse::<std::net::SocketAddr>() {
        if !addr.ip().is_unspecified() {
            return addr.ip().to_string();
        }
    }
    auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Auto-detect the local LAN IP by asking the OS which interface routes to the internet.
fn auto_detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:53").ok()?;
    let addr = socket.local_addr().ok()?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    Some(addr.ip().to_string())
}

/// Initialize tracing subscriber based on configuration.
fn init_tracing(config: &VoicemailServerConfig) {
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

/// Run the event forwarder task that subscribes to local events and POSTs them to fcp-events-server.
async fn run_event_forwarder(
    event_bus: Arc<InMemoryEventBus>,
    events_url: &str,
    auth_token: Option<String>,
    node_id: String,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("Failed to create HTTP client for event forwarding")?;

    let mut stream = event_bus
        .subscribe(EventFilter::new())
        .await
        .context("Failed to subscribe to event bus")?;

    info!(url = %events_url, "Event forwarder started");

    loop {
        match stream.next().await {
            Some(event) => {
                tracing::debug!(?event, "Forwarding event to events server");
                let client = client.clone();
                let url = events_url.to_string();
                let auth_token = auth_token.clone();
                let node_id = node_id.clone();
                tokio::spawn(async move {
                    // Which node raised it: the events server files it
                    // under this on its stream.
                    let mut request = client.post(&url).header("X-Node-ID", node_id).json(&event);
                    if let Some(token) = auth_token.as_deref() {
                        request = request.bearer_auth(token);
                    }

                    match request.send().await {
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

/// The events server's WebSocket for `tenant_id`. The token goes in an
/// `Authorization` header (`events_ws_request`), never the URL, which is
/// logged (#340).
fn events_ws_url(base_url: &str, tenant_id: &str) -> String {
    format!("{}/ws/{}", base_url, tenant_id)
}

/// A connect request for `url` carrying `auth_token` as a bearer token.
fn events_ws_request(
    url: &str,
    auth_token: Option<&str>,
) -> anyhow::Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request()?;
    if let Some(token) = auth_token {
        let value = format!("Bearer {token}")
            .parse()
            .map_err(|_| anyhow::anyhow!("the events token is not a valid header value"))?;
        request.headers_mut().insert("authorization", value);
    }
    Ok(request)
}

/// Subscribe to the events server WebSocket and handle mailbox lifecycle events.
async fn run_events_subscriber(
    ws_url: &str,
    auth_token: Option<String>,
    flow_store: Arc<dyn fcp_flow::FlowStore>,
    flow_handler: Arc<fcp_sip::flow_handler::FlowCallHandler>,
    storage: Arc<fcp_core::DynStorage>,
    tenant_id: fcp_core::TenantId,
    max_message_duration: u32,
    play_beep: bool,
) {
    use futures_util::StreamExt;

    loop {
        info!(url = %ws_url, "Connecting to events server WebSocket for mailbox events");
        let request = match events_ws_request(ws_url, auth_token.as_deref()) {
            Ok(request) => request,
            Err(e) => {
                warn!(url = %ws_url, error = %e, "The events server WebSocket request could not be built");
                return;
            }
        };
        match tokio_tungstenite::connect_async(request).await {
            Ok((ws_stream, _)) => {
                info!("Connected to events server WebSocket");
                let (_write, mut read) = ws_stream.split();

                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                            if let Ok(wrapper) = serde_json::from_str::<serde_json::Value>(&text) {
                                if wrapper.get("type").and_then(|t| t.as_str()) == Some("events") {
                                    if let Some(events) =
                                        wrapper.get("events").and_then(|e| e.as_array())
                                    {
                                        for event_val in events {
                                            if let Ok(event) =
                                                serde_json::from_value::<fcp_events::PlatformEvent>(
                                                    event_val.clone(),
                                                )
                                            {
                                                match event {
                                                    fcp_events::PlatformEvent::Voicemail(fcp_events::VoicemailEvent::MailboxCreated { ref mailbox_id, .. }) => {
                                                        info!(mailbox_id = %mailbox_id, "Received MailboxCreated event");
                                                        handle_mailbox_created(
                                                            mailbox_id,
                                                            &flow_store,
                                                            &flow_handler,
                                                            &storage,
                                                            tenant_id,
                                                            max_message_duration,
                                                            play_beep,
                                                        ).await;
                                                    }
                                                    fcp_events::PlatformEvent::Voicemail(fcp_events::VoicemailEvent::MailboxDeleted { ref mailbox_id, .. }) => {
                                                        info!(mailbox_id = %mailbox_id, "Received MailboxDeleted event");
                                                        let flow_id = format!("vm-auto-{}", mailbox_id);
                                                        flow_handler.remove_routes_for_flow(&flow_id).await;
                                                        let _ = flow_store.release_flow(&flow_id).await;
                                                        info!(mailbox_id = %mailbox_id, flow_id = %flow_id, "Removed voicemail routes");
                                                    }
                                                    fcp_events::PlatformEvent::Voicemail(fcp_events::VoicemailEvent::MailboxUpdated { ref mailbox_id, .. }) => {
                                                        info!(mailbox_id = %mailbox_id, "Received MailboxUpdated event");
                                                        // Re-register routes in case enabled/disabled changed
                                                        let flow_id = format!("vm-auto-{}", mailbox_id);
                                                        flow_handler.remove_routes_for_flow(&flow_id).await;
                                                        handle_mailbox_created(
                                                            mailbox_id,
                                                            &flow_store,
                                                            &flow_handler,
                                                            &storage,
                                                            tenant_id,
                                                            max_message_duration,
                                                            play_beep,
                                                        ).await;
                                                    }
                                                    _ => {} // Ignore other events
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                            info!("Events server WebSocket closed");
                            break;
                        }
                        Err(e) => {
                            warn!(%e, "Events server WebSocket error");
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                warn!(%e, "Failed to connect to events server WebSocket");
            }
        }
        // Reconnect after a short delay
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Handle a MailboxCreated (or MailboxUpdated) event: create flow and register routes.
/// Every name a call for `mb` may arrive under, routed to its flow: the
/// mailbox's id (what the call manager forwards to), its owner's SIP name
/// and extension, the mailbox's own extension, the extensions dialing the
/// owner, and the number on its line with the extensions dialing that.
/// Only the extensions dialing a number were routed once, so a call
/// forwarded as `1002` to a user's extension found no flow (`docs/DEEP_TESTING.md`
/// D8, findings 37–38).
async fn register_mailbox_routes(
    mb: &fcp_core::VoicemailMailbox,
    flow_id: &str,
    flow_handler: &Arc<fcp_sip::flow_handler::FlowCallHandler>,
    storage: &Arc<fcp_core::DynStorage>,
    extensions: &[fcp_core::Extension],
) {
    let mut keys = vec![mb.id.clone(), mb.extension.clone()];
    if let Ok(Some(user)) = storage.get_user_by_id(&mb.user_id).await {
        keys.push(user.sip_name().to_string());
        keys.extend(user.extension.clone());
    }
    let owner = mb.user_id.to_string();
    let mut e164 = None;
    let line = match &mb.line_binding_id {
        Some(id) => storage.get_line_binding(id).await,
        None => Ok(None),
    };
    let phone_number = match line {
        Ok(Some(lb)) => storage
            .get_phone_number(&lb.phone_number_id)
            .await
            .ok()
            .flatten(),
        _ => None,
    };
    if let Some(pn) = &phone_number {
        keys.push(pn.e164.clone());
        keys.extend(pn.e164.strip_prefix('+').map(str::to_string));
        e164 = Some(pn.e164.clone());
    }
    for ext in extensions.iter().filter(|ext| ext.enabled) {
        let dials_mailbox = match &ext.target {
            fcp_core::ExtensionTarget::User(user_id) => user_id.to_string() == owner,
            fcp_core::ExtensionTarget::PhoneNumber(pn_id) => {
                phone_number.as_ref().is_some_and(|pn| pn.id == *pn_id)
            }
            _ => false,
        };
        if dials_mailbox {
            keys.push(ext.number.clone());
        }
    }
    keys.retain(|k| !k.trim().is_empty());
    keys.sort();
    keys.dedup();
    for key in &keys {
        flow_handler
            .add_route(key.clone(), flow_id.to_string())
            .await;
    }
    info!(
        mailbox = %mb.id,
        extension = %mb.extension,
        e164 = ?e164,
        routes = ?keys,
        flow_id = %flow_id,
        "Registered voicemail routes"
    );
}

async fn handle_mailbox_created(
    mailbox_id: &str,
    flow_store: &Arc<dyn fcp_flow::FlowStore>,
    flow_handler: &Arc<fcp_sip::flow_handler::FlowCallHandler>,
    storage: &Arc<fcp_core::DynStorage>,
    tenant_id: fcp_core::TenantId,
    max_message_duration: u32,
    play_beep: bool,
) {
    use fcp_core::storage::ExtensionStore;

    let mb = match storage.get_mailbox(mailbox_id).await {
        Ok(Some(mb)) => mb,
        Ok(None) => {
            warn!(mailbox_id = %mailbox_id, "Mailbox not found in database");
            return;
        }
        Err(e) => {
            warn!(mailbox_id = %mailbox_id, error = %e, "Failed to look up mailbox");
            return;
        }
    };

    if !mb.enabled {
        info!(mailbox_id = %mailbox_id, "Mailbox is disabled, skipping route registration");
        return;
    }

    let flow_id = format!("vm-auto-{}", mb.id);
    let flow_def = build_voicemail_flow(
        &flow_id,
        &mb.id,
        &mb.extension,
        max_message_duration,
        play_beep,
    );

    // Built afresh and held in memory.
    if let Err(e) = flow_store.hold_flow(flow_def).await {
        warn!(flow_id = %flow_id, error = %e, "Failed to build the mailbox's flow");
        return;
    }

    let extensions = match storage.list_extensions(&tenant_id).await {
        Ok(exts) => exts,
        Err(e) => {
            warn!(error = %e, "Failed to list extensions for the mailbox's routes");
            Vec::new()
        }
    };
    register_mailbox_routes(&mb, &flow_id, flow_handler, storage, &extensions).await;
}

#[cfg(test)]
mod events_ws_tests {
    use super::*;

    /// The token rides in a header; the URL, which is logged, has none (#340).
    #[test]
    fn the_events_token_is_a_header_not_in_the_url() {
        let url = events_ws_url("ws://events:8090", "tenant-1");
        assert_eq!(url, "ws://events:8090/ws/tenant-1");
        let request = events_ws_request(&url, Some("s3cret")).unwrap();
        assert!(!request.uri().to_string().contains("s3cret"));
        assert_eq!(request.headers()["authorization"], "Bearer s3cret");
        let request = events_ws_request(&url, None).unwrap();
        assert!(request.headers().get("authorization").is_none());
    }
}
