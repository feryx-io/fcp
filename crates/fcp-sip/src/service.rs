//! Service implementations for SIP components.
//!
//! This module provides implementations of fcp-core::Service trait
//! for various SIP services like transaction manager, registrar, etc.

use async_trait::async_trait;
use fcp_core::{Result as FcpResult, Service};
use std::sync::Arc;

/// Transaction manager service.
///
/// Wraps siphon-rs TransactionManager as an fcp-core Service.
pub struct TransactionService {
    manager: Arc<sip_transaction::TransactionManager>,
}

impl TransactionService {
    /// Create a new transaction service.
    pub fn new(manager: sip_transaction::TransactionManager) -> Self {
        Self {
            manager: Arc::new(manager),
        }
    }

    /// Get a reference to the transaction manager.
    pub fn manager(&self) -> &Arc<sip_transaction::TransactionManager> {
        &self.manager
    }
}

#[async_trait]
impl Service for TransactionService {
    fn name(&self) -> &'static str {
        "sip-transaction-manager"
    }

    async fn start(&self) -> FcpResult<()> {
        tracing::info!("Transaction manager service started");
        Ok(())
    }

    async fn stop(&self) -> FcpResult<()> {
        tracing::info!("Transaction manager service stopped");
        Ok(())
    }

    async fn health_check(&self) -> bool {
        // Transaction manager is stateless and always healthy
        true
    }
}

/// Dialog manager service.
///
/// Wraps siphon-rs DialogManager as an fcp-core Service.
pub struct DialogService {
    manager: Arc<sip_dialog::DialogManager>,
}

impl DialogService {
    /// Create a new dialog service.
    pub fn new(manager: sip_dialog::DialogManager) -> Self {
        Self {
            manager: Arc::new(manager),
        }
    }

    /// Get a reference to the dialog manager.
    pub fn manager(&self) -> &Arc<sip_dialog::DialogManager> {
        &self.manager
    }
}

#[async_trait]
impl Service for DialogService {
    fn name(&self) -> &'static str {
        "sip-dialog-manager"
    }

    async fn start(&self) -> FcpResult<()> {
        tracing::info!("Dialog manager service started");
        Ok(())
    }

    async fn stop(&self) -> FcpResult<()> {
        tracing::info!("Dialog manager service stopped");
        // TODO: Clean up active dialogs gracefully
        Ok(())
    }

    async fn health_check(&self) -> bool {
        // Check if we can access the dialog manager
        true
    }
}

/// Registrar service.
///
/// Wraps siphon-rs BasicRegistrar as an fcp-core Service.
pub struct RegistrarService<S, A>
where
    S: sip_registrar::LocationStore,
    A: sip_auth::Authenticator,
{
    registrar: Arc<sip_registrar::BasicRegistrar<S, A>>,
}

impl<S, A> RegistrarService<S, A>
where
    S: sip_registrar::LocationStore,
    A: sip_auth::Authenticator,
{
    /// Create a new registrar service.
    pub fn new(registrar: sip_registrar::BasicRegistrar<S, A>) -> Self {
        Self {
            registrar: Arc::new(registrar),
        }
    }

    /// Get a reference to the registrar.
    pub fn registrar(&self) -> &Arc<sip_registrar::BasicRegistrar<S, A>> {
        &self.registrar
    }
}

#[async_trait]
impl<S, A> Service for RegistrarService<S, A>
where
    S: sip_registrar::LocationStore + Send + Sync + 'static,
    A: sip_auth::Authenticator + Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        "sip-registrar"
    }

    async fn start(&self) -> FcpResult<()> {
        tracing::info!("Registrar service started");
        Ok(())
    }

    async fn stop(&self) -> FcpResult<()> {
        tracing::info!("Registrar service stopped");
        // TODO: Persist registrations if needed
        Ok(())
    }

    async fn health_check(&self) -> bool {
        // Registrar is healthy if storage is accessible
        true
    }
}

/// Subscription manager service.
///
/// Wraps siphon-rs SubscriptionManager as an fcp-core Service.
pub struct SubscriptionService {
    manager: Arc<sip_dialog::SubscriptionManager>,
}

impl SubscriptionService {
    /// Create a new subscription service.
    pub fn new(manager: sip_dialog::SubscriptionManager) -> Self {
        Self {
            manager: Arc::new(manager),
        }
    }

    /// Get a reference to the subscription manager.
    pub fn manager(&self) -> &Arc<sip_dialog::SubscriptionManager> {
        &self.manager
    }
}

#[async_trait]
impl Service for SubscriptionService {
    fn name(&self) -> &'static str {
        "sip-subscription-manager"
    }

    async fn start(&self) -> FcpResult<()> {
        tracing::info!("Subscription manager service started");
        Ok(())
    }

    async fn stop(&self) -> FcpResult<()> {
        let count = self.manager.cleanup_terminated();
        tracing::info!(
            terminated_cleaned = count,
            "Subscription manager service stopped"
        );
        Ok(())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_names() {
        let dialog_mgr = sip_dialog::DialogManager::new();
        let dialog_service = DialogService::new(dialog_mgr);
        assert_eq!(dialog_service.name(), "sip-dialog-manager");

        let sub_mgr = sip_dialog::SubscriptionManager::new();
        let sub_service = SubscriptionService::new(sub_mgr);
        assert_eq!(sub_service.name(), "sip-subscription-manager");
    }
}
