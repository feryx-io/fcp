use std::sync::Arc;

/// Metrics abstraction so we can plug different backends without changing call sites.
pub trait Metrics: Send + Sync + 'static {
    fn inc_counter(&self, name: &str, labels: &[(&str, &str)]);
    fn observe_ms(&self, name: &str, value: u128, labels: &[(&str, &str)]);
}

/// No-op metrics backend used by default.
#[derive(Clone, Default)]
pub struct NoopMetrics;

impl Metrics for NoopMetrics {
    fn inc_counter(&self, _name: &str, _labels: &[(&str, &str)]) {}
    fn observe_ms(&self, _name: &str, _value: u128, _labels: &[(&str, &str)]) {}
}

pub type MetricsHandle = Arc<dyn Metrics>;

pub fn noop_metrics() -> MetricsHandle {
    Arc::new(NoopMetrics)
}
