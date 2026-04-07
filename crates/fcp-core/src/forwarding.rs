//! Advanced call forwarding rules.
//!
//! This module provides comprehensive call forwarding capabilities including:
//! - Forward unconditional (always forward)
//! - Forward on busy (when line is busy)
//! - Forward on no answer (timeout)
//! - Forward on unavailable (offline/unreachable)
//! - Time-based forwarding
//! - Selective forwarding (based on caller)

use crate::{LineBindingId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Call forwarding rule type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForwardType {
    /// Always forward (unconditional)
    Unconditional,
    /// Forward when busy
    Busy,
    /// Forward when no answer after timeout
    NoAnswer,
    /// Forward when unavailable/offline
    Unavailable,
}

impl std::fmt::Display for ForwardType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForwardType::Unconditional => write!(f, "unconditional"),
            ForwardType::Busy => write!(f, "busy"),
            ForwardType::NoAnswer => write!(f, "no_answer"),
            ForwardType::Unavailable => write!(f, "unavailable"),
        }
    }
}

/// Forward destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForwardDestination {
    /// Forward to a phone number (E.164)
    PhoneNumber(String),
    /// Forward to an extension
    Extension(String),
    /// Forward to a user ID
    User(UserId),
    /// Forward to voicemail
    Voicemail,
}

impl std::fmt::Display for ForwardDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForwardDestination::PhoneNumber(num) => write!(f, "{}", num),
            ForwardDestination::Extension(ext) => write!(f, "ext:{}", ext),
            ForwardDestination::User(id) => write!(f, "user:{}", id),
            ForwardDestination::Voicemail => write!(f, "voicemail"),
        }
    }
}

/// Time range for time-based forwarding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    /// Day of week (0 = Sunday, 6 = Saturday)
    pub day_of_week: Option<u8>,
    /// Start time (minutes since midnight)
    pub start_time: u16,
    /// End time (minutes since midnight)
    pub end_time: u16,
}

impl TimeRange {
    /// Creates a new time range.
    pub fn new(start_time: u16, end_time: u16) -> Self {
        Self {
            day_of_week: None,
            start_time,
            end_time,
        }
    }

    /// Creates a time range for a specific day.
    pub fn new_with_day(day_of_week: u8, start_time: u16, end_time: u16) -> Self {
        Self {
            day_of_week: Some(day_of_week),
            start_time,
            end_time,
        }
    }

    /// Checks if the given time is within this range.
    pub fn contains(&self, now: SystemTime) -> bool {
        use chrono::{DateTime, Datelike, Local, Timelike};

        let dt: DateTime<Local> = now.into();
        let current_minutes = dt.hour() as u16 * 60 + dt.minute() as u16;
        let current_day = dt.weekday().num_days_from_sunday() as u8;

        // Check day of week if specified
        if let Some(day) = self.day_of_week {
            if day != current_day {
                return false;
            }
        }

        // Check time range
        if self.start_time <= self.end_time {
            // Normal range (e.g., 9:00-17:00)
            current_minutes >= self.start_time && current_minutes < self.end_time
        } else {
            // Overnight range (e.g., 22:00-06:00)
            current_minutes >= self.start_time || current_minutes < self.end_time
        }
    }
}

/// Call forwarding rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardingRule {
    /// Rule ID
    pub id: String,

    /// Line binding this rule applies to
    pub line_binding_id: LineBindingId,

    /// Tenant ID
    pub tenant_id: TenantId,

    /// Type of forwarding
    pub forward_type: ForwardType,

    /// Destination to forward to
    pub destination: ForwardDestination,

    /// Whether this rule is enabled
    pub enabled: bool,

    /// Priority (lower = higher priority)
    pub priority: u8,

    /// Time ranges when this rule is active (if empty, always active)
    pub time_ranges: Vec<TimeRange>,

    /// Caller whitelist (if empty, applies to all callers)
    /// E.164 phone numbers or patterns
    pub caller_whitelist: Vec<String>,

    /// Caller blacklist (takes precedence over whitelist)
    pub caller_blacklist: Vec<String>,

    /// Maximum number of forwards to prevent loops
    pub max_forwards: u8,

    /// Created timestamp
    pub created_at: SystemTime,

    /// Updated timestamp
    pub updated_at: SystemTime,
}

impl ForwardingRule {
    /// Creates a new forwarding rule.
    pub fn new(
        line_binding_id: LineBindingId,
        tenant_id: TenantId,
        forward_type: ForwardType,
        destination: ForwardDestination,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            line_binding_id,
            tenant_id,
            forward_type,
            destination,
            enabled: true,
            priority: 0,
            time_ranges: Vec::new(),
            caller_whitelist: Vec::new(),
            caller_blacklist: Vec::new(),
            max_forwards: 5,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        }
    }

    /// Checks if this rule is active for the given time.
    pub fn is_active_at(&self, time: SystemTime) -> bool {
        if !self.enabled {
            return false;
        }

        // If no time ranges specified, always active
        if self.time_ranges.is_empty() {
            return true;
        }

        // Check if current time matches any time range
        self.time_ranges.iter().any(|range| range.contains(time))
    }

    /// Checks if this rule applies to the given caller.
    pub fn applies_to_caller(&self, caller: &str) -> bool {
        // Check blacklist first
        if !self.caller_blacklist.is_empty() {
            if self.matches_pattern_list(caller, &self.caller_blacklist) {
                return false;
            }
        }

        // If whitelist is empty, applies to all
        if self.caller_whitelist.is_empty() {
            return true;
        }

        // Check whitelist
        self.matches_pattern_list(caller, &self.caller_whitelist)
    }

    /// Checks if a number matches any pattern in the list.
    fn matches_pattern_list(&self, number: &str, patterns: &[String]) -> bool {
        for pattern in patterns {
            if self.matches_pattern(number, pattern) {
                return true;
            }
        }
        false
    }

    /// Checks if a number matches a pattern.
    /// Patterns can include wildcards: * (any digits) and ? (single digit)
    fn matches_pattern(&self, number: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        if !pattern.contains('*') && !pattern.contains('?') {
            // Exact match
            return number == pattern;
        }

        // Convert pattern to regex
        let mut regex_pattern = String::from("^");
        for ch in pattern.chars() {
            match ch {
                '*' => regex_pattern.push_str(".*"),
                '?' => regex_pattern.push('.'),
                _ => {
                    if ch.is_ascii_digit() || ch == '+' {
                        regex_pattern.push(ch);
                    } else {
                        regex_pattern.push('\\');
                        regex_pattern.push(ch);
                    }
                }
            }
        }
        regex_pattern.push('$');

        // Use simple string matching for now (full regex would require regex crate)
        // This is a simplified version that supports * and ? wildcards
        self.simple_wildcard_match(number, pattern)
    }

    /// Simple wildcard matching without regex.
    fn simple_wildcard_match(&self, text: &str, pattern: &str) -> bool {
        let text_chars: Vec<char> = text.chars().collect();
        let pattern_chars: Vec<char> = pattern.chars().collect();

        let text_len = text_chars.len();
        let pattern_len = pattern_chars.len();

        // Dynamic programming approach
        let mut dp = vec![vec![false; pattern_len + 1]; text_len + 1];
        dp[0][0] = true;

        // Handle leading asterisks
        for j in 1..=pattern_len {
            if pattern_chars[j - 1] == '*' {
                dp[0][j] = dp[0][j - 1];
            }
        }

        for i in 1..=text_len {
            for j in 1..=pattern_len {
                if pattern_chars[j - 1] == '*' {
                    dp[i][j] = dp[i][j - 1] || dp[i - 1][j];
                } else if pattern_chars[j - 1] == '?' || text_chars[i - 1] == pattern_chars[j - 1] {
                    dp[i][j] = dp[i - 1][j - 1];
                }
            }
        }

        dp[text_len][pattern_len]
    }

    /// Checks if this rule should be applied given all conditions.
    pub fn should_apply(&self, caller: &str, time: SystemTime) -> bool {
        self.is_active_at(time) && self.applies_to_caller(caller)
    }
}

/// Forwarding rules manager.
#[derive(Debug, Clone)]
pub struct ForwardingRulesManager {
    /// All forwarding rules by line binding ID
    rules: std::collections::HashMap<LineBindingId, Vec<ForwardingRule>>,
}

impl ForwardingRulesManager {
    /// Creates a new forwarding rules manager.
    pub fn new() -> Self {
        Self {
            rules: std::collections::HashMap::new(),
        }
    }

    /// Adds a forwarding rule.
    pub fn add_rule(&mut self, rule: ForwardingRule) {
        let line_binding_id = rule.line_binding_id;
        self.rules
            .entry(line_binding_id)
            .or_insert_with(Vec::new)
            .push(rule);

        // Sort by priority
        if let Some(rules) = self.rules.get_mut(&line_binding_id) {
            rules.sort_by_key(|r| r.priority);
        }
    }

    /// Gets all rules for a line binding.
    pub fn get_rules(&self, line_binding_id: &LineBindingId) -> Vec<&ForwardingRule> {
        self.rules
            .get(line_binding_id)
            .map(|rules| rules.iter().collect())
            .unwrap_or_default()
    }

    /// Finds the applicable forwarding rule for a given scenario.
    pub fn find_applicable_rule(
        &self,
        line_binding_id: &LineBindingId,
        forward_type: ForwardType,
        caller: &str,
        time: SystemTime,
    ) -> Option<&ForwardingRule> {
        self.rules.get(line_binding_id)?.iter().find(|rule| {
            rule.enabled && rule.forward_type == forward_type && rule.should_apply(caller, time)
        })
    }

    /// Removes a rule by ID.
    pub fn remove_rule(&mut self, line_binding_id: &LineBindingId, rule_id: &str) -> bool {
        if let Some(rules) = self.rules.get_mut(line_binding_id) {
            let initial_len = rules.len();
            rules.retain(|r| r.id != rule_id);
            return rules.len() < initial_len;
        }
        false
    }
}

impl Default for ForwardingRulesManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_range_normal() {
        // 9:00 AM to 5:00 PM
        let range = TimeRange::new(9 * 60, 17 * 60);

        // This test would need a mock time
        // For now, just test the struct creation
        assert_eq!(range.start_time, 540);
        assert_eq!(range.end_time, 1020);
    }

    #[test]
    fn test_wildcard_matching() {
        let rule = ForwardingRule::new(
            LineBindingId::new(),
            TenantId::nil(),
            ForwardType::Unconditional,
            ForwardDestination::Voicemail,
        );

        assert!(rule.simple_wildcard_match("+15551234567", "+1555*"));
        assert!(rule.simple_wildcard_match("+15551234567", "+1555???????"));
        assert!(rule.simple_wildcard_match("+15551234567", "*"));
        assert!(!rule.simple_wildcard_match("+15551234567", "+1666*"));
    }

    #[test]
    fn test_forwarding_rule_priority() {
        let mut manager = ForwardingRulesManager::new();
        let line_binding_id = LineBindingId::new();

        let mut rule1 = ForwardingRule::new(
            line_binding_id,
            TenantId::nil(),
            ForwardType::Unconditional,
            ForwardDestination::Voicemail,
        );
        rule1.priority = 10;

        let mut rule2 = ForwardingRule::new(
            line_binding_id,
            TenantId::nil(),
            ForwardType::Unconditional,
            ForwardDestination::Extension("1001".to_string()),
        );
        rule2.priority = 5;

        manager.add_rule(rule1);
        manager.add_rule(rule2);

        let rules = manager.get_rules(&line_binding_id);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].priority, 5); // Lower priority (higher precedence) comes first
    }

    #[test]
    fn test_forwarding_rules_no_answer_type() {
        let mut manager = ForwardingRulesManager::new();
        let line_binding_id = LineBindingId::new();

        let rule = ForwardingRule::new(
            line_binding_id,
            TenantId::nil(),
            ForwardType::NoAnswer,
            ForwardDestination::Voicemail,
        );
        manager.add_rule(rule);

        let now = SystemTime::now();
        let found = manager.find_applicable_rule(
            &line_binding_id,
            ForwardType::NoAnswer,
            "+15551234567",
            now,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().forward_type, ForwardType::NoAnswer);
        assert_eq!(found.unwrap().destination, ForwardDestination::Voicemail);

        // Should NOT match Unconditional
        let not_found = manager.find_applicable_rule(
            &line_binding_id,
            ForwardType::Unconditional,
            "+15551234567",
            now,
        );
        assert!(not_found.is_none());
    }

    #[test]
    fn test_forward_destination_voicemail_display() {
        assert_eq!(ForwardDestination::Voicemail.to_string(), "voicemail");
    }

    #[test]
    fn test_forward_destination_extension_display() {
        let dest = ForwardDestination::Extension("5001".to_string());
        assert_eq!(dest.to_string(), "ext:5001");
    }
}
