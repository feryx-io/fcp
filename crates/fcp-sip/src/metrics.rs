//! Metrics abstraction for fcp-sip.
//!
//! This module provides a pluggable metrics interface that allows different
//! backend implementations (Prometheus, StatsD, OpenTelemetry, etc.) without
//! changing call sites throughout the codebase.
//!
//! # Metrics Types
//!
//! The trait supports four metric types:
//!
//! - **Counters**: Monotonically increasing values (requests, errors)
//! - **Histograms**: Value distributions (body sizes, utilization percentages)
//! - **Gauges**: Point-in-time snapshots (active connections, queue depth)
//! - **Duration observations**: Histogram of durations in milliseconds
//!
//! # Emitted Metrics
//!
//! The following metrics are emitted by `SipMessageHandler`:
//!
//! ## Counters
//!
//! - `fcp_sip_requests_total{method, result}` - Total SIP requests processed
//! - `fcp_sip_mid_dialog_forward_total{method, result}` - Mid-dialog requests forwarded
//! - `fcp_sip_session_timer_health_check` - Health check iterations
//!
//! ## Histograms
//!
//! - `fcp_sip_info_body_bytes{content_type}` - INFO message body sizes
//! - `fcp_sip_notify_body_bytes{event}` - NOTIFY message body sizes
//! - `fcp_sip_rate_limit_utilization_percent{method}` - Rate limit usage (0-100%)
//! - `fcp_sip_mid_dialog_forward_ms{method, result}` - Mid-dialog forward duration
//!
//! ## Gauges
//!
//! - `fcp_sip_active_session_timers` - Current active session timers
//! - `fcp_sip_active_dialogs` - Current active dialogs
//! - `fcp_sip_active_calls` - Current active B2BUA calls
//!
//! # Example Implementation
//!
//! ```no_run
//! use fcp_sip::metrics::Metrics;
//! use std::sync::Arc;
//!
//! struct PrometheusMetrics {
//!     // Prometheus client fields
//! }
//!
//! impl Metrics for PrometheusMetrics {
//!     fn inc_counter(&self, name: &str, labels: &[(&str, &str)]) {
//!         // Increment Prometheus counter
//!     }
//!
//!     fn observe_ms(&self, name: &str, value: u128, labels: &[(&str, &str)]) {
//!         // Observe histogram value in milliseconds
//!     }
//!
//!     fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
//!         // Record histogram observation
//!     }
//!
//!     fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
//!         // Set gauge value
//!     }
//! }
//! ```

use std::sync::Arc;

/// Metrics abstraction so we can plug different backends without changing call sites.
pub trait Metrics: Send + Sync + 'static {
    /// Increment a counter metric by 1.
    ///
    /// # Arguments
    /// * `name` - Metric name (e.g., "fcp_sip_requests_total")
    /// * `labels` - Label key-value pairs (e.g., [("method", "INVITE"), ("code", "200")])
    fn inc_counter(&self, name: &str, labels: &[(&str, &str)]);

    /// Observe a duration value in milliseconds (histogram/summary).
    ///
    /// This is a convenience method for duration tracking. For general histogram
    /// values, use `record_histogram()`.
    ///
    /// # Arguments
    /// * `name` - Metric name (e.g., "fcp_sip_request_duration_ms")
    /// * `value` - Duration in milliseconds
    /// * `labels` - Label key-value pairs
    fn observe_ms(&self, name: &str, value: u128, labels: &[(&str, &str)]);

    /// Record a histogram value.
    ///
    /// Histograms track the distribution of values and are useful for:
    /// - Message body sizes
    /// - Queue depths
    /// - Utilization percentages
    /// - Counts (active dialogs, sessions, etc.)
    ///
    /// # Arguments
    /// * `name` - Metric name (e.g., "fcp_sip_message_body_bytes")
    /// * `value` - The value to record
    /// * `labels` - Label key-value pairs
    fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]);

    /// Record a gauge value (current state snapshot).
    ///
    /// Gauges represent a value that can go up or down, such as:
    /// - Active connection count
    /// - Memory usage
    /// - Queue size
    ///
    /// # Arguments
    /// * `name` - Metric name (e.g., "fcp_sip_active_dialogs")
    /// * `value` - The current value
    /// * `labels` - Label key-value pairs
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

pub type MetricsHandle = Arc<dyn Metrics>;

pub fn noop_metrics() -> MetricsHandle {
    Arc::new(NoopMetrics)
}
