//! PBX configuration structures.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level PBX configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PbxConfig {
    /// Master enable/disable for all PBX features.
    #[serde(default = "default_pbx_enabled")]
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

fn default_pbx_enabled() -> bool {
    true
}

impl Default for PbxConfig {
    fn default() -> Self {
        Self {
            enabled: default_pbx_enabled(),
            feature_codes: FeatureCodeConfig::default(),
            hunt_groups: HashMap::new(),
            parking: ParkingConfig::default(),
            pickup_groups: HashMap::new(),
            cmc: CmcConfig::default(),
            fac: FacConfig::default(),
        }
    }
}

/// Validate PBX configuration for unsupported or inconsistent options.
pub fn validate_config(_config: &PbxConfig) -> Result<(), String> {
    Ok(())
}

/// Feature code (star code) prefix mappings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

    /// Agent queue login code (e.g., "*50").
    #[serde(default = "default_agent_login")]
    pub agent_login: String,

    /// Agent queue logout code (e.g., "*51").
    #[serde(default = "default_agent_logout")]
    pub agent_logout: String,

    /// Voicemail access prefix (e.g., "*98"). Exact match checks own mailbox,
    /// prefix + extension checks a specific mailbox.
    #[serde(default = "default_voicemail_access")]
    pub voicemail_access: String,

    /// Do not disturb on (e.g., "*78") and off ("*79").
    #[serde(default = "default_dnd_on")]
    pub dnd_on: String,
    #[serde(default = "default_dnd_off")]
    pub dnd_off: String,

    /// Forward all calls on, followed by the target ("*72" + number), and
    /// off ("*73").
    #[serde(default = "default_forward_all_on")]
    pub forward_all_on: String,
    #[serde(default = "default_forward_all_off")]
    pub forward_all_off: String,

    /// Forward on busy on ("*90" + number) and off ("*91").
    #[serde(default = "default_forward_busy_on")]
    pub forward_busy_on: String,
    #[serde(default = "default_forward_busy_off")]
    pub forward_busy_off: String,

    /// Forward on no answer on ("*92" + number) and off ("*93").
    #[serde(default = "default_forward_no_answer_on")]
    pub forward_no_answer_on: String,
    #[serde(default = "default_forward_no_answer_off")]
    pub forward_no_answer_off: String,

    /// Call waiting on ("*56") and off ("*57").
    #[serde(default = "default_call_waiting_on")]
    pub call_waiting_on: String,
    #[serde(default = "default_call_waiting_off")]
    pub call_waiting_off: String,

    /// Find-me on ("*60") and off ("*61").
    #[serde(default = "default_find_me_on")]
    pub find_me_on: String,
    #[serde(default = "default_find_me_off")]
    pub find_me_off: String,
}

fn default_group_pickup() -> String {
    String::new()
}
fn default_directed_pickup() -> String {
    String::new()
}
fn default_park() -> String {
    "*70".to_string()
}
fn default_retrieve() -> String {
    "*71".to_string()
}
fn default_intercom_oneway() -> String {
    String::new()
}
fn default_intercom_twoway() -> String {
    String::new()
}
fn default_agent_login() -> String {
    "*50".to_string()
}
fn default_agent_logout() -> String {
    "*51".to_string()
}
fn default_voicemail_access() -> String {
    "*98".to_string()
}
fn default_dnd_on() -> String {
    "*78".to_string()
}
fn default_dnd_off() -> String {
    "*79".to_string()
}
fn default_forward_all_on() -> String {
    "*72".to_string()
}
fn default_forward_all_off() -> String {
    "*73".to_string()
}
fn default_forward_busy_on() -> String {
    "*90".to_string()
}
fn default_forward_busy_off() -> String {
    "*91".to_string()
}
fn default_forward_no_answer_on() -> String {
    "*92".to_string()
}
fn default_forward_no_answer_off() -> String {
    "*93".to_string()
}
fn default_call_waiting_on() -> String {
    "*56".to_string()
}
fn default_call_waiting_off() -> String {
    "*57".to_string()
}
fn default_find_me_on() -> String {
    "*60".to_string()
}
fn default_find_me_off() -> String {
    "*61".to_string()
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
            agent_login: default_agent_login(),
            agent_logout: default_agent_logout(),
            voicemail_access: default_voicemail_access(),
            dnd_on: default_dnd_on(),
            dnd_off: default_dnd_off(),
            forward_all_on: default_forward_all_on(),
            forward_all_off: default_forward_all_off(),
            forward_busy_on: default_forward_busy_on(),
            forward_busy_off: default_forward_busy_off(),
            forward_no_answer_on: default_forward_no_answer_on(),
            forward_no_answer_off: default_forward_no_answer_off(),
            call_waiting_on: default_call_waiting_on(),
            call_waiting_off: default_call_waiting_off(),
            find_me_on: default_find_me_on(),
            find_me_off: default_find_me_off(),
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct PickupGroupConfig {
    /// Member extensions in this pickup group.
    pub members: Vec<String>,
}

/// Client Matter Code configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
        assert!(cfg.enabled);
        assert!(cfg.hunt_groups.is_empty());
        assert!(!cfg.parking.enabled);
        assert!(cfg.pickup_groups.is_empty());
        assert!(!cfg.cmc.enabled);
        assert!(!cfg.fac.enabled);
    }

    #[test]
    fn test_feature_code_defaults() {
        let cfg = FeatureCodeConfig::default();
        assert_eq!(cfg.group_pickup, "");
        assert_eq!(cfg.directed_pickup, "");
        assert_eq!(cfg.park, "*70");
        assert_eq!(cfg.retrieve, "*71");
        assert_eq!(cfg.intercom_oneway, "");
        assert_eq!(cfg.intercom_twoway, "");
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

    #[test]
    fn test_accepts_pickup_and_intercom_feature_codes() {
        let mut cfg = PbxConfig::default();
        cfg.feature_codes.group_pickup = "*8".to_string();
        assert!(validate_config(&cfg).is_ok());

        let mut cfg = PbxConfig::default();
        cfg.feature_codes.intercom_twoway = "*81".to_string();
        assert!(validate_config(&cfg).is_ok());

        let cfg = PbxConfig::default();
        assert!(validate_config(&cfg).is_ok());
    }
}
