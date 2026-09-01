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
    /// # Security
    /// This method implements comprehensive security validation:
    /// - **CRLF Injection Prevention**: Validates caller and destination at entry point
    /// - **Identity Validation**: Validates user context after retrieval from storage
    /// - **Policy Validation**: Validates all rules before evaluation (fail-fast)
    /// - **ReDoS Prevention**: Regex patterns validated at configuration time
    /// - **CIDR Validation**: IP ranges validated before matching
    /// - **Multi-layer Defense**: Validation at entry, context, rule, and user levels
    ///
    /// # Arguments
    ///
    /// * `caller` - User ID of the caller (validated for CRLF, e.g., "alice@company.com")
    /// * `destination` - Destination being called (validated for CRLF, e.g., "sip:bob@company.com")
    /// * `tenant_id` - Tenant ID for multi-tenant isolation
    /// * `source_ip` - Source IP address of the call
    ///
    /// # Returns
    ///
    /// A `PolicyDecision` indicating Allow or Deny, with the matched rule and reason.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Caller or destination contains CRLF characters
    /// - Identity context validation fails
    /// - Policy rule validation fails
    /// - User data validation fails
    pub async fn evaluate_call(
        &self,
        caller: &str,
        destination: &str,
        tenant_id: &TenantId,
        source_ip: IpAddr,
    ) -> Result<PolicyDecision> {
        let start = std::time::Instant::now();

        if caller.bytes().any(|b| matches!(b, b'\r' | b'\n')) {
            anyhow::bail!("caller contains invalid characters");
        }
        if destination.bytes().any(|b| matches!(b, b'\r' | b'\n')) {
            anyhow::bail!("destination contains invalid characters");
        }

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
        ctx.validate()
            .map_err(|e| anyhow::anyhow!("Invalid identity context: {}", e))?;

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

        // Validate rules before evaluation to fail fast on invalid config.
        for rule in &rules {
            rule.validate()
                .map_err(|e| anyhow::anyhow!("Invalid policy rule '{}': {}", rule.name, e))?;
        }

        // Load the tenant's feature toggles once, but only when some enabled
        // rule actually gates on a feature.
        let tenant_features = if rules
            .iter()
            .any(|r| r.enabled && !r.require_features.is_empty())
        {
            self.store
                .get_tenant_features(tenant_id)
                .await
                .context("Failed to load tenant features")?
        } else {
            None
        };

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

            // Check required tenant features (absent features are disabled)
            if !rule.require_features.is_empty() {
                let all_enabled = tenant_features
                    .as_ref()
                    .map(|f| rule.require_features.iter().all(|name| f.is_enabled(name)))
                    .unwrap_or(false);
                if !all_enabled {
                    debug!(
                        rule = %rule.name,
                        required = ?rule.require_features,
                        "Tenant is missing a required feature"
                    );
                    continue;
                }
            }

            // Check destination match
            if let Some(dest_match) = &rule.destination_match {
                if !self
                    .matches_destination(destination, &dest_match.pattern)
                    .await?
                {
                    debug!(rule = %rule.name, "Destination does not match pattern");
                    continue;
                }
            }

            // Check time windows (if any specified)
            if !rule.time_windows.is_empty() && !self.matches_time_window(&rule.time_windows) {
                debug!(rule = %rule.name, "Current time not in allowed windows");
                continue;
            }

            // Check source IP ranges (if any specified)
            if !rule.source_ip_ranges.is_empty()
                && !self.matches_ip_range(source_ip, &rule.source_ip_ranges)
            {
                debug!(rule = %rule.name, "Source IP not in allowed ranges");
                continue;
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
    ///
    /// If the user is not found in the routing store, creates a minimal context
    /// with empty roles/groups. This allows `all_users: true` policies to still
    /// match even when users are managed externally (e.g., via fcp-admin API).
    async fn build_identity_context(
        &self,
        user_id: &str,
        tenant_id: &TenantId,
        source_ip: IpAddr,
    ) -> Result<IdentityContext> {
        match self.store.get_user(user_id, tenant_id).await? {
            Some(user) => {
                user.validate()
                    .map_err(|e| anyhow::anyhow!("Invalid user data for '{}': {}", user_id, e))?;

                Ok(IdentityContext {
                    user_id: user.id.clone(),
                    tenant_id: *tenant_id,
                    roles: user.roles.clone(),
                    groups: user.groups.clone(),
                    source_ip,
                    user_agent: None,
                })
            }
            None => {
                // User not found in routing store - create minimal context.
                // This allows policies with `all_users: true` to match callers
                // who are registered via fcp-admin but not defined in routing.yaml.
                debug!(
                    user_id = %user_id,
                    "User not found in routing store, using minimal context"
                );
                Ok(IdentityContext {
                    user_id: user_id.to_string(),
                    tenant_id: *tenant_id,
                    roles: std::collections::HashSet::new(),
                    groups: std::collections::HashSet::new(),
                    source_ip,
                    user_agent: None,
                })
            }
        }
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

    /// Check if the current time falls in any of the time windows.
    ///
    /// Each window is evaluated in *its own* timezone (previously the server's
    /// local timezone was used and `TimeWindow.timezone` was silently
    /// ignored). Windows whose `end_time` precedes `start_time` span
    /// midnight: they match from `start_time` on a listed day through
    /// `end_time` on the following morning.
    fn matches_time_window(&self, windows: &[crate::models::policy::TimeWindow]) -> bool {
        let now_utc = chrono::Utc::now();
        windows
            .iter()
            .any(|window| Self::window_matches_at(window, now_utc))
    }

    /// Whether `now_utc` falls inside one window, evaluated in the window's
    /// own timezone. Split out (and given an explicit instant) so tests are
    /// deterministic.
    fn window_matches_at(
        window: &crate::models::policy::TimeWindow,
        now_utc: chrono::DateTime<chrono::Utc>,
    ) -> bool {
        let tz: chrono_tz::Tz = match window.timezone.parse() {
            Ok(tz) => tz,
            Err(_) => {
                warn!(
                    timezone = %window.timezone,
                    "Invalid timezone in time window - treating window as not matching"
                );
                return false;
            }
        };
        let local = now_utc.with_timezone(&tz);
        let today = local.weekday();
        let time = local.time();

        if window.start_time <= window.end_time {
            window.days.contains(&today) && time >= window.start_time && time <= window.end_time
        } else {
            // Overnight window spanning midnight
            (window.days.contains(&today) && time >= window.start_time)
                || (window.days.contains(&today.pred()) && time <= window.end_time)
        }
    }

    /// Check if IP is in any of the allowed ranges.
    fn matches_ip_range(&self, ip: IpAddr, ranges: &[crate::models::policy::IpRange]) -> bool {
        use ipnetwork::IpNetwork;
        use std::str::FromStr;

        ranges
            .iter()
            .any(|range| match IpNetwork::from_str(&range.cidr) {
                Ok(network) => network.contains(ip),
                Err(e) => {
                    warn!(cidr = %range.cidr, error = %e, "Invalid CIDR notation");
                    false
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
    #[derive(Default)]
    struct MockStore {
        users: Vec<User>,
        policies: Vec<PolicyRule>,
        features: Option<crate::models::TenantFeatures>,
    }

    #[async_trait]
    impl RoutingStore for MockStore {
        async fn find_user(&self, user_id: &str) -> Result<Option<User>> {
            Ok(self.users.iter().find(|u| u.id == user_id).cloned())
        }

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

        async fn get_tenant_features(
            &self,
            tenant_id: &TenantId,
        ) -> Result<Option<crate::models::TenantFeatures>> {
            Ok(self.features.clone().filter(|f| &f.tenant_id == tenant_id))
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
                all_users: false,
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
            require_features: vec![],
            action: PolicyAction::Allow,
            reason: Some("Employees can call extensions".to_string()),
        };

        let store = Arc::new(MockStore {
            users: vec![user],
            policies: vec![rule],
            features: None,
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
            features: None,
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

    fn feature_gated_rule(tenant_id: TenantId, feature: &str) -> PolicyRule {
        PolicyRule {
            name: "feature-gated".to_string(),
            tenant_id,
            priority: 100,
            enabled: true,
            applies_to: AppliesTo {
                all_users: true,
                users: None,
                roles: None,
                groups: None,
            },
            destination_match: None,
            time_windows: vec![],
            source_ip_ranges: vec![],
            require_features: vec![feature.to_string()],
            action: PolicyAction::Allow,
            reason: Some("Requires a tenant feature".to_string()),
        }
    }

    fn test_user(tenant_id: TenantId) -> User {
        User {
            id: "alice@test.com".to_string(),
            tenant_id,
            display_name: None,
            roles: HashSet::new(),
            groups: HashSet::new(),
            endpoints: vec![],
            enabled: true,
        }
    }

    #[tokio::test]
    async fn test_require_features_gates_rule() {
        let tenant_id = TenantId::nil();
        let mut features = crate::models::TenantFeatures {
            tenant_id,
            ..Default::default()
        };

        // Feature disabled (absent): the rule must not match → default deny.
        let store = Arc::new(MockStore {
            users: vec![test_user(tenant_id)],
            policies: vec![feature_gated_rule(tenant_id, "international_calls")],
            features: Some(features.clone()),
        });
        let decision = PolicyEvaluator::new(store)
            .evaluate_call(
                "alice@test.com",
                "0044123",
                &tenant_id,
                "10.0.0.1".parse().unwrap(),
            )
            .await
            .unwrap();
        assert!(decision.action.is_deny());

        // Feature enabled: the rule matches.
        features
            .features
            .insert("international_calls".to_string(), true);
        let store = Arc::new(MockStore {
            users: vec![test_user(tenant_id)],
            policies: vec![feature_gated_rule(tenant_id, "international_calls")],
            features: Some(features),
        });
        let decision = PolicyEvaluator::new(store)
            .evaluate_call(
                "alice@test.com",
                "0044123",
                &tenant_id,
                "10.0.0.1".parse().unwrap(),
            )
            .await
            .unwrap();
        assert!(decision.action.is_allow());
        assert_eq!(decision.matched_rule.as_deref(), Some("feature-gated"));

        // No feature toggles configured at all for the tenant: disabled.
        let store = Arc::new(MockStore {
            users: vec![test_user(tenant_id)],
            policies: vec![feature_gated_rule(tenant_id, "international_calls")],
            features: None,
        });
        let decision = PolicyEvaluator::new(store)
            .evaluate_call(
                "alice@test.com",
                "0044123",
                &tenant_id,
                "10.0.0.1".parse().unwrap(),
            )
            .await
            .unwrap();
        assert!(decision.action.is_deny());
    }

    fn window(
        days: Vec<chrono::Weekday>,
        start: &str,
        end: &str,
        tz: &str,
    ) -> crate::models::TimeWindow {
        crate::models::TimeWindow {
            days,
            start_time: start.parse().unwrap(),
            end_time: end.parse().unwrap(),
            timezone: tz.to_string(),
        }
    }

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn test_time_window_respects_timezone() {
        use chrono::Weekday::*;
        // 2026-01-05 is a Monday. 15:00 UTC.
        let now = at(2026, 1, 5, 15, 0);

        // Business hours in UTC: matches.
        let w = window(vec![Mon], "09:00", "17:00", "UTC");
        assert!(PolicyEvaluator::window_matches_at(&w, now));

        // Same wall-clock window in Tokyo (UTC+9): it is Tue 00:00 there —
        // wrong day and outside hours. The old Local-time code would have
        // matched or not depending on the *server's* timezone.
        let w = window(vec![Mon], "09:00", "17:00", "Asia/Tokyo");
        assert!(!PolicyEvaluator::window_matches_at(&w, now));

        // In New York (UTC-5) it is Mon 10:00: matches.
        let w = window(vec![Mon], "09:00", "17:00", "America/New_York");
        assert!(PolicyEvaluator::window_matches_at(&w, now));

        // Invalid timezone: never matches.
        let w = window(vec![Mon], "09:00", "17:00", "Not/AZone");
        assert!(!PolicyEvaluator::window_matches_at(&w, now));
    }

    #[test]
    fn test_overnight_time_window_spans_midnight() {
        use chrono::Weekday::*;
        // Night shift: Mon 22:00 → Tue 06:00 UTC.
        let w = window(vec![Mon], "22:00", "06:00", "UTC");

        // Mon 23:00 — inside, evening side.
        assert!(PolicyEvaluator::window_matches_at(
            &w,
            at(2026, 1, 5, 23, 0)
        ));
        // Tue 02:00 — inside, morning side (day rolled over).
        assert!(PolicyEvaluator::window_matches_at(&w, at(2026, 1, 6, 2, 0)));
        // Mon 12:00 — outside.
        assert!(!PolicyEvaluator::window_matches_at(
            &w,
            at(2026, 1, 5, 12, 0)
        ));
        // Wed 02:00 — outside (Tuesday night is not in the window).
        assert!(!PolicyEvaluator::window_matches_at(
            &w,
            at(2026, 1, 7, 2, 0)
        ));
    }
}
