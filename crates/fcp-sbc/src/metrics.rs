//! Runtime metrics collection for the SBC.
//!
//! Provides lock-free atomic counters for real-time metrics and a periodic
//! snapshot mechanism using `SbcMetricsSnapshot` from `fcp-sbc-core`.

use fcp_sbc_core::{PerSourceIpSnapshot, SbcMetricsConfig, SbcMetricsSnapshot};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Counters for a single source IP.
struct PerIpEntry {
    calls_attempted: u64,
    calls_rejected: u64,
    rate_limit_rejections: u64,
    security_rejections: u64,
    last_seen: Instant,
    last_seen_epoch_secs: u64,
}

/// Bounded store of per-IP counters with LRU eviction.
struct PerIpStore {
    entries: HashMap<IpAddr, PerIpEntry>,
    max_entries: usize,
}

impl PerIpStore {
    fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
        }
    }

    /// Get or insert an entry, evicting the oldest if at capacity.
    fn get_or_insert(&mut self, ip: IpAddr) -> &mut PerIpEntry {
        if !self.entries.contains_key(&ip) && self.entries.len() >= self.max_entries {
            // LRU eviction: remove the entry with the oldest last_seen
            if let Some(oldest_ip) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_seen)
                .map(|(ip, _)| *ip)
            {
                self.entries.remove(&oldest_ip);
            }
        }
        let now = Instant::now();
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.entries.entry(ip).or_insert(PerIpEntry {
            calls_attempted: 0,
            calls_rejected: 0,
            rate_limit_rejections: 0,
            security_rejections: 0,
            last_seen: now,
            last_seen_epoch_secs: epoch,
        })
    }
}

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
    per_ip_enabled: bool,
    per_ip: RwLock<PerIpStore>,
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
            per_ip_enabled: false,
            per_ip: RwLock::new(PerIpStore::new(10_000)),
        }
    }

    /// Create metrics with per-IP tracking configured from `SbcMetricsConfig`.
    #[allow(dead_code)]
    pub fn new_with_config(config: &SbcMetricsConfig) -> Self {
        Self {
            snapshot: RwLock::new(SbcMetricsSnapshot::default()),
            calls_attempted: AtomicU64::new(0),
            calls_completed: AtomicU64::new(0),
            calls_rejected: AtomicU64::new(0),
            security_rejections: AtomicU64::new(0),
            rate_limit_rejections: AtomicU64::new(0),
            admission_rejections: AtomicU64::new(0),
            peak_cps: AtomicU64::new(0),
            per_ip_enabled: config.per_source_ip_metrics,
            per_ip: RwLock::new(PerIpStore::new(config.max_source_ip_entries)),
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

    // ── Per-source-IP recording ──────────────────────────────────────────

    /// Record a call attempt from the given IP.
    pub fn record_per_ip_call_attempt(&self, ip: IpAddr) {
        if !self.per_ip_enabled {
            return;
        }
        if let Ok(mut store) = self.per_ip.write() {
            let entry = store.get_or_insert(ip);
            entry.calls_attempted += 1;
            entry.last_seen = Instant::now();
            entry.last_seen_epoch_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
    }

    /// Record a call rejection from the given IP.
    pub fn record_per_ip_rejection(&self, ip: IpAddr) {
        if !self.per_ip_enabled {
            return;
        }
        if let Ok(mut store) = self.per_ip.write() {
            let entry = store.get_or_insert(ip);
            entry.calls_rejected += 1;
            entry.last_seen = Instant::now();
            entry.last_seen_epoch_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
    }

    /// Record a rate-limit rejection from the given IP.
    pub fn record_per_ip_rate_limit(&self, ip: IpAddr) {
        if !self.per_ip_enabled {
            return;
        }
        if let Ok(mut store) = self.per_ip.write() {
            let entry = store.get_or_insert(ip);
            entry.rate_limit_rejections += 1;
            entry.last_seen = Instant::now();
            entry.last_seen_epoch_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
    }

    /// Record a security rejection from the given IP.
    pub fn record_per_ip_security(&self, ip: IpAddr) {
        if !self.per_ip_enabled {
            return;
        }
        if let Ok(mut store) = self.per_ip.write() {
            let entry = store.get_or_insert(ip);
            entry.security_rejections += 1;
            entry.last_seen = Instant::now();
            entry.last_seen_epoch_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
    }

    /// Return snapshots of all tracked per-source-IP metrics.
    pub fn per_ip_snapshots(&self) -> Vec<PerSourceIpSnapshot> {
        if !self.per_ip_enabled {
            return Vec::new();
        }
        let store = match self.per_ip.read() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        store
            .entries
            .iter()
            .map(|(ip, e)| PerSourceIpSnapshot {
                ip: ip.to_string(),
                calls_attempted: e.calls_attempted,
                calls_rejected: e.calls_rejected,
                rate_limit_rejections: e.rate_limit_rejections,
                security_rejections: e.security_rejections,
                last_seen_epoch_secs: e.last_seen_epoch_secs,
            })
            .collect()
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

// Re-export for admin API
impl SbcRuntimeMetrics {
    /// Whether per-IP tracking is enabled.
    #[allow(dead_code)]
    pub fn per_ip_enabled(&self) -> bool {
        self.per_ip_enabled
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
    fn test_per_ip_tracking() {
        let config = SbcMetricsConfig {
            per_source_ip_metrics: true,
            ..Default::default()
        };
        let m = SbcRuntimeMetrics::new_with_config(&config);

        let ip1: IpAddr = "10.0.0.1".parse().unwrap();
        let ip2: IpAddr = "10.0.0.2".parse().unwrap();

        m.record_per_ip_call_attempt(ip1);
        m.record_per_ip_call_attempt(ip1);
        m.record_per_ip_security(ip1);
        m.record_per_ip_call_attempt(ip2);
        m.record_per_ip_rate_limit(ip2);

        let snaps = m.per_ip_snapshots();
        assert_eq!(snaps.len(), 2);

        let s1 = snaps.iter().find(|s| s.ip == "10.0.0.1").unwrap();
        assert_eq!(s1.calls_attempted, 2);
        assert_eq!(s1.security_rejections, 1);
        assert_eq!(s1.rate_limit_rejections, 0);

        let s2 = snaps.iter().find(|s| s.ip == "10.0.0.2").unwrap();
        assert_eq!(s2.calls_attempted, 1);
        assert_eq!(s2.rate_limit_rejections, 1);
    }

    #[test]
    fn test_per_ip_lru_eviction() {
        let config = SbcMetricsConfig {
            per_source_ip_metrics: true,
            max_source_ip_entries: 100, // minimum allowed
            ..Default::default()
        };
        let m = SbcRuntimeMetrics::new_with_config(&config);

        // Fill to capacity
        for i in 0..100u32 {
            let ip: IpAddr = format!("10.0.{}.{}", i / 256, i % 256).parse().unwrap();
            m.record_per_ip_call_attempt(ip);
        }
        assert_eq!(m.per_ip_snapshots().len(), 100);

        // Add one more — should evict the oldest
        let new_ip: IpAddr = "192.168.1.1".parse().unwrap();
        m.record_per_ip_call_attempt(new_ip);
        assert_eq!(m.per_ip_snapshots().len(), 100);

        // New IP should be present
        assert!(m.per_ip_snapshots().iter().any(|s| s.ip == "192.168.1.1"));
    }

    #[test]
    fn test_per_ip_disabled_noop() {
        let config = SbcMetricsConfig {
            per_source_ip_metrics: false,
            ..Default::default()
        };
        let m = SbcRuntimeMetrics::new_with_config(&config);

        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        m.record_per_ip_call_attempt(ip);
        m.record_per_ip_security(ip);

        assert!(m.per_ip_snapshots().is_empty());
    }

    #[test]
    fn test_per_ip_snapshots_format() {
        let config = SbcMetricsConfig {
            per_source_ip_metrics: true,
            ..Default::default()
        };
        let m = SbcRuntimeMetrics::new_with_config(&config);

        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        m.record_per_ip_call_attempt(ip);
        m.record_per_ip_rejection(ip);
        m.record_per_ip_rate_limit(ip);
        m.record_per_ip_security(ip);

        let snaps = m.per_ip_snapshots();
        assert_eq!(snaps.len(), 1);
        let s = &snaps[0];
        assert_eq!(s.ip, "10.0.0.1");
        assert_eq!(s.calls_attempted, 1);
        assert_eq!(s.calls_rejected, 1);
        assert_eq!(s.rate_limit_rejections, 1);
        assert_eq!(s.security_rejections, 1);
        assert!(s.last_seen_epoch_secs > 0);
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
