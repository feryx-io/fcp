//! Organizational location (site) entity for multi-location support.
//!
//! Represents a physical office location that users, devices, and extensions
//! can be assigned to. Exposed as "Location" in the REST API and frontend.

use crate::identifiers::{LocationId, TenantId};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// An organizational site/location within a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteLocation {
    /// Unique identifier
    pub id: LocationId,

    /// Tenant this location belongs to
    pub tenant_id: TenantId,

    /// Location name (required)
    pub name: String,

    /// Description
    pub description: Option<String>,

    // ===== Address =====
    /// Street address line 1
    pub address_line1: Option<String>,

    /// Street address line 2
    pub address_line2: Option<String>,

    /// City
    pub city: Option<String>,

    /// State/Province
    pub state: Option<String>,

    /// Postal/ZIP code
    pub postal_code: Option<String>,

    /// Country
    pub country: Option<String>,

    // ===== Building Details =====
    /// Floor
    pub floor: Option<String>,

    /// Building name/number
    pub building: Option<String>,

    // ===== Coordinates =====
    /// Latitude
    pub latitude: Option<f64>,

    /// Longitude
    pub longitude: Option<f64>,

    // ===== Contact =====
    /// Main phone number for this location
    pub phone_number: Option<String>,

    /// Timezone (e.g., "America/New_York")
    pub timezone: Option<String>,

    /// Emergency contact name
    pub emergency_contact_name: Option<String>,

    /// Emergency contact phone number
    pub emergency_contact_phone: Option<String>,
    /// E911 callback number (ELIN) the carrier maps to this address; sent as
    /// caller ID on emergency calls placed from this site.
    #[serde(default)]
    pub e911_callback_number: Option<String>,

    // ===== Timestamps =====
    /// Created timestamp
    pub created_at: SystemTime,

    /// Last modified timestamp
    pub updated_at: SystemTime,
}

impl SiteLocation {
    /// Create a new site location with required fields.
    pub fn new(name: String, tenant_id: TenantId) -> Self {
        let now = SystemTime::now();
        Self {
            id: LocationId::new(),
            tenant_id,
            name,
            description: None,
            address_line1: None,
            address_line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
            floor: None,
            building: None,
            latitude: None,
            longitude: None,
            phone_number: None,
            timezone: None,
            emergency_contact_name: None,
            emergency_contact_phone: None,
            e911_callback_number: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_site_location_creation() {
        let tenant_id = TenantId::new();
        let loc = SiteLocation::new("HQ".to_string(), tenant_id);

        assert_eq!(loc.name, "HQ");
        assert_eq!(loc.tenant_id, tenant_id);
        assert!(loc.description.is_none());
        assert!(loc.city.is_none());
    }
}
