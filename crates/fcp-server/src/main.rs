//! FCP Server - Main entry point for the FCP platform.
//!
//! This binary provides a command-line interface for running the FCP runtime,
//! handling configuration, and managing graceful shutdown.

use anyhow::{Context, Result};
use clap::Parser;
use fcp_runtime::Runtime;
use std::sync::Arc;
use tokio::signal;
use tracing::info;

mod api;
mod cli;
mod config;
mod storage;

use cli::Cli;
use config::load_config;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Load configuration
    let config = load_config(cli.config.as_deref())?;

    // Initialize tracing/logging
    fcp_runtime::init_tracing(&config).context("Failed to initialize tracing")?;

    info!(version = env!("CARGO_PKG_VERSION"), "Starting FCP server");

    // Initialize storage backend if registrar is enabled
    let storage = if config.registrar.enabled {
        Some(
            storage::initialize_storage(&config.storage)
                .await
                .context("Failed to initialize storage backend")?,
        )
    } else {
        None
    };

    // Build runtime with storage
    let runtime = Runtime::builder()
        .with_config(config);

    // Add storage if available
    let runtime = if let Some(storage) = storage {
        runtime.with_storage(storage)
    } else {
        runtime
    };

    let mut runtime = runtime
        .build()
        .await
        .context("Failed to build runtime")?;

    info!(node_id = %runtime.node_id(), "Runtime built successfully");

    // Get configuration reference and runtime components for API
    let api_config = runtime.config().api.clone();
    let realm = runtime.config().registrar.realm.clone();
    let storage = runtime.storage().cloned();
    let flow_store = runtime.flow_store().cloned();
    let dialog_manager = Arc::clone(runtime.dialog_manager());
    let subscription_manager = Arc::clone(runtime.subscription_manager());
    let call_manager = Arc::clone(runtime.call_manager());
    let handler = Arc::clone(runtime.handler());
    let media_manager = Some(Arc::clone(runtime.media_manager()));
    let event_bus = Arc::clone(runtime.event_bus());

    // Start the runtime
    runtime.start().await?;

    // Start API server if enabled
    let api_handle = if api_config.enabled {
        info!(
            bind = %api_config.bind,
            auth_enabled = api_config.auth_enabled,
            "Starting API server"
        );

        let api_state = api::state::ApiState::new(
            storage,
            flow_store,
            dialog_manager,
            subscription_manager,
            call_manager,
            handler,
            media_manager,
            event_bus,
            realm,
        );
        let bind_addr = api_config.bind;
        let api_config_clone = api_config.clone();

        Some(tokio::spawn(async move {
            api::serve(bind_addr, api_state, &api_config_clone).await
        }))
    } else {
        None
    };

    // Wait for shutdown signal
    info!("FCP server running. Press Ctrl+C to shutdown.");
    signal::ctrl_c()
        .await
        .context("Failed to listen for shutdown signal")?;
    info!("Received shutdown signal (Ctrl+C)");

    // Shutdown runtime
    runtime.shutdown().await?;

    // Abort API server if running
    if let Some(handle) = api_handle {
        handle.abort();
    }

    info!("FCP server shutdown complete");
    Ok(())
}
