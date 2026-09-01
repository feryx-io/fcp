//! Database-backed routing rules for FCP.
//!
//! Combines call policy (allow/deny) with trunk routing (route_to_trunk).
//! Rules are evaluated in priority order (lower = higher priority),
//! first match wins.

use crate::identifiers::TenantId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::SystemTime;
use uuid::Uuid;

/// Unique identifier for a routing rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoutingRuleId(String);

impl Default for RoutingRuleId {
    fn default() -> Self {
        Self::new()
    }
}

impl RoutingRuleId {
    pub fn new() -> Self {
        Self(format!("rule-{}", Uuid::new_v4()))
    }

    pub fn from_string(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for RoutingRuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for RoutingRuleId {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

/// Action to take when a routing rule matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingAction {
    /// Allow the call (proceed with normal routing)
    Allow,
    /// Deny the call (respond with 403)
    Deny,
    /// Route to a specific trunk or trunk group
    RouteToTrunk,
}

impl fmt::Display for RoutingAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allow => write!(f, "allow"),
            Self::Deny => write!(f, "deny"),
            Self::RouteToTrunk => write!(f, "route_to_trunk"),
        }
    }
}

impl std::str::FromStr for RoutingAction {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            "route_to_trunk" => Ok(Self::RouteToTrunk),
            _ => Err(format!("Invalid routing action: {}", s)),
        }
    }
}

/// A routing rule for call policy and trunk selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    pub id: RoutingRuleId,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: Option<String>,
    pub priority: i32,
    pub enabled: bool,

    // Match conditions
    /// Regex pattern to match against the dialed number/URI
    pub destination_pattern: Option<String>,
    /// Regex pattern to match against the caller number/URI
    pub source_pattern: Option<String>,
    /// CIDR ranges to match source IP
    pub source_ip_ranges: Vec<String>,
    /// Days of week this rule applies (empty = all days)
    pub time_window_days: Vec<String>,
    /// Start time (HH:MM, 24h format, empty = no time restriction)
    pub time_window_start: Option<String>,
    /// End time (HH:MM, 24h format)
    pub time_window_end: Option<String>,
    /// Timezone for time window
    pub time_window_timezone: String,

    // Action
    /// What to do when this rule matches
    pub action: RoutingAction,
    /// Target trunk ID (for route_to_trunk action)
    pub target_trunk_id: Option<String>,
    /// Target trunk group name (for route_to_trunk with failover)
    pub target_trunk_group: Option<String>,

    /// Human-readable reason for this rule
    pub reason: Option<String>,

    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl RoutingRule {
    pub fn new(name: String, tenant_id: TenantId) -> Self {
        let now = SystemTime::now();
        Self {
            id: RoutingRuleId::new(),
            tenant_id,
            name,
            description: None,
            priority: 100,
            enabled: true,
            destination_pattern: None,
            source_pattern: None,
            source_ip_ranges: vec![],
            time_window_days: vec![],
            time_window_start: None,
            time_window_end: None,
            time_window_timezone: "UTC".to_string(),
            action: RoutingAction::Allow,
            target_trunk_id: None,
            target_trunk_group: None,
            reason: None,
            created_at: now,
            updated_at: now,
        }
    }
}
