//! PBX configuration structures.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level PBX configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PbxConfig {
    /// Master enable/disable for all PBX features.
    #[serde(default)]
    pub enabled: bool,

    /// Feature code (star code) mappings.
    #[serde(default)]
    pub feature_codes: FeatureCodeConfig,

    /// Hunt group definitions keyed by extension number (e.g., "5000").
    #[serde(default)]
    pub hunt_groups: HashMap<String, HuntGroupConfig>,

    /// Call parking configuration.
    #[serde(default)]
    pub parking: ParkingConfig,

    /// Pickup group definitions keyed by group name.
    #[serde(default)]
    pub pickup_groups: HashMap<String, PickupGroupConfig>,

    /// Client Matter Code configuration.
    #[serde(default)]
    pub cmc: CmcConfig,

    /// Forced Authorization Code configuration.
    #[serde(default)]
    pub fac: FacConfig,
}

/// Feature code (star code) prefix mappings.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FeatureCodeConfig {
    /// Group pickup prefix (e.g., "*8").
    #[serde(default = "default_group_pickup")]
    pub group_pickup: String,

    /// Directed pickup prefix (e.g., "*9" followed by extension).
    #[serde(default = "default_directed_pickup")]
    pub directed_pickup: String,

    /// Park call prefix (e.g., "*70").
    #[serde(default = "default_park")]
    pub park: String,

    /// Retrieve parked call prefix (e.g., "*71" followed by orbit).
    #[serde(default = "default_retrieve")]
    pub retrieve: String,

    /// One-way intercom prefix (e.g., "*80" followed by extension).
    #[serde(default = "default_intercom_oneway")]
    pub intercom_oneway: String,

    /// Two-way intercom prefix (e.g., "*81" followed by extension).
    #[serde(default = "default_intercom_twoway")]
    pub intercom_twoway: String,
}

fn default_group_pickup() -> String {
    "*8".to_string()
}
fn default_directed_pickup() -> String {
    "*9".to_string()
}
fn default_park() -> String {
    "*70".to_string()
}
fn default_retrieve() -> String {
    "*71".to_string()
}
fn default_intercom_oneway() -> String {
    "*80".to_string()
}
fn default_intercom_twoway() -> String {
    "*81".to_string()
}

impl Default for FeatureCodeConfig {
    fn default() -> Self {
        Self {
            group_pickup: default_group_pickup(),
            directed_pickup: default_directed_pickup(),
            park: default_park(),
            retrieve: default_retrieve(),
            intercom_oneway: default_intercom_oneway(),
            intercom_twoway: default_intercom_twoway(),
        }
    }
}

/// Ring strategy for hunt groups.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RingStrategy {
    /// Ring all members simultaneously.
    #[default]
    Simultaneous,
    /// Ring members one at a time in order.
    Sequential,
    /// Ring members one at a time, rotating the start position.
    RoundRobin,
    /// Shuffle members then ring simultaneously.
    Random,
}

/// Configuration for a single hunt group.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HuntGroupConfig {
    /// Human-readable name (e.g., "Sales").
    pub name: String,

    /// Ring strategy.
    #[serde(default)]
    pub strategy: RingStrategy,

    /// Member extensions.
    pub members: Vec<String>,

    /// Per-member ring timeout in seconds (sequential/round-robin only).
    #[serde(default = "default_ring_timeout")]
    pub ring_timeout: u32,

    /// Total timeout in seconds before giving up.
    #[serde(default = "default_total_timeout")]
    pub total_timeout: u32,

    /// Destination when no member answers (e.g., "voicemail:5000").
    pub no_answer_destination: Option<String>,

    /// Skip members that are known busy.
    #[serde(default)]
    pub skip_busy: bool,
}

fn default_ring_timeout() -> u32 {
    20
}

fn default_total_timeout() -> u32 {
    30
}

/// Call parking configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParkingConfig {
    /// Enable call parking.
    #[serde(default)]
    pub enabled: bool,

    /// First parking orbit number.
    #[serde(default = "default_orbit_start")]
    pub orbit_start: u16,

    /// Last parking orbit number.
    #[serde(default = "default_orbit_end")]
    pub orbit_end: u16,

    /// Seconds before a parked call times out.
    #[serde(default = "default_parking_timeout")]
    pub timeout_seconds: u64,
}

fn default_orbit_start() -> u16 {
    701
}
fn default_orbit_end() -> u16 {
    720
}
fn default_parking_timeout() -> u64 {
    60
}

impl Default for ParkingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            orbit_start: default_orbit_start(),
            orbit_end: default_orbit_end(),
            timeout_seconds: default_parking_timeout(),
        }
    }
}

/// A pickup group — a set of extensions that can pick up each other's ringing calls.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PickupGroupConfig {
    /// Member extensions in this pickup group.
    pub members: Vec<String>,
}

/// Client Matter Code configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CmcConfig {
    /// Enable CMC collection.
    #[serde(default)]
    pub enabled: bool,

    /// Destination patterns that require a CMC (e.g., `["_1NXXNXXXXXX"]`).
    #[serde(default)]
    pub required_for: Vec<String>,

    /// Optional allowlist of valid CMC codes. If empty, any numeric code is accepted.
    #[serde(default)]
    pub valid_codes: Vec<String>,

    /// Number of CMC digits to collect.
    #[serde(default = "default_cmc_digits")]
    pub digits: u32,
}

fn default_cmc_digits() -> u32 {
    6
}

/// Forced Authorization Code configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FacConfig {
    /// Enable FAC enforcement.
    #[serde(default)]
    pub enabled: bool,

    /// Destination patterns that require a FAC (e.g., `["011*"]`).
    #[serde(default)]
    pub required_for: Vec<String>,

    /// Allowlist of valid FAC codes. At least one must be configured when enabled.
    #[serde(default)]
    pub valid_codes: Vec<String>,

    /// Number of FAC digits to collect.
    #[serde(default = "default_fac_digits")]
    pub digits: u32,
}

fn default_fac_digits() -> u32 {
    4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_pbx_config() {
        let cfg = PbxConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.hunt_groups.is_empty());
        assert!(!cfg.parking.enabled);
        assert!(cfg.pickup_groups.is_empty());
        assert!(!cfg.cmc.enabled);
        assert!(!cfg.fac.enabled);
    }

    #[test]
    fn test_feature_code_defaults() {
        let cfg = FeatureCodeConfig::default();
        assert_eq!(cfg.group_pickup, "*8");
        assert_eq!(cfg.directed_pickup, "*9");
        assert_eq!(cfg.park, "*70");
        assert_eq!(cfg.retrieve, "*71");
        assert_eq!(cfg.intercom_oneway, "*80");
        assert_eq!(cfg.intercom_twoway, "*81");
    }

    #[test]
    fn test_parking_defaults() {
        let cfg = ParkingConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.orbit_start, 701);
        assert_eq!(cfg.orbit_end, 720);
        assert_eq!(cfg.timeout_seconds, 60);
    }

    #[test]
    fn test_ring_strategy_serde() {
        let json = serde_json::to_string(&RingStrategy::RoundRobin).unwrap();
        assert_eq!(json, "\"round_robin\"");
        let deserialized: RingStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, RingStrategy::RoundRobin);
    }

    #[test]
    fn test_toml_roundtrip() {
        let toml_str = r#"
enabled = true

[feature_codes]
group_pickup = "*8"

[parking]
enabled = true
orbit_start = 701
orbit_end = 720
timeout_seconds = 60

[hunt_groups.5000]
name = "Sales"
strategy = "simultaneous"
members = ["1001", "1002", "1003"]
ring_timeout = 20
total_timeout = 30

[pickup_groups.floor1]
members = ["1001", "1002"]

[cmc]
enabled = true
required_for = ["_1NXXNXXXXXX"]
digits = 6

[fac]
enabled = true
required_for = ["011*"]
valid_codes = ["1234", "5678"]
digits = 4
"#;
        let cfg: PbxConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.enabled);
        assert!(cfg.hunt_groups.contains_key("5000"));
        assert_eq!(cfg.hunt_groups["5000"].name, "Sales");
        assert_eq!(cfg.hunt_groups["5000"].members.len(), 3);
        assert!(cfg.parking.enabled);
        assert!(cfg.cmc.enabled);
        assert!(cfg.fac.enabled);
        assert_eq!(cfg.fac.valid_codes.len(), 2);
    }
}
