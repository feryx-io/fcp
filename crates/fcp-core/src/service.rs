use crate::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// Marker trait for long-running services inside the FCP runtime.
#[async_trait]
pub trait Service: Send + Sync {
    /// Human-friendly name for diagnostics.
    fn name(&self) -> &'static str;

    /// Initialize the service.
    async fn start(&self) -> Result<()> {
        Ok(())
    }

    /// Gracefully shutdown the service.
    async fn stop(&self) -> Result<()> {
        Ok(())
    }

    /// Health check - returns true if service is healthy.
    async fn health_check(&self) -> bool {
        true
    }
}

/// Service manager for coordinating multiple services.
pub struct ServiceManager {
    services: Vec<Arc<dyn Service>>,
}

impl ServiceManager {
    /// Create a new service manager.
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
        }
    }

    /// Register a service.
    pub fn register<S>(&mut self, service: S)
    where
        S: Service + 'static,
    {
        self.services.push(Arc::new(service));
    }

    /// Start all registered services.
    pub async fn start_all(&self) -> Result<()> {
        for service in &self.services {
            tracing::info!(service = service.name(), "Starting service");
            service.start().await?;
        }
        Ok(())
    }

    /// Stop all registered services in reverse order.
    pub async fn stop_all(&self) -> Result<()> {
        for service in self.services.iter().rev() {
            tracing::info!(service = service.name(), "Stopping service");
            service.stop().await?;
        }
        Ok(())
    }

    /// Check health of all services.
    pub async fn health_check_all(&self) -> Vec<(String, bool)> {
        let mut results = Vec::new();
        for service in &self.services {
            let healthy = service.health_check().await;
            results.push((service.name().to_string(), healthy));
        }
        results
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}
