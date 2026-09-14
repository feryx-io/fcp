//! What the pipeline has done, for the status routes and `/metrics`
//! (`docs/CDR_PIPELINE.md` §11).

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use fcp_core::storage::CallDetailRecord;
use serde::Serialize;

/// Counters kept by a processor. Cheap to update from any task.
#[derive(Debug, Default)]
pub struct ProcessorStats {
    stored: AtomicU64,
    duplicates: AtomicU64,
    rejected: AtomicU64,
    batches: AtomicU64,
    batch_errors: AtomicU64,
    last_batch_micros: AtomicU64,
    last_lag_millis: AtomicU64,
}

/// A copy of the counters at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct StatsSnapshot {
    /// Records stored.
    pub stored: u64,
    /// Records that were stored already (replays).
    pub duplicates: u64,
    /// Records refused and kept for a person.
    pub rejected: u64,
    /// Batches processed.
    pub batches: u64,
    /// Batches that failed because the store was unavailable.
    pub batch_errors: u64,
    /// How long the last batch took, in microseconds.
    pub last_batch_micros: u64,
    /// `created_at - end_time` of the last record: how far behind the
    /// calls the table is, in milliseconds.
    pub last_lag_millis: u64,
}

impl ProcessorStats {
    pub(crate) fn record_batch(
        &self,
        stored: usize,
        duplicates: usize,
        rejected: usize,
        took: Duration,
    ) {
        self.stored.fetch_add(stored as u64, Ordering::Relaxed);
        self.duplicates
            .fetch_add(duplicates as u64, Ordering::Relaxed);
        self.rejected.fetch_add(rejected as u64, Ordering::Relaxed);
        self.batches.fetch_add(1, Ordering::Relaxed);
        self.last_batch_micros
            .store(took.as_micros() as u64, Ordering::Relaxed);
    }

    pub(crate) fn record_batch_error(&self) {
        self.batch_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Count records rejected outside a processed batch (a stream entry
    /// that did not decode).
    pub fn record_rejected(&self, count: u64) {
        self.rejected.fetch_add(count, Ordering::Relaxed);
    }

    pub(crate) fn observe_lag(&self, record: &CallDetailRecord) {
        let (Some(end), Some(created)) = (record.end_time, record.created_at) else {
            return;
        };
        let lag = created.duration_since(end).unwrap_or_default();
        self.last_lag_millis
            .store(lag.as_millis() as u64, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            stored: self.stored.load(Ordering::Relaxed),
            duplicates: self.duplicates.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            batches: self.batches.load(Ordering::Relaxed),
            batch_errors: self.batch_errors.load(Ordering::Relaxed),
            last_batch_micros: self.last_batch_micros.load(Ordering::Relaxed),
            last_lag_millis: self.last_lag_millis.load(Ordering::Relaxed),
        }
    }
}

impl StatsSnapshot {
    /// The counters in the Prometheus text format, with `labels` (already
    /// formatted, e.g. `node="jobs-1"`) on every sample.
    pub fn prometheus(&self, labels: &str) -> String {
        let mut out = String::new();
        family(
            &mut out,
            "cdr_processed_total",
            "counter",
            "Records processed, by outcome.",
        );
        for (outcome, value) in [
            ("stored", self.stored),
            ("duplicate", self.duplicates),
            ("dead_letter", self.rejected),
        ] {
            let _ = writeln!(
                out,
                "cdr_processed_total{{{}outcome=\"{outcome}\"}} {value}",
                with_comma(labels)
            );
        }
        for (name, kind, help, value) in [
            (
                "cdr_batches_total",
                "counter",
                "Batches processed.",
                self.batches.to_string(),
            ),
            (
                "cdr_batch_errors_total",
                "counter",
                "Batches that failed because the store was unavailable.",
                self.batch_errors.to_string(),
            ),
            (
                "cdr_batch_seconds",
                "gauge",
                "How long the last batch took.",
                format!("{:.6}", self.last_batch_micros as f64 / 1_000_000.0),
            ),
            (
                "cdr_end_to_write_seconds",
                "gauge",
                "How long after its call ended the last record was written.",
                format!("{:.3}", self.last_lag_millis as f64 / 1000.0),
            ),
        ] {
            family(&mut out, name, kind, help);
            let _ = writeln!(out, "{name}{{{labels}}} {value}");
        }
        out
    }
}

/// A metric family's `HELP` and `TYPE` lines.
pub fn family(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {kind}");
}

fn with_comma(labels: &str) -> String {
    if labels.is_empty() {
        String::new()
    } else {
        format!("{labels},")
    }
}

/// Seconds since `t`, for a status route.
pub fn age_seconds(t: SystemTime) -> f64 {
    SystemTime::now()
        .duration_since(t)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prometheus_text_has_one_family_per_metric_and_the_labels() {
        let stats = ProcessorStats::default();
        stats.record_batch(3, 1, 2, Duration::from_millis(12));
        let text = stats.snapshot().prometheus("node=\"jobs-1\"");
        assert!(
            text.contains("cdr_processed_total{node=\"jobs-1\",outcome=\"stored\"} 3"),
            "{text}"
        );
        assert!(
            text.contains("cdr_processed_total{node=\"jobs-1\",outcome=\"dead_letter\"} 2"),
            "{text}"
        );
        assert!(
            text.contains("cdr_batches_total{node=\"jobs-1\"} 1"),
            "{text}"
        );
        assert!(
            text.contains("cdr_batch_seconds{node=\"jobs-1\"} 0.012000"),
            "{text}"
        );
        assert_eq!(text.matches("# TYPE cdr_processed_total").count(), 1);
    }
}
