//! Policy evaluation engine.
//!
//! This module implements the core policy evaluation logic.
//! It evaluates policy rules in priority order and returns Allow/Deny decisions.

use crate::models::{IdentityContext, PolicyAction, PolicyDecision};
use crate::storage::RoutingStore;
use anyhow::{Context as _, Result};
use chrono::Datelike;
use fcp_core::TenantId;
use regex::Regex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Policy evaluator.
///
/// Evaluates policy rules to determine if a call is allowed or denied.
/// Rules are evaluated in priority order (lower number = higher priority).
pub struct PolicyEvaluator {
    /// Reference to the routing store
    store: Arc<dyn RoutingStore>,

    /// Cache of compiled regex patterns for performance
    regex_cache: Arc<RwLock<HashMap<String, Regex>>>,
}

impl PolicyEvaluator {
    /// Create a new policy evaluator.
    pub fn new(store: Arc<dyn RoutingStore>) -> Self {
        Self {
            store,
            regex_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Evaluate if a call is allowed.
    ///
    /// # Arguments
    ///
    /// * `caller` - User ID of the caller (e.g., "alice@company.com")
    /// * `destination` - Destination being called (e.g., "sip:bob@company.com" or "+15551234567")
    /// * `tenant_id` - Tenant ID for multi-tenant isolation
    /// * `source_ip` - Source IP address of the call
    ///
    /// # Returns
    ///
    /// A `PolicyDecision` indicating Allow or Deny, with the matched rule and reason.
    pub async fn evaluate_call(
        &self,
        caller: &str,
        destination: &str,
        tenant_id: &TenantId,
        source_ip: IpAddr,
    ) -> Result<PolicyDecision> {
        let start = std::time::Instant::now();

        debug!(
            caller = %caller,
            destination = %destination,
            tenant_id = %tenant_id.to_string(),
            source_ip = %source_ip,
            "Starting policy evaluation"
        );

        // Step 1: Build identity context
        let ctx = self
            .build_identity_context(caller, tenant_id, source_ip)
            .await
            .context("Failed to build identity context")?;

        debug!(
            user_id = %ctx.user_id,
            roles = ?ctx.roles,
            groups = ?ctx.groups,
            "Built identity context"
        );

        // Step 2: Load policy rules for this tenant
        let mut rules = self
            .store
            .list_policy_rules(tenant_id)
            .await
            .context("Failed to load policy rules")?;

        debug!(rules_count = rules.len(), "Loaded policy rules");

        // Step 3: Sort by priority (lower number = higher priority)
        rules.sort_by_key(|r| r.priority);

        // Step 4: Evaluate rules in priority order
        for rule in rules {
            if !rule.enabled {
                debug!(rule = %rule.name, "Skipping disabled rule");
                continue;
            }

            debug!(rule = %rule.name, priority = rule.priority, "Evaluating rule");

            // Check if rule applies to this caller
            if !rule.applies_to.matches(&ctx) {
                debug!(rule = %rule.name, "Rule does not apply to this user");
                continue;
            }

            // Check destination match
            if let Some(dest_match) = &rule.destination_match {
                if !self.matches_destination(destination, &dest_match.pattern).await? {
                    debug!(rule = %rule.name, "Destination does not match pattern");
                    continue;
                }
            }

            // Check time windows (if any specified)
            if !rule.time_windows.is_empty() {
                if !self.matches_time_window(&rule.time_windows) {
                    debug!(rule = %rule.name, "Current time not in allowed windows");
                    continue;
                }
            }

            // Check source IP ranges (if any specified)
            if !rule.source_ip_ranges.is_empty() {
                if !self.matches_ip_range(source_ip, &rule.source_ip_ranges) {
                    debug!(rule = %rule.name, "Source IP not in allowed ranges");
                    continue;
                }
            }

            // Rule matched!
            let elapsed = start.elapsed();
            let reason = rule
                .reason
                .clone()
                .unwrap_or_else(|| format!("Matched rule: {}", rule.name));

            debug!(
                rule = %rule.name,
                action = ?rule.action,
                elapsed_us = elapsed.as_micros(),
                "Rule matched"
            );

            return Ok(PolicyDecision {
                action: rule.action,
                matched_rule: Some(rule.name.clone()),
                reason,
                evaluation_time_us: elapsed.as_micros() as u64,
            });
        }

        // No rule matched - default DENY
        let elapsed = start.elapsed();
        warn!(
            caller = %caller,
            destination = %destination,
            "No policy rule matched (default deny)"
        );

        Ok(PolicyDecision {
            action: PolicyAction::Deny,
            matched_rule: None,
            reason: "No policy rule matched (default deny)".to_string(),
            evaluation_time_us: elapsed.as_micros() as u64,
        })
    }

    /// Build identity context from user information.
    async fn build_identity_context(
        &self,
        user_id: &str,
        tenant_id: &TenantId,
        source_ip: IpAddr,
    ) -> Result<IdentityContext> {
        let user = self
            .store
            .get_user(user_id, tenant_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("User not found: {}", user_id))?;

        Ok(IdentityContext {
            user_id: user.id.clone(),
            tenant_id: *tenant_id,
            roles: user.roles.clone(),
            groups: user.groups.clone(),
            source_ip,
            user_agent: None,
        })
    }

    /// Check if destination matches the regex pattern.
    async fn matches_destination(&self, destination: &str, pattern: &str) -> Result<bool> {
        // Get or compile regex
        let mut cache = self.regex_cache.write().await;

        let regex = cache.entry(pattern.to_string()).or_insert_with(|| {
            Regex::new(pattern).unwrap_or_else(|e| {
                warn!(pattern = %pattern, error = %e, "Invalid regex pattern, using match-nothing pattern");
                Regex::new("^$").unwrap() // Match nothing if regex is invalid
            })
        });

        Ok(regex.is_match(destination))
    }

    /// Check if current time is within any of the time windows.
    fn matches_time_window(&self, windows: &[crate::models::policy::TimeWindow]) -> bool {
        let now = chrono::Local::now();
        let today = now.weekday();
        let time = now.time();

        windows.iter().any(|window| {
            // Check if today is in the allowed days
            if !window.days.contains(&today) {
                return false;
            }

            // Check if current time is within the window
            time >= window.start_time && time <= window.end_time
        })
    }

    /// Check if IP is in any of the allowed ranges.
    fn matches_ip_range(&self, ip: IpAddr, ranges: &[crate::models::policy::IpRange]) -> bool {
        use ipnetwork::IpNetwork;
        use std::str::FromStr;

        ranges.iter().any(|range| {
            match IpNetwork::from_str(&range.cidr) {
                Ok(network) => network.contains(ip),
                Err(e) => {
                    warn!(cidr = %range.cidr, error = %e, "Invalid CIDR notation");
                    false
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AppliesTo, DestinationMatch, PolicyRule, User};
    use crate::storage::RoutingStore;
    use async_trait::async_trait;
    use fcp_core::TenantId;
    use std::collections::HashSet;

    // Mock storage for testing
    struct MockStore {
        users: Vec<User>,
        policies: Vec<PolicyRule>,
    }

    #[async_trait]
    impl RoutingStore for MockStore {
        async fn get_user(&self, user_id: &str, tenant_id: &TenantId) -> Result<Option<User>> {
            Ok(self
                .users
                .iter()
                .find(|u| u.id == user_id && &u.tenant_id == tenant_id)
                .cloned())
        }

        async fn list_users(&self, tenant_id: &TenantId) -> Result<Vec<User>> {
            Ok(self
                .users
                .iter()
                .filter(|u| &u.tenant_id == tenant_id)
                .cloned()
                .collect())
        }

        async fn list_policy_rules(&self, tenant_id: &TenantId) -> Result<Vec<PolicyRule>> {
            Ok(self
                .policies
                .iter()
                .filter(|p| &p.tenant_id == tenant_id)
                .cloned()
                .collect())
        }

        async fn get_policy_rule(
            &self,
            name: &str,
            tenant_id: &TenantId,
        ) -> Result<Option<PolicyRule>> {
            Ok(self
                .policies
                .iter()
                .find(|p| p.name == name && &p.tenant_id == tenant_id)
                .cloned())
        }
    }

    #[tokio::test]
    async fn test_evaluator_allow() {
        let tenant_id = TenantId::nil();

        let mut roles = HashSet::new();
        roles.insert("employee".to_string());

        let user = User {
            id: "alice@test.com".to_string(),
            tenant_id,
            display_name: Some("Alice".to_string()),
            roles,
            groups: HashSet::new(),
            endpoints: vec![],
            enabled: true,
        };

        let rule = PolicyRule {
            name: "allow-employees".to_string(),
            tenant_id,
            priority: 100,
            enabled: true,
            applies_to: AppliesTo {
                users: None,
                roles: Some(vec!["employee".to_string()]),
                groups: None,
            },
            destination_match: Some(DestinationMatch {
                pattern: "^[0-9]+$".to_string(),
                tags: None,
            }),
            time_windows: vec![],
            source_ip_ranges: vec![],
            action: PolicyAction::Allow,
            reason: Some("Employees can call extensions".to_string()),
        };

        let store = Arc::new(MockStore {
            users: vec![user],
            policies: vec![rule],
        });

        let evaluator = PolicyEvaluator::new(store);

        let decision = evaluator
            .evaluate_call(
                "alice@test.com",
                "1234",
                &tenant_id,
                "10.0.1.100".parse().unwrap(),
            )
            .await
            .unwrap();

        assert!(decision.action.is_allow());
        assert_eq!(decision.matched_rule, Some("allow-employees".to_string()));
    }

    #[tokio::test]
    async fn test_evaluator_deny_default() {
        let tenant_id = TenantId::nil();

        let user = User {
            id: "bob@test.com".to_string(),
            tenant_id,
            display_name: Some("Bob".to_string()),
            roles: HashSet::new(),
            groups: HashSet::new(),
            endpoints: vec![],
            enabled: true,
        };

        let store = Arc::new(MockStore {
            users: vec![user],
            policies: vec![], // No policies
        });

        let evaluator = PolicyEvaluator::new(store);

        let decision = evaluator
            .evaluate_call(
                "bob@test.com",
                "1234",
                &tenant_id,
                "10.0.1.100".parse().unwrap(),
            )
            .await
            .unwrap();

        // Should deny by default
        assert!(decision.action.is_deny());
        assert_eq!(decision.matched_rule, None);
    }
}
