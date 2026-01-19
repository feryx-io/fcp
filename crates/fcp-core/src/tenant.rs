//! Tenant data structures and types
//!
//! Multi-tenancy is a core design principle of FCP. All resources (users, devices, phone numbers)
//! belong to a tenant.

use crate::identifiers::TenantId;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Account status for a tenant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStatus {
    /// Tenant is active and can use the system
    Active,
    /// Tenant is suspended (cannot make/receive calls)
    Suspended,
    /// Tenant is in trial period
    Trial,
    /// Tenant account is closed
    Closed,
}

impl Default for TenantStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl std::fmt::Display for TenantStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "Active"),
            Self::Suspended => write!(f, "Suspended"),
            Self::Trial => write!(f, "Trial"),
            Self::Closed => write!(f, "Closed"),
        }
    }
}

/// A tenant in the FCP system
///
/// Tenants are the top-level organizational unit. All users, devices, phone numbers,
/// and other resources belong to a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    /// Unique tenant identifier
    pub id: TenantId,

    /// Tenant name (e.g., "Acme Corporation")
    pub name: String,

    /// Optional description or notes
    pub description: Option<String>,

    /// Account status
    pub status: TenantStatus,

    /// Contact email for the tenant
    pub contact_email: Option<String>,

    /// Contact phone number
    pub contact_phone: Option<String>,

    /// Maximum number of users allowed
    pub max_users: Option<u32>,

    /// Maximum number of phone numbers allowed
    pub max_phone_numbers: Option<u32>,

    /// Maximum number of concurrent calls allowed
    pub max_concurrent_calls: Option<u32>,

    /// Billing identifier (for external billing system)
    pub billing_id: Option<String>,

    /// When the tenant was created
    pub created_at: SystemTime,

    /// When the tenant was last updated
    pub updated_at: SystemTime,

    /// Optional trial expiration date
    pub trial_expires_at: Option<SystemTime>,

    /// Additional metadata (JSON)
    pub metadata: Option<String>,
}

impl Tenant {
    /// Create a new tenant
    pub fn new(name: String) -> Self {
        let now = SystemTime::now();
        Self {
            id: TenantId::new(),
            name,
            description: None,
            status: TenantStatus::Active,
            contact_email: None,
            contact_phone: None,
            max_users: None,
            max_phone_numbers: None,
            max_concurrent_calls: None,
            billing_id: None,
            created_at: now,
            updated_at: now,
            trial_expires_at: None,
            metadata: None,
        }
    }

    /// Check if tenant is active
    pub fn is_active(&self) -> bool {
        self.status == TenantStatus::Active || self.status == TenantStatus::Trial
    }

    /// Check if tenant is in trial
    pub fn is_trial(&self) -> bool {
        self.status == TenantStatus::Trial
    }

    /// Check if trial has expired
    pub fn is_trial_expired(&self) -> bool {
        if let Some(expires_at) = self.trial_expires_at {
            SystemTime::now() > expires_at
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_creation() {
        let tenant = Tenant::new("Test Corp".to_string());
        assert_eq!(tenant.name, "Test Corp");
        assert_eq!(tenant.status, TenantStatus::Active);
        assert!(tenant.is_active());
        assert!(!tenant.is_trial());
    }

    #[test]
    fn test_tenant_status() {
        let mut tenant = Tenant::new("Test Corp".to_string());

        tenant.status = TenantStatus::Suspended;
        assert!(!tenant.is_active());

        tenant.status = TenantStatus::Trial;
        assert!(tenant.is_active());
        assert!(tenant.is_trial());
    }

    #[test]
    fn test_trial_expiration() {
        let mut tenant = Tenant::new("Test Corp".to_string());
        tenant.status = TenantStatus::Trial;

        // No expiration set
        assert!(!tenant.is_trial_expired());

        // Set expiration in the past
        tenant.trial_expires_at = Some(SystemTime::UNIX_EPOCH);
        assert!(tenant.is_trial_expired());

        // Set expiration in the future
        tenant.trial_expires_at = Some(SystemTime::now() + std::time::Duration::from_secs(86400));
        assert!(!tenant.is_trial_expired());
    }
}
