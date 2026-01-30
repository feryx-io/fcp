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
use fcp_events::{EventFilter, EventSubscriber};
use std::sync::Arc;
use tokio::signal;
use tracing::{info, warn};

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

    // Create minimal FCP config for runtime
    // Admin API doesn't start SIP transport, but uses runtime to create proper components
    let mut fcp_config = fcp_core::FcpConfig::default();
    fcp_config.node.name = config.service.node_id.clone();
    fcp_config.registrar.enabled = false; // Admin doesn't need registrar
    fcp_config.routing.enabled = false; // Admin doesn't need routing
    fcp_config.api = fcp_core::ApiConfig {
        enabled: true,
        bind: config.api.bind_address,
        auth_enabled: config.api.auth_enabled,
        auth_token: config.api.auth_token.clone(),
        metrics_enabled: false,
        metrics_path: "/metrics".to_string(),
    };

    // Build runtime with storage (but don't start SIP transport)
    let runtime = fcp_runtime::Runtime::builder()
        .with_config(fcp_config)
        .with_storage(storage.clone())
        .build()
        .await
        .context("Failed to build runtime")?;

    info!(node_id = %runtime.node_id(), "Runtime components initialized");

    // Extract components from runtime for API
    let api_config = runtime.config().api.clone();
    let realm = runtime.config().registrar.realm.clone();
    let dialog_manager = Arc::clone(runtime.dialog_manager());
    let subscription_manager = Arc::clone(runtime.subscription_manager());
    let call_manager = Arc::clone(runtime.call_manager());
    let handler = Arc::clone(runtime.handler());
    let media_manager = Some(Arc::clone(runtime.media_manager()));
    let event_bus = Arc::clone(runtime.event_bus());

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
        media_manager,
        event_bus,
        realm,
        fcp_core::TenantId::nil(),
    );

    // Start API server
    info!(
        bind = %config.api.bind_address,
        auth_enabled = config.api.auth_enabled,
        "Starting Admin API server"
    );

    let bind_addr = config.api.bind_address;
    let api_handle =
        tokio::spawn(async move { fcp_api_core::serve(bind_addr, api_state, &api_config).await });

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
