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
use fcp_events::{EventBus, EventFilter, InMemoryEventBus};
use fcp_sip::exports::{
    DialogManager, SubscriptionManager, TransactionManager, TransportContext,
    TransportDispatcher,
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
    let config_store = storage_result.config_store;

    info!("Storage backend initialized successfully");

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
    let handler = Arc::new(SipMessageHandler::new(
        transaction_manager,
        dialog_manager.clone(),
        local_uri,
        None,                    // media_profile
        "127.0.0.1".to_string(), // local_addr
        10000,                   // rtp_port
        None,                    // registrar
        None,                    // routing_service
        call_manager.clone(),
        None,                    // uac (no outbound SIP needed)
        Some(storage.clone()),
        None,                    // media_manager
        None,                    // media_config
        None,                    // flow_handler
        B2buaConfig::default(),
        None,                    // outbound_route_provider
        None,                    // pbx_handler
        None,                    // conference_handler
        None,                    // queue_handler
        None,                    // presence_handler
        None,                    // cluster_parking
        String::new(),           // node_id
    )?);

    let api_config = fcp_core::ApiConfig {
        enabled: true,
        bind: config.api.bind_address,
        auth_enabled: config.api.auth_enabled,
        auth_token: config.api.auth_token.clone(),
        metrics_enabled: false,
        metrics_path: "/metrics".to_string(),
    };
    let realm = "fcp.local".to_string();

    info!(node_id = %config.service.node_id, "SIP components initialized (lightweight, no transport)");

    // Clone event_bus for event forwarding and subscribing before moving into api_state
    let event_bus_for_forward = Arc::clone(&event_bus);
    let event_bus_for_subscribe: Arc<dyn EventBus> = Arc::clone(&event_bus);

    // Derive events server base URL from ingest URL (strip /ingest suffix)
    let events_server_url = config.events.server_url.as_ref().map(|url| {
        url.trim_end_matches("/ingest")
            .trim_end_matches('/')
            .to_string()
    });

    // Create API state
    let api_state = fcp_api_core::ApiState::new(
        Some(storage),
        None, // flow_store
        dialog_manager,
        subscription_manager,
        call_manager,
        handler,
        None, // media_manager (admin doesn't need media)
        event_bus,
        None, // queue_manager (admin doesn't use queues)
        None, // presence_manager (admin doesn't run presence)
        realm,
        fcp_core::TenantId::nil(),
    )
    .with_events_server_url(events_server_url.clone())
    .with_config_store(config_store);

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
        api_state.with_voicemail_greetings_dir(std::path::PathBuf::from("/tmp/fcp/voicemail/greetings"))
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
        api_state.with_voicemail_recordings_dir(std::path::PathBuf::from("/tmp/fcp/voicemail/recordings"))
    };

    // Set audio prompts directory (default: /tmp/fcp/audio/prompts)
    let api_state = if let Some(ref dir) = config.prompts_dir {
        api_state.with_prompts_dir(std::path::PathBuf::from(dir))
    } else {
        api_state.with_prompts_dir(std::path::PathBuf::from("/tmp/fcp/audio/prompts"))
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
        // Build the API router with cluster routes merged into the v1 namespace.
        // We inline fcp_api_core::create_router logic here so we can inject the
        // cluster routes *inside* the /api/v1 nest (routes merged outside the
        // nest would be shadowed by axum's prefix matching).
        let cluster_routes: axum::Router<fcp_api_core::ApiState> =
            cluster_api::cluster_router(cluster_state);
        let v1 = fcp_api_core::v1::router().merge(cluster_routes);

        let cors = tower_http::cors::CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
            ]);

        let mut router = axum::Router::new()
            .route(
                "/metrics",
                axum::routing::get(fcp_api_core::v1::metrics::prometheus_metrics),
            )
            .nest("/api/v1", v1);

        if api_config.auth_enabled {
            if let Some(auth_token) = &api_config.auth_token {
                tracing::info!("API authentication enabled");
                let mut auth_config = fcp_api_core::auth::AuthConfig::new(auth_token.clone());
                if let Some(store) = api_state.storage.as_ref().and_then(|s| s.api_token_store().cloned()) {
                    auth_config = auth_config.with_api_token_store(store);
                    tracing::info!("API token authentication enabled");
                }
                router = router.layer(axum::middleware::from_fn(move |req, next| {
                    let config = auth_config.clone();
                    fcp_api_core::auth::validate_bearer_token(config, req, next)
                }));
            }
        }

        let mut app = router
            .layer(cors)
            .layer(tower_http::trace::TraceLayer::new_for_http())
            .with_state(api_state);

        app = app.merge(fcp_admin_ui::admin_ui_router());

        let listener = tokio::net::TcpListener::bind(bind_addr).await?;
        tracing::info!("API server listening on {}", bind_addr);
        axum::serve(listener, app).await?;
        Ok::<(), anyhow::Error>(())
    });

    // Note: event forwarding is intentionally disabled for the admin service.
    // The admin subscribes to the events server WebSocket to receive cluster-wide
    // events for the UI, but does not generate events of its own. Forwarding
    // received events back to the events server would create an infinite loop.
    let _ = event_bus_for_forward;

    // Start events server subscriber (receives cluster-wide events for the admin UI WebSocket)
    if let Some(ref events_url) = events_server_url {
        let ws_url = events_url
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        let tenant_id = "tenant-00000000-0000-0000-0000-000000000000";
        let ws_url = format!("{}/ws/{}", ws_url, tenant_id);
        let event_bus_sub = Arc::clone(&event_bus_for_subscribe);
        tokio::spawn(async move {
            run_events_subscriber(event_bus_sub, &ws_url).await;
        });
        info!("Events server subscriber started");
    }

    // Wait for shutdown signal
    info!("FCP Admin API running. Press Ctrl+C to shutdown.");
    signal::ctrl_c()
        .await
        .context("Failed to listen for shutdown signal")?;
    info!("Received shutdown signal (Ctrl+C)");

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
                                        if let Some(events) = wrapper.get("events").and_then(|e| e.as_array()) {
                                            for event_val in events {
                                                match serde_json::from_value::<fcp_events::PlatformEvent>(event_val.clone()) {
                                                    Ok(event) => {
                                                        if let Err(e) = event_bus.publish(event).await {
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
