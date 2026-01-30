//! Runtime metrics collection for the SBC.
//!
//! Provides lock-free atomic counters for real-time metrics and a periodic
//! snapshot mechanism using `SbcMetricsSnapshot` from `fcp-sbc-core`.

use fcp_sbc_core::SbcMetricsSnapshot;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

/// Runtime SBC metrics with lock-free counters.
pub struct SbcRuntimeMetrics {
    snapshot: RwLock<SbcMetricsSnapshot>,
    calls_attempted: AtomicU64,
    calls_completed: AtomicU64,
    calls_rejected: AtomicU64,
    security_rejections: AtomicU64,
    rate_limit_rejections: AtomicU64,
    admission_rejections: AtomicU64,
    peak_cps: AtomicU64,
}

impl SbcRuntimeMetrics {
    pub fn new() -> Self {
        Self {
            snapshot: RwLock::new(SbcMetricsSnapshot::default()),
            calls_attempted: AtomicU64::new(0),
            calls_completed: AtomicU64::new(0),
            calls_rejected: AtomicU64::new(0),
            security_rejections: AtomicU64::new(0),
            rate_limit_rejections: AtomicU64::new(0),
            admission_rejections: AtomicU64::new(0),
            peak_cps: AtomicU64::new(0),
        }
    }

    pub fn record_call_attempt(&self) {
        self.calls_attempted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_call_completed(&self) {
        self.calls_completed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_call_rejected(&self) {
        self.calls_rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_security_rejection(&self) {
        self.security_rejections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_rate_limit_rejection(&self) {
        self.rate_limit_rejections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_admission_rejection(&self) {
        self.admission_rejections.fetch_add(1, Ordering::Relaxed);
    }

    /// Update the stored snapshot with current counter values.
    ///
    /// Called periodically by a background task.
    pub fn update_snapshot(
        &self,
        current_cps: u64,
        concurrent_calls: u64,
        concurrent_registrations: u64,
    ) {
        // Update peak CPS
        loop {
            let peak = self.peak_cps.load(Ordering::Relaxed);
            if current_cps <= peak {
                break;
            }
            if self
                .peak_cps
                .compare_exchange_weak(peak, current_cps, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }

        let snap = SbcMetricsSnapshot {
            current_cps,
            peak_cps: self.peak_cps.load(Ordering::Relaxed),
            total_calls_attempted: self.calls_attempted.load(Ordering::Relaxed),
            total_calls_completed: self.calls_completed.load(Ordering::Relaxed),
            total_calls_rejected: self.calls_rejected.load(Ordering::Relaxed),
            total_admission_rejections: self.admission_rejections.load(Ordering::Relaxed),
            total_rate_limit_rejections: self.rate_limit_rejections.load(Ordering::Relaxed),
            total_security_rejections: self.security_rejections.load(Ordering::Relaxed),
            concurrent_calls,
            concurrent_registrations,
        };

        if let Ok(mut guard) = self.snapshot.write() {
            *guard = snap;
        }
    }

    /// Return the most recent snapshot.
    pub fn snapshot(&self) -> SbcMetricsSnapshot {
        self.snapshot.read().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Default for SbcRuntimeMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counters() {
        let m = SbcRuntimeMetrics::new();
        m.record_call_attempt();
        m.record_call_attempt();
        m.record_call_completed();
        m.record_security_rejection();
        m.record_rate_limit_rejection();
        m.record_admission_rejection();
        m.record_call_rejected();

        m.update_snapshot(42, 10, 20);
        let snap = m.snapshot();

        assert_eq!(snap.total_calls_attempted, 2);
        assert_eq!(snap.total_calls_completed, 1);
        assert_eq!(snap.total_calls_rejected, 1);
        assert_eq!(snap.total_security_rejections, 1);
        assert_eq!(snap.total_rate_limit_rejections, 1);
        assert_eq!(snap.total_admission_rejections, 1);
        assert_eq!(snap.current_cps, 42);
        assert_eq!(snap.peak_cps, 42);
        assert_eq!(snap.concurrent_calls, 10);
        assert_eq!(snap.concurrent_registrations, 20);
    }

    #[test]
    fn test_peak_cps_tracks_maximum() {
        let m = SbcRuntimeMetrics::new();
        m.update_snapshot(100, 0, 0);
        assert_eq!(m.snapshot().peak_cps, 100);
        m.update_snapshot(50, 0, 0);
        assert_eq!(m.snapshot().peak_cps, 100);
        m.update_snapshot(200, 0, 0);
        assert_eq!(m.snapshot().peak_cps, 200);
    }
}
