//! FCP Server - Main entry point for the FCP platform.
//!
//! This binary provides a command-line interface for running the FCP runtime,
//! handling configuration, and managing graceful shutdown.

use anyhow::{Context, Result};
use clap::Parser;
use fcp_runtime::Runtime;
use tokio::signal;
use tracing::info;

mod api;
mod cli;
mod config;

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

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Starting FCP server"
    );

    // Build runtime
    let mut runtime = Runtime::builder()
        .with_config(config)
        .build()
        .await
        .context("Failed to build runtime")?;

    info!(node_id = %runtime.node_id(), "Runtime built successfully");

    // Get configuration reference
    let api_config = runtime.config().api.clone();
    let storage = runtime.storage().cloned();

    // Start the runtime
    runtime.start().await?;

    // Start API server if enabled and storage is available
    let api_handle = if api_config.enabled {
        if let Some(storage) = storage {
            info!(
                bind = %api_config.bind,
                "Starting API server"
            );

            let api_state = api::state::ApiState::new(storage);
            let bind_addr = api_config.bind;

            Some(tokio::spawn(async move {
                api::serve(bind_addr, api_state).await
            }))
        } else {
            tracing::warn!("API enabled but registrar/storage not available, skipping API server");
            None
        }
    } else {
        None
    };

    // Wait for shutdown signal
    info!("FCP server running. Press Ctrl+C to shutdown.");
    signal::ctrl_c().await.context("Failed to listen for shutdown signal")?;
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
