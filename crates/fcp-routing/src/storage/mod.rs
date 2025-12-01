//! Storage layer for routing data.
//!
//! This module provides a trait-based abstraction for storing and retrieving
//! routing data (users, policies, routes, etc.). It supports multiple backends:
//! - YAML files (for small deployments)
//! - Database (PostgreSQL, MySQL) [FUTURE]

pub mod yaml;

use crate::models::{PolicyRule, User};
use anyhow::Result;
use async_trait::async_trait;
use fcp_core::TenantId;

/// Storage trait for routing data.
///
/// Implementations of this trait can use any backend (files, database, etc.).
/// All methods are async to support database operations.
#[async_trait]
pub trait RoutingStore: Send + Sync {
    // === Identity Layer ===

    /// Get a user by ID and tenant
    async fn get_user(&self, user_id: &str, tenant_id: &TenantId) -> Result<Option<User>>;

    /// List all users for a tenant
    async fn list_users(&self, tenant_id: &TenantId) -> Result<Vec<User>>;

    // === Policy Layer ===

    /// List all policy rules for a tenant
    async fn list_policy_rules(&self, tenant_id: &TenantId) -> Result<Vec<PolicyRule>>;

    /// Get a specific policy rule by name
    async fn get_policy_rule(
        &self,
        name: &str,
        tenant_id: &TenantId,
    ) -> Result<Option<PolicyRule>>;

    // === Future Layers (stubs for now) ===
    // These will be implemented when we add Reachability, Routing, and Transform layers

    // async fn list_destination_classes(&self, tenant_id: &TenantId) -> Result<Vec<DestinationClass>>;
    // async fn list_routes(&self, tenant_id: &TenantId) -> Result<Vec<Route>>;
    // async fn list_transforms(&self, tenant_id: &TenantId) -> Result<Vec<TransformRule>>;
}
