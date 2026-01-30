//! Trunk health monitoring for the SBC.
//!
//! Tracks per-trunk failure counts, success/failure timestamps, and health
//! status transitions. Optionally runs periodic OPTIONS probe health checks.

use dashmap::DashMap;
use fcp_sbc_core::metrics::TrunkHealthStatus;
use fcp_sbc_core::trunk::TrunkConfig;
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
pub struct TrunkHealthMonitor {
    state: DashMap<String, TrunkHealthState>,
}

impl TrunkHealthMonitor {
    /// Create a new monitor pre-populated with trunk names.
    pub fn new(trunks: &[TrunkConfig]) -> Self {
        let state = DashMap::new();
        for trunk in trunks {
            state.insert(
                trunk.name.clone(),
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
        Self { state }
    }

    /// Record a failure for the given trunk.
    ///
    /// If consecutive failures reach `max_failures`, transitions to `Down`.
    pub fn record_failure(&self, trunk: &str) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
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
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.consecutive_failures = 0;
            entry.last_success = Some(Instant::now());
            entry.status = TrunkHealthStatus::Healthy;
        }
    }

    /// Returns `true` if the trunk is healthy (not `Down`).
    pub fn is_healthy(&self, trunk: &str) -> bool {
        self.state
            .get(trunk)
            .map(|e| e.status != TrunkHealthStatus::Down)
            .unwrap_or(false)
    }

    /// Get the current health status for a trunk.
    pub fn status(&self, trunk: &str) -> Option<TrunkHealthStatus> {
        self.state.get(trunk).map(|e| e.status)
    }

    /// Returns `true` if at least one trunk is healthy, or no trunks exist.
    pub fn any_healthy(&self) -> bool {
        if self.state.is_empty() {
            return true;
        }
        self.state
            .iter()
            .any(|e| e.status != TrunkHealthStatus::Down)
    }

    /// Get concurrent calls for a trunk.
    pub fn concurrent_calls(&self, trunk: &str) -> u64 {
        self.state
            .get(trunk)
            .map(|e| e.concurrent_calls.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Increment concurrent calls for a trunk.
    pub fn call_started(&self, trunk: &str) {
        if let Some(entry) = self.state.get(trunk) {
            entry.concurrent_calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Decrement concurrent calls for a trunk.
    pub fn call_ended(&self, trunk: &str) {
        if let Some(entry) = self.state.get(trunk) {
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
        if let Some(entry) = self.state.get(trunk) {
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
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.last_probe = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_trunks() -> Vec<TrunkConfig> {
        vec![TrunkConfig {
            name: "carrier-a".into(),
            hosts: vec!["sip.carrier.com".into()],
            failover: fcp_sbc_core::trunk::TrunkFailover {
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
                failover: fcp_sbc_core::trunk::TrunkFailover {
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
            failover: fcp_sbc_core::trunk::TrunkFailover {
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
            failover: fcp_sbc_core::trunk::TrunkFailover {
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
}
