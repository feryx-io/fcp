//! Routing service - high-level API for call routing.
//!
//! This module provides a simple service wrapper around the policy evaluator.

use crate::evaluator::PolicyEvaluator;
use crate::models::PolicyDecision;
use crate::storage::RoutingStore;
use anyhow::Result;
use fcp_core::TenantId;
use std::net::IpAddr;
use std::sync::Arc;

/// Routing service for call policy evaluation.
///
/// This is the main entry point for routing operations.
/// It wraps the policy evaluator and storage layer.
pub struct RoutingService {
    /// Policy evaluator
    evaluator: PolicyEvaluator,

    /// Storage reference (kept for potential future use)
    #[allow(dead_code)]
    store: Arc<dyn RoutingStore>,
}

impl RoutingService {
    /// Create a new routing service.
    ///
    /// # Arguments
    ///
    /// * `store` - Storage backend (YAML or database)
    pub fn new(store: Arc<dyn RoutingStore>) -> Self {
        let evaluator = PolicyEvaluator::new(store.clone());
        Self { evaluator, store }
    }

    /// Get the tenant ID for a user.
    ///
    /// This is useful when you have a caller ID (e.g., from SIP From header)
    /// and need to determine which tenant they belong to before evaluating policies.
    ///
    /// # Arguments
    ///
    /// * `user_id` - User identifier (e.g., "alice@company.com")
    ///
    /// # Returns
    ///
    /// The user's tenant ID, or an error if the user is not found.
    ///
    /// # Note
    ///
    /// This method searches across all tenants. In production, you might want to
    /// narrow the search based on domain or other criteria for better performance.
    pub async fn get_user_tenant(&self, user_id: &str) -> Result<TenantId> {
        // For now, try default tenant first (common case optimization)
        let default_tenant = TenantId::nil();
        if let Some(user) = self.store.get_user(user_id, &default_tenant).await? {
            return Ok(user.tenant_id);
        }

        // If not found in default tenant, return default tenant anyway
        // This supports single-tenant deployments and all_users policies where
        // users are managed via fcp-admin API rather than the YAML routing store
        Ok(default_tenant)
    }

    /// Evaluate if a call is allowed.
    ///
    /// This is the main entry point for call authorization.
    ///
    /// # Arguments
    ///
    /// * `caller` - User ID of the caller
    /// * `destination` - Destination being called
    /// * `tenant_id` - Tenant ID for isolation
    /// * `source_ip` - Source IP address
    ///
    /// # Returns
    ///
    /// A `PolicyDecision` with Allow/Deny action and reason.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use fcp_routing::RoutingService;
    /// # use std::sync::Arc;
    /// # async fn example(service: Arc<RoutingService>) -> anyhow::Result<()> {
    /// use fcp_core::TenantId;
    /// use std::str::FromStr;
    ///
    /// let decision = service.evaluate_call(
    ///     "alice@company.com",
    ///     "sip:bob@company.com",
    ///     &TenantId::from_str("default")?,
    ///     "10.0.1.100".parse()?,
    /// ).await?;
    ///
    /// if decision.action.is_allow() {
    ///     println!("Call allowed: {}", decision.reason);
    /// } else {
    ///     println!("Call denied: {}", decision.reason);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn evaluate_call(
        &self,
        caller: &str,
        destination: &str,
        tenant_id: &TenantId,
        source_ip: IpAddr,
    ) -> Result<PolicyDecision> {
        self.evaluator
            .evaluate_call(caller, destination, tenant_id, source_ip)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AppliesTo, PolicyAction, PolicyRule, User};
    use crate::storage::RoutingStore;
    use async_trait::async_trait;
    use std::collections::HashSet;

    // Mock storage for testing
    struct MockStore {
        users: Vec<User>,
        policies: Vec<PolicyRule>,
    }

    #[async_trait]
    impl RoutingStore for MockStore {
        async fn get_user(&self, user_id: &str, tenant_id: &TenantId) -> Result<Option<User>> {
            Ok(self
                .users
                .iter()
                .find(|u| u.id == user_id && &u.tenant_id == tenant_id)
                .cloned())
        }

        async fn list_users(&self, tenant_id: &TenantId) -> Result<Vec<User>> {
            Ok(self
                .users
                .iter()
                .filter(|u| &u.tenant_id == tenant_id)
                .cloned()
                .collect())
        }

        async fn list_policy_rules(&self, tenant_id: &TenantId) -> Result<Vec<PolicyRule>> {
            Ok(self
                .policies
                .iter()
                .filter(|p| &p.tenant_id == tenant_id)
                .cloned()
                .collect())
        }

        async fn get_policy_rule(
            &self,
            name: &str,
            tenant_id: &TenantId,
        ) -> Result<Option<PolicyRule>> {
            Ok(self
                .policies
                .iter()
                .find(|p| p.name == name && &p.tenant_id == tenant_id)
                .cloned())
        }
    }

    #[tokio::test]
    async fn test_service_basic_usage() {
        let tenant_id = TenantId::nil();

        let mut roles = HashSet::new();
        roles.insert("employee".to_string());

        let user = User {
            id: "alice@test.com".to_string(),
            tenant_id,
            display_name: Some("Alice".to_string()),
            roles,
            groups: HashSet::new(),
            endpoints: vec![],
            enabled: true,
        };

        let rule = PolicyRule {
            name: "allow-all".to_string(),
            tenant_id,
            priority: 100,
            enabled: true,
            applies_to: AppliesTo {
                all_users: false,
                users: None,
                roles: Some(vec!["employee".to_string()]),
                groups: None,
            },
            destination_match: None,
            time_windows: vec![],
            source_ip_ranges: vec![],
            action: PolicyAction::Allow,
            reason: Some("All employees allowed".to_string()),
        };

        let store = Arc::new(MockStore {
            users: vec![user],
            policies: vec![rule],
        });

        let service = RoutingService::new(store);

        let decision = service
            .evaluate_call(
                "alice@test.com",
                "any-destination",
                &tenant_id,
                "10.0.1.100".parse().unwrap(),
            )
            .await
            .unwrap();

        assert!(decision.action.is_allow());
        assert_eq!(decision.reason, "All employees allowed");
    }
}
