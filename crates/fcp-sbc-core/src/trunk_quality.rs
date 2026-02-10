//! Per-trunk quality metrics with sliding window.
//!
//! Tracks ASR (Answer Seizure Ratio), ACD (Average Call Duration), and
//! PDD (Post-Dial Delay) over a configurable time window. Used by the
//! LCR engine to filter low-quality trunks and by the admin API to
//! expose per-trunk quality snapshots.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// A single call outcome sample in the sliding window.
struct CallSample {
    timestamp: Instant,
    answered: bool,
    duration_secs: f64,
    post_dial_delay_ms: u64,
    sip_code: u16,
}

/// Per-trunk quality state with sliding window.
struct TrunkQualityState {
    samples: VecDeque<CallSample>,
    window_secs: u64,
    round_robin_counter: AtomicU64,
    concurrent_calls: AtomicU64,
}

/// Computed quality snapshot for a trunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrunkQualitySnapshot {
    pub total_calls: u64,
    pub answered_calls: u64,
    pub failed_calls: u64,
    pub asr: f64,
    pub acd_secs: f64,
    pub avg_pdd_ms: f64,
    pub concurrent_calls: u64,
}

/// Per-trunk quality tracker with sliding window metrics.
pub struct TrunkQualityTracker {
    state: DashMap<String, TrunkQualityState>,
    _window_secs: u64,
}

impl TrunkQualityTracker {
    /// Create a new tracker for the given trunk names.
    pub fn new(trunk_names: &[String], window_secs: u64) -> Self {
        let state = DashMap::new();
        for name in trunk_names {
            state.insert(
                name.clone(),
                TrunkQualityState {
                    samples: VecDeque::new(),
                    window_secs,
                    round_robin_counter: AtomicU64::new(0),
                    concurrent_calls: AtomicU64::new(0),
                },
            );
        }
        Self { state, _window_secs: window_secs }
    }

    /// Record that a call attempt was initiated on this trunk.
    pub fn record_call_attempt(&self, trunk: &str) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.concurrent_calls.fetch_add(1, Ordering::Relaxed);
            entry.samples.push_back(CallSample {
                timestamp: Instant::now(),
                answered: false,
                duration_secs: 0.0,
                post_dial_delay_ms: 0,
                sip_code: 0,
            });
        }
    }

    /// Record that the most recent call on this trunk was answered.
    pub fn record_call_answered(&self, trunk: &str, pdd_ms: u64) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            // Find the most recent unanswered sample and mark it answered
            if let Some(sample) = entry.samples.iter_mut().rev().find(|s| !s.answered && s.sip_code == 0) {
                sample.answered = true;
                sample.post_dial_delay_ms = pdd_ms;
            }
        }
    }

    /// Record that a call on this trunk ended successfully.
    pub fn record_call_ended(&self, trunk: &str, duration_secs: f64, sip_code: u16) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            // Decrement concurrent calls (saturating)
            loop {
                let current = entry.concurrent_calls.load(Ordering::Relaxed);
                if current == 0 {
                    break;
                }
                if entry
                    .concurrent_calls
                    .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    break;
                }
            }

            // Update the most recent answered sample with duration
            if let Some(sample) = entry.samples.iter_mut().rev().find(|s| s.answered && s.sip_code == 0) {
                sample.duration_secs = duration_secs;
                sample.sip_code = sip_code;
            }
        }
    }

    /// Record that a call on this trunk failed.
    pub fn record_call_failed(&self, trunk: &str, sip_code: u16) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            // Decrement concurrent calls (saturating)
            loop {
                let current = entry.concurrent_calls.load(Ordering::Relaxed);
                if current == 0 {
                    break;
                }
                if entry
                    .concurrent_calls
                    .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    break;
                }
            }

            // Mark the most recent unanswered sample with the failure code
            if let Some(sample) = entry.samples.iter_mut().rev().find(|s| !s.answered && s.sip_code == 0) {
                sample.sip_code = sip_code;
            }
        }
    }

    /// Compute a quality snapshot for a trunk.
    pub fn snapshot(&self, trunk: &str) -> Option<TrunkQualitySnapshot> {
        let entry = self.state.get(trunk)?;
        let now = Instant::now();
        let window = std::time::Duration::from_secs(entry.window_secs);

        let mut total = 0u64;
        let mut answered = 0u64;
        let mut failed = 0u64;
        let mut total_duration = 0.0f64;
        let mut total_pdd = 0u64;
        let mut pdd_count = 0u64;

        for sample in entry.samples.iter() {
            if now.duration_since(sample.timestamp) > window {
                continue;
            }
            total += 1;
            if sample.answered {
                answered += 1;
                total_duration += sample.duration_secs;
                if sample.post_dial_delay_ms > 0 {
                    total_pdd += sample.post_dial_delay_ms;
                    pdd_count += 1;
                }
            } else if sample.sip_code > 0 {
                failed += 1;
            }
        }

        let asr = if total > 0 {
            answered as f64 / total as f64
        } else {
            0.0
        };
        let acd_secs = if answered > 0 {
            total_duration / answered as f64
        } else {
            0.0
        };
        let avg_pdd_ms = if pdd_count > 0 {
            total_pdd as f64 / pdd_count as f64
        } else {
            0.0
        };

        Some(TrunkQualitySnapshot {
            total_calls: total,
            answered_calls: answered,
            failed_calls: failed,
            asr,
            acd_secs,
            avg_pdd_ms,
            concurrent_calls: entry.concurrent_calls.load(Ordering::Relaxed),
        })
    }

    /// Quick ASR lookup for routing decisions.
    pub fn asr(&self, trunk: &str) -> f64 {
        self.snapshot(trunk).map(|s| s.asr).unwrap_or(0.0)
    }

    /// Get the total number of calls in the window for a trunk.
    pub fn total_calls_in_window(&self, trunk: &str) -> u64 {
        self.snapshot(trunk).map(|s| s.total_calls).unwrap_or(0)
    }

    /// Get the concurrent call count for a trunk.
    pub fn concurrent_calls(&self, trunk: &str) -> u64 {
        self.state
            .get(trunk)
            .map(|e| e.concurrent_calls.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Increment and return the round-robin counter for a trunk.
    pub fn next_round_robin(&self, trunk: &str) -> u64 {
        self.state
            .get(trunk)
            .map(|e| e.round_robin_counter.fetch_add(1, Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Remove samples older than the window from all trunks.
    pub fn prune_expired(&self) {
        let now = Instant::now();
        for mut entry in self.state.iter_mut() {
            let window = std::time::Duration::from_secs(entry.window_secs);
            while let Some(front) = entry.samples.front() {
                if now.duration_since(front.timestamp) > window {
                    entry.samples.pop_front();
                } else {
                    break;
                }
            }
        }
    }

    /// Get all trunk names tracked by this quality tracker.
    pub fn trunk_names(&self) -> Vec<String> {
        self.state.iter().map(|e| e.key().clone()).collect()
    }

    /// Get snapshots for all trunks.
    pub fn all_snapshots(&self) -> std::collections::HashMap<String, TrunkQualitySnapshot> {
        let mut result = std::collections::HashMap::new();
        for entry in self.state.iter() {
            if let Some(snap) = self.snapshot(entry.key()) {
                result.insert(entry.key().clone(), snap);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker(names: &[&str]) -> TrunkQualityTracker {
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        TrunkQualityTracker::new(&names, 3600)
    }

    #[test]
    fn test_new_tracker() {
        let t = tracker(&["a", "b"]);
        assert!(t.snapshot("a").is_some());
        assert!(t.snapshot("b").is_some());
        assert!(t.snapshot("c").is_none());
    }

    #[test]
    fn test_asr_no_samples() {
        let t = tracker(&["a"]);
        let snap = t.snapshot("a").unwrap();
        assert_eq!(snap.total_calls, 0);
        assert_eq!(snap.asr, 0.0);
    }

    #[test]
    fn test_asr_all_answered() {
        let t = tracker(&["a"]);
        for _ in 0..10 {
            t.record_call_attempt("a");
            t.record_call_answered("a", 200);
            t.record_call_ended("a", 60.0, 200);
        }
        let snap = t.snapshot("a").unwrap();
        assert_eq!(snap.total_calls, 10);
        assert_eq!(snap.answered_calls, 10);
        assert!((snap.asr - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_asr_mixed() {
        let t = tracker(&["a"]);
        // 7 answered, 3 failed
        for _ in 0..7 {
            t.record_call_attempt("a");
            t.record_call_answered("a", 150);
            t.record_call_ended("a", 45.0, 200);
        }
        for _ in 0..3 {
            t.record_call_attempt("a");
            t.record_call_failed("a", 503);
        }
        let snap = t.snapshot("a").unwrap();
        assert_eq!(snap.total_calls, 10);
        assert_eq!(snap.answered_calls, 7);
        assert_eq!(snap.failed_calls, 3);
        assert!((snap.asr - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_acd() {
        let t = tracker(&["a"]);
        t.record_call_attempt("a");
        t.record_call_answered("a", 100);
        t.record_call_ended("a", 120.0, 200);

        t.record_call_attempt("a");
        t.record_call_answered("a", 150);
        t.record_call_ended("a", 60.0, 200);

        let snap = t.snapshot("a").unwrap();
        assert!((snap.acd_secs - 90.0).abs() < 0.01); // (120 + 60) / 2
    }

    #[test]
    fn test_pdd() {
        let t = tracker(&["a"]);
        t.record_call_attempt("a");
        t.record_call_answered("a", 200);
        t.record_call_ended("a", 30.0, 200);

        t.record_call_attempt("a");
        t.record_call_answered("a", 400);
        t.record_call_ended("a", 30.0, 200);

        let snap = t.snapshot("a").unwrap();
        assert!((snap.avg_pdd_ms - 300.0).abs() < 0.01); // (200 + 400) / 2
    }

    #[test]
    fn test_concurrent_calls() {
        let t = tracker(&["a"]);
        assert_eq!(t.concurrent_calls("a"), 0);

        t.record_call_attempt("a");
        assert_eq!(t.concurrent_calls("a"), 1);

        t.record_call_attempt("a");
        assert_eq!(t.concurrent_calls("a"), 2);

        t.record_call_answered("a", 100);
        t.record_call_ended("a", 30.0, 200);
        assert_eq!(t.concurrent_calls("a"), 1);

        t.record_call_failed("a", 503);
        assert_eq!(t.concurrent_calls("a"), 0);
    }

    #[test]
    fn test_round_robin() {
        let t = tracker(&["a"]);
        assert_eq!(t.next_round_robin("a"), 0);
        assert_eq!(t.next_round_robin("a"), 1);
        assert_eq!(t.next_round_robin("a"), 2);
    }

    #[test]
    fn test_window_expiry() {
        // Use a very short window
        let names = vec!["a".to_string()];
        let t = TrunkQualityTracker::new(&names, 1); // 1 second window

        t.record_call_attempt("a");
        t.record_call_answered("a", 100);
        t.record_call_ended("a", 30.0, 200);

        // Immediately after, sample should be in window
        let snap = t.snapshot("a").unwrap();
        assert_eq!(snap.total_calls, 1);

        // Sleep slightly over 1 second
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // After window expires, sample should be excluded
        let snap = t.snapshot("a").unwrap();
        assert_eq!(snap.total_calls, 0);
    }

    #[test]
    fn test_prune_expired() {
        let names = vec!["a".to_string()];
        let t = TrunkQualityTracker::new(&names, 1);

        t.record_call_attempt("a");
        t.record_call_answered("a", 100);
        t.record_call_ended("a", 30.0, 200);

        std::thread::sleep(std::time::Duration::from_millis(1100));

        t.prune_expired();

        // After pruning, the VecDeque should be empty
        let entry = t.state.get("a").unwrap();
        assert!(entry.samples.is_empty());
    }

    #[test]
    fn test_all_snapshots() {
        let t = tracker(&["a", "b"]);
        t.record_call_attempt("a");
        t.record_call_answered("a", 100);
        t.record_call_ended("a", 30.0, 200);

        let snaps = t.all_snapshots();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps["a"].total_calls, 1);
        assert_eq!(snaps["b"].total_calls, 0);
    }

    #[test]
    fn test_unknown_trunk() {
        let t = tracker(&["a"]);
        // Operations on unknown trunks should be no-ops
        t.record_call_attempt("nonexistent");
        t.record_call_answered("nonexistent", 100);
        t.record_call_ended("nonexistent", 30.0, 200);
        t.record_call_failed("nonexistent", 503);
        assert!(t.snapshot("nonexistent").is_none());
        assert_eq!(t.concurrent_calls("nonexistent"), 0);
    }
}
