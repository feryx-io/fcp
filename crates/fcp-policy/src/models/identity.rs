//! Identity Layer - WHO is calling?
//!
//! This module defines user identities with roles and groups for policy evaluation.

use fcp_core::TenantId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// User identity with roles and group memberships.
///
/// Users are the primary identity in the routing system. Each user can have:
/// - Multiple roles (e.g., "employee", "sales-team", "management")
/// - Multiple groups (e.g., "us-west", "engineering")
/// - Multiple endpoints (desk phone, softphone, mobile)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    /// Unique user identifier (e.g., "alice@company.com")
    pub id: String,

    /// Tenant this user belongs to
    pub tenant_id: TenantId,

    /// Human-friendly display name
    pub display_name: Option<String>,

    /// Set of roles assigned to this user
    /// Roles are used for permission matching (e.g., ["employee", "sales-team"])
    pub roles: HashSet<String>,

    /// Set of groups this user belongs to
    /// Groups can be used for organizational or location-based policies
    pub groups: HashSet<String>,

    /// List of endpoints (devices) registered for this user
    pub endpoints: Vec<Endpoint>,

    /// Whether this user account is enabled
    pub enabled: bool,
}

/// An endpoint (device) belonging to a user.
///
/// Each user can have multiple endpoints (desk phone, softphone, mobile).
/// The routing system can use endpoint information for routing decisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Endpoint {
    /// SIP URI for this endpoint (e.g., "sip:alice@10.0.1.100:5060")
    pub uri: String,

    /// Type of device
    pub device_type: DeviceType,

    /// Physical or logical location (optional)
    /// Can be used for emergency services or location-based routing
    pub location: Option<String>,
}

/// Device types for endpoints.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum DeviceType {
    /// Physical desk phone
    DeskPhone,

    /// Software-based SIP client
    Softphone,

    /// Mobile device (cellular or app)
    Mobile,

    /// Web-based WebRTC client
    WebRTC,
}

/// Identity context for policy evaluation.
///
/// This is built from the incoming SIP request and user information.
/// It contains all identity-related information needed for policy decisions.
#[derive(Debug, Clone)]
pub struct IdentityContext {
    /// User ID making the call
    pub user_id: String,

    /// Tenant ID
    pub tenant_id: TenantId,

    /// Set of roles the user has
    pub roles: HashSet<String>,

    /// Set of groups the user belongs to
    pub groups: HashSet<String>,

    /// Source IP address of the request
    pub source_ip: std::net::IpAddr,

    /// User-Agent header from SIP request (if present)
    pub user_agent: Option<String>,
}

impl User {
    /// Validates user data for security and correctness.
    ///
    /// # Security Checks
    /// - Prevents CRLF injection in all string fields (id, display_name, roles, groups)
    /// - Ensures user ID is not empty
    /// - Recursively validates all endpoints
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("user.id must not be empty".to_string());
        }
        reject_crlf("user.id", &self.id)?;
        if let Some(name) = &self.display_name {
            reject_crlf("user.display_name", name)?;
        }
        for role in &self.roles {
            reject_crlf("user.roles", role)?;
        }
        for group in &self.groups {
            reject_crlf("user.groups", group)?;
        }
        for endpoint in &self.endpoints {
            endpoint.validate()?;
        }
        Ok(())
    }
}

impl Endpoint {
    /// Validates endpoint fields for security and correctness.
    ///
    /// # Security Checks
    /// - Prevents CRLF injection in URI and location fields
    /// - Validates URI scheme is sip: or sips: (RFC 3261)
    /// - Ensures URI is not empty
    pub fn validate(&self) -> Result<(), String> {
        if self.uri.is_empty() {
            return Err("endpoint.uri must not be empty".to_string());
        }
        reject_crlf("endpoint.uri", &self.uri)?;

        // Validate URI scheme (RFC 3261 - SIP URIs)
        let uri_lower = self.uri.to_lowercase();
        if !uri_lower.starts_with("sip:") && !uri_lower.starts_with("sips:") {
            return Err(format!(
                "endpoint.uri must start with 'sip:' or 'sips:', got: {}",
                self.uri
            ));
        }

        if let Some(loc) = &self.location {
            reject_crlf("endpoint.location", loc)?;
        }
        Ok(())
    }
}

impl IdentityContext {
    /// Validate identity context strings for CR/LF.
    pub fn validate(&self) -> Result<(), String> {
        reject_crlf("identity.user_id", &self.user_id)?;
        if let Some(ua) = &self.user_agent {
            reject_crlf("identity.user_agent", ua)?;
        }
        Ok(())
    }
}

fn reject_crlf(field: &str, value: &str) -> Result<(), String> {
    if value.bytes().any(|b| matches!(b, b'\r' | b'\n')) {
        Err(format!("{field} must not contain CR or LF characters"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::TenantId;

    #[test]
    fn test_user_creation() {
        let mut roles = HashSet::new();
        roles.insert("employee".to_string());
        roles.insert("sales".to_string());

        let mut groups = HashSet::new();
        groups.insert("us-west".to_string());

        let user = User {
            id: "alice@company.com".to_string(),
            tenant_id: TenantId::nil(),
            display_name: Some("Alice Smith".to_string()),
            roles,
            groups,
            endpoints: vec![Endpoint {
                uri: "sip:alice@10.0.1.100:5060".to_string(),
                device_type: DeviceType::DeskPhone,
                location: Some("Building A".to_string()),
            }],
            enabled: true,
        };

        assert_eq!(user.id, "alice@company.com");
        assert!(user.roles.contains("employee"));
        assert!(user.roles.contains("sales"));
        assert!(user.groups.contains("us-west"));
        assert_eq!(user.endpoints.len(), 1);
        assert!(user.enabled);
    }

    #[test]
    fn test_identity_context_creation() {
        let mut roles = HashSet::new();
        roles.insert("employee".to_string());

        let mut groups = HashSet::new();
        groups.insert("engineering".to_string());

        let ctx = IdentityContext {
            user_id: "bob@company.com".to_string(),
            tenant_id: TenantId::nil(),
            roles,
            groups,
            source_ip: "10.0.1.50".parse().unwrap(),
            user_agent: Some("Zoiper/1.0".to_string()),
        };

        assert_eq!(ctx.user_id, "bob@company.com");
        assert!(ctx.roles.contains("employee"));
        assert!(ctx.groups.contains("engineering"));
        assert_eq!(ctx.source_ip.to_string(), "10.0.1.50");
    }

    #[test]
    fn test_device_type_serialization() {
        // Test that device types serialize with PascalCase
        let desk_phone = DeviceType::DeskPhone;
        let json = serde_json::to_string(&desk_phone).unwrap();
        assert_eq!(json, "\"DeskPhone\"");

        let softphone = DeviceType::Softphone;
        let json = serde_json::to_string(&softphone).unwrap();
        assert_eq!(json, "\"Softphone\"");
    }
}
