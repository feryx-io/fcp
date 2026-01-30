//! SBC metrics configuration and snapshot types.
//!
//! Defines the metrics collection policy and a point-in-time snapshot
//! struct for SBC operational metrics.

use serde::{Deserialize, Serialize};

/// Health status of a trunk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrunkHealthStatus {
    /// Trunk is healthy and accepting calls.
    Healthy,

    /// Trunk is experiencing elevated failures.
    Degraded,

    /// Trunk is down (exceeded failure threshold).
    Down,

    /// Health status is not yet determined.
    #[default]
    Unknown,
}

/// SBC metrics collection configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SbcMetricsConfig {
    /// Whether metrics collection is enabled.
    pub enabled: bool,

    /// Seconds between metrics snapshots.
    pub snapshot_interval_secs: u64,

    /// Collect per-trunk metrics.
    pub per_trunk_metrics: bool,

    /// Collect per-source-IP metrics.
    pub per_source_ip_metrics: bool,

    /// Maximum number of per-source-IP entries to track.
    pub max_source_ip_entries: usize,
}

impl Default for SbcMetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            snapshot_interval_secs: 60,
            per_trunk_metrics: true,
            per_source_ip_metrics: false,
            max_source_ip_entries: 10_000,
        }
    }
}

impl SbcMetricsConfig {
    /// Validate the configuration.
    ///
    /// Returns `Err` if:
    /// - `snapshot_interval_secs` is not in 5..=3600
    /// - `max_source_ip_entries` is not in 100..=1,000,000
    pub fn validate(&self) -> Result<(), String> {
        if self.snapshot_interval_secs < 5 || self.snapshot_interval_secs > 3600 {
            return Err("snapshot_interval_secs must be between 5 and 3600".into());
        }
        if self.max_source_ip_entries < 100 || self.max_source_ip_entries > 1_000_000 {
            return Err("max_source_ip_entries must be between 100 and 1,000,000".into());
        }
        Ok(())
    }
}

/// Point-in-time snapshot of SBC operational metrics.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SbcMetricsSnapshot {
    /// Current calls per second.
    pub current_cps: u64,

    /// Peak CPS observed.
    pub peak_cps: u64,

    /// Total call attempts since startup.
    pub total_calls_attempted: u64,

    /// Total calls completed (BYE exchanged).
    pub total_calls_completed: u64,

    /// Total calls rejected (4xx/5xx before answer).
    pub total_calls_rejected: u64,

    /// Total calls rejected due to admission control.
    pub total_admission_rejections: u64,

    /// Total calls rejected due to rate limiting.
    pub total_rate_limit_rejections: u64,

    /// Total calls rejected due to security policy.
    pub total_security_rejections: u64,

    /// Current concurrent active calls.
    pub concurrent_calls: u64,

    /// Current concurrent registrations.
    pub concurrent_registrations: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let cfg = SbcMetricsConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.snapshot_interval_secs, 60);
        assert!(cfg.per_trunk_metrics);
        assert!(!cfg.per_source_ip_metrics);
        assert_eq!(cfg.max_source_ip_entries, 10_000);
    }

    #[test]
    fn test_validate_valid() {
        assert!(SbcMetricsConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_snapshot_interval_too_low() {
        let mut cfg = SbcMetricsConfig::default();
        cfg.snapshot_interval_secs = 4;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("snapshot_interval_secs"));
    }

    #[test]
    fn test_validate_snapshot_interval_too_high() {
        let mut cfg = SbcMetricsConfig::default();
        cfg.snapshot_interval_secs = 3601;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("snapshot_interval_secs"));
    }

    #[test]
    fn test_validate_source_ip_entries_too_low() {
        let mut cfg = SbcMetricsConfig::default();
        cfg.max_source_ip_entries = 99;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_source_ip_entries"));
    }

    #[test]
    fn test_validate_source_ip_entries_too_high() {
        let mut cfg = SbcMetricsConfig::default();
        cfg.max_source_ip_entries = 1_000_001;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_source_ip_entries"));
    }

    #[test]
    fn test_validate_boundary_values() {
        let mut cfg = SbcMetricsConfig::default();

        cfg.snapshot_interval_secs = 5;
        assert!(cfg.validate().is_ok());
        cfg.snapshot_interval_secs = 3600;
        assert!(cfg.validate().is_ok());

        cfg.max_source_ip_entries = 100;
        assert!(cfg.validate().is_ok());
        cfg.max_source_ip_entries = 1_000_000;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_snapshot_default() {
        let snap = SbcMetricsSnapshot::default();
        assert_eq!(snap.current_cps, 0);
        assert_eq!(snap.peak_cps, 0);
        assert_eq!(snap.total_calls_attempted, 0);
        assert_eq!(snap.total_calls_completed, 0);
        assert_eq!(snap.total_calls_rejected, 0);
        assert_eq!(snap.total_admission_rejections, 0);
        assert_eq!(snap.total_rate_limit_rejections, 0);
        assert_eq!(snap.total_security_rejections, 0);
        assert_eq!(snap.concurrent_calls, 0);
        assert_eq!(snap.concurrent_registrations, 0);
    }

    #[test]
    fn test_serde_roundtrip_config() {
        let cfg = SbcMetricsConfig {
            per_source_ip_metrics: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SbcMetricsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_serde_roundtrip_snapshot() {
        let snap = SbcMetricsSnapshot {
            current_cps: 42,
            peak_cps: 100,
            total_calls_attempted: 5000,
            total_calls_completed: 4500,
            total_calls_rejected: 300,
            total_admission_rejections: 50,
            total_rate_limit_rejections: 150,
            total_security_rejections: 100,
            concurrent_calls: 85,
            concurrent_registrations: 2000,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let deserialized: SbcMetricsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, deserialized);
    }
}
