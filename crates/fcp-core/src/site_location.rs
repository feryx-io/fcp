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

    /// Who is told when an emergency call is placed from this site, before
    /// the tenant's own contacts (`docs/E911.md` "On-site notification"):
    /// the front desk or security of this building. Each may be allowed to
    /// listen in to the call.
    #[serde(default)]
    pub emergency_contacts: Vec<crate::e911::notify::NotifyContact>,

    /// Whether the tenant's contacts are told too about a call from this
    /// site; true unless the site's own contacts are all that should be.
    #[serde(default = "default_true")]
    pub notify_tenant_contacts: bool,
    /// E911 callback number (ELIN) the carrier maps to this address; sent as
    /// caller ID on emergency calls placed from this site.
    #[serde(default)]
    pub e911_callback_number: Option<String>,

    /// The site's street address as RFC 5139 civic elements, when it has
    /// been structured (`docs/E911_BLOCK_2.md` §5.1). Floor and building
    /// stay in their own fields.
    #[serde(default)]
    pub civic: crate::phone_number::CivicElements,

    /// When a validator last confirmed this address
    /// (`POST /v1/locations/:id/e911/validate`, `docs/E911_BLOCK_2.md`
    /// §7). Editing the address clears it.
    #[serde(default)]
    pub e911_validated_at: Option<SystemTime>,

    /// The validator that confirmed it (`basic`, `http`, …).
    #[serde(default)]
    pub e911_validated_by: Option<String>,

    /// The reference that validator gave for its answer.
    #[serde(default)]
    pub e911_validation_reference: Option<String>,

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
            emergency_contacts: Vec::new(),
            notify_tenant_contacts: true,
            e911_callback_number: None,
            civic: Default::default(),
            e911_validated_at: None,
            e911_validated_by: None,
            e911_validation_reference: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Whether a validator has confirmed the site's address.
    pub fn e911_validated(&self) -> bool {
        self.e911_validated_at.is_some()
    }

    /// Forget that the address was validated, as editing any part of it
    /// must (`docs/E911_BLOCK_2.md` §7).
    pub fn clear_e911_validation(&mut self) {
        self.e911_validated_at = None;
        self.e911_validated_by = None;
        self.e911_validation_reference = None;
    }

    /// The parts of the site a PSAP is dispatched to, for telling whether
    /// an edit changed the address.
    pub fn address_fingerprint(&self) -> String {
        let field = |v: &Option<String>| v.as_deref().unwrap_or_default().trim().to_string();
        [
            field(&self.address_line1),
            field(&self.address_line2),
            field(&self.city),
            field(&self.state),
            field(&self.postal_code),
            field(&self.country),
            field(&self.floor),
            field(&self.building),
            serde_json::to_string(&self.civic).unwrap_or_default(),
        ]
        .join("\u{1f}")
    }
}

fn default_true() -> bool {
    true
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
