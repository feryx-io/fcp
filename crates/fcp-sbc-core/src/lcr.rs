//! Least Cost Routing (LCR) configuration and route table.
//!
//! Provides prefix-based destination matching with cost-ordered trunk selection,
//! quality-aware filtering, and multiple load balancing strategies.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ── Configuration ────────────────────────────────────────────────────────────

/// LCR configuration controlling cost-based route selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcrConfig {
    /// Enable LCR routing. When false, TrunkRouter uses legacy priority/weight selection.
    #[serde(default)]
    pub enabled: bool,

    /// Route entries mapping destination prefixes to trunks with costs.
    #[serde(default)]
    pub routes: Vec<LcrRouteEntry>,

    /// Selection strategy when multiple trunks match at equal cost.
    #[serde(default)]
    pub load_balance_strategy: LoadBalanceStrategy,

    /// Minimum ASR (0.0–1.0) for a trunk to be eligible. 0.0 = no minimum.
    #[serde(default)]
    pub min_asr: f64,

    /// Maximum number of failover attempts per call (0 = no failover, just use first).
    #[serde(default = "default_max_failover_attempts")]
    pub max_failover_attempts: u32,

    /// Quality metrics sliding window in seconds.
    #[serde(default = "default_quality_window_secs")]
    pub quality_window_secs: u64,
}

impl Default for LcrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            routes: Vec::new(),
            load_balance_strategy: LoadBalanceStrategy::default(),
            min_asr: 0.0,
            max_failover_attempts: default_max_failover_attempts(),
            quality_window_secs: default_quality_window_secs(),
        }
    }
}

impl LcrConfig {
    /// Validate the LCR configuration.
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0..=1.0).contains(&self.min_asr) {
            return Err(format!(
                "lcr.min_asr must be between 0.0 and 1.0, got {}",
                self.min_asr
            ));
        }
        if self.max_failover_attempts > 10 {
            return Err(format!(
                "lcr.max_failover_attempts must be <= 10, got {}",
                self.max_failover_attempts
            ));
        }
        if !(60..=86400).contains(&self.quality_window_secs) {
            return Err(format!(
                "lcr.quality_window_secs must be between 60 and 86400, got {}",
                self.quality_window_secs
            ));
        }

        let mut seen = HashSet::new();
        for (i, route) in self.routes.iter().enumerate() {
            route
                .validate()
                .map_err(|e| format!("lcr.routes[{i}]: {e}"))?;

            let key = (route.prefix.as_str(), route.trunk_name.as_str());
            if !seen.insert(key) {
                return Err(format!(
                    "lcr.routes: duplicate (prefix, trunk_name) pair: ({:?}, {:?})",
                    route.prefix, route.trunk_name
                ));
            }
        }

        Ok(())
    }

    /// Validate that all route trunk_names reference trunks that exist.
    pub fn validate_trunk_refs(&self, trunk_names: &[String]) -> Result<(), String> {
        for route in &self.routes {
            if !trunk_names.iter().any(|n| n == &route.trunk_name) {
                return Err(format!(
                    "lcr.routes: trunk_name {:?} in prefix {:?} does not reference a configured trunk",
                    route.trunk_name, route.prefix
                ));
            }
        }
        Ok(())
    }
}

/// A single LCR route entry mapping a destination prefix to a trunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcrRouteEntry {
    /// Destination prefix (e.g. "+1", "+44", "+1212"). Longest match wins.
    pub prefix: String,

    /// Trunk name (must reference a trunk in SbcConfig.trunks).
    pub trunk_name: String,

    /// Cost per minute (arbitrary units — used for relative ordering only).
    pub cost_per_minute: f64,

    /// Optional priority override (lower = preferred). When set, takes
    /// precedence over cost within same-prefix matches. Default: 100.
    #[serde(default = "default_route_priority")]
    pub priority: u32,

    /// Whether this route entry is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl LcrRouteEntry {
    fn validate(&self) -> Result<(), String> {
        if self.prefix.is_empty() {
            return Err("prefix must not be empty".into());
        }
        if self.prefix.contains('\r') || self.prefix.contains('\n') {
            return Err("prefix must not contain CR/LF characters".into());
        }
        // Prefix must contain only E.164 characters: digits, +, *, #
        if !self
            .prefix
            .chars()
            .all(|c| c.is_ascii_digit() || c == '+' || c == '*' || c == '#')
        {
            return Err(format!(
                "prefix {:?} contains invalid characters (allowed: digits, +, *, #)",
                self.prefix
            ));
        }
        if self.cost_per_minute < 0.0 {
            return Err(format!(
                "cost_per_minute must be >= 0.0, got {}",
                self.cost_per_minute
            ));
        }
        if self.priority > 1000 {
            return Err(format!("priority must be <= 1000, got {}", self.priority));
        }
        if self.trunk_name.is_empty() {
            return Err("trunk_name must not be empty".into());
        }
        Ok(())
    }
}

/// Load balancing strategy for selecting among equal-cost trunks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadBalanceStrategy {
    /// Select cheapest trunk (break ties by priority, then weight).
    #[default]
    LeastCost,
    /// Round-robin among equal-cost candidates.
    RoundRobin,
    /// Weighted random among equal-cost candidates (uses trunk weight).
    WeightedRandom,
    /// Route to trunk with fewest concurrent calls.
    LeastConnections,
}

fn default_max_failover_attempts() -> u32 {
    2
}
fn default_quality_window_secs() -> u64 {
    3600
}
fn default_route_priority() -> u32 {
    100
}
fn default_true() -> bool {
    true
}

// ── Route Table ──────────────────────────────────────────────────────────────

/// A compiled route entry for fast lookup.
#[derive(Debug, Clone)]
struct CompiledRouteEntry {
    prefix: String,
    trunk_name: String,
    cost_per_minute: f64,
    priority: u32,
}

/// Compiled LCR route table for fast prefix lookup.
pub struct LcrRouteTable {
    /// Routes sorted by prefix length descending (longest match first),
    /// then by cost ascending, then by priority ascending.
    entries: Vec<CompiledRouteEntry>,
}

impl LcrRouteTable {
    /// Build a route table from config entries. Filters to enabled entries only.
    pub fn new(routes: &[LcrRouteEntry]) -> Self {
        let mut entries: Vec<CompiledRouteEntry> = routes
            .iter()
            .filter(|r| r.enabled)
            .map(|r| CompiledRouteEntry {
                prefix: r.prefix.clone(),
                trunk_name: r.trunk_name.clone(),
                cost_per_minute: r.cost_per_minute,
                priority: r.priority,
            })
            .collect();

        // Sort by prefix length desc, then cost asc, then priority asc
        entries.sort_by(|a, b| {
            b.prefix
                .len()
                .cmp(&a.prefix.len())
                .then(
                    a.cost_per_minute
                        .partial_cmp(&b.cost_per_minute)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.priority.cmp(&b.priority))
        });

        Self { entries }
    }

    /// Extract the digit portion from a SIP URI or raw number for prefix matching.
    ///
    /// Examples:
    /// - `sip:+12125551234@example.com` → `+12125551234`
    /// - `+12125551234` → `+12125551234`
    /// - `12125551234` → `12125551234`
    fn extract_digits(destination: &str) -> &str {
        let stripped = destination
            .strip_prefix("sip:")
            .or_else(|| destination.strip_prefix("sips:"))
            .or_else(|| destination.strip_prefix("tel:"))
            .unwrap_or(destination);

        // Take user part (before @)
        stripped.split('@').next().unwrap_or(stripped)
    }

    /// Find all routes matching the longest prefix for `destination`.
    ///
    /// Returns entries ordered by cost ascending, then priority ascending.
    /// Only the longest matching prefix is considered.
    pub fn find_routes(&self, destination: &str) -> Vec<MatchedRoute<'_>> {
        let user = Self::extract_digits(destination);

        // Find the longest matching prefix length
        let mut best_prefix_len = 0;
        for entry in &self.entries {
            if user.starts_with(&entry.prefix) && entry.prefix.len() > best_prefix_len {
                best_prefix_len = entry.prefix.len();
            }
        }

        if best_prefix_len == 0 {
            return Vec::new();
        }

        // Return all entries matching the longest prefix
        self.entries
            .iter()
            .filter(|e| e.prefix.len() == best_prefix_len && user.starts_with(&e.prefix))
            .map(|e| MatchedRoute {
                trunk_name: &e.trunk_name,
                cost_per_minute: e.cost_per_minute,
                priority: e.priority,
                prefix_len: e.prefix.len(),
            })
            .collect()
    }

    /// Find routes with fallback tiers: longest-prefix matches first,
    /// then next-longest prefix matches, etc.
    ///
    /// Returns entries ordered by: prefix length desc → cost asc → priority asc.
    pub fn find_routes_ordered(&self, destination: &str) -> Vec<MatchedRoute<'_>> {
        let user = Self::extract_digits(destination);

        // Collect all matching entries (already sorted by prefix len desc, cost asc, priority asc)
        self.entries
            .iter()
            .filter(|e| user.starts_with(&e.prefix))
            .map(|e| MatchedRoute {
                trunk_name: &e.trunk_name,
                cost_per_minute: e.cost_per_minute,
                priority: e.priority,
                prefix_len: e.prefix.len(),
            })
            .collect()
    }
}

/// A route match result from the LCR table.
#[derive(Debug, Clone)]
pub struct MatchedRoute<'a> {
    pub trunk_name: &'a str,
    pub cost_per_minute: f64,
    pub priority: u32,
    pub prefix_len: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── LcrConfig tests ──────────────────────────────────────────────────

    #[test]
    fn test_lcr_config_defaults() {
        let cfg = LcrConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.routes.is_empty());
        assert_eq!(cfg.load_balance_strategy, LoadBalanceStrategy::LeastCost);
        assert_eq!(cfg.min_asr, 0.0);
        assert_eq!(cfg.max_failover_attempts, 2);
        assert_eq!(cfg.quality_window_secs, 3600);
    }

    #[test]
    fn test_lcr_config_validation_valid() {
        let cfg = LcrConfig {
            enabled: true,
            routes: vec![LcrRouteEntry {
                prefix: "+1".into(),
                trunk_name: "carrier-a".into(),
                cost_per_minute: 0.01,
                priority: 100,
                enabled: true,
            }],
            min_asr: 0.5,
            max_failover_attempts: 3,
            quality_window_secs: 1800,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_lcr_config_validation_min_asr_out_of_range() {
        let cfg = LcrConfig {
            min_asr: 1.5,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("min_asr"));
    }

    #[test]
    fn test_lcr_config_validation_min_asr_negative() {
        let cfg = LcrConfig {
            min_asr: -0.1,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("min_asr"));
    }

    #[test]
    fn test_lcr_config_validation_max_failover() {
        let cfg = LcrConfig {
            max_failover_attempts: 11,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_failover_attempts"));
    }

    #[test]
    fn test_lcr_config_validation_quality_window_too_small() {
        let cfg = LcrConfig {
            quality_window_secs: 10,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("quality_window_secs"));
    }

    #[test]
    fn test_lcr_config_validation_quality_window_too_large() {
        let cfg = LcrConfig {
            quality_window_secs: 100_000,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("quality_window_secs"));
    }

    #[test]
    fn test_lcr_route_entry_empty_prefix() {
        let cfg = LcrConfig {
            routes: vec![LcrRouteEntry {
                prefix: "".into(),
                trunk_name: "carrier-a".into(),
                cost_per_minute: 0.01,
                priority: 100,
                enabled: true,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("prefix must not be empty"));
    }

    #[test]
    fn test_lcr_route_entry_invalid_prefix_chars() {
        let cfg = LcrConfig {
            routes: vec![LcrRouteEntry {
                prefix: "+1abc".into(),
                trunk_name: "carrier-a".into(),
                cost_per_minute: 0.01,
                priority: 100,
                enabled: true,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("invalid characters"));
    }

    #[test]
    fn test_lcr_route_entry_negative_cost() {
        let cfg = LcrConfig {
            routes: vec![LcrRouteEntry {
                prefix: "+1".into(),
                trunk_name: "carrier-a".into(),
                cost_per_minute: -0.5,
                priority: 100,
                enabled: true,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("cost_per_minute"));
    }

    #[test]
    fn test_lcr_route_entry_priority_too_high() {
        let cfg = LcrConfig {
            routes: vec![LcrRouteEntry {
                prefix: "+1".into(),
                trunk_name: "carrier-a".into(),
                cost_per_minute: 0.01,
                priority: 1001,
                enabled: true,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("priority"));
    }

    #[test]
    fn test_lcr_duplicate_prefix_trunk() {
        let cfg = LcrConfig {
            routes: vec![
                LcrRouteEntry {
                    prefix: "+1".into(),
                    trunk_name: "carrier-a".into(),
                    cost_per_minute: 0.01,
                    priority: 100,
                    enabled: true,
                },
                LcrRouteEntry {
                    prefix: "+1".into(),
                    trunk_name: "carrier-a".into(),
                    cost_per_minute: 0.02,
                    priority: 200,
                    enabled: true,
                },
            ],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn test_lcr_route_entry_serde_roundtrip() {
        let entry = LcrRouteEntry {
            prefix: "+1212".into(),
            trunk_name: "carrier-a".into(),
            cost_per_minute: 0.015,
            priority: 50,
            enabled: true,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: LcrRouteEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, deserialized);
    }

    #[test]
    fn test_lcr_config_serde_roundtrip() {
        let cfg = LcrConfig {
            enabled: true,
            routes: vec![
                LcrRouteEntry {
                    prefix: "+1".into(),
                    trunk_name: "carrier-a".into(),
                    cost_per_minute: 0.01,
                    priority: 100,
                    enabled: true,
                },
                LcrRouteEntry {
                    prefix: "+44".into(),
                    trunk_name: "carrier-b".into(),
                    cost_per_minute: 0.05,
                    priority: 200,
                    enabled: true,
                },
            ],
            load_balance_strategy: LoadBalanceStrategy::RoundRobin,
            min_asr: 0.85,
            max_failover_attempts: 3,
            quality_window_secs: 1800,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: LcrConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_lcr_config_deserialize_defaults() {
        let json = r#"{"enabled": true}"#;
        let cfg: LcrConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.enabled);
        assert!(cfg.routes.is_empty());
        assert_eq!(cfg.max_failover_attempts, 2);
        assert_eq!(cfg.quality_window_secs, 3600);
    }

    #[test]
    fn test_lcr_validate_trunk_refs() {
        let cfg = LcrConfig {
            routes: vec![LcrRouteEntry {
                prefix: "+1".into(),
                trunk_name: "nonexistent".into(),
                cost_per_minute: 0.01,
                priority: 100,
                enabled: true,
            }],
            ..Default::default()
        };
        let err = cfg
            .validate_trunk_refs(&["carrier-a".into(), "carrier-b".into()])
            .unwrap_err();
        assert!(err.contains("nonexistent"));
    }

    #[test]
    fn test_lcr_validate_trunk_refs_ok() {
        let cfg = LcrConfig {
            routes: vec![LcrRouteEntry {
                prefix: "+1".into(),
                trunk_name: "carrier-a".into(),
                cost_per_minute: 0.01,
                priority: 100,
                enabled: true,
            }],
            ..Default::default()
        };
        assert!(cfg
            .validate_trunk_refs(&["carrier-a".into(), "carrier-b".into()])
            .is_ok());
    }

    // ── Route Table tests ────────────────────────────────────────────────

    fn test_routes() -> Vec<LcrRouteEntry> {
        vec![
            LcrRouteEntry {
                prefix: "+1".into(),
                trunk_name: "us-cheap".into(),
                cost_per_minute: 0.01,
                priority: 100,
                enabled: true,
            },
            LcrRouteEntry {
                prefix: "+1".into(),
                trunk_name: "us-premium".into(),
                cost_per_minute: 0.05,
                priority: 100,
                enabled: true,
            },
            LcrRouteEntry {
                prefix: "+1212".into(),
                trunk_name: "nyc-local".into(),
                cost_per_minute: 0.005,
                priority: 100,
                enabled: true,
            },
            LcrRouteEntry {
                prefix: "+44".into(),
                trunk_name: "uk-carrier".into(),
                cost_per_minute: 0.08,
                priority: 100,
                enabled: true,
            },
            LcrRouteEntry {
                prefix: "+44".into(),
                trunk_name: "uk-disabled".into(),
                cost_per_minute: 0.02,
                priority: 100,
                enabled: false,
            },
        ]
    }

    #[test]
    fn test_lcr_route_table_longest_prefix() {
        let table = LcrRouteTable::new(&test_routes());
        let routes = table.find_routes("sip:+12125551234@example.com");

        // +1212 is longer match than +1
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].trunk_name, "nyc-local");
    }

    #[test]
    fn test_lcr_route_table_cost_ordering() {
        let table = LcrRouteTable::new(&test_routes());
        let routes = table.find_routes("sip:+15551234567@example.com");

        // +1 matches, two trunks: us-cheap (0.01) before us-premium (0.05)
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].trunk_name, "us-cheap");
        assert_eq!(routes[1].trunk_name, "us-premium");
    }

    #[test]
    fn test_lcr_route_table_no_match() {
        let table = LcrRouteTable::new(&test_routes());
        let routes = table.find_routes("sip:+8612345@example.com");
        assert!(routes.is_empty());
    }

    #[test]
    fn test_lcr_route_table_sip_uri() {
        let table = LcrRouteTable::new(&test_routes());
        let routes = table.find_routes("sip:+441234567890@sip.carrier.com");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].trunk_name, "uk-carrier");
    }

    #[test]
    fn test_lcr_route_table_raw_number() {
        let table = LcrRouteTable::new(&test_routes());
        let routes = table.find_routes("+441234567890");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].trunk_name, "uk-carrier");
    }

    #[test]
    fn test_lcr_route_table_disabled_excluded() {
        let table = LcrRouteTable::new(&test_routes());
        // uk-disabled has cost 0.02 (cheaper than uk-carrier 0.08) but is disabled
        let routes = table.find_routes("+441234567890");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].trunk_name, "uk-carrier");
    }

    #[test]
    fn test_lcr_route_table_ordered_with_fallback() {
        let table = LcrRouteTable::new(&test_routes());
        let routes = table.find_routes_ordered("sip:+12125551234@example.com");

        // Should get: nyc-local (+1212), then us-cheap (+1), then us-premium (+1)
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].trunk_name, "nyc-local");
        assert_eq!(routes[0].prefix_len, 5); // "+1212"
        assert_eq!(routes[1].trunk_name, "us-cheap");
        assert_eq!(routes[1].prefix_len, 2); // "+1"
        assert_eq!(routes[2].trunk_name, "us-premium");
        assert_eq!(routes[2].prefix_len, 2); // "+1"
    }

    #[test]
    fn test_lcr_route_table_tel_uri() {
        let table = LcrRouteTable::new(&test_routes());
        let routes = table.find_routes("tel:+12125551234");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].trunk_name, "nyc-local");
    }

    #[test]
    fn test_lcr_route_table_empty() {
        let table = LcrRouteTable::new(&[]);
        let routes = table.find_routes("+12125551234");
        assert!(routes.is_empty());
    }

    #[test]
    fn test_load_balance_strategy_serde() {
        let strategies = vec![
            (LoadBalanceStrategy::LeastCost, "\"LeastCost\""),
            (LoadBalanceStrategy::RoundRobin, "\"RoundRobin\""),
            (LoadBalanceStrategy::WeightedRandom, "\"WeightedRandom\""),
            (
                LoadBalanceStrategy::LeastConnections,
                "\"LeastConnections\"",
            ),
        ];
        for (strategy, expected) in strategies {
            let json = serde_json::to_string(&strategy).unwrap();
            assert_eq!(json, expected);
            let deserialized: LoadBalanceStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(strategy, deserialized);
        }
    }
}
