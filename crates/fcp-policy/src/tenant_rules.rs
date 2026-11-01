//! A tenant's own routing rules, as a node holds them
//! (`docs/TENANT_TRUNKS.md` §12, phase 6b).
//!
//! The rules `/v1/routing/rules` manages are each tenant's, stored in the
//! database. A node reads every tenant's at startup, again whenever the API
//! says one changed, and on a slow tick, and evaluates them on the call
//! path without touching the store — the same pattern as the tenants'
//! trunks.
//!
//! A call meets them after the node's own YAML policy (the platform's) and
//! after emergency detection: no rule, of either kind, applies to an
//! emergency call. The tenant's rules are taken in priority order (lower
//! first) and the first that matches decides: `deny` refuses the call,
//! `allow` lets it go on to ordinary routing, and `route_to_trunk` lets it
//! leave only on the trunk or trunk group the rule names. A call no rule
//! matches is routed as it always was.
//!
//! What a rule matches:
//!
//! - `destination_pattern` — a regex over what was dialled: it matches when
//!   it matches the Request-URI's user part (`+15125550100`) or the whole
//!   URI (`sip:+15125550100@pbx`), so `^\+44` works as written.
//! - `source_pattern` — the same over the caller: the From user or URI.
//! - `source_ip_ranges` — CIDR ranges the call's source address is in.
//! - `time_window_days` / `_start` / `_end` / `_timezone` — when the rule is
//!   in force, in the rule's own timezone; an end before the start spans
//!   midnight. Days alone mean the whole of those days; times alone mean
//!   every day.
//!
//! Every condition a rule sets must hold; one it leaves empty holds always.
//!
//! A rule may be for one location's callers (`docs/TENANT_SCOPE.md` §3.6).
//! A call from a location meets that location's rules first, in priority
//! order, then the tenant's rules without a location; a location's rule
//! never decides a call from elsewhere, or from no known location.

use crate::evaluator::PolicyEvaluator;
use crate::models::policy::TimeWindow;
use chrono::{NaiveTime, Weekday};
use fcp_core::storage::RoutingRuleStore;
use fcp_core::{LocationId, RoutingAction, RoutingRule, TenantId, TrunkTarget};
use ipnetwork::IpNetwork;
use regex::Regex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::RwLock;
use tracing::{debug, error, warn};

/// What a call looks like to a rule.
#[derive(Debug, Clone, Copy)]
pub struct CallFacts<'a> {
    /// The caller: the From URI.
    pub caller: &'a str,
    /// What was dialled: the Request-URI.
    pub destination: &'a str,
    /// Where the INVITE came from.
    pub source_ip: IpAddr,
    /// Where the caller is: their device's location, else their
    /// extension's, else their line's number's. `None` when not known.
    pub location: Option<LocationId>,
}

/// What the tenant's rules decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleOutcome {
    /// No rule matched: the call is routed as it always was.
    NoMatch,
    /// An `allow` rule matched.
    Allow { rule: String },
    /// A `deny` rule matched: the call is refused.
    Deny { rule: String, reason: String },
    /// A `route_to_trunk` rule matched: only `target` may carry the call.
    Route { rule: String, target: TrunkTarget },
}

impl RuleOutcome {
    /// The name of the rule that decided, if one did.
    pub fn rule(&self) -> Option<&str> {
        match self {
            Self::NoMatch => None,
            Self::Allow { rule } | Self::Deny { rule, .. } | Self::Route { rule, .. } => Some(rule),
        }
    }
}

/// One rule, ready to be matched on the call path: patterns compiled,
/// ranges parsed, the time window built.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    name: String,
    priority: i32,
    location: Option<LocationId>,
    destination: Option<Regex>,
    source: Option<Regex>,
    ranges: Vec<IpNetwork>,
    window: Option<TimeWindow>,
    action: RoutingAction,
    target: Option<TrunkTarget>,
    reason: Option<String>,
}

/// A day as the API and the admin UI write it: `Monday`, `mon`, `MON`.
fn parse_day(day: &str) -> Result<Weekday, String> {
    Weekday::from_str(day.trim()).map_err(|_| format!("'{day}' is not a day of the week"))
}

/// A time of day as `HH:MM` (or `HH:MM:SS`).
fn parse_time(field: &str, time: &str) -> Result<NaiveTime, String> {
    let time = time.trim();
    NaiveTime::parse_from_str(time, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(time, "%H:%M:%S"))
        .map_err(|_| format!("{field} '{time}' is not a time of day (HH:MM)"))
}

fn pattern(field: &str, pattern: &Option<String>) -> Result<Option<Regex>, String> {
    match pattern.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Regex::new(p)
            .map(Some)
            .map_err(|e| format!("{field} is not a valid pattern: {e}")),
        None => Ok(None),
    }
}

/// The user part of a SIP or tel URI, when it has one: what a person
/// dialled or is calling from.
fn user_part(uri: &str) -> Option<&str> {
    let rest = uri
        .trim()
        .trim_start_matches('<')
        .split_once(':')
        .map(|(_, rest)| rest)?;
    let user = match rest.split_once('@') {
        Some((user, _)) => user,
        // A tel URI is all user.
        None if uri.trim_start().starts_with("tel:") => rest,
        None => return None,
    };
    Some(user.split(';').next().unwrap_or(user))
}

/// Whether `re` matches `uri`'s user part or the whole of it.
fn matches_uri(re: &Regex, uri: &str) -> bool {
    user_part(uri).is_some_and(|user| re.is_match(user)) || re.is_match(uri)
}

impl CompiledRule {
    /// Compile a stored rule, or say what is wrong with it. The API runs
    /// the same check before it stores a rule, so what a node refuses here
    /// is a rule written around the API.
    pub fn compile(rule: &RoutingRule) -> Result<Self, String> {
        let destination = pattern("destination_pattern", &rule.destination_pattern)?;
        let source = pattern("source_pattern", &rule.source_pattern)?;
        let ranges = rule
            .source_ip_ranges
            .iter()
            .map(|r| r.trim())
            .filter(|r| !r.is_empty())
            .map(|r| {
                IpNetwork::from_str(r)
                    .or_else(|_| IpAddr::from_str(r).map(IpNetwork::from))
                    .map_err(|_| format!("source_ip_ranges: '{r}' is not an address or CIDR range"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let days = rule
            .time_window_days
            .iter()
            .filter(|d| !d.trim().is_empty())
            .map(|d| parse_day(d))
            .collect::<Result<Vec<_>, _>>()?;
        let start = match rule
            .time_window_start
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            Some(s) => Some(parse_time("time_window_start", s)?),
            None => None,
        };
        let end = match rule
            .time_window_end
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            Some(s) => Some(parse_time("time_window_end", s)?),
            None => None,
        };
        let timezone = match rule.time_window_timezone.trim() {
            "" => "UTC".to_string(),
            tz => tz.to_string(),
        };
        let window = if days.is_empty() && start.is_none() && end.is_none() {
            None
        } else {
            chrono_tz::Tz::from_str(&timezone).map_err(|_| {
                format!("time_window_timezone '{timezone}' is not an IANA timezone")
            })?;
            let all_week = vec![
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
                Weekday::Sat,
                Weekday::Sun,
            ];
            Some(TimeWindow {
                days: if days.is_empty() { all_week } else { days },
                start_time: start.unwrap_or(NaiveTime::MIN),
                end_time: end
                    .unwrap_or_else(|| NaiveTime::from_hms_opt(23, 59, 59).expect("a valid time")),
                timezone,
            })
        };

        let target = rule.trunk_target();
        if rule.action == RoutingAction::RouteToTrunk && target.is_none() {
            return Err(
                "a route_to_trunk rule must name a trunk (target_trunk_id) or a trunk group \
                 (target_trunk_group)"
                    .to_string(),
            );
        }

        Ok(Self {
            name: rule.name.clone(),
            priority: rule.priority,
            location: rule.location_id,
            destination,
            source,
            ranges,
            window,
            action: rule.action.clone(),
            target,
            reason: rule.reason.clone().filter(|r| !r.trim().is_empty()),
        })
    }

    /// Whether this rule matches the call at `now`.
    pub fn matches_at(&self, call: &CallFacts<'_>, now: chrono::DateTime<chrono::Utc>) -> bool {
        if self.location.is_some() && self.location != call.location {
            return false;
        }
        if let Some(re) = &self.destination {
            if !matches_uri(re, call.destination) {
                return false;
            }
        }
        if let Some(re) = &self.source {
            if !matches_uri(re, call.caller) {
                return false;
            }
        }
        if !self.ranges.is_empty() && !self.ranges.iter().any(|n| n.contains(call.source_ip)) {
            return false;
        }
        if let Some(window) = &self.window {
            if !PolicyEvaluator::window_matches_at(window, now) {
                return false;
            }
        }
        true
    }

    fn outcome(&self) -> RuleOutcome {
        let rule = self.name.clone();
        match (&self.action, &self.target) {
            (RoutingAction::Deny, _) => RuleOutcome::Deny {
                reason: self
                    .reason
                    .clone()
                    .unwrap_or_else(|| "Call denied by routing rule".to_string()),
                rule,
            },
            (RoutingAction::RouteToTrunk, Some(target)) => RuleOutcome::Route {
                rule,
                target: target.clone(),
            },
            // `compile` refuses a route_to_trunk rule without a target.
            (RoutingAction::RouteToTrunk, None) | (RoutingAction::Allow, _) => {
                RuleOutcome::Allow { rule }
            }
        }
    }
}

/// Check a rule before it is stored: the same compilation a node runs.
pub fn validate(rule: &RoutingRule) -> Result<(), String> {
    CompiledRule::compile(rule).map(|_| ())
}

/// Every tenant's rules, as this node knows them.
#[derive(Default)]
pub struct TenantRuleSet {
    rules: RwLock<HashMap<TenantId, Vec<CompiledRule>>>,
}

impl TenantRuleSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace what is known of the tenants' rules. Disabled rules are
    /// left out; a rule that does not compile is left out and logged.
    /// Returns how many rules are in force.
    pub fn set_rules(&self, rules: Vec<RoutingRule>) -> usize {
        let mut by_tenant: HashMap<TenantId, Vec<CompiledRule>> = HashMap::new();
        let mut count = 0;
        for rule in rules.iter().filter(|r| r.enabled) {
            match CompiledRule::compile(rule) {
                Ok(compiled) => {
                    by_tenant.entry(rule.tenant_id).or_default().push(compiled);
                    count += 1;
                }
                Err(e) => error!(
                    rule = %rule.name,
                    rule_id = %rule.id,
                    tenant_id = %rule.tenant_id,
                    error = %e,
                    "A routing rule does not compile; it is not in force"
                ),
            }
        }
        for rules in by_tenant.values_mut() {
            // A location's rules before the tenant's; then lower priority
            // first; the name breaks a tie, as the store orders.
            rules.sort_by(|a, b| {
                a.location
                    .is_none()
                    .cmp(&b.location.is_none())
                    .then(a.priority.cmp(&b.priority))
                    .then(a.name.cmp(&b.name))
            });
        }
        match self.rules.write() {
            Ok(mut current) => *current = by_tenant,
            Err(e) => warn!(error = %e, "Routing rule set lock poisoned; rules not refreshed"),
        }
        count
    }

    /// Read every tenant's rules from the store.
    pub async fn refresh<S>(&self, store: &S) -> anyhow::Result<usize>
    where
        S: RoutingRuleStore + ?Sized,
    {
        let rules = store.list_all_routing_rules().await?;
        let count = self.set_rules(rules);
        debug!(rules = count, "Tenant routing rules refreshed");
        Ok(count)
    }

    /// Whether any of `tenant`'s rules is for a location: only then is a
    /// call's location worth looking up.
    pub fn has_location_rules(&self, tenant: &TenantId) -> bool {
        self.rules
            .read()
            .map(|r| {
                r.get(tenant)
                    .is_some_and(|rules| rules.iter().any(|rule| rule.location.is_some()))
            })
            .unwrap_or(false)
    }

    /// How many rules `tenant` has in force here.
    pub fn count(&self, tenant: &TenantId) -> usize {
        self.rules
            .read()
            .map(|r| r.get(tenant).map_or(0, Vec::len))
            .unwrap_or(0)
    }

    /// What `tenant`'s rules decide about `call`, now.
    pub fn evaluate(&self, tenant: &TenantId, call: &CallFacts<'_>) -> RuleOutcome {
        self.evaluate_at(tenant, call, chrono::Utc::now())
    }

    /// As [`TenantRuleSet::evaluate`], at a given instant.
    pub fn evaluate_at(
        &self,
        tenant: &TenantId,
        call: &CallFacts<'_>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> RuleOutcome {
        let Ok(rules) = self.rules.read() else {
            warn!("Routing rule set lock poisoned; no tenant rule applied");
            return RuleOutcome::NoMatch;
        };
        rules
            .get(tenant)
            .and_then(|rules| rules.iter().find(|r| r.matches_at(call, now)))
            .map(CompiledRule::outcome)
            .unwrap_or(RuleOutcome::NoMatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn rule(tenant: TenantId, name: &str, priority: i32, action: RoutingAction) -> RoutingRule {
        let mut rule = RoutingRule::new(name.to_string(), tenant);
        rule.priority = priority;
        rule.action = action;
        rule
    }

    fn call<'a>(caller: &'a str, destination: &'a str) -> CallFacts<'a> {
        CallFacts {
            caller,
            destination,
            source_ip: "10.0.0.5".parse().unwrap(),
            location: None,
        }
    }

    /// A Wednesday, 15:00 UTC.
    fn wednesday_afternoon() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 9, 16, 15, 0, 0).unwrap()
    }

    #[test]
    fn the_first_matching_rule_by_priority_decides() {
        let tenant = TenantId::new();
        let mut international = rule(tenant, "international", 10, RoutingAction::RouteToTrunk);
        international.destination_pattern = Some(r"^\+44".into());
        international.target_trunk_group = Some("intl".into());
        let mut premium = rule(tenant, "premium", 5, RoutingAction::Deny);
        premium.destination_pattern = Some(r"^\+1900".into());
        premium.reason = Some("Premium numbers are barred".into());
        let everything = rule(tenant, "everything", 100, RoutingAction::Allow);

        let set = TenantRuleSet::new();
        assert_eq!(set.set_rules(vec![international, premium, everything]), 3);

        let at = wednesday_afternoon();
        let alice = "sip:alice@pbx";
        assert_eq!(
            set.evaluate_at(&tenant, &call(alice, "sip:+442079460000@pbx"), at),
            RuleOutcome::Route {
                rule: "international".into(),
                target: TrunkTarget::Group("intl".into())
            }
        );
        assert_eq!(
            set.evaluate_at(&tenant, &call(alice, "sip:+19005550100@pbx"), at),
            RuleOutcome::Deny {
                rule: "premium".into(),
                reason: "Premium numbers are barred".into()
            }
        );
        assert_eq!(
            set.evaluate_at(&tenant, &call(alice, "sip:+15125550100@pbx"), at),
            RuleOutcome::Allow {
                rule: "everything".into()
            }
        );
        // Another tenant's rules are not this tenant's.
        assert_eq!(
            set.evaluate_at(&TenantId::new(), &call(alice, "sip:+19005550100@pbx"), at),
            RuleOutcome::NoMatch
        );
    }

    #[test]
    fn a_pattern_matches_the_user_part_or_the_whole_uri() {
        let tenant = TenantId::new();
        let mut by_user = rule(tenant, "by-user", 1, RoutingAction::Deny);
        by_user.destination_pattern = Some(r"^011".into());
        let mut by_host = rule(tenant, "by-host", 2, RoutingAction::Deny);
        by_host.destination_pattern = Some(r"@partner\.example$".into());
        let set = TenantRuleSet::new();
        set.set_rules(vec![by_user, by_host]);
        let at = wednesday_afternoon();

        let decided = |dest: &str| {
            set.evaluate_at(&tenant, &call("sip:alice@pbx", dest), at)
                .rule()
                .map(str::to_string)
        };
        assert_eq!(decided("sip:01144207946@pbx").as_deref(), Some("by-user"));
        assert_eq!(decided("tel:01144207946").as_deref(), Some("by-user"));
        assert_eq!(
            decided("sip:bob@partner.example").as_deref(),
            Some("by-host")
        );
        assert_eq!(decided("sip:+15125550100@pbx"), None);
    }

    #[test]
    fn every_condition_a_rule_sets_must_hold() {
        let tenant = TenantId::new();
        let mut rule = rule(tenant, "branch-office", 1, RoutingAction::Deny);
        rule.source_pattern = Some(r"^2\d{3}$".into());
        rule.source_ip_ranges = vec!["10.0.0.0/24".into()];
        let set = TenantRuleSet::new();
        set.set_rules(vec![rule]);
        let at = wednesday_afternoon();

        assert!(matches!(
            set.evaluate_at(&tenant, &call("sip:2001@pbx", "sip:+15125550100@pbx"), at),
            RuleOutcome::Deny { .. }
        ));
        // The wrong caller.
        assert_eq!(
            set.evaluate_at(&tenant, &call("sip:1001@pbx", "sip:+15125550100@pbx"), at),
            RuleOutcome::NoMatch
        );
        // The right caller from the wrong network.
        let elsewhere = CallFacts {
            source_ip: "192.0.2.7".parse().unwrap(),
            ..call("sip:2001@pbx", "sip:+15125550100@pbx")
        };
        assert_eq!(
            set.evaluate_at(&tenant, &elsewhere, at),
            RuleOutcome::NoMatch
        );
    }

    #[test]
    fn a_time_window_is_the_rules_own_timezone() {
        let tenant = TenantId::new();
        // Business hours in New York: 15:00 UTC on a Wednesday is 11:00 there.
        let mut hours = rule(tenant, "business-hours", 1, RoutingAction::RouteToTrunk);
        hours.target_trunk_group = Some("day".into());
        hours.time_window_days = vec!["Monday".into(), "wednesday".into(), "Fri".into()];
        hours.time_window_start = Some("09:00".into());
        hours.time_window_end = Some("17:00".into());
        hours.time_window_timezone = "America/New_York".into();
        let set = TenantRuleSet::new();
        set.set_rules(vec![hours]);
        let facts = call("sip:alice@pbx", "sip:+15125550100@pbx");

        assert!(matches!(
            set.evaluate_at(&tenant, &facts, wednesday_afternoon()),
            RuleOutcome::Route { .. }
        ));
        // 23:00 UTC is 19:00 in New York: out of hours.
        let evening = chrono::Utc.with_ymd_and_hms(2026, 9, 16, 23, 0, 0).unwrap();
        assert_eq!(
            set.evaluate_at(&tenant, &facts, evening),
            RuleOutcome::NoMatch
        );
        // Thursday is not one of its days.
        let thursday = chrono::Utc.with_ymd_and_hms(2026, 9, 17, 15, 0, 0).unwrap();
        assert_eq!(
            set.evaluate_at(&tenant, &facts, thursday),
            RuleOutcome::NoMatch
        );
    }

    #[test]
    fn an_overnight_window_spans_midnight() {
        let tenant = TenantId::new();
        let mut night = rule(tenant, "night", 1, RoutingAction::Deny);
        night.time_window_start = Some("22:00".into());
        night.time_window_end = Some("06:00".into());
        let set = TenantRuleSet::new();
        set.set_rules(vec![night]);
        let facts = call("sip:alice@pbx", "sip:+15125550100@pbx");
        let at = |h| chrono::Utc.with_ymd_and_hms(2026, 9, 16, h, 30, 0).unwrap();

        assert!(matches!(
            set.evaluate_at(&tenant, &facts, at(23)),
            RuleOutcome::Deny { .. }
        ));
        assert!(matches!(
            set.evaluate_at(&tenant, &facts, at(3)),
            RuleOutcome::Deny { .. }
        ));
        assert_eq!(
            set.evaluate_at(&tenant, &facts, at(12)),
            RuleOutcome::NoMatch
        );
    }

    #[test]
    fn disabled_rules_and_rules_that_do_not_compile_are_not_in_force() {
        let tenant = TenantId::new();
        let mut off = rule(tenant, "off", 1, RoutingAction::Deny);
        off.enabled = false;
        let mut broken = rule(tenant, "broken", 2, RoutingAction::Deny);
        broken.destination_pattern = Some("([".into());
        let set = TenantRuleSet::new();
        assert_eq!(set.set_rules(vec![off, broken]), 0);
        assert_eq!(set.count(&tenant), 0);
    }

    #[test]
    fn what_the_api_refuses() {
        let tenant = TenantId::new();
        let refused = |edit: &dyn Fn(&mut RoutingRule)| {
            let mut rule = rule(tenant, "r", 1, RoutingAction::Allow);
            edit(&mut rule);
            validate(&rule).unwrap_err()
        };
        assert!(refused(&|r| r.destination_pattern = Some("([".into())).contains("destination"));
        assert!(
            refused(&|r| r.source_ip_ranges = vec!["10.0.0.0/33".into()]).contains("10.0.0.0/33")
        );
        assert!(refused(&|r| r.time_window_days = vec!["Someday".into()]).contains("Someday"));
        assert!(refused(&|r| r.time_window_start = Some("25:00".into())).contains("25:00"));
        assert!(refused(&|r| {
            r.time_window_start = Some("09:00".into());
            r.time_window_timezone = "Mars/Olympus".into();
        })
        .contains("Mars/Olympus"));
        assert!(refused(&|r| r.action = RoutingAction::RouteToTrunk).contains("must name a trunk"));

        let mut fine = rule(tenant, "fine", 1, RoutingAction::RouteToTrunk);
        fine.target_trunk_group = Some("intl".into());
        fine.source_ip_ranges = vec!["10.0.0.0/8".into(), "192.0.2.7".into()];
        assert_eq!(validate(&fine), Ok(()));
    }

    /// A location's rules decide its callers' calls before the tenant's;
    /// they decide no one else's (docs/TENANT_SCOPE.md §3.6).
    #[test]
    fn a_locations_rules_come_first_and_only_for_its_callers() {
        let tenant = TenantId::new();
        let branch = LocationId::new();
        let mut tenant_wide = rule(tenant, "tenant-wide", 1, RoutingAction::RouteToTrunk);
        tenant_wide.target_trunk_group = Some("hq".into());
        let mut at_branch = rule(tenant, "branch", 50, RoutingAction::RouteToTrunk);
        at_branch.target_trunk_group = Some("branch".into());
        at_branch.location_id = Some(branch);
        let mut branch_premium = rule(tenant, "branch-premium", 10, RoutingAction::Deny);
        branch_premium.destination_pattern = Some(r"^\+1900".into());
        branch_premium.location_id = Some(branch);
        let set = TenantRuleSet::new();
        set.set_rules(vec![tenant_wide, at_branch, branch_premium]);
        assert!(set.has_location_rules(&tenant));
        assert!(!set.has_location_rules(&TenantId::new()));

        let at = |location| CallFacts {
            location,
            ..call("sip:1001@pbx", "sip:+15125550100@pbx")
        };
        // At the branch: its rule, though the tenant-wide one's priority is lower.
        assert_eq!(
            set.evaluate_at(&tenant, &at(Some(branch)), wednesday_afternoon()),
            RuleOutcome::Route {
                rule: "branch".into(),
                target: TrunkTarget::Group("branch".into())
            }
        );
        // Its priority order among its own rules.
        let premium = CallFacts {
            location: Some(branch),
            ..call("sip:1001@pbx", "sip:+19005550100@pbx")
        };
        assert_eq!(
            set.evaluate_at(&tenant, &premium, wednesday_afternoon()).rule(),
            Some("branch-premium")
        );
        // Elsewhere, and nowhere known: the tenant's.
        for location in [Some(LocationId::new()), None] {
            assert_eq!(
                set.evaluate_at(&tenant, &at(location), wednesday_afternoon()).rule(),
                Some("tenant-wide"),
                "{location:?}"
            );
        }
    }
}
