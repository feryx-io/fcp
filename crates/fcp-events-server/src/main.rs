//! FCP Events Server - WebSocket event streaming service
//!
//! This service provides real-time event streaming to clients via WebSocket connections.
//! It subscribes to the FCP event bus and broadcasts events to connected clients based on
//! their subscriptions and tenant isolation.

mod cli;
mod config;
mod connection;

use anyhow::{Context, Result};
use axum::{
    extract::{
        ws::WebSocketUpgrade,
        Path, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use clap::Parser;
use fcp_core::TenantId;
use fcp_events::InMemoryEventBus;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::cli::Cli;
use crate::config::{load_config, EventsConfig};
use crate::connection::{cleanup_task, handle_websocket, ConnectionManager};

/// Application state shared across handlers
#[derive(Clone)]
struct AppState {
    config: EventsConfig,
    manager: Arc<ConnectionManager>,
    event_bus: Arc<InMemoryEventBus>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Load configuration
    let config_path = cli.config.as_deref();
    let mut config = load_config(config_path)?;

    // Apply CLI overrides
    if let Some(log_level) = cli.log_level {
        config.logging.level = log_level;
    }
    if let Some(log_format) = cli.log_format {
        config.logging.format = log_format;
    }
    if let Some(bind_addr) = cli.bind_addr {
        config.websocket.bind_address = bind_addr
            .parse()
            .context("Invalid bind address format")?;
    }

    // Initialize logging
    init_logging(&config.logging.level, &config.logging.format)?;

    info!(
        service = "fcp-events-server",
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "Starting FCP Events Server"
    );

    // Create event bus
    let event_bus = Arc::new(InMemoryEventBus::new());

    // Create connection manager
    let manager = Arc::new(ConnectionManager::new(
        config.websocket.heartbeat_interval,
        config.websocket.client_timeout,
        config.limits.max_connections,
        config.limits.max_connections_per_tenant,
    ));

    // Create application state
    let state = AppState {
        config: config.clone(),
        manager: manager.clone(),
        event_bus: event_bus.clone(),
    };

    // Build router
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/ws/:tenant_id", get(websocket_handler))
        .route("/status", get(status_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Start background tasks
    let manager_clone = manager.clone();
    tokio::spawn(async move {
        cleanup_task(manager_clone).await;
    });

    let event_bus_clone = event_bus.clone();
    let manager_clone = manager.clone();
    tokio::spawn(async move {
        event_broadcast_task(event_bus_clone, manager_clone).await;
    });

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

/// Health check endpoint
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Status endpoint with connection statistics
async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let status = serde_json::json!({
        "status": "operational",
        "service": "fcp-events-server",
        "version": env!("CARGO_PKG_VERSION"),
        "node_id": state.config.service.node_id,
        "connections": {
            "total": state.manager.connection_count(),
            "max": state.config.limits.max_connections,
        },
        "config": {
            "heartbeat_interval_secs": state.config.websocket.heartbeat_interval.as_secs(),
            "client_timeout_secs": state.config.websocket.client_timeout.as_secs(),
            "batch_size": state.config.websocket.batch_size,
        }
    });

    Json(status)
}

/// WebSocket upgrade handler
async fn websocket_handler(
    ws: WebSocketUpgrade,
    Path(tenant_id_str): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Parse tenant ID
    let tenant_id = match tenant_id_str.parse::<TenantId>() {
        Ok(id) => id,
        Err(e) => {
            error!(tenant_id = %tenant_id_str, error = %e, "Invalid tenant ID");
            return (StatusCode::BAD_REQUEST, "Invalid tenant ID").into_response();
        }
    };

    info!(tenant_id = %tenant_id, "WebSocket upgrade request");

    // Create event channel for this connection
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Register connection
    let conn_id = match state.manager.register(tenant_id.clone(), event_tx) {
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

/// Background task to broadcast events from event bus to connections
async fn event_broadcast_task(event_bus: Arc<InMemoryEventBus>, manager: Arc<ConnectionManager>) {
    use fcp_events::{EventFilter, EventSubscriber};

    // Subscribe to all events
    let mut stream = match event_bus.subscribe(EventFilter::new()).await {
        Ok(stream) => stream,
        Err(e) => {
            error!(error = %e, "Failed to subscribe to event bus");
            return;
        }
    };

    info!("Event broadcast task started");

    loop {
        match stream.next().await {
            Some(event) => {
                manager.publish(event);
            }
            None => {
                error!("Event stream ended unexpectedly");
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
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
