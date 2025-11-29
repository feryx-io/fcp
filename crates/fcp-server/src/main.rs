//! FCP Server - Main entry point for the FCP platform.
//!
//! This binary provides a command-line interface for running the FCP runtime,
//! handling configuration, and managing graceful shutdown.

use anyhow::{Context, Result};
use clap::Parser;
use fcp_runtime::Runtime;
use tokio::signal;
use tracing::info;

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

    // Set up signal handler for graceful shutdown
    let runtime_handle = tokio::spawn(async move {
        runtime.start().await?;

        // Wait for shutdown signal
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("Received shutdown signal (Ctrl+C)");
            }
            Err(err) => {
                tracing::error!("Failed to listen for shutdown signal: {}", err);
            }
        }

        // Shutdown gracefully
        runtime.shutdown().await
    });

    // Wait for runtime to complete
    runtime_handle
        .await
        .context("Runtime task failed")??;

    info!("FCP server shutdown complete");
    Ok(())
}
