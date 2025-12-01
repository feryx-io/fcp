//! FCP Runtime - The execution environment for the FCP platform.
//!
//! This crate provides the `Runtime` type which coordinates all FCP services,
//! manages their lifecycle, and handles incoming SIP messages.

use anyhow::{Context, Result};
use fcp_core::{FcpConfig, NodeId, RegistrationStore, ServiceManager};
use fcp_sip::prelude::*;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

pub mod builder;
pub mod handlers;

pub use builder::RuntimeBuilder;

/// The FCP runtime manages all services and coordinates message handling.
pub struct Runtime {
    /// Runtime configuration
    config: Arc<FcpConfig>,

    /// Node identifier
    node_id: NodeId,

    /// Service manager coordinating all services
    services: ServiceManager,

    /// Transaction manager for SIP transactions
    transaction_manager: Arc<TransactionManager>,

    /// Dialog manager for call state
    dialog_manager: Arc<DialogManager>,

    /// Subscription manager for event notifications
    subscription_manager: Arc<SubscriptionManager>,

    /// User Agent Client for outbound calls
    uac: Arc<SipUserAgentClient>,

    /// Transport listener for receiving inbound SIP messages
    listener: Option<TransportListener>,

    /// Shutdown signal sender
    shutdown_tx: Option<mpsc::Sender<()>>,

    /// Storage for registrations and users (if registrar enabled)
    storage: Option<Arc<fcp_core::FileBackedStorage>>,

    /// Background cleanup task handle
    cleanup_task: Option<tokio::task::JoinHandle<()>>,
}

impl Runtime {
    /// Create a new runtime builder.
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::new()
    }

    /// Get the node ID.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Get the configuration.
    pub fn config(&self) -> &FcpConfig {
        &self.config
    }

    /// Get the transaction manager.
    pub fn transaction_manager(&self) -> &Arc<TransactionManager> {
        &self.transaction_manager
    }

    /// Get the dialog manager.
    pub fn dialog_manager(&self) -> &Arc<DialogManager> {
        &self.dialog_manager
    }

    /// Get the subscription manager.
    pub fn subscription_manager(&self) -> &Arc<SubscriptionManager> {
        &self.subscription_manager
    }

    /// Get the User Agent Client for making outbound calls.
    pub fn uac(&self) -> &Arc<SipUserAgentClient> {
        &self.uac
    }

    /// Get the storage (if registrar is enabled).
    pub fn storage(&self) -> Option<&Arc<fcp_core::FileBackedStorage>> {
        self.storage.as_ref()
    }

    /// Start the runtime and all services.
    pub async fn start(&mut self) -> Result<()> {
        info!(node_id = %self.node_id, "Starting FCP runtime");

        // Start transport listeners
        if let Some(listener) = &mut self.listener {
            listener
                .start()
                .await
                .context("Failed to start transport listeners")?;
        }

        // Start all services
        self.services
            .start_all()
            .await
            .context("Failed to start services")?;

        // Start registration cleanup task if storage is available
        if let Some(storage) = self.storage.clone() {
            let cleanup_interval = std::time::Duration::from_secs(60); // Run every 60 seconds
            info!("Starting registration cleanup task (interval: {:?})", cleanup_interval);

            let task = tokio::spawn(async move {
                let mut interval = tokio::time::interval(cleanup_interval);
                loop {
                    interval.tick().await;

                    match storage.cleanup_expired().await {
                        Ok(count) => {
                            if count > 0 {
                                info!("Cleaned up {} expired registrations", count);
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to cleanup expired registrations: {}", e);
                        }
                    }
                }
            });

            self.cleanup_task = Some(task);
        }

        info!("FCP runtime started successfully");
        Ok(())
    }

    /// Run the runtime until shutdown signal is received.
    pub async fn run(mut self) -> Result<()> {
        // Start the runtime
        self.start().await?;

        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Wait for shutdown signal
        info!("Runtime is running. Waiting for shutdown signal...");
        shutdown_rx.recv().await;

        // Shutdown
        self.shutdown().await?;

        Ok(())
    }

    /// Shutdown the runtime gracefully.
    pub async fn shutdown(mut self) -> Result<()> {
        info!(node_id = %self.node_id, "Shutting down FCP runtime");

        // Stop cleanup task if running
        if let Some(task) = self.cleanup_task.take() {
            info!("Stopping registration cleanup task");
            task.abort();
        }

        // Stop transport listeners first
        if let Some(mut listener) = self.listener.take() {
            listener
                .stop()
                .await
                .context("Failed to stop transport listeners")?;
        }

        // Stop all services
        self.services
            .stop_all()
            .await
            .context("Failed to stop services")?;

        info!("FCP runtime shutdown complete");
        Ok(())
    }

    /// Trigger shutdown from another task.
    pub fn trigger_shutdown(&self) -> Result<()> {
        if let Some(tx) = &self.shutdown_tx {
            tx.try_send(())
                .context("Failed to send shutdown signal")?;
        }
        Ok(())
    }

    /// Check health of all services.
    pub async fn health_check(&self) -> Vec<(String, bool)> {
        self.services.health_check_all().await
    }
}

/// Initialize tracing subscriber for logging.
pub fn init_tracing(config: &FcpConfig) -> Result<()> {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&config.observability.log_level))
        .context("Failed to create env filter")?;

    let registry = tracing_subscriber::registry().with(env_filter);

    match config.observability.log_format {
        fcp_core::LogFormat::Json => {
            let fmt_layer = fmt::layer().json();
            registry.with(fmt_layer).init();
        }
        fcp_core::LogFormat::Pretty => {
            let fmt_layer = fmt::layer().pretty();
            registry.with(fmt_layer).init();
        }
        fcp_core::LogFormat::Compact => {
            let fmt_layer = fmt::layer().compact();
            registry.with(fmt_layer).init();
        }
    }

    info!("Tracing initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_runtime_builder() {
        let config = FcpConfig::default();
        let runtime = Runtime::builder()
            .with_config(config)
            .build()
            .await
            .unwrap();

        assert!(!runtime.node_id().is_nil());
    }

    #[tokio::test]
    async fn test_runtime_lifecycle() {
        let mut config = FcpConfig::default();
        // Use port 0 to let OS assign random available ports
        config.transport.udp_bind = Some("0.0.0.0:0".parse().unwrap());
        config.transport.tcp_bind = Some("0.0.0.0:0".parse().unwrap());

        let mut runtime = Runtime::builder()
            .with_config(config)
            .build()
            .await
            .unwrap();

        // Start
        runtime.start().await.expect("Failed to start runtime");

        // Health check
        let health = runtime.health_check().await;
        for (service, healthy) in &health {
            assert!(*healthy, "Service {} is not healthy", service);
        }

        // Shutdown
        runtime.shutdown().await.expect("Failed to shutdown");
    }
}
