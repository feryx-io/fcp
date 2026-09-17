//! Trunk health monitoring for the SBC.
//!
//! Tracks per-trunk failure counts, success/failure timestamps, and health
//! status transitions. Optionally runs periodic OPTIONS probe health checks.

use crate::metrics::TrunkHealthStatus;
use crate::trunk::TrunkConfig;
use dashmap::{DashMap, DashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Per-trunk health state.
struct TrunkHealthState {
    status: TrunkHealthStatus,
    consecutive_failures: u32,
    max_failures: u32,
    recovery_interval_secs: u64,
    last_failure: Option<Instant>,
    last_success: Option<Instant>,
    concurrent_calls: AtomicU64,
    health_check_enabled: bool,
    health_check_interval_secs: u64,
    last_probe: Option<Instant>,
}

/// Monitors trunk health, tracking failures and triggering status transitions.
///
/// State is kept per trunk **key** (`TrunkConfig::key`, the trunk's id), not
/// per name, so two tenants may both have a `carrier-a` without sharing
/// health or capacity (`docs/TENANT_TRUNKS.md` §4). The methods that people
/// and the admin API use take a name and resolve it.
pub struct TrunkHealthMonitor {
    state: DashMap<String, TrunkHealthState>,
    /// Trunks that have been administratively disabled via the admin API,
    /// by key.
    admin_disabled: DashSet<String>,
    /// Trunk name → key, for the admin API and the logs, which speak names.
    keys_by_name: DashMap<String, String>,
}

impl TrunkHealthMonitor {
    /// Create a new monitor pre-populated with the trunks it watches.
    pub fn new(trunks: &[TrunkConfig]) -> Self {
        let state = DashMap::new();
        let keys_by_name = DashMap::new();
        for trunk in trunks {
            keys_by_name.insert(trunk.name.clone(), trunk.key());
            state.insert(
                trunk.key(),
                TrunkHealthState {
                    status: TrunkHealthStatus::Unknown,
                    consecutive_failures: 0,
                    max_failures: trunk.failover.max_failures,
                    recovery_interval_secs: trunk.failover.recovery_interval_secs,
                    last_failure: None,
                    last_success: None,
                    concurrent_calls: AtomicU64::new(0),
                    health_check_enabled: trunk.failover.health_check_enabled,
                    health_check_interval_secs: trunk.failover.health_check_interval_secs,
                    last_probe: None,
                },
            );
        }
        Self {
            state,
            admin_disabled: DashSet::new(),
            keys_by_name,
        }
    }

    /// Start watching trunks this monitor has not seen — the tenants'
    /// trunks, which arrive after startup and change while the node runs
    /// (`docs/TENANT_TRUNKS.md` §5). Trunks it already knows keep the
    /// health they have earned.
    pub fn track(&self, trunks: &[std::sync::Arc<TrunkConfig>]) {
        for trunk in trunks {
            let key = trunk.key();
            self.keys_by_name.insert(trunk.name.clone(), key.clone());
            if self.state.contains_key(&key) {
                continue;
            }
            self.state.insert(
                key,
                TrunkHealthState {
                    status: TrunkHealthStatus::Unknown,
                    consecutive_failures: 0,
                    max_failures: trunk.failover.max_failures,
                    recovery_interval_secs: trunk.failover.recovery_interval_secs,
                    last_failure: None,
                    last_success: None,
                    concurrent_calls: AtomicU64::new(0),
                    health_check_enabled: trunk.failover.health_check_enabled,
                    health_check_interval_secs: trunk.failover.health_check_interval_secs,
                    last_probe: None,
                },
            );
        }
    }

    /// The key of a trunk named `name`, or the name itself when nothing
    /// knows it — so a caller that has only a name still reads and writes
    /// the same entry it did before tenant trunks existed.
    fn key_of(&self, name: &str) -> String {
        self.keys_by_name
            .get(name)
            .map(|k| k.clone())
            .unwrap_or_else(|| name.to_string())
    }

    /// The name of the trunk `key` belongs to, for logs and the admin API.
    pub fn name_of(&self, key: &str) -> String {
        self.keys_by_name
            .iter()
            .find(|entry| entry.value() == key)
            .map(|entry| entry.key().clone())
            .unwrap_or_else(|| key.to_string())
    }

    /// Record a failure for the given trunk.
    ///
    /// If consecutive failures reach `max_failures`, transitions to `Down`.
    pub fn record_failure(&self, trunk: &str) {
        if let Some(mut entry) = self.state.get_mut(&self.key_of(trunk)) {
            entry.consecutive_failures += 1;
            entry.last_failure = Some(Instant::now());
            if entry.consecutive_failures >= entry.max_failures {
                entry.status = TrunkHealthStatus::Down;
            } else if entry.consecutive_failures >= entry.max_failures / 2 {
                entry.status = TrunkHealthStatus::Degraded;
            }
        }
    }

    /// Record a success for the given trunk.
    ///
    /// Resets failure count and transitions to `Healthy`.
    pub fn record_success(&self, trunk: &str) {
        if let Some(mut entry) = self.state.get_mut(&self.key_of(trunk)) {
            entry.consecutive_failures = 0;
            entry.last_success = Some(Instant::now());
            entry.status = TrunkHealthStatus::Healthy;
        }
    }

    /// Returns `true` if the trunk is healthy (not `Down` and not admin-disabled).
    pub fn is_healthy(&self, trunk: &str) -> bool {
        let key = self.key_of(trunk);
        if self.admin_disabled.contains(&key) {
            return false;
        }
        self.state
            .get(&key)
            .map(|e| e.status != TrunkHealthStatus::Down)
            .unwrap_or(false)
    }

    /// Returns `true` if the trunk is in Degraded state.
    pub fn is_degraded(&self, trunk: &str) -> bool {
        self.state
            .get(&self.key_of(trunk))
            .map(|e| e.status == TrunkHealthStatus::Degraded)
            .unwrap_or(false)
    }

    /// Auto-recover Degraded trunks whose last failure is older than
    /// `recovery_interval_secs`. Returns names of recovered trunks.
    pub fn check_degraded_recovery(&self) -> Vec<String> {
        let now = Instant::now();
        let mut keys = Vec::new();
        for mut entry in self.state.iter_mut() {
            if entry.status != TrunkHealthStatus::Degraded {
                continue;
            }
            let should_recover = match entry.last_failure {
                Some(last_fail) => {
                    now.duration_since(last_fail).as_secs() >= entry.recovery_interval_secs
                }
                None => true,
            };
            if should_recover {
                entry.status = TrunkHealthStatus::Healthy;
                entry.consecutive_failures = 0;
                keys.push(entry.key().clone());
            }
        }
        // Names, because this is what the logs and the operator read.
        keys.iter().map(|key| self.name_of(key)).collect()
    }

    /// Get the current health status for a trunk.
    pub fn status(&self, trunk: &str) -> Option<TrunkHealthStatus> {
        self.state.get(&self.key_of(trunk)).map(|e| e.status)
    }

    /// Returns `true` if at least one trunk is healthy (not Down, not admin-disabled),
    /// or no trunks exist.
    pub fn any_healthy(&self) -> bool {
        if self.state.is_empty() {
            return true;
        }
        self.state
            .iter()
            .any(|e| e.status != TrunkHealthStatus::Down && !self.admin_disabled.contains(e.key()))
    }

    /// Administratively disable a trunk. Returns `false` if the trunk name is unknown.
    pub fn admin_disable(&self, name: &str) -> bool {
        if !self.state.contains_key(&self.key_of(name)) {
            return false;
        }
        self.admin_disabled.insert(self.key_of(name));
        true
    }

    /// Re-enable an admin-disabled trunk. Returns `false` if the trunk name is unknown.
    pub fn admin_enable(&self, name: &str) -> bool {
        if !self.state.contains_key(&self.key_of(name)) {
            return false;
        }
        self.admin_disabled.remove(&self.key_of(name));
        true
    }

    /// Returns `true` if the trunk is currently admin-disabled.
    pub fn is_admin_disabled(&self, name: &str) -> bool {
        self.admin_disabled.contains(&self.key_of(name))
    }

    /// Returns all known trunk names.
    pub fn trunk_names(&self) -> Vec<String> {
        self.state.iter().map(|e| self.name_of(e.key())).collect()
    }

    /// Returns the key of every trunk this monitor watches.
    pub fn trunk_keys(&self) -> Vec<String> {
        self.state.iter().map(|e| e.key().clone()).collect()
    }

    /// Returns the consecutive failure count for a trunk, or 0 if unknown.
    pub fn consecutive_failures(&self, name: &str) -> u32 {
        self.state
            .get(&self.key_of(name))
            .map(|e| e.consecutive_failures)
            .unwrap_or(0)
    }

    /// Get concurrent calls for a trunk.
    pub fn concurrent_calls(&self, trunk: &str) -> u64 {
        self.state
            .get(&self.key_of(trunk))
            .map(|e| e.concurrent_calls.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Increment concurrent calls for a trunk.
    pub fn call_started(&self, trunk: &str) {
        if let Some(entry) = self.state.get(&self.key_of(trunk)) {
            entry.concurrent_calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Decrement concurrent calls for a trunk.
    pub fn call_ended(&self, trunk: &str) {
        if let Some(entry) = self.state.get(&self.key_of(trunk)) {
            // Saturating decrement
            loop {
                let current = entry.concurrent_calls.load(Ordering::Relaxed);
                if current == 0 {
                    break;
                }
                if entry
                    .concurrent_calls
                    .compare_exchange_weak(
                        current,
                        current - 1,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    break;
                }
            }
        }
    }

    /// Check if any trunk needs recovery probing (has been Down for
    /// longer than `recovery_interval_secs`).
    pub fn trunks_needing_probe(&self) -> Vec<String> {
        let now = Instant::now();
        self.state
            .iter()
            .filter_map(|entry| {
                if entry.status != TrunkHealthStatus::Down {
                    return None;
                }
                if let Some(last_fail) = entry.last_failure {
                    let recovery_secs = entry.recovery_interval_secs;
                    if now.duration_since(last_fail).as_secs() >= recovery_secs {
                        return Some(entry.key().clone());
                    }
                }
                None
            })
            .collect()
    }

    /// Returns true if the trunk is due for a health probe based on its
    /// state and configured interval.
    ///
    /// - Never-probed trunks: always due
    /// - Down trunks: use `recovery_interval_secs` (less aggressive)
    /// - Other trunks: use `health_check_interval_secs`
    pub fn should_probe(&self, trunk: &str) -> bool {
        if let Some(entry) = self.state.get(&self.key_of(trunk)) {
            if !entry.health_check_enabled {
                return false;
            }
            let interval = if entry.status == TrunkHealthStatus::Down {
                entry.recovery_interval_secs
            } else {
                entry.health_check_interval_secs
            };
            match entry.last_probe {
                Some(last) => Instant::now().duration_since(last).as_secs() >= interval,
                None => true,
            }
        } else {
            false
        }
    }

    /// Record that a probe was sent for the trunk (updates `last_probe`
    /// timestamp).
    pub fn mark_probed(&self, trunk: &str) {
        if let Some(mut entry) = self.state.get_mut(&self.key_of(trunk)) {
            entry.last_probe = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tenant_keying_tests {
    use super::*;
    use crate::trunk::TrunkConfig;

    /// `None` is a trunk a node read from its own TOML: no stored id.
    fn trunk(name: &str, id: Option<fcp_core::TrunkId>) -> TrunkConfig {
        TrunkConfig {
            name: name.to_string(),
            id: id.unwrap_or_else(|| fcp_core::TrunkId::from_string(String::new())),
            max_concurrent_calls: 1,
            ..Default::default()
        }
    }

    /// Two tenants' trunks of the same name keep their own health and
    /// their own capacity (`docs/TENANT_TRUNKS.md` §4).
    #[test]
    fn health_and_capacity_are_per_trunk_not_per_name() {
        let one = trunk(
            "carrier-a",
            Some(fcp_core::TrunkId::from_string("trunk-1".into())),
        );
        let two = trunk(
            "carrier-a",
            Some(fcp_core::TrunkId::from_string("trunk-2".into())),
        );
        let monitor = TrunkHealthMonitor::new(&[one.clone(), two.clone()]);

        monitor.call_started(&one.key());
        assert_eq!(monitor.concurrent_calls(&one.key()), 1);
        assert_eq!(
            monitor.concurrent_calls(&two.key()),
            0,
            "the other tenant's trunk is untouched"
        );

        for _ in 0..10 {
            monitor.record_failure(&one.key());
        }
        assert!(!monitor.is_healthy(&one.key()));
        assert!(monitor.is_healthy(&two.key()));
    }

    /// A platform trunk is still reachable by the name an operator typed,
    /// which is what the admin API and the logs have.
    #[test]
    fn a_platform_trunk_answers_to_its_name() {
        let carrier = trunk("carrier-a", None);
        let monitor = TrunkHealthMonitor::new(std::slice::from_ref(&carrier));
        assert_eq!(carrier.key(), "platform:carrier-a");

        monitor.call_started("carrier-a");
        assert_eq!(monitor.concurrent_calls(&carrier.key()), 1);
        assert!(monitor.admin_disable("carrier-a"));
        assert!(monitor.is_admin_disabled("carrier-a"));
        assert_eq!(monitor.trunk_names(), vec!["carrier-a".to_string()]);
        assert_eq!(monitor.name_of(&carrier.key()), "carrier-a");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_trunks() -> Vec<TrunkConfig> {
        vec![TrunkConfig {
            name: "carrier-a".into(),
            hosts: vec!["sip.carrier.com".into()],
            failover: crate::trunk::TrunkFailover {
                max_failures: 3,
                ..Default::default()
            },
            ..Default::default()
        }]
    }

    #[test]
    fn test_new_monitor_unknown_status() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        assert_eq!(
            monitor.status("carrier-a"),
            Some(TrunkHealthStatus::Unknown)
        );
    }

    #[test]
    fn test_success_transitions_to_healthy() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        monitor.record_success("carrier-a");
        assert_eq!(
            monitor.status("carrier-a"),
            Some(TrunkHealthStatus::Healthy)
        );
        assert!(monitor.is_healthy("carrier-a"));
    }

    #[test]
    fn test_failures_transition_to_down() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        monitor.record_failure("carrier-a");
        assert!(monitor.is_healthy("carrier-a")); // 1 failure < 3 max
        monitor.record_failure("carrier-a");
        // 2 failures >= max_failures/2 (3/2=1), should be Degraded
        assert_eq!(
            monitor.status("carrier-a"),
            Some(TrunkHealthStatus::Degraded)
        );
        monitor.record_failure("carrier-a");
        // 3 failures >= max_failures, should be Down
        assert_eq!(monitor.status("carrier-a"), Some(TrunkHealthStatus::Down));
        assert!(!monitor.is_healthy("carrier-a"));
    }

    #[test]
    fn test_success_resets_failures() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        monitor.record_failure("carrier-a");
        monitor.record_failure("carrier-a");
        monitor.record_success("carrier-a");
        assert_eq!(
            monitor.status("carrier-a"),
            Some(TrunkHealthStatus::Healthy)
        );
    }

    #[test]
    fn test_any_healthy_empty() {
        let monitor = TrunkHealthMonitor::new(&[]);
        assert!(monitor.any_healthy());
    }

    #[test]
    fn test_any_healthy_with_one_down() {
        let trunks = vec![
            TrunkConfig {
                name: "a".into(),
                hosts: vec!["h".into()],
                failover: crate::trunk::TrunkFailover {
                    max_failures: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
            TrunkConfig {
                name: "b".into(),
                hosts: vec!["h".into()],
                ..Default::default()
            },
        ];
        let monitor = TrunkHealthMonitor::new(&trunks);
        monitor.record_failure("a"); // goes Down (max_failures=1)
        assert!(monitor.any_healthy()); // "b" is still Unknown (not Down)
    }

    #[test]
    fn test_concurrent_calls() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        assert_eq!(monitor.concurrent_calls("carrier-a"), 0);
        monitor.call_started("carrier-a");
        assert_eq!(monitor.concurrent_calls("carrier-a"), 1);
        monitor.call_ended("carrier-a");
        assert_eq!(monitor.concurrent_calls("carrier-a"), 0);
    }

    #[test]
    fn test_unknown_trunk_not_healthy() {
        let monitor = TrunkHealthMonitor::new(&[]);
        assert!(!monitor.is_healthy("nonexistent"));
    }

    #[test]
    fn test_should_probe_never_probed() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        // Never-probed trunk with health_check_enabled=true should be due
        assert!(monitor.should_probe("carrier-a"));
    }

    #[test]
    fn test_should_probe_disabled() {
        let trunks = vec![TrunkConfig {
            name: "no-probe".into(),
            hosts: vec!["h".into()],
            failover: crate::trunk::TrunkFailover {
                health_check_enabled: false,
                ..Default::default()
            },
            ..Default::default()
        }];
        let monitor = TrunkHealthMonitor::new(&trunks);
        assert!(!monitor.should_probe("no-probe"));
    }

    #[test]
    fn test_should_probe_not_due_yet() {
        let trunks = vec![TrunkConfig {
            name: "recent".into(),
            hosts: vec!["h".into()],
            failover: crate::trunk::TrunkFailover {
                health_check_interval_secs: 60,
                ..Default::default()
            },
            ..Default::default()
        }];
        let monitor = TrunkHealthMonitor::new(&trunks);
        monitor.mark_probed("recent");
        // Just probed — should not be due for another 60 seconds
        assert!(!monitor.should_probe("recent"));
    }

    #[test]
    fn test_should_probe_unknown_trunk() {
        let monitor = TrunkHealthMonitor::new(&[]);
        assert!(!monitor.should_probe("nonexistent"));
    }

    #[test]
    fn test_mark_probed_updates_last_probe() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        assert!(monitor.should_probe("carrier-a")); // never probed
        monitor.mark_probed("carrier-a");
        // With default interval of 30s, just-probed trunk should not be due
        assert!(!monitor.should_probe("carrier-a"));
    }

    #[test]
    fn test_is_degraded() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        // Unknown is not degraded
        assert!(!monitor.is_degraded("carrier-a"));
        // Push to Degraded (2 failures with max_failures=3, 2 >= 3/2)
        monitor.record_failure("carrier-a");
        monitor.record_failure("carrier-a");
        assert!(monitor.is_degraded("carrier-a"));
        // Push to Down (3 failures)
        monitor.record_failure("carrier-a");
        assert!(!monitor.is_degraded("carrier-a"));
    }

    #[test]
    fn test_check_degraded_recovery_no_failures() {
        let trunks = vec![TrunkConfig {
            name: "trunk-a".into(),
            hosts: vec!["h".into()],
            failover: crate::trunk::TrunkFailover {
                max_failures: 4,
                recovery_interval_secs: 60,
                ..Default::default()
            },
            ..Default::default()
        }];
        let monitor = TrunkHealthMonitor::new(&trunks);
        // Manually push to Degraded via failures then clear last_failure scenario:
        // 2 failures with max_failures=4 → 2 >= 4/2=2 → Degraded
        monitor.record_failure("trunk-a");
        monitor.record_failure("trunk-a");
        assert!(monitor.is_degraded("trunk-a"));
        // last_failure is set, but recovery_interval is 60s — won't recover yet
        let recovered = monitor.check_degraded_recovery();
        assert!(recovered.is_empty());
    }

    #[test]
    fn test_check_degraded_recovery_with_zero_interval() {
        let trunks = vec![TrunkConfig {
            name: "trunk-a".into(),
            hosts: vec!["h".into()],
            failover: crate::trunk::TrunkFailover {
                max_failures: 4,
                recovery_interval_secs: 0,
                ..Default::default()
            },
            ..Default::default()
        }];
        let monitor = TrunkHealthMonitor::new(&trunks);
        monitor.record_failure("trunk-a");
        monitor.record_failure("trunk-a");
        assert!(monitor.is_degraded("trunk-a"));
        // recovery_interval_secs=0 means recover immediately
        let recovered = monitor.check_degraded_recovery();
        assert_eq!(recovered, vec!["trunk-a".to_string()]);
        assert_eq!(monitor.status("trunk-a"), Some(TrunkHealthStatus::Healthy));
    }

    #[test]
    fn test_check_degraded_recovery_recent_failure() {
        let trunks = vec![TrunkConfig {
            name: "trunk-a".into(),
            hosts: vec!["h".into()],
            failover: crate::trunk::TrunkFailover {
                max_failures: 4,
                recovery_interval_secs: 3600, // 1 hour
                ..Default::default()
            },
            ..Default::default()
        }];
        let monitor = TrunkHealthMonitor::new(&trunks);
        monitor.record_failure("trunk-a");
        monitor.record_failure("trunk-a");
        assert!(monitor.is_degraded("trunk-a"));
        // Recent failure with long recovery interval — should NOT recover
        let recovered = monitor.check_degraded_recovery();
        assert!(recovered.is_empty());
        assert!(monitor.is_degraded("trunk-a"));
    }

    #[test]
    fn test_admin_disable_makes_unhealthy() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        monitor.record_success("carrier-a");
        assert!(monitor.is_healthy("carrier-a"));

        assert!(monitor.admin_disable("carrier-a"));
        assert!(!monitor.is_healthy("carrier-a"));
    }

    #[test]
    fn test_admin_enable_restores() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        monitor.record_success("carrier-a");
        monitor.admin_disable("carrier-a");
        assert!(!monitor.is_healthy("carrier-a"));

        assert!(monitor.admin_enable("carrier-a"));
        assert!(monitor.is_healthy("carrier-a"));
    }

    #[test]
    fn test_admin_disable_nonexistent() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        assert!(!monitor.admin_disable("nonexistent"));
        assert!(!monitor.admin_enable("nonexistent"));
    }

    #[test]
    fn test_admin_disabled_excluded_from_any_healthy() {
        let trunks = vec![TrunkConfig {
            name: "only-trunk".into(),
            hosts: vec!["h".into()],
            ..Default::default()
        }];
        let monitor = TrunkHealthMonitor::new(&trunks);
        monitor.record_success("only-trunk");
        assert!(monitor.any_healthy());

        monitor.admin_disable("only-trunk");
        assert!(!monitor.any_healthy());
    }

    #[test]
    fn test_trunk_names() {
        let trunks = vec![
            TrunkConfig {
                name: "a".into(),
                hosts: vec!["h".into()],
                ..Default::default()
            },
            TrunkConfig {
                name: "b".into(),
                hosts: vec!["h".into()],
                ..Default::default()
            },
        ];
        let monitor = TrunkHealthMonitor::new(&trunks);
        let mut names = monitor.trunk_names();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn test_is_admin_disabled() {
        let monitor = TrunkHealthMonitor::new(&test_trunks());
        assert!(!monitor.is_admin_disabled("carrier-a"));
        monitor.admin_disable("carrier-a");
        assert!(monitor.is_admin_disabled("carrier-a"));
        monitor.admin_enable("carrier-a");
        assert!(!monitor.is_admin_disabled("carrier-a"));
    }
}
