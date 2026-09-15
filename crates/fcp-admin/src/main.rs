//! FCP Admin - Management and Administration API
//!
//! This binary provides the Admin API for FCP, focused on management and
//! administration operations. It's designed to run internally (behind an API gateway)
//! and handles CRUD operations for:
//!
//! - Tenant management
//! - User administration
//! - Device provisioning
//! - Extension configuration
//! - Phone number management
//! - System monitoring

use anyhow::{Context, Result};
use clap::Parser;
use fcp_events::{EventBus, InMemoryEventBus};
use fcp_sip::exports::{
    DialogManager, SubscriptionManager, TransactionManager, TransportContext, TransportDispatcher,
};
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::{B2buaConfig, CallManager};
use std::sync::Arc;
use tokio::signal;
use tracing::{info, warn};

/// No-op transport dispatcher for admin API.
///
/// The admin API doesn't do real SIP transport — it only needs the handler
/// for API operations (e.g. hangup calls via BYE). The handler requires a
/// `TransactionManager`, which requires a dispatcher.
struct NoopDispatcher;

#[async_trait::async_trait]
impl TransportDispatcher for NoopDispatcher {
    async fn dispatch(
        &self,
        _ctx: &TransportContext,
        _payload: bytes::Bytes,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

mod cli;
mod cluster_api;
mod config;
mod storage;

use cli::Cli;
use config::load_config;

/// Initialize tracing/logging for fcp-admin.
fn init_tracing(config: &config::AdminConfig) -> Result<()> {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let log_level = &config.logging.level;
    let log_format = &config.logging.format;

    // Create the EnvFilter from the log level
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .context("Failed to create log filter")?;

    // Build the subscriber based on format
    match log_format.as_str() {
        "json" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())
                .init();
        }
        "pretty" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().pretty())
                .init();
        }
        "compact" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().compact())
                .init();
        }
        _ => {
            // Default to JSON for production
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())
                .init();
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Load configuration
    let mut config = load_config(cli.config.as_deref())?;

    // Apply CLI overrides
    if let Some(log_level) = cli.log_level {
        config.logging.level = log_level;
    }
    if let Some(log_format) = cli.log_format {
        config.logging.format = log_format;
    }
    if let Some(bind_addr) = cli.bind_addr {
        config.api.bind_address = bind_addr.parse().context("Invalid bind address format")?;
    }
    if let Some(auth_token) = cli.auth_token {
        config.api.auth_token = Some(auth_token);
    }
    if let Some(provisioning_auth_token) = cli.provisioning_auth_token {
        config.api.provisioning_auth_token = Some(provisioning_auth_token);
    }
    if cli.provisioning_no_https {
        config.api.provisioning_require_https = false;
    }

    config::validate_config(&config)?;

    // Initialize tracing/logging
    init_tracing(&config).context("Failed to initialize tracing")?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        service = "fcp-admin",
        "Starting FCP Admin API"
    );

    // Initialize storage backend
    let storage_result = storage::initialize_storage(&config.storage)
        .await
        .context("Failed to initialize storage backend")?;
    let storage = storage_result.dyn_storage;
    let flow_store = storage_result.flow_store;
    let config_store = storage_result.config_store;

    info!("Storage backend initialized successfully");

    // The record table's upkeep: the coming months' partitions and the
    // hourly usage rollups (docs/CDR_PIPELINE.md §8-9).
    fcp_cdr::spawn_maintenance(
        Arc::clone(&storage) as Arc<dyn fcp_core::storage::CdrStore>,
        fcp_cdr::MaintenanceConfig::default(),
    );

    // Retention sweep: apply every tenant's compliance policy once an hour,
    // then the platform's upkeep (docs/COMPLIANCE_FOLLOWUPS.md §4).
    // Tenants without a policy (or with no retention windows) are skipped.
    {
        let sweep_storage = Arc::clone(&storage);
        let actor = format!("fcp-admin:{}", config.service.node_id);
        let upkeep = fcp_core::compliance::UpkeepConfig {
            file_roots: [
                config.call_recordings_dir.as_deref(),
                config.voicemail_recordings_dir.as_deref(),
                config.voicemail_greetings_dir.as_deref(),
                config.profile_pictures_dir.as_deref(),
            ]
            .into_iter()
            .flatten()
            .map(String::from)
            .chain(config.retention.extra_file_roots.iter().cloned())
            .map(std::path::PathBuf::from)
            .collect(),
            orphan_files_after: config
                .retention
                .orphan_files_days
                .map(|d| std::time::Duration::from_secs(u64::from(d) * 86_400)),
            system_traces_after: config
                .retention
                .system_trace_days
                .map(|d| std::time::Duration::from_secs(u64::from(d) * 86_400)),
            deleted_voicemail_grace: std::time::Duration::from_secs(
                u64::from(config.retention.deleted_voicemail_grace_days) * 86_400,
            ),
            ..Default::default()
        };
        tokio::spawn(async move {
            use fcp_core::TenantStore;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                let Some(store) = sweep_storage.compliance_store().cloned() else {
                    continue;
                };
                // One admin replica sweeps at a time.
                let lock = {
                    use fcp_core::storage::CdrStore;
                    match sweep_storage.try_maintenance_lock("fcp:retention").await {
                        Ok(Some(lock)) => lock,
                        Ok(None) => continue,
                        Err(e) => {
                            warn!(error = %e, "Retention sweep: could not take the lock");
                            continue;
                        }
                    }
                };
                let tenants = match sweep_storage.list_tenants().await {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(error = %e, "Retention sweep: failed to list tenants");
                        lock.release().await;
                        continue;
                    }
                };
                // The oldest start time every tenant lets go of: whole months
                // before it can be dropped once their rows are deleted.
                let now = std::time::SystemTime::now();
                let mut partition_cutoff = (!tenants.is_empty()).then_some(now);
                for tenant in tenants {
                    let policy = match store.get_compliance_policy(&tenant.id).await {
                        Ok(Some(p)) if p.has_retention() => p,
                        Ok(_) => {
                            partition_cutoff = None;
                            continue;
                        }
                        Err(e) => {
                            warn!(tenant_id = %tenant.id, error = %e, "Retention sweep: failed to load policy");
                            continue;
                        }
                    };
                    // A tenant whose policy requires the CDR archive lets
                    // go only of what the archive has confirmed.
                    let cdr_cutoff = match fcp_core::compliance::cdr_retention_cutoff(
                        &*sweep_storage,
                        &policy,
                        now,
                    )
                    .await
                    {
                        Ok((cutoff, _)) => cutoff,
                        Err(e) => {
                            warn!(tenant_id = %tenant.id, error = %e, "Retention sweep: no CDR archive frontier");
                            None
                        }
                    };
                    partition_cutoff = match (partition_cutoff, cdr_cutoff) {
                        (Some(c), Some(t)) => Some(c.min(t)),
                        _ => None,
                    };
                    let report =
                        fcp_core::compliance::run_retention(&sweep_storage, &policy, &actor).await;
                    if report.total_deleted() > 0 || !report.errors.is_empty() {
                        info!(
                            tenant_id = %tenant.id,
                            deleted = report.total_deleted(),
                            errors = ?report.errors,
                            "Retention sweep completed"
                        );
                    }
                    if let Err(e) = store.record_retention_run(&report).await {
                        warn!(tenant_id = %tenant.id, error = %e, "Retention sweep: failed to record run");
                    }
                }
                let upkept =
                    fcp_core::compliance::run_upkeep(&sweep_storage, &upkeep, &actor).await;
                if upkept != fcp_core::compliance::UpkeepReport::default() {
                    info!(report = ?upkept, "Platform upkeep completed");
                }
                if let Some(cutoff) = partition_cutoff {
                    use fcp_core::storage::CdrStore;
                    match sweep_storage.drop_cdr_partitions_before(cutoff).await {
                        Ok(dropped) if !dropped.is_empty() => {
                            info!(partitions = ?dropped, "Retention sweep: dropped CDR partitions")
                        }
                        Ok(_) => {}
                        Err(e) => {
                            warn!(error = %e, "Retention sweep: could not drop CDR partitions")
                        }
                    }
                }
                lock.release().await;
            }
        });
    }

    // Build lightweight SIP components (no real transport).
    // The admin API only needs the component graph for REST API operations
    // (e.g. hangup calls via BYE), not actual SIP traffic.
    let event_bus: Arc<dyn EventBus> = Arc::new(InMemoryEventBus::new());

    let dispatcher = Arc::new(NoopDispatcher);
    let transaction_manager = Arc::new(TransactionManager::new(
        dispatcher as Arc<dyn TransportDispatcher>,
    ));
    let dialog_manager = Arc::new(DialogManager::new());
    let subscription_manager = Arc::new(SubscriptionManager::new());
    let call_manager = Arc::new(CallManager::with_storage_and_events(
        storage.clone(),
        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
    ));

    let local_uri = fcp_sip::exports::SipUri::parse("sip:admin@127.0.0.1").unwrap();
    let handler = Arc::new(
        SipMessageHandler::new(
            transaction_manager,
            dialog_manager.clone(),
            local_uri,
            None,                    // media_profile
            "127.0.0.1".to_string(), // local_addr
            10000,                   // rtp_port
            None,                    // registrar
            None,                    // routing_service
            call_manager.clone(),
            None, // uac (no outbound SIP needed)
            Some(storage.clone()),
            None, // media_manager
            None, // media_config
            None, // flow_handler
            B2buaConfig::default(),
            None,          // outbound_route_provider
            None,          // pbx_handler
            None,          // conference_handler
            None,          // queue_handler
            None,          // presence_handler
            None,          // cluster_parking
            String::new(), // node_id
        )?
        .with_default_tenant_id(config.service.default_tenant_id),
    );

    let api_config = fcp_core::ApiConfig {
        enabled: true,
        bind: config.api.bind_address,
        auth_enabled: config.api.auth_enabled,
        auth_token: config.api.auth_token.clone(),
        provisioning_auth_token: config.api.provisioning_auth_token.clone(),
        provisioning_require_https: config.api.provisioning_require_https,
        allow_public_signup: config.api.allow_public_signup,
        metrics_enabled: false,
        metrics_path: "/metrics".to_string(),
        deployment_mode: config.api.deployment_mode,
    };
    let realm = "fcp.local".to_string();
    let default_tenant_id = config.service.default_tenant_id;

    info!(node_id = %config.service.node_id, "SIP components initialized (lightweight, no transport)");

    // Clone event_bus for event forwarding, subscribing, and presence listener
    let event_bus_for_forward = Arc::clone(&event_bus);
    let event_bus_for_subscribe: Arc<dyn EventBus> = Arc::clone(&event_bus);
    let event_bus_for_presence: Arc<dyn EventBus> = Arc::clone(&event_bus);

    // Derive events server base URL from ingest URL (strip /ingest suffix)
    let events_server_url = config.events.server_url.as_ref().map(|url| {
        url.trim_end_matches("/ingest")
            .trim_end_matches('/')
            .to_string()
    });

    // Create read-only PresenceManager so the admin UI can display presence data.
    // Keep event publishing disabled here: the admin only mirrors presence from
    // cluster events and must not emit those updates back onto its local bus.
    let presence_manager = {
        let presence_config = fcp_presence::PresenceConfig {
            enabled: true,
            ..Default::default()
        };
        // `set_pubsub` takes `&mut self`, but is only called under the `redis` feature.
        #[cfg_attr(not(feature = "redis"), allow(unused_mut))]
        let mut pm = fcp_presence::PresenceManager::new(presence_config, None);

        // Wire up Redis Pub/Sub for cross-node presence sync
        #[cfg(feature = "redis")]
        if let Some(ref redis_url) = config.cluster.redis_url {
            match fcp_presence::redis_pubsub::RedisPresencePubSub::new(
                redis_url,
                Some(config.service.node_id.clone()),
            ) {
                Ok(pubsub) => {
                    info!(
                        node_id = %pubsub.node_id(),
                        "Admin presence Redis Pub/Sub initialized"
                    );
                    pm.set_pubsub(std::sync::Arc::new(pubsub));
                    match pm.run_pubsub_listener().await {
                        Ok(_handle) => info!("Admin presence Redis Pub/Sub listener started"),
                        Err(e) => {
                            warn!(error = %e, "Failed to start admin presence Pub/Sub listener")
                        }
                    }
                }
                Err(e) => warn!(error = %e, "Failed to initialize admin presence Redis Pub/Sub"),
            }
        }

        pm
    };

    // Spawn presence event listener to keep the admin's PresenceManager in sync
    // with registration/call events received via the events server WebSocket.
    {
        let pm_clone = presence_manager.clone();
        let tenant_id = default_tenant_id;
        let storage_for_presence = Arc::clone(&storage);
        tokio::spawn(async move {
            run_presence_listener(
                event_bus_for_presence,
                pm_clone,
                tenant_id,
                storage_for_presence,
            )
            .await;
        });
        info!("Admin presence event listener started");
    }

    // Spawn trunk health probe background task
    let trunk_health_map = fcp_api_core::trunk_health::new_health_map();
    {
        let storage_for_health = Arc::clone(&storage);
        let health_map_clone = Arc::clone(&trunk_health_map);
        let tenant_id = default_tenant_id;
        tokio::spawn(fcp_api_core::trunk_health::run_trunk_health_probes(
            storage_for_health,
            health_map_clone,
            tenant_id,
        ));
        info!("Trunk health probe task started");
    }

    // Create API state
    let api_state = fcp_api_core::ApiState::new(
        Some(storage),
        Some(flow_store),
        dialog_manager,
        subscription_manager,
        call_manager,
        handler,
        None, // media_manager (admin doesn't need media)
        event_bus,
        None, // queue_manager (admin doesn't use queues)
        Some(presence_manager),
        realm,
        default_tenant_id,
    )
    .with_events_server_url(events_server_url.clone())
    .with_events_server_auth_token(config.events.auth_token.clone())
    .with_conference_server_url(config.conference.server_url.clone())
    .with_conference_server_auth_token(config.conference.auth_token.clone())
    .with_conference_join_secret(config.conference.join_secret.clone())
    .with_conference_gateway_url(config.conference.gateway_url.clone())
    .with_config_store(config_store)
    .with_trunk_health(trunk_health_map)
    .with_deployment_mode(config.api.deployment_mode);

    // Set security policy from config
    let api_state = api_state.with_security_policy(fcp_api_core::SecurityPolicy {
        max_failed_attempts: config.security.max_failed_attempts,
        lockout_duration_secs: config.security.lockout_duration_secs,
        password_expiration_days: config.security.password_expiration_days,
    });

    // Set voicemail greetings directory (default: /tmp/fcp/voicemail/greetings)
    let api_state = if let Some(ref dir) = config.voicemail_greetings_dir {
        api_state.with_voicemail_greetings_dir(std::path::PathBuf::from(dir))
    } else {
        api_state
            .with_voicemail_greetings_dir(std::path::PathBuf::from("/tmp/fcp/voicemail/greetings"))
    };

    // Set profile pictures directory (default: /tmp/fcp/profile-pictures)
    let api_state = if let Some(ref dir) = config.profile_pictures_dir {
        api_state.with_profile_pictures_dir(std::path::PathBuf::from(dir))
    } else {
        api_state.with_profile_pictures_dir(std::path::PathBuf::from("/tmp/fcp/profile-pictures"))
    };

    // Set voicemail recordings directory (default: /tmp/fcp/voicemail/recordings)
    let api_state = if let Some(ref dir) = config.voicemail_recordings_dir {
        api_state.with_voicemail_recordings_dir(std::path::PathBuf::from(dir))
    } else {
        api_state.with_voicemail_recordings_dir(std::path::PathBuf::from(
            "/tmp/fcp/voicemail/recordings",
        ))
    };

    // Set audio prompts directory (default: /tmp/fcp/audio/prompts)
    let api_state = if let Some(ref dir) = config.prompts_dir {
        api_state.with_prompts_dir(std::path::PathBuf::from(dir))
    } else {
        api_state.with_prompts_dir(std::path::PathBuf::from("/tmp/fcp/audio/prompts"))
    };
    let api_state = match config.call_recordings_dir.as_deref() {
        Some(dir) => api_state.with_call_recordings_dir(std::path::PathBuf::from(dir)),
        None => api_state,
    };

    // Initialize TTS service for prompt audio synthesis (optional)
    let api_state = if let Some(ref tts_config) = config.tts {
        match fcp_media::tts::TtsService::new(fcp_media::tts::TtsConfig {
            provider: fcp_media::tts::TtsProviderConfig::Local {
                engine: tts_config.engine.clone(),
                executable_path: tts_config
                    .python_path
                    .as_ref()
                    .map(std::path::PathBuf::from),
                device: None,
            },
            cache_dir: std::path::PathBuf::from(&tts_config.cache_dir),
            cache_max_size_mb: 500,
            default_language: "en-US".to_string(),
            default_voice: tts_config.voice.clone(),
        })
        .await
        {
            Ok(tts_service) => {
                info!(engine = %tts_config.engine, "TTS service initialized for prompt synthesis");
                api_state.with_tts_service(std::sync::Arc::new(tts_service))
            }
            Err(e) => {
                warn!(error = %e, "TTS service initialization failed — prompt synthesis unavailable");
                api_state
            }
        }
    } else {
        api_state
    };

    // Initialize cluster reader (optional — only when Redis is configured)
    let cluster_reader = if let Some(ref redis_url) = config.cluster.redis_url {
        match fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: redis_url.clone(),
            key_prefix: config.cluster.key_prefix.clone(),
        })
        .await
        {
            Ok(redis_storage) => {
                let reader = fcp_cluster::ClusterReader::new(redis_storage, 15);
                info!(redis_url = %redis_url, "Cluster monitoring enabled via Redis");
                Some(std::sync::Arc::new(reader))
            }
            Err(e) => {
                warn!(error = %e, "Failed to connect to Redis for cluster monitoring — disabled");
                None
            }
        }
    } else {
        info!("Cluster monitoring disabled (no [cluster] redis_url configured)");
        None
    };

    let cluster_state = cluster_api::ClusterApiState {
        reader: cluster_reader,
    };

    // Start API server with admin UI
    info!(
        bind = %config.api.bind_address,
        auth_enabled = config.api.auth_enabled,
        "Starting Admin API server (with web UI at /admin/)"
    );

    let bind_addr = config.api.bind_address;
    let api_handle = tokio::spawn(async move {
        // Cluster routes live inside the /api/v1 nest next to the core API
        // (routes merged outside the nest would be shadowed by axum's prefix
        // matching); create_router_with does the auth/CORS/pre-auth wiring.
        let cluster_routes: axum::Router<fcp_api_core::ApiState> =
            cluster_api::cluster_router(cluster_state)
                .layer(axum::middleware::from_fn(fcp_api_core::auth::require_admin));
        let mut app = fcp_api_core::create_router_with(api_state, &api_config, cluster_routes);
        app = app.merge(fcp_admin_ui::admin_ui_router());

        let listener = tokio::net::TcpListener::bind(bind_addr).await?;
        tracing::info!("API server listening on {}", bind_addr);
        axum::serve(listener, app).await?;
        Ok::<(), anyhow::Error>(())
    });

    // Event forwarding from admin is handled by ApiState::publish_event(),
    // which POSTs directly to the events server ingest endpoint. This avoids
    // the infinite loop that would occur if we forwarded all local bus events
    // (since the WebSocket subscriber also republishes to the local bus).
    let _ = event_bus_for_forward;

    // Start events server subscriber (receives cluster-wide events for the admin UI WebSocket)
    if let Some(ref events_url) = events_server_url {
        let ws_url = events_url
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        let tenant_id = default_tenant_id.to_string();
        let ws_url = build_events_ws_url(&ws_url, tenant_id, config.events.auth_token.as_deref());
        let event_bus_sub = Arc::clone(&event_bus_for_subscribe);
        tokio::spawn(async move {
            run_events_subscriber(event_bus_sub, &ws_url).await;
        });
        info!("Events server subscriber started");
    }

    // Wait for shutdown signal
    info!("FCP Admin API running. Press Ctrl+C or send SIGTERM to shutdown.");
    shutdown_signal().await;

    // Shutdown API server
    api_handle.abort();

    info!("FCP Admin API shutdown complete");
    Ok(())
}

/// Subscribe to the events server's WebSocket and republish events to the local event bus.
///
/// This allows the admin UI's WebSocket to see cluster-wide events (calls, registrations, etc.)
/// that originate from other services (e.g. fcp-call-manager).
async fn run_events_subscriber(event_bus: Arc<dyn fcp_events::EventBus>, ws_url: &str) {
    use futures_util::StreamExt;

    loop {
        info!(url = %ws_url, "Connecting to events server WebSocket");
        match tokio_tungstenite::connect_async(ws_url).await {
            Ok((ws_stream, _)) => {
                info!("Connected to events server WebSocket");
                let (_write, mut read) = ws_stream.split();

                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                            // Events server sends batch-wrapped messages:
                            //   {"type":"connected", ...}
                            //   {"type":"events", "count":N, "events":[...]}
                            // Parse the wrapper and extract individual PlatformEvents.
                            if let Ok(wrapper) = serde_json::from_str::<serde_json::Value>(&text) {
                                match wrapper.get("type").and_then(|t| t.as_str()) {
                                    Some("events") => {
                                        if let Some(events) =
                                            wrapper.get("events").and_then(|e| e.as_array())
                                        {
                                            for event_val in events {
                                                match serde_json::from_value::<
                                                    fcp_events::PlatformEvent,
                                                >(
                                                    event_val.clone()
                                                ) {
                                                    Ok(event) => {
                                                        if let Err(e) =
                                                            event_bus.publish(event).await
                                                        {
                                                            tracing::debug!(%e, "Failed to republish event to local bus");
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::debug!(%e, "Failed to parse event from batch");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Some("connected") => {
                                        tracing::debug!("Events server confirmed connection");
                                    }
                                    other => {
                                        tracing::debug!(msg_type = ?other, "Unknown message type from events server");
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
                        _ => {} // Ignore ping/pong/binary
                    }
                }
            }
            Err(e) => {
                warn!(%e, "Failed to connect to events server WebSocket");
            }
        }

        // Reconnect after delay
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

fn build_events_ws_url(
    base_url: &str,
    tenant_id: impl AsRef<str>,
    auth_token: Option<&str>,
) -> String {
    let base = format!("{}/ws/{}", base_url, tenant_id.as_ref());
    match auth_token {
        Some(token) => format!("{}?token={}", base, token),
        None => base,
    }
}

/// Extract the IP:port from a SIP contact URI like "sip:alice@192.168.1.80:5060"
// Kept as part of the module's intended API; not currently called from this binary.
#[allow(dead_code)]
fn extract_ip_from_contact(contact: &str) -> Option<String> {
    let stripped = contact
        .trim_start_matches("sip:")
        .trim_start_matches("sips:");
    // After stripping scheme, format is "user@host:port;params" or "host:port"
    if let Some(at_pos) = stripped.find('@') {
        let host_part = &stripped[at_pos + 1..];
        // Take up to first semicolon (strip params)
        let host_port = host_part.split(';').next().unwrap_or(host_part);
        // Strip angle brackets
        let host_port = host_port.trim_end_matches('>');
        Some(host_port.to_string())
    } else {
        None
    }
}

/// Subscribe to registration and call events on the local event bus and update
/// the admin's PresenceManager accordingly.  Events arrive via the events server
/// WebSocket subscriber that republishes cluster-wide events to the local bus.
async fn run_presence_listener(
    event_bus: Arc<dyn EventBus>,
    presence: fcp_presence::PresenceManager,
    tenant_id: fcp_core::TenantId,
    _storage: Arc<fcp_core::DynStorage>,
) {
    use fcp_events::filter::EventType;
    use fcp_events::{EventFilter, PlatformEvent};
    use fcp_sip::presence::extract_aor;

    // NOTE: Do NOT subscribe to EventType::Presence here. StatusChanged events
    // are published by update_presence(), so subscribing to them creates an
    // infinite feedback loop (receive StatusChanged → update_presence → publish
    // StatusChanged → receive …). Cross-node presence sync is handled by the
    // Redis Pub/Sub listener in the PresenceManager itself.
    let filter = EventFilter::new().types(vec![EventType::Call, EventType::Registration]);

    let mut stream = match event_bus.subscribe(filter.clone()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%e, "Failed to subscribe to event bus for presence");
            return;
        }
    };

    tracing::info!("Admin presence event listener running");

    loop {
        match stream.next().await {
            Some(event) => match &event {
                PlatformEvent::Registration(fcp_events::RegistrationEvent::Registered {
                    aor,
                    ..
                }) => {
                    if let Some(aor_str) = extract_aor(aor) {
                        let update = fcp_presence::PresenceUpdate {
                            aor: aor_str,
                            state: fcp_presence::PresenceState::Open,
                            note: None,
                        };
                        let _ = presence.update_presence(update, &tenant_id).await;
                    }
                    // Device registration state is enriched at query time in list_devices()
                    // using live registration data — no need to write to DB here.
                }
                PlatformEvent::Registration(fcp_events::RegistrationEvent::Unregistered {
                    aor,
                    ..
                }) => {
                    if let Some(aor_str) = extract_aor(aor) {
                        let update = fcp_presence::PresenceUpdate {
                            aor: aor_str,
                            state: fcp_presence::PresenceState::Closed,
                            note: None,
                        };
                        let _ = presence.update_presence(update, &tenant_id).await;
                    }
                    // Device registration state is enriched at query time in list_devices()
                }
                PlatformEvent::Call(fcp_events::CallEvent::Initiated {
                    from, to, call_id, ..
                }) => {
                    let call_id_str = call_id.to_string();
                    if let Some(callee_aor) = extract_aor(to) {
                        let update = fcp_presence::PresenceUpdate {
                            aor: callee_aor.clone(),
                            state: fcp_presence::PresenceState::Ringing,
                            note: Some("Ringing".to_string()),
                        };
                        let _ = presence.update_presence(update, &tenant_id).await;
                        // Track active call for callee
                        presence.add_active_call(
                            &callee_aor,
                            &tenant_id,
                            fcp_presence::ActiveCallInfo {
                                call_id: call_id_str.clone(),
                                remote_uri: from.clone(),
                                remote_display_name: None,
                                direction: fcp_presence::CallDirection::Recipient,
                                dialog_state: fcp_presence::DialogState::Early,
                            },
                        );
                    }
                    if let Some(caller_aor) = extract_aor(from) {
                        presence.set_on_the_phone(&caller_aor, &tenant_id).await;
                        // Track active call for caller
                        presence.add_active_call(
                            &caller_aor,
                            &tenant_id,
                            fcp_presence::ActiveCallInfo {
                                call_id: call_id_str.clone(),
                                remote_uri: to.clone(),
                                remote_display_name: None,
                                direction: fcp_presence::CallDirection::Initiator,
                                dialog_state: fcp_presence::DialogState::Confirmed,
                            },
                        );
                    }
                }
                // Call answered → transition Ringing → OnThePhone
                PlatformEvent::Call(fcp_events::CallEvent::Answered { call_id, .. }) => {
                    let call_id_str = call_id.to_string();
                    let affected_aors = presence.get_aors_with_active_call(&call_id_str);
                    for aor in &affected_aors {
                        presence.update_call_state(
                            aor,
                            &tenant_id,
                            &call_id_str,
                            fcp_presence::DialogState::Confirmed,
                        );
                        if let Some(info) = presence.get_presence_for_tenant(aor, &tenant_id) {
                            if info.state == fcp_presence::PresenceState::Ringing {
                                let update = fcp_presence::PresenceUpdate {
                                    aor: aor.clone(),
                                    state: fcp_presence::PresenceState::OnThePhone,
                                    note: Some("On the phone".to_string()),
                                };
                                let _ = presence.update_presence(update, &tenant_id).await;
                            }
                        }
                    }
                }
                // Call ended or failed → clean up presence
                PlatformEvent::Call(fcp_events::CallEvent::Ended { call_id, .. })
                | PlatformEvent::Call(fcp_events::CallEvent::Failed { call_id, .. }) => {
                    let call_id_str = call_id.to_string();
                    let affected_aors = presence.get_aors_with_active_call(&call_id_str);
                    presence.remove_active_call_by_id(&call_id_str);
                    for aor in &affected_aors {
                        if presence
                            .get_active_calls_for_tenant(aor, &tenant_id)
                            .is_empty()
                        {
                            let is_call_state = presence
                                .get_presence_for_tenant(aor, &tenant_id)
                                .map(|p| {
                                    matches!(
                                        p.state,
                                        fcp_presence::PresenceState::OnThePhone
                                            | fcp_presence::PresenceState::Ringing
                                    )
                                })
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
                }
                _ => {}
            },
            None => {
                tracing::warn!("Admin presence event stream ended, resubscribing...");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                match event_bus.subscribe(filter.clone()).await {
                    Ok(new_stream) => {
                        stream = new_stream;
                        tracing::info!("Admin presence event listener resubscribed");
                    }
                    Err(e) => {
                        tracing::warn!(%e, "Failed to resubscribe admin presence event listener");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

/// Resolve on Ctrl+C or SIGTERM so container orchestrators can stop the
/// service gracefully (`docker stop`, Kubernetes pod termination).
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("Received Ctrl+C"),
        _ = terminate => info!("Received SIGTERM"),
    }
}
