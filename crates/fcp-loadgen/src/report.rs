//! What a run measured: the callers' view and the node's, in one file
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §8).
//!
//! Latencies are kept as every sample rather than as buckets — a run is
//! thousands of calls, not millions of requests, and exact percentiles
//! are worth more than a histogram's approximation when the number is
//! going to be written down as this platform's capacity.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// A set of latency samples, summarised when the run ends.
#[derive(Debug, Default)]
pub struct Latencies(Mutex<Vec<f64>>);

impl Latencies {
    pub fn record(&self, d: Duration) {
        self.0.lock().unwrap().push(d.as_secs_f64() * 1000.0);
    }

    /// Take another set's samples into this one.
    pub fn absorb(&self, other: &Latencies) {
        let mut mine = self.0.lock().unwrap();
        mine.extend(other.0.lock().unwrap().iter().copied());
    }

    pub fn summary(&self) -> LatencySummary {
        let mut ms = self.0.lock().unwrap().clone();
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        LatencySummary {
            count: ms.len() as u64,
            p50_ms: percentile(&ms, 0.50),
            p90_ms: percentile(&ms, 0.90),
            p99_ms: percentile(&ms, 0.99),
            max_ms: ms.last().copied().unwrap_or(0.0),
        }
    }
}

/// The `p`th percentile of sorted samples, by nearest rank; zero when
/// there are none, which the `count` beside it makes plain.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatencySummary {
    pub count: u64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// Everything a run counted, as it counted it. Shared across the tasks
/// driving the phones.
#[derive(Debug, Default)]
pub struct Counters {
    /// INVITE → the first provisional.
    pub invite_to_provisional: Latencies,
    /// INVITE → the first `18x`: the far end ringing.
    pub invite_to_ringing: Latencies,
    /// INVITE → `200 OK`.
    pub invite_to_answer: Latencies,
    /// REGISTER → its final response.
    pub register: Latencies,
    /// Final responses to INVITEs by status code, `0` for a call that
    /// never got one. Registrations are counted apart, or a storm's
    /// `200`s would read as answered calls.
    outcomes: Mutex<BTreeMap<u16, u64>>,
    /// Final responses to REGISTERs by status code.
    register_outcomes: Mutex<BTreeMap<u16, u64>>,
    /// Calls offered, whatever became of them.
    offered: Mutex<u64>,
    /// Registrations offered.
    registers: Mutex<u64>,
    /// What each answered call's own leg heard.
    media: Mutex<Vec<LegMedia>>,
    /// Calls that ended on their own before the run hung them up.
    dropped: Mutex<u64>,
}

/// One answered call's media, from the caller's side.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct LegMedia {
    pub received: u64,
    pub sent: u64,
    pub lost: u64,
    pub out_of_order: u64,
    pub duplicates: u64,
    pub jitter_ms: f64,
}

impl Counters {
    pub fn offered(&self) {
        *self.offered.lock().unwrap() += 1;
    }

    /// One REGISTER, and what the registrar said to it.
    pub fn registered(&self, code: u16) {
        *self.registers.lock().unwrap() += 1;
        *self
            .register_outcomes
            .lock()
            .unwrap()
            .entry(code)
            .or_default() += 1;
    }

    pub fn outcome(&self, code: u16) {
        *self.outcomes.lock().unwrap().entry(code).or_default() += 1;
    }

    pub fn dropped(&self) {
        *self.dropped.lock().unwrap() += 1;
    }

    pub fn media(&self, leg: LegMedia) {
        self.media.lock().unwrap().push(leg);
    }

    /// Take everything another set of counters holds into this one: a
    /// ramp counts each rung separately, and the run's totals are all of
    /// them together.
    pub fn absorb(&self, other: &Counters) {
        self.invite_to_provisional
            .absorb(&other.invite_to_provisional);
        self.invite_to_ringing.absorb(&other.invite_to_ringing);
        self.invite_to_answer.absorb(&other.invite_to_answer);
        self.register.absorb(&other.register);
        {
            let mut mine = self.outcomes.lock().unwrap();
            for (code, n) in other.outcomes.lock().unwrap().iter() {
                *mine.entry(*code).or_default() += n;
            }
        }
        {
            let mut mine = self.register_outcomes.lock().unwrap();
            for (code, n) in other.register_outcomes.lock().unwrap().iter() {
                *mine.entry(*code).or_default() += n;
            }
        }
        *self.offered.lock().unwrap() += *other.offered.lock().unwrap();
        *self.registers.lock().unwrap() += *other.registers.lock().unwrap();
        *self.dropped.lock().unwrap() += *other.dropped.lock().unwrap();
        self.media
            .lock()
            .unwrap()
            .extend(other.media.lock().unwrap().iter().copied());
    }

    /// The counts and percentiles as they stand.
    pub fn summarise(&self, elapsed: Duration) -> Measurements {
        let outcomes = self.outcomes.lock().unwrap().clone();
        let offered = *self.offered.lock().unwrap();
        let answered = outcomes
            .iter()
            .filter(|(code, _)| (200..300).contains(*code))
            .map(|(_, n)| *n)
            .sum::<u64>();
        let refused = outcomes
            .iter()
            .filter(|(code, _)| **code >= 400)
            .map(|(_, n)| *n)
            .sum::<u64>();
        let media = self.media.lock().unwrap().clone();
        let secs = elapsed.as_secs_f64().max(0.001);
        Measurements {
            offered,
            answered,
            refused,
            dropped: *self.dropped.lock().unwrap(),
            registers: *self.registers.lock().unwrap(),
            offered_cps: offered as f64 / secs,
            answered_cps: answered as f64 / secs,
            refused_fraction: if offered == 0 {
                0.0
            } else {
                refused as f64 / offered as f64
            },
            outcomes: outcomes
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            register_outcomes: self
                .register_outcomes
                .lock()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
            invite_to_provisional: self.invite_to_provisional.summary(),
            invite_to_ringing: self.invite_to_ringing.summary(),
            invite_to_answer: self.invite_to_answer.summary(),
            register_latency: self.register.summary(),
            media: MediaSummary::of(&media),
        }
    }
}

/// What the callers saw.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Measurements {
    pub offered: u64,
    pub answered: u64,
    pub refused: u64,
    /// Calls that ended by themselves — the node hung them up, or the
    /// media stopped and a session timer did.
    pub dropped: u64,
    pub registers: u64,
    pub offered_cps: f64,
    pub answered_cps: f64,
    pub refused_fraction: f64,
    /// Final responses to INVITEs by status code.
    pub outcomes: BTreeMap<String, u64>,
    /// Final responses to REGISTERs by status code.
    #[serde(default)]
    pub register_outcomes: BTreeMap<String, u64>,
    pub invite_to_provisional: LatencySummary,
    pub invite_to_ringing: LatencySummary,
    pub invite_to_answer: LatencySummary,
    pub register_latency: LatencySummary,
    pub media: MediaSummary,
}

/// What the answered calls heard, across their legs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaSummary {
    pub legs: u64,
    pub packets_received: u64,
    pub packets_sent: u64,
    /// Loss across every leg, as a fraction of what was sent to them.
    pub loss_fraction: f64,
    /// The worst leg's loss, which an average hides.
    pub worst_leg_loss_fraction: f64,
    pub out_of_order: u64,
    pub duplicates: u64,
    pub mean_jitter_ms: f64,
    pub max_jitter_ms: f64,
    /// Legs that heard nothing at all: one-way audio, not merely lossy.
    pub silent_legs: u64,
}

impl MediaSummary {
    fn of(legs: &[LegMedia]) -> Self {
        if legs.is_empty() {
            return Self::default();
        }
        let received: u64 = legs.iter().map(|l| l.received).sum();
        let lost: u64 = legs.iter().map(|l| l.lost).sum();
        let sent_to_us = received + lost;
        let worst = legs
            .iter()
            .map(|l| {
                let expected = l.received + l.lost;
                if expected == 0 {
                    1.0
                } else {
                    l.lost as f64 / expected as f64
                }
            })
            .fold(0.0_f64, f64::max);
        Self {
            legs: legs.len() as u64,
            packets_received: received,
            packets_sent: legs.iter().map(|l| l.sent).sum(),
            loss_fraction: if sent_to_us == 0 {
                0.0
            } else {
                lost as f64 / sent_to_us as f64
            },
            worst_leg_loss_fraction: worst,
            out_of_order: legs.iter().map(|l| l.out_of_order).sum(),
            duplicates: legs.iter().map(|l| l.duplicates).sum(),
            mean_jitter_ms: legs.iter().map(|l| l.jitter_ms).sum::<f64>() / legs.len() as f64,
            max_jitter_ms: legs.iter().map(|l| l.jitter_ms).fold(0.0_f64, f64::max),
            silent_legs: legs.iter().filter(|l| l.received == 0).count() as u64,
        }
    }
}

/// One run, start to finish.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// The harness's version, so a number can be traced to what produced it.
    pub version: String,
    pub profile: String,
    /// What the run was told to do, as given.
    pub options: serde_json::Value,
    pub started_at: String,
    pub duration_secs: f64,
    /// What the callers saw.
    pub measurements: Measurements,
    /// What the node said about itself, sampled while the run went on.
    pub samples: Vec<crate::sampler::Sample>,
    /// What each rung of a ramp held, when the profile climbs one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rungs: Vec<Rung>,
    /// Budgets that were asserted, and whether they held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub budgets: Vec<BudgetResult>,
}

/// One step of a call ramp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rung {
    pub offered_cps: f64,
    pub measurements: Measurements,
    /// Why the ramp stopped here, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_because: Option<String>,
}

/// One budget assertion's verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetResult {
    pub name: String,
    pub limit: f64,
    pub measured: f64,
    pub held: bool,
}

impl Report {
    /// The one-page summary an operator reads before opening the JSON.
    pub fn summary(&self) -> String {
        let m = &self.measurements;
        let mut out = String::new();
        out.push_str(&format!(
            "fcp-loadgen {} — profile {} — {:.1}s\n",
            self.version, self.profile, self.duration_secs
        ));
        out.push_str(&format!(
            "  calls    offered {} ({:.1}/s), answered {} ({:.1}/s), refused {} ({:.1}%), dropped {}\n",
            m.offered,
            m.offered_cps,
            m.answered,
            m.answered_cps,
            m.refused,
            m.refused_fraction * 100.0,
            m.dropped,
        ));
        if m.registers > 0 {
            let codes: Vec<String> = m
                .register_outcomes
                .iter()
                .map(|(code, n)| format!("{code}×{n}"))
                .collect();
            out.push_str(&format!(
                "  register {} sent, p50 {:.0}ms p99 {:.0}ms max {:.0}ms, {}\n",
                m.registers,
                m.register_latency.p50_ms,
                m.register_latency.p99_ms,
                m.register_latency.max_ms,
                codes.join(" "),
            ));
        }
        for (what, l) in [
            ("→1xx", &m.invite_to_provisional),
            ("→18x", &m.invite_to_ringing),
            ("→200", &m.invite_to_answer),
        ] {
            if l.count > 0 {
                out.push_str(&format!(
                    "  invite {what}  n={} p50 {:.0}ms p90 {:.0}ms p99 {:.0}ms max {:.0}ms\n",
                    l.count, l.p50_ms, l.p90_ms, l.p99_ms, l.max_ms
                ));
            }
        }
        if !m.outcomes.is_empty() {
            let codes: Vec<String> = m
                .outcomes
                .iter()
                .map(|(code, n)| format!("{code}×{n}"))
                .collect();
            out.push_str(&format!("  outcomes {}\n", codes.join(" ")));
        }
        if m.media.legs > 0 {
            out.push_str(&format!(
                "  media    {} legs, loss {:.2}% (worst leg {:.2}%), jitter mean {:.1}ms max {:.1}ms, silent {}\n",
                m.media.legs,
                m.media.loss_fraction * 100.0,
                m.media.worst_leg_loss_fraction * 100.0,
                m.media.mean_jitter_ms,
                m.media.max_jitter_ms,
                m.media.silent_legs,
            ));
        }
        for rung in &self.rungs {
            out.push_str(&format!(
                "  rung {:>6.1}/s  answered {:.1}/s, refused {:.1}%, p99 →200 {:.0}ms{}\n",
                rung.offered_cps,
                rung.measurements.answered_cps,
                rung.measurements.refused_fraction * 100.0,
                rung.measurements.invite_to_answer.p99_ms,
                rung.stopped_because
                    .as_ref()
                    .map(|why| format!("  ← stopped: {why}"))
                    .unwrap_or_default(),
            ));
        }
        if let Some(last) = self.samples.last() {
            out.push_str(&format!(
                "  node     rss {:.0} MiB, cpu {:.0}%{}\n",
                last.rss_bytes.unwrap_or(0) as f64 / 1024.0 / 1024.0,
                last.cpu_percent.unwrap_or(0.0),
                last.metrics
                    .get("fcp_active_calls")
                    .map(|v| format!(", active calls {v:.0}"))
                    .unwrap_or_default(),
            ));
        }
        for b in &self.budgets {
            out.push_str(&format!(
                "  budget   {} {} (limit {}, measured {:.2})\n",
                b.name,
                if b.held { "held" } else { "BREACHED" },
                b.limit,
                b.measured
            ));
        }
        out
    }

    /// Whether every asserted budget held.
    pub fn within_budget(&self) -> bool {
        self.budgets.iter().all(|b| b.held)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_by_nearest_rank() {
        let s: Vec<f64> = (1..=100).map(|n| n as f64).collect();
        assert_eq!(percentile(&s, 0.5), 50.0);
        assert_eq!(percentile(&s, 0.9), 90.0);
        assert_eq!(percentile(&s, 0.99), 99.0);
        assert_eq!(percentile(&[], 0.5), 0.0, "no samples, no number");
        assert_eq!(percentile(&[7.0], 0.99), 7.0);
    }

    #[test]
    fn a_run_with_nothing_offered_reports_zeroes_rather_than_dividing_by_them() {
        let counters = Counters::default();
        let m = counters.summarise(Duration::from_secs(10));
        assert_eq!(m.offered, 0);
        assert_eq!(m.offered_cps, 0.0);
        assert_eq!(m.refused_fraction, 0.0);
        assert_eq!(m.media.legs, 0);
    }

    /// A storm's `200`s are not answered calls: the two are counted
    /// apart, which is what the first run of the harness got wrong.
    #[test]
    fn registrations_are_not_counted_as_calls() {
        let counters = Counters::default();
        counters.offered();
        counters.outcome(200);
        for _ in 0..5 {
            counters.registered(200);
        }
        let m = counters.summarise(Duration::from_secs(1));
        assert_eq!(m.offered, 1);
        assert_eq!(m.answered, 1, "one call, five registrations");
        assert_eq!(m.registers, 5);
        assert_eq!(m.register_outcomes["200"], 5);
        assert_eq!(m.outcomes["200"], 1);
    }

    #[test]
    fn outcomes_are_counted_by_code_and_split_into_answered_and_refused() {
        let counters = Counters::default();
        for _ in 0..3 {
            counters.offered();
            counters.outcome(200);
        }
        counters.offered();
        counters.outcome(503);
        counters.offered();
        counters.outcome(486);
        let m = counters.summarise(Duration::from_secs(5));
        assert_eq!(m.offered, 5);
        assert_eq!(m.answered, 3);
        assert_eq!(m.refused, 2, "486 and 503 are both refusals");
        assert_eq!(m.outcomes["503"], 1);
        assert!((m.offered_cps - 1.0).abs() < 1e-9);
    }

    /// The average hides a single silent leg; the worst leg and the
    /// silent count do not.
    #[test]
    fn media_reports_the_worst_leg_as_well_as_the_mean() {
        let counters = Counters::default();
        counters.media(LegMedia {
            received: 1000,
            sent: 1000,
            lost: 0,
            jitter_ms: 1.0,
            ..LegMedia::default()
        });
        counters.media(LegMedia {
            received: 0,
            sent: 1000,
            lost: 500,
            jitter_ms: 9.0,
            ..LegMedia::default()
        });
        let m = counters.summarise(Duration::from_secs(1)).media;
        assert_eq!(m.legs, 2);
        assert_eq!(m.silent_legs, 1);
        assert!((m.loss_fraction - 500.0 / 1500.0).abs() < 1e-9);
        assert!((m.worst_leg_loss_fraction - 1.0).abs() < 1e-9);
        assert_eq!(m.max_jitter_ms, 9.0);
        assert_eq!(m.mean_jitter_ms, 5.0);
    }
}
