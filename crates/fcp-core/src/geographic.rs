//! Geographic routing for location-based call distribution.
//!
//! This module provides geographic routing capabilities including:
//! - Location-based routing rules
//! - Distance calculations (haversine formula)
//! - Regional routing (country, state, city)
//! - Emergency services routing (E911)
//! - Time zone aware routing
//! - Nearest location routing

use crate::{LineBindingId, PhoneNumberId, TenantId};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Geographic coordinates (latitude and longitude).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Coordinates {
    /// Latitude in decimal degrees (-90 to 90)
    pub latitude: f64,

    /// Longitude in decimal degrees (-180 to 180)
    pub longitude: f64,
}

impl Coordinates {
    /// Creates new coordinates.
    pub fn new(latitude: f64, longitude: f64) -> Result<Self, &'static str> {
        if !(-90.0..=90.0).contains(&latitude) {
            return Err("Latitude must be between -90 and 90");
        }
        if !(-180.0..=180.0).contains(&longitude) {
            return Err("Longitude must be between -180 and 180");
        }
        Ok(Self {
            latitude,
            longitude,
        })
    }

    /// Calculates the distance to another coordinate using the Haversine formula.
    /// Returns distance in kilometers.
    pub fn distance_to(&self, other: &Coordinates) -> f64 {
        const EARTH_RADIUS_KM: f64 = 6371.0;

        let lat1_rad = self.latitude.to_radians();
        let lat2_rad = other.latitude.to_radians();
        let delta_lat = (other.latitude - self.latitude).to_radians();
        let delta_lon = (other.longitude - self.longitude).to_radians();

        let a = (delta_lat / 2.0).sin().powi(2)
            + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);

        let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

        EARTH_RADIUS_KM * c
    }

    /// Checks if this location is within a given radius of another location.
    pub fn is_within_radius(&self, other: &Coordinates, radius_km: f64) -> bool {
        self.distance_to(other) <= radius_km
    }
}

/// Geographic address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    /// Street address line 1
    pub street1: String,

    /// Street address line 2 (optional)
    pub street2: Option<String>,

    /// City
    pub city: String,

    /// State/province code (e.g., "CA", "NY")
    pub state: String,

    /// Postal/ZIP code
    pub postal_code: String,

    /// ISO 3166-1 alpha-2 country code (e.g., "US", "CA", "GB")
    pub country: String,
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref street2) = self.street2 {
            write!(
                f,
                "{}, {}, {}, {} {}, {}",
                self.street1, street2, self.city, self.state, self.postal_code, self.country
            )
        } else {
            write!(
                f,
                "{}, {}, {} {}, {}",
                self.street1, self.city, self.state, self.postal_code, self.country
            )
        }
    }
}

/// Geographic region for routing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Region {
    /// Country (ISO 3166-1 alpha-2 code)
    Country(String),

    /// State/province within a country
    State { country: String, state: String },

    /// City within a state
    City {
        country: String,
        state: String,
        city: String,
    },

    /// Postal code area
    PostalCode {
        country: String,
        postal_code: String,
    },

    /// Area code (for phone numbers)
    AreaCode { country: String, area_code: String },

    /// Custom region defined by coordinates and radius
    Radius { center: Coordinates, radius_km: f64 },
}

impl std::fmt::Display for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Region::Country(country) => write!(f, "{}", country),
            Region::State { country, state } => write!(f, "{}, {}", state, country),
            Region::City {
                country,
                state,
                city,
            } => write!(f, "{}, {}, {}", city, state, country),
            Region::PostalCode {
                country,
                postal_code,
            } => write!(f, "{} {}", country, postal_code),
            Region::AreaCode { country, area_code } => write!(f, "{} ({})", country, area_code),
            Region::Radius { center, radius_km } => {
                write!(
                    f,
                    "within {}km of ({}, {})",
                    radius_km, center.latitude, center.longitude
                )
            }
        }
    }
}

impl Region {
    /// Checks if an address matches this region.
    pub fn matches_address(&self, address: &Address) -> bool {
        match self {
            Region::Country(country) => address.country == *country,
            Region::State {
                country,
                state: region_state,
            } => address.country == *country && address.state == *region_state,
            Region::City {
                country,
                state: region_state,
                city: region_city,
            } => {
                address.country == *country
                    && address.state == *region_state
                    && address.city == *region_city
            }
            Region::PostalCode {
                country,
                postal_code,
            } => address.country == *country && address.postal_code == *postal_code,
            Region::AreaCode { .. } => {
                // Area code matching would require phone number context
                false
            }
            Region::Radius { .. } => {
                // Radius matching requires coordinates
                false
            }
        }
    }

    /// Checks if coordinates match this region.
    pub fn matches_coordinates(&self, coords: &Coordinates) -> bool {
        match self {
            Region::Radius { center, radius_km } => coords.is_within_radius(center, *radius_km),
            _ => false, // Other region types require address context
        }
    }
}

/// Geographic location combining coordinates and address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    /// Optional coordinates
    pub coordinates: Option<Coordinates>,

    /// Optional address
    pub address: Option<Address>,

    /// IANA time zone identifier (e.g., "America/New_York")
    pub timezone: Option<String>,
}

impl Location {
    /// Creates a new location from coordinates.
    pub fn from_coordinates(coordinates: Coordinates) -> Self {
        Self {
            coordinates: Some(coordinates),
            address: None,
            timezone: None,
        }
    }

    /// Creates a new location from an address.
    pub fn from_address(address: Address) -> Self {
        Self {
            coordinates: None,
            address: Some(address),
            timezone: None,
        }
    }

    /// Creates a complete location with coordinates and address.
    pub fn new(coordinates: Coordinates, address: Address, timezone: Option<String>) -> Self {
        Self {
            coordinates: Some(coordinates),
            address: Some(address),
            timezone,
        }
    }

    /// Checks if this location matches a region.
    pub fn matches_region(&self, region: &Region) -> bool {
        // Try coordinates first (for radius regions)
        if let Some(coords) = &self.coordinates {
            if region.matches_coordinates(coords) {
                return true;
            }
        }

        // Try address
        if let Some(addr) = &self.address {
            if region.matches_address(addr) {
                return true;
            }
        }

        false
    }

    /// Calculates distance to another location (if both have coordinates).
    pub fn distance_to(&self, other: &Location) -> Option<f64> {
        if let (Some(c1), Some(c2)) = (&self.coordinates, &other.coordinates) {
            Some(c1.distance_to(c2))
        } else {
            None
        }
    }
}

/// Geographic routing strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeoRoutingStrategy {
    /// Route to nearest location based on distance
    Nearest,

    /// Route to specific region
    Regional,

    /// Route based on time zone (prefer same timezone)
    Timezone,

    /// Route to emergency services (E911)
    Emergency,
}

impl std::fmt::Display for GeoRoutingStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GeoRoutingStrategy::Nearest => write!(f, "nearest"),
            GeoRoutingStrategy::Regional => write!(f, "regional"),
            GeoRoutingStrategy::Timezone => write!(f, "timezone"),
            GeoRoutingStrategy::Emergency => write!(f, "emergency"),
        }
    }
}

/// Geographic routing rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoRoutingRule {
    /// Rule ID
    pub id: String,

    /// Tenant ID
    pub tenant_id: TenantId,

    /// Rule name
    pub name: String,

    /// Rule description
    pub description: Option<String>,

    /// Whether this rule is enabled
    pub enabled: bool,

    /// Priority (lower = higher priority)
    pub priority: u8,

    /// Routing strategy
    pub strategy: GeoRoutingStrategy,

    /// Source regions (if empty, applies to all sources)
    pub source_regions: Vec<Region>,

    /// Target regions for regional routing
    pub target_regions: Vec<Region>,

    /// Destination line bindings for each target region
    pub region_destinations: Vec<RegionDestination>,

    /// Created timestamp
    pub created_at: SystemTime,

    /// Updated timestamp
    pub updated_at: SystemTime,
}

/// Destination for a geographic region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionDestination {
    /// Region this destination serves
    pub region: Region,

    /// Line binding to route to
    pub line_binding_id: LineBindingId,

    /// Location of this destination
    pub location: Location,

    /// Whether this destination is enabled
    pub enabled: bool,
}

impl GeoRoutingRule {
    /// Creates a new geographic routing rule.
    pub fn new(tenant_id: TenantId, name: String, strategy: GeoRoutingStrategy) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            tenant_id,
            name,
            description: None,
            enabled: true,
            priority: 0,
            strategy,
            source_regions: Vec::new(),
            target_regions: Vec::new(),
            region_destinations: Vec::new(),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        }
    }

    /// Checks if this rule applies to a given source location.
    pub fn applies_to_source(&self, source_location: &Location) -> bool {
        if !self.enabled {
            return false;
        }

        // If no source regions specified, applies to all
        if self.source_regions.is_empty() {
            return true;
        }

        // Check if source location matches any source region
        self.source_regions
            .iter()
            .any(|region| source_location.matches_region(region))
    }

    /// Finds the best destination for a given source location.
    pub fn find_destination(&self, source_location: &Location) -> Option<&RegionDestination> {
        if !self.applies_to_source(source_location) {
            return None;
        }

        match self.strategy {
            GeoRoutingStrategy::Nearest => {
                // Find nearest destination by distance
                self.find_nearest_destination(source_location)
            }
            GeoRoutingStrategy::Regional => {
                // Find destination in matching region
                self.find_regional_destination(source_location)
            }
            GeoRoutingStrategy::Timezone => {
                // Find destination in same timezone
                self.find_timezone_destination(source_location)
            }
            GeoRoutingStrategy::Emergency => {
                // Emergency routing (E911) - find nearest enabled destination
                self.find_emergency_destination(source_location)
            }
        }
    }

    fn find_nearest_destination(&self, source_location: &Location) -> Option<&RegionDestination> {
        let mut nearest: Option<(&RegionDestination, f64)> = None;

        for dest in &self.region_destinations {
            if !dest.enabled {
                continue;
            }

            if let Some(distance) = source_location.distance_to(&dest.location) {
                match nearest {
                    None => nearest = Some((dest, distance)),
                    Some((_, current_min)) if distance < current_min => {
                        nearest = Some((dest, distance));
                    }
                    _ => {}
                }
            }
        }

        nearest.map(|(dest, _)| dest)
    }

    fn find_regional_destination(&self, source_location: &Location) -> Option<&RegionDestination> {
        // Find first enabled destination in a matching region
        self.region_destinations
            .iter()
            .find(|&dest| dest.enabled && source_location.matches_region(&dest.region))
            .map(|v| v as _)
    }

    fn find_timezone_destination(&self, source_location: &Location) -> Option<&RegionDestination> {
        let source_tz = source_location.timezone.as_ref()?;

        // Find first enabled destination in same timezone
        for dest in &self.region_destinations {
            if dest.enabled {
                if let Some(ref dest_tz) = dest.location.timezone {
                    if dest_tz == source_tz {
                        return Some(dest);
                    }
                }
            }
        }

        None
    }

    fn find_emergency_destination(&self, source_location: &Location) -> Option<&RegionDestination> {
        // For emergency services, prefer regional match first, then nearest
        if let Some(dest) = self.find_regional_destination(source_location) {
            return Some(dest);
        }

        self.find_nearest_destination(source_location)
    }
}

/// Geographic routing manager.
#[derive(Debug, Clone)]
pub struct GeoRoutingManager {
    /// All geographic routing rules
    rules: std::collections::HashMap<TenantId, Vec<GeoRoutingRule>>,
}

impl GeoRoutingManager {
    /// Creates a new geographic routing manager.
    pub fn new() -> Self {
        Self {
            rules: std::collections::HashMap::new(),
        }
    }

    /// Adds a routing rule.
    pub fn add_rule(&mut self, rule: GeoRoutingRule) {
        let tenant_id = rule.tenant_id;
        self.rules.entry(tenant_id).or_default().push(rule);

        // Sort by priority
        if let Some(rules) = self.rules.get_mut(&tenant_id) {
            rules.sort_by_key(|r| r.priority);
        }
    }

    /// Gets all rules for a tenant.
    pub fn get_rules(&self, tenant_id: &TenantId) -> Vec<&GeoRoutingRule> {
        self.rules
            .get(tenant_id)
            .map(|rules| rules.iter().collect())
            .unwrap_or_default()
    }

    /// Finds the best destination for a given source location.
    pub fn route(&self, tenant_id: &TenantId, source_location: &Location) -> Option<LineBindingId> {
        let rules = self.rules.get(tenant_id)?;

        for rule in rules {
            if let Some(dest) = rule.find_destination(source_location) {
                return Some(dest.line_binding_id);
            }
        }

        None
    }

    /// Removes a rule by ID.
    pub fn remove_rule(&mut self, tenant_id: &TenantId, rule_id: &str) -> bool {
        if let Some(rules) = self.rules.get_mut(tenant_id) {
            let initial_len = rules.len();
            rules.retain(|r| r.id != rule_id);
            return rules.len() < initial_len;
        }
        false
    }
}

impl Default for GeoRoutingManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Phone number location information (for E911 and geographic routing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhoneNumberLocation {
    /// Phone number ID
    pub phone_number_id: PhoneNumberId,

    /// Tenant ID
    pub tenant_id: TenantId,

    /// Location information
    pub location: Location,

    /// Whether this is an emergency services location (E911)
    pub is_emergency: bool,

    /// Additional emergency information
    pub emergency_info: Option<String>,

    /// Last verified timestamp
    pub verified_at: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinates_creation() {
        let coords = Coordinates::new(37.7749, -122.4194).unwrap();
        assert_eq!(coords.latitude, 37.7749);
        assert_eq!(coords.longitude, -122.4194);

        // Test invalid coordinates
        assert!(Coordinates::new(91.0, 0.0).is_err());
        assert!(Coordinates::new(0.0, 181.0).is_err());
    }

    #[test]
    fn test_distance_calculation() {
        // San Francisco
        let sf = Coordinates::new(37.7749, -122.4194).unwrap();
        // Los Angeles
        let la = Coordinates::new(34.0522, -118.2437).unwrap();

        let distance = sf.distance_to(&la);
        // Distance should be approximately 559 km
        assert!((distance - 559.0).abs() < 10.0);
    }

    #[test]
    fn test_within_radius() {
        let center = Coordinates::new(37.7749, -122.4194).unwrap();
        let nearby = Coordinates::new(37.8, -122.4).unwrap();
        let far = Coordinates::new(34.0522, -118.2437).unwrap();

        assert!(center.is_within_radius(&nearby, 10.0));
        assert!(!center.is_within_radius(&far, 100.0));
        assert!(center.is_within_radius(&far, 600.0));
    }

    #[test]
    fn test_region_matching() {
        let addr = Address {
            street1: "123 Main St".to_string(),
            street2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94102".to_string(),
            country: "US".to_string(),
        };

        let country_region = Region::Country("US".to_string());
        assert!(country_region.matches_address(&addr));

        let state_region = Region::State {
            country: "US".to_string(),
            state: "CA".to_string(),
        };
        assert!(state_region.matches_address(&addr));

        let wrong_state = Region::State {
            country: "US".to_string(),
            state: "NY".to_string(),
        };
        assert!(!wrong_state.matches_address(&addr));
    }

    #[test]
    fn test_geo_routing_nearest() {
        let tenant_id = TenantId::nil();
        let mut rule = GeoRoutingRule::new(
            tenant_id,
            "Nearest office".to_string(),
            GeoRoutingStrategy::Nearest,
        );

        // Add destinations
        let sf = Coordinates::new(37.7749, -122.4194).unwrap();
        let la = Coordinates::new(34.0522, -118.2437).unwrap();

        rule.region_destinations.push(RegionDestination {
            region: Region::City {
                country: "US".to_string(),
                state: "CA".to_string(),
                city: "San Francisco".to_string(),
            },
            line_binding_id: LineBindingId::new(),
            location: Location::from_coordinates(sf),
            enabled: true,
        });

        rule.region_destinations.push(RegionDestination {
            region: Region::City {
                country: "US".to_string(),
                state: "CA".to_string(),
                city: "Los Angeles".to_string(),
            },
            line_binding_id: LineBindingId::new(),
            location: Location::from_coordinates(la),
            enabled: true,
        });

        // Source closer to LA
        let source = Coordinates::new(34.1, -118.3).unwrap();
        let source_location = Location::from_coordinates(source);

        let dest = rule.find_destination(&source_location);
        assert!(dest.is_some());

        // Should route to LA (closer)
        if let Some(destination) = dest {
            assert_eq!(
                destination.region,
                Region::City {
                    country: "US".to_string(),
                    state: "CA".to_string(),
                    city: "Los Angeles".to_string(),
                }
            );
        }
    }

    #[test]
    fn test_geo_routing_manager() {
        let mut manager = GeoRoutingManager::new();
        let tenant_id = TenantId::nil();

        let rule = GeoRoutingRule::new(
            tenant_id,
            "Test rule".to_string(),
            GeoRoutingStrategy::Nearest,
        );

        manager.add_rule(rule);

        let rules = manager.get_rules(&tenant_id);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "Test rule");
    }
}
