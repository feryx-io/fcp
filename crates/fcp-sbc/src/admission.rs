//! Admission control for the SBC.
//!
//! Tracks global calls-per-second (CPS) via a sliding window and concurrent
//! call/registration counts via atomics. Used by `SbcEnforcementHandler` to
//! reject traffic when the SBC is overloaded.

use fcp_sbc_core::AdmissionConfig;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Runtime admission controller.
///
/// Thread-safe: atomics for counters, mutex only for CPS sliding windows.
/// Supports both global CPS limits and per-trunk CPS limits.
pub struct AdmissionController {
    config: AdmissionConfig,
    concurrent_calls: AtomicU64,
    concurrent_registrations: AtomicU64,
    /// Sliding window of call-start timestamps for global CPS calculation.
    cps_window: Mutex<VecDeque<Instant>>,
    /// Per-trunk CPS limits (trunk_name -> max_cps).
    trunk_cps_limits: HashMap<String, u32>,
    /// Per-trunk sliding windows (trunk_name -> timestamps).
    trunk_cps_windows: HashMap<String, Mutex<VecDeque<Instant>>>,
}

impl AdmissionController {
    pub fn new(config: AdmissionConfig) -> Self {
        let mut trunk_cps_limits = HashMap::new();
        let mut trunk_cps_windows = HashMap::new();

        for entry in &config.per_trunk_cps {
            trunk_cps_limits.insert(entry.trunk_name.clone(), entry.max_cps);
            trunk_cps_windows.insert(entry.trunk_name.clone(), Mutex::new(VecDeque::new()));
        }

        Self {
            config,
            concurrent_calls: AtomicU64::new(0),
            concurrent_registrations: AtomicU64::new(0),
            cps_window: Mutex::new(VecDeque::new()),
            trunk_cps_limits,
            trunk_cps_windows,
        }
    }

    /// Try to admit a new call. Returns `true` if allowed.
    ///
    /// Checks:
    /// 1. Concurrent call limit
    /// 2. CPS sliding window
    pub fn try_admit_call(&self) -> bool {
        // Check concurrent calls
        let current = self.concurrent_calls.load(Ordering::Relaxed);
        if current >= self.config.max_concurrent_calls as u64 {
            return false;
        }

        // Check CPS
        let now = Instant::now();
        let mut window = self.cps_window.lock().unwrap();

        // Prune entries older than 1 second
        while let Some(&front) = window.front() {
            if now.duration_since(front).as_secs() >= 1 {
                window.pop_front();
            } else {
                break;
            }
        }

        if window.len() as u32 >= self.config.max_cps {
            return false;
        }

        window.push_back(now);
        true
    }

    /// Try to admit a new registration. Returns `true` if allowed.
    pub fn try_admit_registration(&self) -> bool {
        let current = self.concurrent_registrations.load(Ordering::Relaxed);
        current < self.config.max_concurrent_registrations as u64
    }

    /// Record that a call has started (increment concurrent count).
    pub fn call_started(&self) {
        self.concurrent_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a call has ended (decrement concurrent count).
    pub fn call_ended(&self) {
        // Saturating sub via compare-exchange loop
        loop {
            let current = self.concurrent_calls.load(Ordering::Relaxed);
            if current == 0 {
                break;
            }
            if self
                .concurrent_calls
                .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
    }

    /// Record that a registration was added.
    pub fn registration_added(&self) {
        self.concurrent_registrations
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a registration was removed.
    pub fn registration_removed(&self) {
        loop {
            let current = self.concurrent_registrations.load(Ordering::Relaxed);
            if current == 0 {
                break;
            }
            if self
                .concurrent_registrations
                .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
    }

    /// Current concurrent calls count.
    pub fn concurrent_calls(&self) -> u64 {
        self.concurrent_calls.load(Ordering::Relaxed)
    }

    /// Current concurrent registrations count.
    pub fn concurrent_registrations(&self) -> u64 {
        self.concurrent_registrations.load(Ordering::Relaxed)
    }

    /// Current CPS (calls in the last second).
    pub fn current_cps(&self) -> u64 {
        let now = Instant::now();
        let window = self.cps_window.lock().unwrap();
        window
            .iter()
            .filter(|&&ts| now.duration_since(ts).as_secs() < 1)
            .count() as u64
    }

    /// Check whether a trunk is within its per-trunk CPS limit.
    ///
    /// Returns `true` if the trunk has no CPS limit configured or if the
    /// current call rate is below the limit. Does NOT record a new call;
    /// use [`record_trunk_call`] after the trunk is actually selected.
    pub fn is_trunk_within_cps(&self, trunk_name: &str) -> bool {
        let limit = match self.trunk_cps_limits.get(trunk_name) {
            Some(&limit) => limit,
            None => return true, // no limit configured
        };

        let window = match self.trunk_cps_windows.get(trunk_name) {
            Some(w) => w,
            None => return true,
        };

        let now = Instant::now();
        let mut guard = window.lock().unwrap();

        // Prune entries older than 1 second
        while let Some(&front) = guard.front() {
            if now.duration_since(front).as_secs() >= 1 {
                guard.pop_front();
            } else {
                break;
            }
        }

        (guard.len() as u32) < limit
    }

    /// Record a call attempt for a trunk's CPS sliding window.
    ///
    /// Call this after a trunk has been selected for an outbound call.
    pub fn record_trunk_call(&self, trunk_name: &str) {
        if let Some(window) = self.trunk_cps_windows.get(trunk_name) {
            let mut guard = window.lock().unwrap();
            guard.push_back(Instant::now());
        }
    }

    /// Current CPS for a specific trunk (calls in the last second).
    pub fn trunk_cps(&self, trunk_name: &str) -> u64 {
        let window = match self.trunk_cps_windows.get(trunk_name) {
            Some(w) => w,
            None => return 0,
        };
        let now = Instant::now();
        let guard = window.lock().unwrap();
        guard
            .iter()
            .filter(|&&ts| now.duration_since(ts).as_secs() < 1)
            .count() as u64
    }

    /// Access the admission config.
    pub fn config(&self) -> &AdmissionConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_sbc_core::TrunkCpsLimit;

    fn default_controller() -> AdmissionController {
        AdmissionController::new(AdmissionConfig::default())
    }

    fn controller_with_trunk_cps(trunk_name: &str, max_cps: u32) -> AdmissionController {
        let config = AdmissionConfig {
            per_trunk_cps: vec![TrunkCpsLimit {
                trunk_name: trunk_name.into(),
                max_cps,
            }],
            ..Default::default()
        };
        AdmissionController::new(config)
    }

    #[test]
    fn test_admit_call_under_limits() {
        let ctrl = default_controller();
        assert!(ctrl.try_admit_call());
    }

    #[test]
    fn test_concurrent_call_tracking() {
        let ctrl = default_controller();
        ctrl.call_started();
        assert_eq!(ctrl.concurrent_calls(), 1);
        ctrl.call_started();
        assert_eq!(ctrl.concurrent_calls(), 2);
        ctrl.call_ended();
        assert_eq!(ctrl.concurrent_calls(), 1);
        ctrl.call_ended();
        assert_eq!(ctrl.concurrent_calls(), 0);
        // Underflow protection
        ctrl.call_ended();
        assert_eq!(ctrl.concurrent_calls(), 0);
    }

    #[test]
    fn test_concurrent_registration_tracking() {
        let ctrl = default_controller();
        ctrl.registration_added();
        assert_eq!(ctrl.concurrent_registrations(), 1);
        ctrl.registration_removed();
        assert_eq!(ctrl.concurrent_registrations(), 0);
        ctrl.registration_removed();
        assert_eq!(ctrl.concurrent_registrations(), 0);
    }

    #[test]
    fn test_concurrent_call_limit() {
        let config = AdmissionConfig {
            max_concurrent_calls: 2,
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);
        ctrl.call_started();
        ctrl.call_started();
        // At limit: new admission should fail
        assert!(!ctrl.try_admit_call());
        ctrl.call_ended();
        assert!(ctrl.try_admit_call());
    }

    #[test]
    fn test_registration_limit() {
        let config = AdmissionConfig {
            max_concurrent_registrations: 1,
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);
        assert!(ctrl.try_admit_registration());
        ctrl.registration_added();
        assert!(!ctrl.try_admit_registration());
        ctrl.registration_removed();
        assert!(ctrl.try_admit_registration());
    }

    #[test]
    fn test_trunk_cps_no_limit_always_allowed() {
        let ctrl = default_controller(); // no per_trunk_cps entries
        assert!(ctrl.is_trunk_within_cps("any-trunk"));
    }

    #[test]
    fn test_trunk_cps_within_limit() {
        let ctrl = controller_with_trunk_cps("carrier-a", 5);
        ctrl.record_trunk_call("carrier-a");
        ctrl.record_trunk_call("carrier-a");
        assert!(ctrl.is_trunk_within_cps("carrier-a"));
        assert_eq!(ctrl.trunk_cps("carrier-a"), 2);
    }

    #[test]
    fn test_trunk_cps_at_limit() {
        let ctrl = controller_with_trunk_cps("carrier-a", 2);
        ctrl.record_trunk_call("carrier-a");
        ctrl.record_trunk_call("carrier-a");
        assert!(!ctrl.is_trunk_within_cps("carrier-a"));
    }

    #[test]
    fn test_trunk_cps_unknown_trunk_allowed() {
        let ctrl = controller_with_trunk_cps("carrier-a", 1);
        // "carrier-b" has no limit configured
        assert!(ctrl.is_trunk_within_cps("carrier-b"));
    }

    #[test]
    fn test_trunk_cps_independent_per_trunk() {
        let config = AdmissionConfig {
            per_trunk_cps: vec![
                TrunkCpsLimit {
                    trunk_name: "trunk-a".into(),
                    max_cps: 1,
                },
                TrunkCpsLimit {
                    trunk_name: "trunk-b".into(),
                    max_cps: 1,
                },
            ],
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);

        ctrl.record_trunk_call("trunk-a");
        // trunk-a is at limit, trunk-b is still open
        assert!(!ctrl.is_trunk_within_cps("trunk-a"));
        assert!(ctrl.is_trunk_within_cps("trunk-b"));
    }
}
