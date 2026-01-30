//! Admission control for the SBC.
//!
//! Tracks global calls-per-second (CPS) via a sliding window and concurrent
//! call/registration counts via atomics. Used by `SbcEnforcementHandler` to
//! reject traffic when the SBC is overloaded.

use fcp_sbc_core::AdmissionConfig;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Runtime admission controller.
///
/// Thread-safe: atomics for counters, mutex only for the CPS sliding window.
pub struct AdmissionController {
    config: AdmissionConfig,
    concurrent_calls: AtomicU64,
    concurrent_registrations: AtomicU64,
    /// Sliding window of call-start timestamps for CPS calculation.
    cps_window: Mutex<VecDeque<Instant>>,
}

impl AdmissionController {
    pub fn new(config: AdmissionConfig) -> Self {
        Self {
            config,
            concurrent_calls: AtomicU64::new(0),
            concurrent_registrations: AtomicU64::new(0),
            cps_window: Mutex::new(VecDeque::new()),
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
        self.concurrent_registrations.fetch_add(1, Ordering::Relaxed);
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

    /// Access the admission config.
    pub fn config(&self) -> &AdmissionConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_controller() -> AdmissionController {
        AdmissionController::new(AdmissionConfig::default())
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
}
