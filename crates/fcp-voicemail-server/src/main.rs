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
use fcp_core::{storage::ExtensionStore, CredentialStore};
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
    init_tracing(&config);

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
            match cs.get_config(svc, section, tid).await {
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
    };

    let voicemail_service = Arc::new(VoicemailService::new(
        Arc::clone(&dyn_storage) as Arc<dyn fcp_core::storage::VoicemailStore>,
        Arc::clone(&media_manager),
        vm_config,
    ));

    info!("VoicemailService created");

    // 17. Create flow store and FlowCallHandler with voicemail service wired in
    let flow_store: Arc<dyn fcp_flow::FlowStore> = Arc::clone(&flow_store_db);

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

    // Auto-discover mailboxes from DB and register routes by extension, username,
    // phone number E.164, and any dial plan extensions that target the same phone number.
    {
        let tenant_id = config.service.default_tenant_id;

        // Pre-load extensions for alias resolution
        let all_extensions: Vec<fcp_core::Extension> =
            match dyn_storage.list_extensions(&tenant_id).await {
                Ok(exts) => {
                    info!(
                        count = exts.len(),
                        "Loaded extensions for voicemail alias resolution"
                    );
                    exts
                }
                Err(e) => {
                    warn!(error = %e, "Failed to list extensions for voicemail alias resolution");
                    Vec::new()
                }
            };

        match dyn_storage.list_mailboxes(&tenant_id).await {
            Ok(mailboxes) => {
                let mut auto_routes = 0u32;
                for mb in &mailboxes {
                    if !mb.enabled {
                        continue;
                    }
                    let flow_id = format!("vm-auto-{}", mb.id);
                    let flow_def = build_voicemail_flow(
                        &flow_id,
                        &mb.id,
                        &mb.extension,
                        config.voicemail.max_message_duration,
                        config.voicemail.play_beep,
                    );
                    match flow_store.get_flow(&flow_id).await {
                        Ok(Some(_)) => {} // Flow already exists, preserve user edits
                        _ => {
                            if let Err(e) = flow_store.insert_flow(flow_def).await {
                                warn!(mailbox_id = %mb.id, error = %e, "Failed to create auto-flow for mailbox");
                                continue;
                            }
                        }
                    }

                    // Register by owner's SIP username (primary identifier)
                    if let Ok(Some(user)) = dyn_storage.get_user_by_id(&mb.user_id).await {
                        flow_handler
                            .add_route(user.username.clone(), flow_id.clone())
                            .await;
                        // Also register by mailbox extension if different from username
                        if mb.extension != user.username {
                            flow_handler
                                .add_route(mb.extension.clone(), flow_id.clone())
                                .await;
                        }
                    } else {
                        // Fallback: register by extension if user not found
                        flow_handler
                            .add_route(mb.extension.clone(), flow_id.clone())
                            .await;
                    }
                    auto_routes += 1;

                    // Register by associated phone number E.164 and dial plan extensions
                    if let Ok(Some(lb)) = dyn_storage.get_line_binding(&mb.line_binding_id).await {
                        if let Ok(Some(pn)) =
                            dyn_storage.get_phone_number(&lb.phone_number_id).await
                        {
                            flow_handler
                                .add_route(pn.e164.clone(), flow_id.clone())
                                .await;
                            // Also register without leading +
                            if let Some(stripped) = pn.e164.strip_prefix('+') {
                                flow_handler
                                    .add_route(stripped.to_string(), flow_id.clone())
                                    .await;
                            }

                            // Register any dial plan extensions that target this phone number
                            for ext in &all_extensions {
                                if ext.enabled {
                                    if let fcp_core::ExtensionTarget::PhoneNumber(ref pn_id) =
                                        ext.target
                                    {
                                        if *pn_id == pn.id && ext.number != mb.extension {
                                            flow_handler
                                                .add_route(ext.number.clone(), flow_id.clone())
                                                .await;
                                        }
                                    }
                                }
                            }

                            info!(
                                mailbox = %mb.id,
                                extension = %mb.extension,
                                e164 = %pn.e164,
                                "Auto-registered voicemail route"
                            );
                        }
                    }
                }
                info!(auto_routes, "Auto-discovered mailbox routes from database");

                // Clean up orphaned vm-auto-* flows that no longer have a mailbox
                let mailbox_ids: std::collections::HashSet<String> =
                    mailboxes.iter().map(|mb| mb.id.clone()).collect();
                if let Ok(all_flows) = flow_store.list_flows(None).await {
                    for flow in &all_flows {
                        if let Some(mb_id) = flow.id.strip_prefix("vm-auto-") {
                            if !mailbox_ids.contains(mb_id) {
                                if let Err(e) = flow_store.delete_flow(&flow.id).await {
                                    warn!(flow_id = %flow.id, error = %e, "Failed to delete orphaned voicemail flow");
                                } else {
                                    info!(flow_id = %flow.id, "Deleted orphaned voicemail flow");
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to list mailboxes for auto-discovery");
            }
        }
    }

    // Register voicemail access extension if configured
    if let Some(ref access_ext) = config.voicemail.access_extension {
        let flow_id = "vm-access-flow";
        let flow_def = build_voicemail_access_flow(flow_id);
        if flow_store.get_flow(flow_id).await?.is_none() {
            flow_store
                .insert_flow(flow_def)
                .await
                .context("Failed to save voicemail access flow")?;
        }
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
        let event_bus_clone = Arc::clone(&event_bus);
        tokio::spawn(async move {
            if let Err(e) =
                run_event_forwarder(event_bus_clone, &events_url, events_auth_token).await
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
        let ws_url = build_events_ws_url(&ws_url, &tenant_id, config.events.auth_token.as_deref());

        let sub_flow_store = Arc::clone(&flow_store);
        let sub_flow_handler = Arc::clone(&flow_handler);
        let sub_storage = Arc::clone(&dyn_storage);
        let sub_max_duration = config.voicemail.max_message_duration;
        let sub_play_beep = config.voicemail.play_beep;
        tokio::spawn(async move {
            run_events_subscriber(
                &ws_url,
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
        webhook_url: None,
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
        webhook_url: None,
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
                tokio::spawn(async move {
                    let mut request = client.post(&url).json(&event);
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

fn build_events_ws_url(base_url: &str, tenant_id: &str, auth_token: Option<&str>) -> String {
    let base = format!("{}/ws/{}", base_url, tenant_id);
    match auth_token {
        Some(token) => format!("{}?token={}", base, token),
        None => base,
    }
}

/// Subscribe to the events server WebSocket and handle mailbox lifecycle events.
async fn run_events_subscriber(
    ws_url: &str,
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
        match tokio_tungstenite::connect_async(ws_url).await {
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
                                                        if let Err(e) = flow_store.delete_flow(&flow_id).await {
                                                            warn!(flow_id = %flow_id, error = %e, "Failed to delete flow for mailbox");
                                                        }
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
async fn handle_mailbox_created(
    mailbox_id: &str,
    flow_store: &Arc<dyn fcp_flow::FlowStore>,
    flow_handler: &Arc<fcp_sip::flow_handler::FlowCallHandler>,
    storage: &Arc<fcp_core::DynStorage>,
    tenant_id: fcp_core::TenantId,
    max_message_duration: u32,
    play_beep: bool,
) {
    use fcp_core::storage::{ExtensionStore, LineBindingStore, PhoneNumberStore};

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

    // Insert or update flow
    match flow_store.get_flow(&flow_id).await {
        Ok(Some(_)) => {
            // Flow exists — update it
            if let Err(e) = flow_store.update_flow(flow_def).await {
                warn!(flow_id = %flow_id, error = %e, "Failed to update flow");
            }
        }
        _ => {
            if let Err(e) = flow_store.insert_flow(flow_def).await {
                warn!(flow_id = %flow_id, error = %e, "Failed to create flow");
                return;
            }
        }
    }

    // Register by owner's SIP username
    if let Ok(Some(user)) = storage.get_user_by_id(&mb.user_id).await {
        flow_handler
            .add_route(user.username.clone(), flow_id.clone())
            .await;
        if mb.extension != user.username {
            flow_handler
                .add_route(mb.extension.clone(), flow_id.clone())
                .await;
        }
    } else {
        flow_handler
            .add_route(mb.extension.clone(), flow_id.clone())
            .await;
    }

    // Register by phone number and dial plan extensions
    if let Ok(Some(lb)) = storage.get_line_binding(&mb.line_binding_id).await {
        if let Ok(Some(pn)) = storage.get_phone_number(&lb.phone_number_id).await {
            flow_handler
                .add_route(pn.e164.clone(), flow_id.clone())
                .await;
            if let Some(stripped) = pn.e164.strip_prefix('+') {
                flow_handler
                    .add_route(stripped.to_string(), flow_id.clone())
                    .await;
            }

            // Register dial plan extensions targeting this phone number
            if let Ok(exts) = storage.list_extensions(&tenant_id).await {
                for ext in &exts {
                    if ext.enabled {
                        if let fcp_core::ExtensionTarget::PhoneNumber(ref pn_id) = ext.target {
                            if *pn_id == pn.id && ext.number != mb.extension {
                                flow_handler
                                    .add_route(ext.number.clone(), flow_id.clone())
                                    .await;
                            }
                        }
                    }
                }
            }

            info!(
                mailbox = %mb.id,
                extension = %mb.extension,
                e164 = %pn.e164,
                flow_id = %flow_id,
                "Dynamically registered voicemail route"
            );
        }
    }
}
