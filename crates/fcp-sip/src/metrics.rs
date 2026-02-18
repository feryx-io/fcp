//! Metrics abstraction for fcp-sip.
//!
//! Re-exports the metrics types from `fcp-core` so existing call sites continue
//! to work. New code should prefer importing from `fcp_core::metrics` directly.
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

pub use fcp_core::metrics::*;
