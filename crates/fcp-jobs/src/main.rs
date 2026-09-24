//! FCP Jobs - Async job processing service
//!
//! This service processes background jobs asynchronously using a Redis-backed queue.
//! It supports multiple job types, retry logic, and priority queuing.

mod archive;
mod cdr;
mod cli;
mod config;
mod job;
mod queue;
mod worker;

use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use clap::Parser;
use std::sync::Arc;
use tokio::signal;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::cli::Cli;
use crate::config::{load_config, JobsConfig};
use crate::queue::JobQueue;
use crate::worker::WorkerPool;

/// Application state shared across handlers
#[derive(Clone)]
struct AppState {
    config: JobsConfig,
    queue: Arc<JobQueue>,
    worker_pool: Arc<WorkerPool>,
    cdr: Option<Arc<cdr::CdrRole>>,
    cdr_archive: Option<Arc<archive::CdrArchiveRole>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Load configuration
    let config_path = cli.config.as_deref();
    let mut config = fcp_core::config::check_stage(load_config(config_path), cli.check_config)?;
    // Secrets from the environment win over the file
    // (`docs/SHIP_AND_OPERATE.md` phase 2).
    if let Some(v) = &cli.database_url {
        config.cdr.database_url = v.clone();
        config.cdr_archive.database_url = v.clone();
    }

    // Apply CLI overrides
    if let Some(log_level) = cli.log_level {
        config.logging.level = log_level;
    }
    if let Some(log_format) = cli.log_format {
        config.logging.format = log_format;
    }
    if let Some(workers) = cli.workers {
        config.workers.count = workers;
    }
    if let Some(redis_url) = cli.redis_url {
        config.redis.url = redis_url;
    }

    // Initialize logging
    if cli.check_config {
        println!(
            "{}",
            fcp_core::config::masked_toml(&config).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }
    init_logging(&config.logging.level, &config.logging.format)?;

    info!(
        service = "fcp-jobs",
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "Starting FCP Jobs service"
    );

    // Connect to Redis and create job queue
    let queue = JobQueue::new(&config.redis.url, config.redis.queue_prefix.clone())
        .await
        .context("Failed to create job queue")?;

    // Create worker pool
    let worker_pool = WorkerPool::new(queue.clone(), config.workers.clone(), config.retry.clone());

    // Start workers
    let worker_handles = worker_pool.start().await;

    // The CDR processor role (docs/CDR_PIPELINE.md)
    let cdr_role = if config.cdr.enabled {
        Some(Arc::new(
            cdr::CdrRole::start(&config)
                .await
                .context("Failed to start the CDR processor")?,
        ))
    } else {
        None
    };

    // The CDR archive role (docs/CDR_PIPELINE.md §13, phase 5)
    let archive_role = if config.cdr_archive.enabled {
        Some(Arc::new(
            archive::CdrArchiveRole::start(&config)
                .await
                .context("Failed to start the CDR archive")?,
        ))
    } else {
        None
    };

    // Create application state
    let state = AppState {
        config: config.clone(),
        queue: Arc::new(queue),
        worker_pool: Arc::new(worker_pool),
        cdr: cdr_role.clone(),
        cdr_archive: archive_role.clone(),
    };

    // Start health check server if enabled
    let server_handle = if config.health.enabled {
        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/status", get(status_handler))
            .route("/metrics", get(metrics_handler))
            .layer(TraceLayer::new_for_http())
            .with_state(state.clone());

        let bind_addr = config.health.bind_address;
        info!(bind_address = %bind_addr, "Starting health check server");

        let listener = tokio::net::TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("Failed to bind to {}", bind_addr))?;

        Some(tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal())
                .await
            {
                error!(error = %e, "Health check server error");
            }
        }))
    } else {
        None
    };

    info!("FCP Jobs service started successfully");

    // Wait for shutdown signal
    shutdown_signal().await;

    info!("Shutdown signal received, stopping workers");

    // Shutdown worker pool
    state
        .worker_pool
        .shutdown(config.workers.shutdown_timeout)
        .await;

    // Wait for workers to finish
    for handle in worker_handles {
        let _ = handle.await;
    }

    // Finish the CDR batch in hand; what is unacknowledged stays in the
    // stream for the next consumer.
    if let Some(role) = &cdr_role {
        role.stop().await;
    }
    if let Some(role) = &archive_role {
        role.stop();
    }

    // Shutdown health check server
    if let Some(handle) = server_handle {
        handle.abort();
    }

    info!("FCP Jobs service shutdown complete");
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

/// Status endpoint with queue and worker statistics
async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let stats = match state.queue.get_stats().await {
        Ok(stats) => stats,
        Err(e) => {
            error!(error = %e, "Failed to get queue stats");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to get queue stats"
                })),
            )
                .into_response();
        }
    };

    let status = serde_json::json!({
        "status": "operational",
        "service": "fcp-jobs",
        "version": env!("CARGO_PKG_VERSION"),
        "node_id": state.config.service.node_id,
        "workers": {
            "total": state.config.workers.count,
            "active": state.worker_pool.active_workers(),
        },
        "queue": {
            "pending": {
                "critical": stats.critical_pending,
                "high": stats.high_pending,
                "normal": stats.normal_pending,
                "low": stats.low_pending,
                "total": stats.total_pending(),
            },
            "completed": stats.completed,
            "failed": stats.failed,
        },
        "cdr": match &state.cdr {
            Some(role) => role.status().await,
            None => serde_json::Value::Null,
        },
        "cdr_archive": match &state.cdr_archive {
            Some(role) => role.status().await,
            None => serde_json::Value::Null,
        },
        "config": {
            "max_retries": state.config.retry.max_retries,
            "job_timeout_secs": state.config.workers.job_timeout.as_secs(),
            "poll_interval_secs": state.config.workers.poll_interval.as_secs(),
        }
    });

    (StatusCode::OK, Json(status)).into_response()
}

/// Prometheus text: the CDR processor's and the archive's counters, for
/// the roles that run here.
async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let mut body = match &state.cdr {
        Some(role) => role.metrics(&state.config.service.node_id).await,
        None => String::new(),
    };
    if let Some(role) = &state.cdr_archive {
        body.push_str(&role.metrics(&state.config.service.node_id).await);
    }
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
}

/// Graceful shutdown signal handler
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
        _ = ctrl_c => {
            info!("Received Ctrl+C signal");
        }
        _ = terminate => {
            info!("Received SIGTERM signal");
        }
    }

    info!("Initiating graceful shutdown");
}
