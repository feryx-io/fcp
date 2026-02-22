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
mod routes;
mod state;

use anyhow::{Context, Result};
use axum::{
    extract::{ws::WebSocketUpgrade, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use fcp_core::TenantId;
use fcp_events::{CallEvent, EventPublisher, EventSubscriber, InMemoryEventBus, PlatformEvent, SystemEvent};
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
    manager: Arc<ConnectionManager>,
    event_bus: Arc<InMemoryEventBus>,
    call_store: Arc<CallStateStore>,
    registration_store: Arc<RegistrationStateStore>,
    node_registry: Arc<NodeRegistry>,
    #[allow(dead_code)]
    webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
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

    // Create event bus
    let event_bus = Arc::new(InMemoryEventBus::new());

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

    // Create webhook dispatcher if enabled
    let webhook_dispatcher = if config.webhook.enabled {
        info!("Webhook dispatching enabled, initializing...");

        // Initialize PostgreSQL storage for webhooks
        let pg_config =
            fcp_storage_postgres::PostgresConfig::new(&config.webhook.database_url);
        let pg_storage = fcp_storage_postgres::PostgresStorage::new(pg_config)
            .await
            .context("Failed to initialize PostgreSQL storage for webhooks")?;
        let webhook_store: Arc<dyn fcp_core::storage::WebhookStore> =
            Arc::new(pg_storage);

        let dispatcher_config = WebhookDispatcherConfig {
            request_timeout: Duration::from_secs(config.webhook.request_timeout_secs),
            max_retries: config.webhook.max_retries,
            base_retry_delay: Duration::from_secs(config.webhook.base_retry_delay_secs),
            max_retry_delay: Duration::from_secs(config.webhook.max_retry_delay_secs),
            max_concurrent: config.webhook.max_concurrent,
            retry_interval: Duration::from_secs(config.webhook.retry_interval_secs),
        };

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
    } else {
        info!("Webhook dispatching disabled");
        None
    };

    // Create application state
    let state = AppState {
        config: config.clone(),
        manager: manager.clone(),
        event_bus: event_bus.clone(),
        call_store: call_store.clone(),
        registration_store: registration_store.clone(),
        node_registry: node_registry.clone(),
        webhook_dispatcher: webhook_dispatcher.clone(),
    };

    // Build router with new endpoints
    let app = Router::new()
        // Health and status
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        // WebSocket
        .route("/ws/:tenant_id", get(websocket_handler))
        // Event ingestion
        .route("/ingest", post(ingest_handler))
        .route("/ingest/batch", post(ingest_batch_handler))
        // Call aggregation API
        .route("/calls", get(list_calls_handler))
        .route("/calls/:call_id", get(get_call_handler))
        // Registration aggregation API
        .route("/registrations", get(list_registrations_handler))
        // Node registry API
        .route("/nodes", get(list_nodes_handler))
        // Aggregated statistics
        .route("/stats", get(stats_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Merge webhook routes if dispatcher is available
    let app = if let Some(ref dispatcher) = webhook_dispatcher {
        let webhook_state = routes::webhooks::WebhookRouteState {
            dispatcher: dispatcher.clone(),
        };
        app.merge(routes::webhooks::webhook_router(webhook_state))
    } else {
        app
    };

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
                        redis_storage,
                        stream_key,
                        config.event_bus.consumer_group.clone(),
                        config.event_bus.consumer_name.clone(),
                    );

                    // Ensure consumer group exists
                    if let Err(e) = consumer.ensure_group().await {
                        error!(error = %e, "Failed to create Redis consumer group");
                    } else {
                        let event_bus_redis = event_bus.clone();
                        let call_store_redis = call_store.clone();
                        let registration_store_redis = registration_store.clone();
                        let node_registry_redis = node_registry.clone();

                        tokio::spawn(async move {
                            redis_stream_consumer_task(
                                consumer,
                                event_bus_redis,
                                call_store_redis,
                                registration_store_redis,
                                node_registry_redis,
                            )
                            .await;
                        });
                        info!("Redis stream consumer started (replacing HTTP /ingest)");
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

    // Spawn stale cleanup task
    let call_store_cleanup = call_store.clone();
    let registration_store_cleanup = registration_store.clone();
    let node_registry_cleanup = node_registry.clone();
    tokio::spawn(async move {
        stale_cleanup_task(call_store_cleanup, registration_store_cleanup, node_registry_cleanup).await;
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

/// Ingest a single event from another service (e.g., fcp-call-manager)
async fn ingest_handler(
    State(state): State<AppState>,
    Json(event): Json<PlatformEvent>,
) -> impl IntoResponse {
    // Process event for state tracking
    // Note: source_node_id could be extracted from X-Node-ID header if needed
    process_event_for_state(
        &event,
        &state.call_store,
        &state.registration_store,
        &state.node_registry,
        None,
    );

    // Publish to event bus for WebSocket broadcast
    let _ = state.event_bus.publish(event).await;
    StatusCode::ACCEPTED
}

/// Ingest a batch of events from another service
async fn ingest_batch_handler(
    State(state): State<AppState>,
    Json(events): Json<Vec<PlatformEvent>>,
) -> impl IntoResponse {
    for event in events {
        // Process event for state tracking
        process_event_for_state(
            &event,
            &state.call_store,
            &state.registration_store,
            &state.node_registry,
            None,
        );

        // Publish to event bus for WebSocket broadcast
        let _ = state.event_bus.publish(event).await;
    }
    StatusCode::ACCEPTED
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
                CallEvent::Initiated { node_id, .. } => {
                    if let Some(ref nid) = node_id {
                        node_registry.increment_calls(nid);
                    }
                }
                CallEvent::Ended { node_id, .. } | CallEvent::Failed { node_id, .. } => {
                    if let Some(ref nid) = node_id {
                        node_registry.decrement_calls(nid);
                    }
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
                SystemEvent::HealthCheck { node_id, healthy, .. } => {
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
            }
        }
        _ => {
            // Other events don't affect state tracking
        }
    }
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
        CallEvent::Parked { .. } | CallEvent::Unparked { .. } => None,
    }
}

/// Background task to broadcast events from event bus to connections
async fn event_broadcast_task(
    event_bus: Arc<InMemoryEventBus>,
    manager: Arc<ConnectionManager>,
) {
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
                // Broadcast to WebSocket connections
                // (State tracking is done in ingest_handler to avoid double-processing)
                manager.publish(event);
            }
            None => {
                error!("Event stream ended unexpectedly");
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
    }
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
#[cfg(feature = "redis-events")]
async fn redis_stream_consumer_task(
    consumer: fcp_storage_redis::event_bus::RedisStreamConsumer,
    event_bus: Arc<InMemoryEventBus>,
    call_store: Arc<CallStateStore>,
    registration_store: Arc<RegistrationStateStore>,
    node_registry: Arc<NodeRegistry>,
) {
    info!("Redis stream consumer task running");

    loop {
        match consumer.read_events(100, 2000).await {
            Ok(events) => {
                if events.is_empty() {
                    continue;
                }

                let mut ack_ids = Vec::with_capacity(events.len());

                for (stream_id, event) in events {
                    // Process event for state tracking (same as ingest_handler)
                    process_event_for_state(
                        &event,
                        &call_store,
                        &registration_store,
                        &node_registry,
                        None,
                    );

                    // Publish to local event bus for WebSocket broadcast
                    let _ = event_bus.publish(event).await;

                    ack_ids.push(stream_id);
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
