//! Whether something grew and kept growing (`docs/DEEP_TESTING.md` D6).
//!
//! A soak's question is not "did it grow" — a process warms its caches,
//! a pool fills to its size, a table of call records grows with every
//! call — but "did it stop". After a warm-up the run is cut into four
//! windows of equal length and each series is summarised by its median
//! in each, so a burst or a sawtooth (calls placed and ended, a cache
//! swept every minute) does not read as a trend. A series **grows** when
//! its last window is above its first by more than the series' tolerance
//! *and* still above its third by more than a third of it — a leak keeps
//! climbing to the end, where a pool that filled in the first hour
//! stands still in the last.
//!
//! What grows by design — a call record per call, an audit row per admin
//! write — is named in [`EXPECTED_GROWTH`] with why, and reported but
//! not failed.

use super::stack::StackSample;
use serde::Serialize;

/// Series that grow with the traffic by design, each with why. A prefix
/// match on the series name.
pub const EXPECTED_GROWTH: &[(&str, &str)] = &[
    (
        "table/cdrs",
        "a call record per call leg, kept for retention",
    ),
    (
        "table/cdr_usage_hourly",
        "an hourly usage rollup per tenant: one row an hour",
    ),
    (
        "table/cdr_number_usage_hourly",
        "an hourly usage rollup per number: rows an hour",
    ),
    (
        "table/cdr_rollup_state",
        "the rollup's progress, a row per hour rolled up",
    ),
    (
        "table/admin_audit_log",
        "an audit row per admin write and sign-in, kept for compliance",
    ),
    (
        "table/config_audit_log",
        "an audit row per configuration write",
    ),
    (
        "table/sync_journal",
        "a journal row per change to a synced resource, read by sites",
    ),
    (
        "table/deletion_log",
        "a row per deleted record, kept for compliance",
    ),
    (
        "table/presence_history",
        "a row per presence change, swept by retention",
    ),
    ("table/balance_transactions", "a ledger row per rated call"),
    (
        "table/flow_execution_traces",
        "a trace per flow run, swept by retention",
    ),
    ("table/webhook_delivery_logs", "a row per webhook delivery"),
    (
        "table/sip_trace_messages",
        "every SIP message of a call, kept until the tenant's retention policy says \
         (docs/COMPLIANCE.md: forever by default)",
    ),
    (
        "table/login_sessions",
        "a row per sign-in, deleted by fcp-admin's sweep 30 days after it ends",
    ),
];

/// How much a series may move before it counts.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Tolerance {
    /// A fraction of the first window's median …
    pub relative: f64,
    /// … plus this much, in the series' own unit.
    pub absolute: f64,
}

impl Tolerance {
    fn of(&self, base: f64) -> f64 {
        base.abs() * self.relative + self.absolute
    }
}

/// The tolerance for a series, by what it measures.
pub fn tolerance_for(series: &str) -> Tolerance {
    let t = |relative, absolute| Tolerance { relative, absolute };
    if series.ends_with("/rss_bytes") {
        // An allocator holds on to what a burst asked for.
        t(0.10, 16.0 * 1024.0 * 1024.0)
    } else if series == "redis/used_memory" {
        t(0.10, 4.0 * 1024.0 * 1024.0)
    } else if series.ends_with("/fds") || series.ends_with("/udp_sockets") {
        t(0.05, 10.0)
    } else if series.ends_with("/threads") {
        t(0.0, 8.0)
    } else if series.starts_with("postgres/") || series == "redis/clients" {
        t(0.0, 5.0)
    } else {
        // Rows and keys.
        t(0.05, 20.0)
    }
}

/// One series, judged.
#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub series: String,
    /// The median of each window.
    pub windows: [f64; 4],
    pub grows: bool,
    /// Why it is allowed to, when it grows by design.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<&'static str>,
}

impl Verdict {
    /// A growth nobody said to expect.
    pub fn fails(&self) -> bool {
        self.grows && self.expected.is_none()
    }
}

/// Judge every series sampled after `warmup_secs`. Too few samples to
/// cut into windows is no verdict at all (an empty list), not a pass.
pub fn judge(samples: &[StackSample], warmup_secs: f64) -> Vec<Verdict> {
    let kept: Vec<&StackSample> = samples
        .iter()
        .filter(|s| s.at_secs >= warmup_secs)
        .collect();
    if kept.len() < 8 {
        return Vec::new();
    }
    let first = kept.first().map(|s| s.at_secs).unwrap_or(0.0);
    let last = kept.last().map(|s| s.at_secs).unwrap_or(0.0);
    let span = (last - first).max(f64::EPSILON);
    let window_of = |at: f64| (((at - first) / span) * 4.0).floor().clamp(0.0, 3.0) as usize;

    let mut names: Vec<&String> = kept.iter().flat_map(|s| s.series.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter_map(|name| {
            let mut windows: [Vec<f64>; 4] = Default::default();
            for sample in &kept {
                if let Some(v) = sample.series.get(name) {
                    windows[window_of(sample.at_secs)].push(*v);
                }
            }
            // A series missing from a whole window cannot be judged.
            if windows.iter().any(Vec::is_empty) {
                return None;
            }
            let medians = windows.map(|mut w| median(&mut w));
            let tolerance = tolerance_for(name);
            let grows = medians[3] - medians[0] > tolerance.of(medians[0])
                && medians[3] - medians[2] > tolerance.of(medians[0]) / 3.0;
            Some(Verdict {
                series: name.clone(),
                windows: medians,
                grows,
                expected: EXPECTED_GROWTH
                    .iter()
                    .find(|(prefix, _)| name.starts_with(prefix))
                    .map(|(_, why)| *why),
            })
        })
        .collect()
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A series' value at a sample's index.
    type Series<'a> = (&'a str, fn(usize) -> f64);

    /// `n` samples a second apart, each series a function of the index.
    fn samples(n: usize, series: &[Series]) -> Vec<StackSample> {
        (0..n)
            .map(|i| StackSample {
                at_secs: i as f64,
                series: series
                    .iter()
                    .map(|(name, f)| (name.to_string(), f(i)))
                    .collect::<BTreeMap<_, _>>(),
            })
            .collect()
    }

    fn verdict<'a>(verdicts: &'a [Verdict], name: &str) -> &'a Verdict {
        verdicts.iter().find(|v| v.series == name).unwrap()
    }

    #[test]
    fn a_leak_grows_and_a_pool_that_filled_does_not() {
        let v = judge(
            &samples(
                400,
                &[
                    // 1 MiB a sample, to the end.
                    ("process/a/rss_bytes", |i| {
                        100.0 * 1048576.0 + i as f64 * 1048576.0
                    }),
                    // Climbs fast, then stands from a quarter of the way.
                    ("process/b/rss_bytes", |i| {
                        100.0 * 1048576.0 + (i.min(100) as f64) * 1048576.0
                    }),
                    // Steady.
                    ("process/c/fds", |_| 40.0),
                ],
            ),
            0.0,
        );
        assert!(verdict(&v, "process/a/rss_bytes").fails());
        assert!(
            !verdict(&v, "process/b/rss_bytes").grows,
            "{:?}",
            verdict(&v, "process/b/rss_bytes")
        );
        assert!(!verdict(&v, "process/c/fds").grows);
    }

    #[test]
    fn a_sawtooth_is_not_a_trend() {
        // Calls up and down every ten samples: the medians stand still.
        let v = judge(
            &samples(
                400,
                &[("process/cm/udp_sockets", |i| 20.0 + (i % 10) as f64 * 8.0)],
            ),
            0.0,
        );
        assert!(!v[0].grows, "{:?}", v[0]);
    }

    #[test]
    fn growth_within_the_tolerance_is_noise() {
        // 15 rows over the run: under the 20-row floor.
        let v = judge(
            &samples(
                400,
                &[("table/registrations/rows", |i| 40.0 + (i / 27) as f64)],
            ),
            0.0,
        );
        assert!(!v[0].grows, "{:?}", v[0]);
    }

    #[test]
    fn growth_by_design_is_reported_but_does_not_fail() {
        let v = judge(
            &samples(400, &[("table/cdrs_2026_10/rows", |i| i as f64 * 2.0)]),
            0.0,
        );
        assert!(v[0].grows);
        assert!(v[0].expected.is_some());
        assert!(!v[0].fails());
    }

    #[test]
    fn the_warm_up_is_left_out() {
        // A thousand rows in the warm-up, flat after.
        let v = judge(
            &samples(
                400,
                &[("table/login_sessions/rows", |i| (i.min(50) * 20) as f64)],
            ),
            60.0,
        );
        assert!(!v[0].grows, "{:?}", v[0]);
    }

    #[test]
    fn too_few_samples_is_no_verdict() {
        assert!(judge(&samples(5, &[("redis/keys", |i| i as f64)]), 0.0).is_empty());
    }
}
