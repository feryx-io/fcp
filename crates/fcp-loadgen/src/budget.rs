//! What a run is allowed to measure before it fails
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §8).
//!
//! A budget turns a profile into a gate: the run exits non-zero when one
//! is breached, so a nightly job or a CI step can hold a node to a number
//! that was measured on that hardware. CI asserts behaviour — no
//! refusals, no silent legs — never a rate, since a shared runner's
//! throughput says nothing (§12 Q8).

use crate::report::{BudgetResult, Measurements};
use serde::{Deserialize, Serialize};

/// The bounds a run asserts. Every one is optional; a run with none
/// asserts nothing and always exits zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Budget {
    /// The p99 of INVITE → `200 OK`.
    pub max_p99_invite_ms: Option<f64>,
    /// Calls answered per second, over the run.
    pub min_admitted_cps: Option<f64>,
    /// Loss across every leg, as a fraction (0.05 is five percent).
    pub max_rtp_loss: Option<f64>,
    /// Refused calls as a fraction of those offered.
    pub max_refused_fraction: Option<f64>,
    /// Calls that ended by themselves.
    pub max_dropped: Option<u64>,
    /// Legs that heard nothing at all.
    pub max_silent_legs: Option<u64>,
    /// The p99 of REGISTER → its final response.
    pub max_p99_register_ms: Option<f64>,
}

impl Budget {
    /// Whether anything is asserted at all.
    pub fn is_empty(&self) -> bool {
        self.max_p99_invite_ms.is_none()
            && self.min_admitted_cps.is_none()
            && self.max_rtp_loss.is_none()
            && self.max_refused_fraction.is_none()
            && self.max_dropped.is_none()
            && self.max_silent_legs.is_none()
            && self.max_p99_register_ms.is_none()
    }

    /// Judge a run's measurements, one verdict per bound asserted.
    pub fn judge(&self, m: &Measurements) -> Vec<BudgetResult> {
        let mut out = Vec::new();
        let mut at_most = |name: &str, limit: Option<f64>, measured: f64| {
            if let Some(limit) = limit {
                out.push(BudgetResult {
                    name: name.to_string(),
                    limit,
                    measured,
                    held: measured <= limit,
                });
            }
        };
        at_most(
            "max-p99-invite-ms",
            self.max_p99_invite_ms,
            m.invite_to_answer.p99_ms,
        );
        at_most("max-rtp-loss", self.max_rtp_loss, m.media.loss_fraction);
        at_most(
            "max-refused-fraction",
            self.max_refused_fraction,
            m.refused_fraction,
        );
        at_most(
            "max-dropped",
            self.max_dropped.map(|n| n as f64),
            m.dropped as f64,
        );
        at_most(
            "max-silent-legs",
            self.max_silent_legs.map(|n| n as f64),
            m.media.silent_legs as f64,
        );
        at_most(
            "max-p99-register-ms",
            self.max_p99_register_ms,
            m.register_latency.p99_ms,
        );
        if let Some(limit) = self.min_admitted_cps {
            out.push(BudgetResult {
                name: "min-admitted-cps".to_string(),
                limit,
                measured: m.answered_cps,
                held: m.answered_cps >= limit,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{LatencySummary, MediaSummary};

    fn measurements() -> Measurements {
        Measurements {
            offered: 100,
            answered: 90,
            refused: 10,
            refused_fraction: 0.1,
            answered_cps: 9.0,
            invite_to_answer: LatencySummary {
                count: 90,
                p99_ms: 250.0,
                ..LatencySummary::default()
            },
            media: MediaSummary {
                legs: 90,
                loss_fraction: 0.02,
                silent_legs: 1,
                ..MediaSummary::default()
            },
            ..Measurements::default()
        }
    }

    #[test]
    fn a_run_with_no_budget_asserts_nothing() {
        assert!(Budget::default().is_empty());
        assert!(Budget::default().judge(&measurements()).is_empty());
    }

    #[test]
    fn an_upper_bound_holds_at_the_limit_and_breaks_past_it() {
        let m = measurements();
        let held = Budget {
            max_p99_invite_ms: Some(250.0),
            ..Budget::default()
        }
        .judge(&m);
        assert!(held[0].held, "at the limit is within it");

        let broken = Budget {
            max_p99_invite_ms: Some(249.0),
            ..Budget::default()
        }
        .judge(&m);
        assert!(!broken[0].held);
        assert_eq!(broken[0].measured, 250.0);
    }

    /// The one bound that is a floor rather than a ceiling.
    #[test]
    fn the_admitted_rate_is_a_minimum() {
        let m = measurements();
        assert!(
            Budget {
                min_admitted_cps: Some(9.0),
                ..Budget::default()
            }
            .judge(&m)[0]
                .held
        );
        assert!(
            !Budget {
                min_admitted_cps: Some(9.5),
                ..Budget::default()
            }
            .judge(&m)[0]
                .held
        );
    }

    #[test]
    fn every_bound_asserted_gets_a_verdict() {
        let all = Budget {
            max_p99_invite_ms: Some(1000.0),
            min_admitted_cps: Some(1.0),
            max_rtp_loss: Some(0.05),
            max_refused_fraction: Some(0.2),
            max_dropped: Some(0),
            max_silent_legs: Some(0),
            max_p99_register_ms: Some(500.0),
        };
        let verdicts = all.judge(&measurements());
        assert_eq!(verdicts.len(), 7);
        let silent = verdicts
            .iter()
            .find(|v| v.name == "max-silent-legs")
            .unwrap();
        assert!(
            !silent.held,
            "one leg heard nothing and the budget said none"
        );
    }
}
