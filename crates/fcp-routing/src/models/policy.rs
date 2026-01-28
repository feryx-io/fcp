//! Policy Layer - IS IT ALLOWED?
//!
//! This module defines policy rules for call authorization decisions.

use super::identity::IdentityContext;
use chrono::{NaiveTime, Weekday};
use fcp_core::TenantId;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// A policy rule for call authorization.
///
/// Rules are evaluated in priority order (lower number = higher priority).
/// The first matching rule determines the outcome (Allow or Deny).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyRule {
    /// Rule name (for logging and debugging)
    pub name: String,

    /// Tenant this rule belongs to
    pub tenant_id: TenantId,

    /// Priority (lower number = higher priority)
    /// Rules are sorted by priority before evaluation
    pub priority: i32,

    /// Whether this rule is enabled
    pub enabled: bool,

    /// Who this rule applies to (users, roles, groups)
    pub applies_to: AppliesTo,

    /// Destination pattern matching (optional)
    /// If None, matches all destinations
    pub destination_match: Option<DestinationMatch>,

    /// Time windows when this rule is active (optional)
    /// If empty, rule is active 24/7
    pub time_windows: Vec<TimeWindow>,

    /// Source IP ranges this rule applies to (optional)
    /// If empty, matches all source IPs
    pub source_ip_ranges: Vec<IpRange>,

    /// Action to take if rule matches
    pub action: PolicyAction,

    /// Human-readable reason for this policy
    pub reason: Option<String>,
}

impl PolicyRule {
    /// Validates policy rule for security and logical consistency.
    ///
    /// # Security Checks
    /// - Prevents CRLF injection in name and reason fields
    /// - Ensures rule name is not empty
    /// - Ensures priority is non-negative
    /// - Recursively validates all child components:
    ///   * applies_to (users, roles, groups)
    ///   * destination_match (patterns, tags)
    ///   * time_windows (days, times, timezones)
    ///   * source_ip_ranges (CIDR validation)
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("policy.name must not be empty".to_string());
        }
        reject_crlf("policy.name", &self.name)?;
        if self.priority < 0 {
            return Err("policy.priority must be non-negative".to_string());
        }
        self.applies_to.validate()?;
        if let Some(dest) = &self.destination_match {
            dest.validate()?;
        }
        for window in &self.time_windows {
            window.validate()?;
        }
        for range in &self.source_ip_ranges {
            range.validate()?;
        }
        if let Some(reason) = &self.reason {
            reject_crlf("policy.reason", reason)?;
        }
        Ok(())
    }
}

/// Defines who a policy rule applies to.
///
/// Conditions within the same type use OR logic.
/// Example: `users: ["alice", "bob"]` matches if user is alice OR bob
///
/// If `all_users: true`, the rule matches any authenticated caller regardless
/// of roles or groups. This is useful for broad policies like "allow all internal calls".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliesTo {
    /// If true, this rule applies to all authenticated users.
    /// When set, users/roles/groups are ignored.
    #[serde(default)]
    pub all_users: bool,

    /// Specific user IDs (e.g., ["alice@company.com", "bob@company.com"])
    pub users: Option<Vec<String>>,

    /// Role names (e.g., ["employee", "sales-team"])
    pub roles: Option<Vec<String>>,

    /// Group names (e.g., ["us-west", "management"])
    pub groups: Option<Vec<String>>,
}

impl AppliesTo {
    /// Check if this rule applies to the given identity context.
    ///
    /// Returns true if:
    /// - `all_users` is true, OR
    /// - ANY condition matches (OR logic within each type)
    pub fn matches(&self, ctx: &IdentityContext) -> bool {
        // If all_users is set, always match
        if self.all_users {
            return true;
        }

        // If no conditions specified, matches nothing
        if self.users.is_none() && self.roles.is_none() && self.groups.is_none() {
            return false;
        }

        // User match
        if let Some(users) = &self.users {
            if users.contains(&ctx.user_id) {
                return true;
            }
        }

        // Role match (any role in the list)
        if let Some(roles) = &self.roles {
            if roles.iter().any(|r| ctx.roles.contains(r)) {
                return true;
            }
        }

        // Group match (any group in the list)
        if let Some(groups) = &self.groups {
            if groups.iter().any(|g| ctx.groups.contains(g)) {
                return true;
            }
        }

        false
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(users) = &self.users {
            for user in users {
                reject_crlf("policy.applies_to.users", user)?;
            }
        }
        if let Some(roles) = &self.roles {
            for role in roles {
                reject_crlf("policy.applies_to.roles", role)?;
            }
        }
        if let Some(groups) = &self.groups {
            for group in groups {
                reject_crlf("policy.applies_to.groups", group)?;
            }
        }
        Ok(())
    }
}

impl Default for AppliesTo {
    fn default() -> Self {
        Self {
            all_users: false,
            users: None,
            roles: None,
            groups: None,
        }
    }
}

/// Destination pattern matching.
///
/// Used to match the destination (called party) against patterns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DestinationMatch {
    /// Regex pattern for destination matching
    /// Example: "^[2-5]\\d{3}$" matches 4-digit extensions 2000-5999
    pub pattern: String,

    /// Destination tags (for future Reachability layer)
    /// Example: ["internal", "free"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

impl DestinationMatch {
    /// Validates destination match for security and correctness.
    ///
    /// # Security Checks
    /// - Prevents CRLF injection in pattern and tags
    /// - Ensures pattern is not empty
    /// - **ReDoS Prevention**: Compiles regex at validation time to catch invalid
    ///   or malicious patterns before runtime evaluation
    pub fn validate(&self) -> Result<(), String> {
        if self.pattern.is_empty() {
            return Err("policy.destination_match.pattern must not be empty".to_string());
        }
        reject_crlf("policy.destination_match.pattern", &self.pattern)?;
        if let Some(tags) = &self.tags {
            for tag in tags {
                reject_crlf("policy.destination_match.tags", tag)?;
            }
        }
        // Regex compilation check - prevents ReDoS attacks
        regex::Regex::new(&self.pattern).map_err(|e| format!("Invalid destination regex: {e}"))?;
        Ok(())
    }
}

/// Time window for policy activation.
///
/// Defines when a policy rule is active based on day of week and time of day.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeWindow {
    /// Days of week when this window is active
    pub days: Vec<Weekday>,

    /// Start time (24-hour format)
    #[serde(with = "time_format")]
    pub start_time: NaiveTime,

    /// End time (24-hour format)
    #[serde(with = "time_format")]
    pub end_time: NaiveTime,

    /// Timezone for time evaluation
    /// Example: "America/New_York", "Europe/London"
    pub timezone: String,
}

impl TimeWindow {
    /// Validates time window for security and logical consistency.
    ///
    /// # Security Checks
    /// - Prevents CRLF injection in timezone field
    /// - Validates timezone is a recognized IANA timezone
    /// - Ensures days list is not empty
    /// - Ensures start_time is before end_time
    pub fn validate(&self) -> Result<(), String> {
        if self.days.is_empty() {
            return Err("policy.time_windows.days must not be empty".to_string());
        }
        if self.start_time >= self.end_time {
            return Err("policy.time_windows.start_time must be before end_time".to_string());
        }
        reject_crlf("policy.time_windows.timezone", &self.timezone)?;

        // Validate timezone is a recognized IANA timezone
        use std::str::FromStr;
        chrono_tz::Tz::from_str(&self.timezone).map_err(|_| {
            format!(
                "Invalid timezone '{}'. Must be a valid IANA timezone (e.g., 'America/New_York', 'UTC')",
                self.timezone
            )
        })?;

        Ok(())
    }
}

/// Custom serialization for NaiveTime
mod time_format {
    use chrono::NaiveTime;
    use serde::{self, Deserialize, Deserializer, Serializer};

    const FORMAT: &str = "%H:%M:%S";

    pub fn serialize<S>(time: &NaiveTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = format!("{}", time.format(FORMAT));
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<NaiveTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        NaiveTime::parse_from_str(&s, FORMAT).map_err(serde::de::Error::custom)
    }
}

/// IP address range in CIDR notation.
///
/// Used for source IP-based policy matching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpRange {
    /// CIDR notation (e.g., "10.0.0.0/8", "192.168.1.0/24")
    pub cidr: String,
}

impl IpRange {
    /// Validates IP range for security and correctness.
    ///
    /// # Security Checks
    /// - Prevents CRLF injection in CIDR field
    /// - Ensures CIDR is not empty
    /// - Validates CIDR notation is parseable (prevents runtime errors)
    pub fn validate(&self) -> Result<(), String> {
        if self.cidr.is_empty() {
            return Err("policy.source_ip_ranges.cidr must not be empty".to_string());
        }
        reject_crlf("policy.source_ip_ranges.cidr", &self.cidr)?;
        ipnetwork::IpNetwork::from_str(&self.cidr)
            .map_err(|e| format!("Invalid CIDR '{}': {}", self.cidr, e))?;
        Ok(())
    }
}

/// Action to take when a policy rule matches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum PolicyAction {
    /// Allow the call to proceed
    Allow,

    /// Deny the call (403 Forbidden)
    Deny,
}

impl PolicyAction {
    /// Check if this action allows the call
    pub fn is_allow(&self) -> bool {
        matches!(self, PolicyAction::Allow)
    }

    /// Check if this action denies the call
    pub fn is_deny(&self) -> bool {
        matches!(self, PolicyAction::Deny)
    }
}

/// Result of policy evaluation.
///
/// Contains the decision (Allow/Deny), which rule matched, and evaluation metrics.
#[derive(Debug, Clone)]
pub struct PolicyDecision {
    /// Action to take (Allow or Deny)
    pub action: PolicyAction,

    /// Name of the rule that matched (if any)
    pub matched_rule: Option<String>,

    /// Human-readable reason for the decision
    pub reason: String,

    /// Time taken to evaluate the policy (microseconds)
    pub evaluation_time_us: u64,
}

impl PolicyDecision {
    /// Create an Allow decision
    pub fn allow(reason: impl Into<String>, rule: Option<String>, time_us: u64) -> Self {
        Self {
            action: PolicyAction::Allow,
            matched_rule: rule,
            reason: reason.into(),
            evaluation_time_us: time_us,
        }
    }

    /// Create a Deny decision
    pub fn deny(reason: impl Into<String>, rule: Option<String>, time_us: u64) -> Self {
        Self {
            action: PolicyAction::Deny,
            matched_rule: rule,
            reason: reason.into(),
            evaluation_time_us: time_us,
        }
    }
}

/// Helper function to reject strings containing CR or LF characters.
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
    use crate::models::identity::IdentityContext;
    use fcp_core::TenantId;
    use std::collections::HashSet;

    #[test]
    fn test_applies_to_user_match() {
        let mut roles = HashSet::new();
        roles.insert("employee".to_string());

        let ctx = IdentityContext {
            user_id: "alice@company.com".to_string(),
            tenant_id: TenantId::nil(),
            roles,
            groups: HashSet::new(),
            source_ip: "10.0.1.100".parse().unwrap(),
            user_agent: None,
        };

        let applies_to = AppliesTo {
            all_users: false,
            users: Some(vec!["alice@company.com".to_string()]),
            roles: None,
            groups: None,
        };

        assert!(applies_to.matches(&ctx));
    }

    #[test]
    fn test_applies_to_role_match() {
        let mut roles = HashSet::new();
        roles.insert("employee".to_string());
        roles.insert("sales".to_string());

        let ctx = IdentityContext {
            user_id: "alice@company.com".to_string(),
            tenant_id: TenantId::nil(),
            roles,
            groups: HashSet::new(),
            source_ip: "10.0.1.100".parse().unwrap(),
            user_agent: None,
        };

        let applies_to = AppliesTo {
            all_users: false,
            users: None,
            roles: Some(vec!["sales".to_string(), "management".to_string()]),
            groups: None,
        };

        // Should match because user has "sales" role
        assert!(applies_to.matches(&ctx));
    }

    #[test]
    fn test_applies_to_no_match() {
        let mut roles = HashSet::new();
        roles.insert("employee".to_string());

        let ctx = IdentityContext {
            user_id: "bob@company.com".to_string(),
            tenant_id: TenantId::nil(),
            roles,
            groups: HashSet::new(),
            source_ip: "10.0.1.100".parse().unwrap(),
            user_agent: None,
        };

        let applies_to = AppliesTo {
            all_users: false,
            users: Some(vec!["alice@company.com".to_string()]),
            roles: Some(vec!["sales".to_string()]),
            groups: None,
        };

        // Should not match - wrong user and no matching role
        assert!(!applies_to.matches(&ctx));
    }

    #[test]
    fn test_applies_to_all_users() {
        // Create a user with no matching roles or groups
        let ctx = IdentityContext {
            user_id: "unknown@company.com".to_string(),
            tenant_id: TenantId::nil(),
            roles: HashSet::new(),
            groups: HashSet::new(),
            source_ip: "10.0.1.100".parse().unwrap(),
            user_agent: None,
        };

        // Policy with all_users: true should match any user
        let applies_to = AppliesTo {
            all_users: true,
            users: None,
            roles: None,
            groups: None,
        };
        assert!(applies_to.matches(&ctx), "all_users: true should match any user");

        // Policy with all_users: true should match even when other fields are set
        let applies_to_with_roles = AppliesTo {
            all_users: true,
            users: Some(vec!["different@company.com".to_string()]),
            roles: Some(vec!["admin".to_string()]),
            groups: Some(vec!["managers".to_string()]),
        };
        assert!(
            applies_to_with_roles.matches(&ctx),
            "all_users: true should match regardless of other fields"
        );

        // Policy with all_users: false and no conditions should not match
        let applies_to_empty = AppliesTo {
            all_users: false,
            users: None,
            roles: None,
            groups: None,
        };
        assert!(
            !applies_to_empty.matches(&ctx),
            "Empty applies_to should not match"
        );
    }

    #[test]
    fn test_policy_action_helpers() {
        assert!(PolicyAction::Allow.is_allow());
        assert!(!PolicyAction::Allow.is_deny());

        assert!(PolicyAction::Deny.is_deny());
        assert!(!PolicyAction::Deny.is_allow());
    }

    #[test]
    fn test_policy_decision_creation() {
        let decision = PolicyDecision::allow("Test reason", Some("rule1".to_string()), 100);
        assert!(decision.action.is_allow());
        assert_eq!(decision.matched_rule, Some("rule1".to_string()));
        assert_eq!(decision.reason, "Test reason");
        assert_eq!(decision.evaluation_time_us, 100);

        let decision = PolicyDecision::deny("Access denied", None, 50);
        assert!(decision.action.is_deny());
        assert_eq!(decision.matched_rule, None);
        assert_eq!(decision.evaluation_time_us, 50);
    }

    #[test]
    fn test_time_window_serialization() {
        use chrono::Weekday;

        let window = TimeWindow {
            days: vec![Weekday::Mon, Weekday::Tue],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            timezone: "America/New_York".to_string(),
        };

        // Test serialization
        let json = serde_json::to_string(&window).unwrap();
        assert!(json.contains("09:00:00"));
        assert!(json.contains("17:00:00"));

        // Test deserialization
        let deserialized: TimeWindow = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.days, window.days);
        assert_eq!(deserialized.start_time, window.start_time);
        assert_eq!(deserialized.end_time, window.end_time);
    }
}
