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
    let storage = storage::initialize_storage(&config.storage)
        .await
        .context("Failed to initialize storage backend")?;

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

    // Clone event_bus for event forwarding before moving into api_state
    let event_bus_for_forward = Arc::clone(&event_bus);

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
    );

    // Start API server with admin UI
    info!(
        bind = %config.api.bind_address,
        auth_enabled = config.api.auth_enabled,
        "Starting Admin API server (with web UI at /admin/)"
    );

    let bind_addr = config.api.bind_address;
    let api_handle = tokio::spawn(async move {
        // Build the API router and merge in the admin UI
        let mut app = fcp_api_core::create_router(api_state, &api_config);
        app = app.merge(fcp_admin_ui::admin_ui_router());

        let listener = tokio::net::TcpListener::bind(bind_addr).await?;
        tracing::info!("API server listening on {}", bind_addr);
        axum::serve(listener, app).await?;
        Ok::<(), anyhow::Error>(())
    });

    // Start event forwarding task (if configured)
    if let Some(ref events_url) = config.events.server_url {
        let events_url = events_url.clone();
        tokio::spawn(async move {
            if let Err(e) = run_event_forwarder(event_bus_for_forward, &events_url).await {
                tracing::error!(%e, "Event forwarder task failed");
            }
        });
        info!(events_url = %config.events.server_url.as_deref().unwrap_or(""), "Event forwarding enabled");
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

/// Run the event forwarder task that subscribes to local events and POSTs them to fcp-events-server.
async fn run_event_forwarder(
    event_bus: Arc<dyn fcp_events::EventBus>,
    events_url: &str,
) -> Result<()> {
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
