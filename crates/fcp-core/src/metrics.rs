//! Metrics abstraction shared across FCP crates.
//!
//! Provides a pluggable metrics interface that allows different backend
//! implementations (Prometheus, StatsD, OpenTelemetry, etc.) without
//! changing call sites.

use std::sync::Arc;

/// Metrics abstraction so we can plug different backends without changing call sites.
pub trait Metrics: Send + Sync + 'static {
    /// Increment a counter metric by 1.
    fn inc_counter(&self, name: &str, labels: &[(&str, &str)]);

    /// Observe a duration value in milliseconds (histogram/summary).
    fn observe_ms(&self, name: &str, value: u128, labels: &[(&str, &str)]);

    /// Record a histogram value.
    fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]);

    /// Record a gauge value (current state snapshot).
    fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]);
}

/// No-op metrics backend used by default.
#[derive(Clone, Default)]
pub struct NoopMetrics;

impl Metrics for NoopMetrics {
    fn inc_counter(&self, _name: &str, _labels: &[(&str, &str)]) {}
    fn observe_ms(&self, _name: &str, _value: u128, _labels: &[(&str, &str)]) {}
    fn record_histogram(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
    fn set_gauge(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
}

/// Type alias for a shared metrics handle.
pub type MetricsHandle = Arc<dyn Metrics>;

/// Create a no-op metrics handle.
pub fn noop_metrics() -> MetricsHandle {
    Arc::new(NoopMetrics)
}
