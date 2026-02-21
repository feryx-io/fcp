//! Prometheus metrics for Redis operations and cache statistics.
//!
//! Feature-gated behind `metrics`. Registers counters, histograms, and gauges
//! for observing Redis operation latency, throughput, and error rates, plus
//! in-memory cache hit/miss/eviction statistics.

use lazy_static::lazy_static;
use prometheus::{
    HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec, Opts,
};

lazy_static! {
    /// Total Redis operations by operation name and result (ok/err).
    pub static ref REDIS_OPERATIONS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("fcp_redis_operations_total", "Total Redis operations"),
        &["operation", "result"],
    )
    .expect("metric creation failed");

    /// Redis operation latency in seconds, by operation name.
    pub static ref REDIS_OPERATION_DURATION_SECONDS: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "fcp_redis_operation_duration_seconds",
            "Redis operation latency in seconds",
        )
        .buckets(vec![0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]),
        &["operation"],
    )
    .expect("metric creation failed");

    /// Total Redis connection errors.
    pub static ref REDIS_CONNECTION_ERRORS_TOTAL: IntCounter = IntCounter::new(
        "fcp_redis_connection_errors_total",
        "Total Redis connection errors",
    )
    .expect("metric creation failed");

    /// Cache hits by store name.
    pub static ref CACHE_HITS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("fcp_cache_hits_total", "Total cache hits"),
        &["store"],
    )
    .expect("metric creation failed");

    /// Cache misses by store name.
    pub static ref CACHE_MISSES_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("fcp_cache_misses_total", "Total cache misses"),
        &["store"],
    )
    .expect("metric creation failed");

    /// Cache evictions by store name.
    pub static ref CACHE_EVICTIONS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("fcp_cache_evictions_total", "Total cache evictions"),
        &["store"],
    )
    .expect("metric creation failed");

    /// Current cache size (number of entries) by store name.
    pub static ref CACHE_SIZE: IntGaugeVec = IntGaugeVec::new(
        Opts::new("fcp_cache_size", "Current cache size (entries)"),
        &["store"],
    )
    .expect("metric creation failed");
}

/// Register all Redis metrics with the default Prometheus registry.
///
/// Call this once at startup. Panics if metrics are already registered
/// (which is harmless during tests — just ignore the error).
pub fn register_metrics() {
    let registry = prometheus::default_registry();
    let _ = registry.register(Box::new(REDIS_OPERATIONS_TOTAL.clone()));
    let _ = registry.register(Box::new(REDIS_OPERATION_DURATION_SECONDS.clone()));
    let _ = registry.register(Box::new(REDIS_CONNECTION_ERRORS_TOTAL.clone()));
    let _ = registry.register(Box::new(CACHE_HITS_TOTAL.clone()));
    let _ = registry.register(Box::new(CACHE_MISSES_TOTAL.clone()));
    let _ = registry.register(Box::new(CACHE_EVICTIONS_TOTAL.clone()));
    let _ = registry.register(Box::new(CACHE_SIZE.clone()));
}

/// Record a successful Redis operation with duration.
pub fn record_redis_op_success(operation: &str, duration: std::time::Duration) {
    REDIS_OPERATIONS_TOTAL
        .with_label_values(&[operation, "ok"])
        .inc();
    REDIS_OPERATION_DURATION_SECONDS
        .with_label_values(&[operation])
        .observe(duration.as_secs_f64());
}

/// Record a failed Redis operation with duration.
pub fn record_redis_op_failure(operation: &str, duration: std::time::Duration) {
    REDIS_OPERATIONS_TOTAL
        .with_label_values(&[operation, "err"])
        .inc();
    REDIS_OPERATION_DURATION_SECONDS
        .with_label_values(&[operation])
        .observe(duration.as_secs_f64());
}

/// Record a Redis connection error.
pub fn record_connection_error() {
    REDIS_CONNECTION_ERRORS_TOTAL.inc();
}

/// Cache statistics snapshot for a named store.
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub size: usize,
}

/// Update Prometheus cache gauges from a `CacheStats` snapshot.
///
/// Call this periodically (e.g. every 30s from a health endpoint or
/// background task) to keep cache metrics current.
pub fn update_cache_metrics(name: &str, stats: &CacheStats) {
    CACHE_HITS_TOTAL
        .with_label_values(&[name])
        .inc_by(stats.hits);
    CACHE_MISSES_TOTAL
        .with_label_values(&[name])
        .inc_by(stats.misses);
    CACHE_EVICTIONS_TOTAL
        .with_label_values(&[name])
        .inc_by(stats.evictions);
    CACHE_SIZE
        .with_label_values(&[name])
        .set(stats.size as i64);
}

/// Render all registered Prometheus metrics in text exposition format.
///
/// Returns the metrics as a String suitable for serving from a `/metrics`
/// HTTP endpoint.
pub fn render_metrics() -> String {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let families = prometheus::default_registry().gather();
    let mut buffer = Vec::new();
    encoder.encode(&families, &mut buffer).unwrap_or_default();
    String::from_utf8(buffer).unwrap_or_default()
}
