//! Whether the primary answers (`docs/DEEP_TESTING.md` D5).
//!
//! A node on its call path cannot wait for a database that has gone away:
//! a phone gives up on a REGISTER or an INVITE after 32 seconds, and a
//! partitioned Postgres holds a query open for as long as the network
//! keeps the connection, with no answer and no error. So a probe asks the
//! primary `SELECT 1` every second, giving up after two, and the node
//! reads the verdict before it starts work that needs the store: while
//! the primary is down, a new registration or call is refused at once
//! with a 503 the phone can act on, rather than left to its timer.
//!
//! Two failed probes in a row mark the primary down, so one slow answer
//! under load does not refuse anyone; one good probe marks it up again.
//!
//! The probe also asks whether the server is in recovery. A standby
//! answers every read and refuses every write, and the primary's address
//! leads to one during a failover that has moved the address before the
//! promotion, or to an old primary come back as a standby while a node
//! still resolves the old address (D5c). Such a store is up and not
//! writable: a node keeps placing calls, which only read, and refuses
//! registrations, which write, with the same 503.

use sqlx::{Pool, Postgres};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// How often the primary is asked.
pub const PROBE_EVERY: Duration = Duration::from_secs(1);
/// How long a probe waits for its answer.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Failed probes in a row before the primary is down.
const FAILURES_TO_DOWN: u32 = 2;

/// The primary's health, as the last probes found it.
#[derive(Debug)]
pub struct PrimaryHealth {
    up: AtomicBool,
    /// Whether the server takes writes: false while it is a standby.
    writable: AtomicBool,
    /// Unix seconds when it went down; 0 while up.
    down_since: AtomicU64,
    /// Times it has gone down since the node started.
    outages: AtomicU64,
}

impl PrimaryHealth {
    /// Up, and probed by nothing: what a node without a database, or a
    /// test, holds.
    pub fn always_up() -> Arc<Self> {
        Arc::new(Self {
            up: AtomicBool::new(true),
            writable: AtomicBool::new(true),
            down_since: AtomicU64::new(0),
            outages: AtomicU64::new(0),
        })
    }

    /// Probe `pool` for as long as the returned health is held.
    pub fn watch(pool: Pool<Postgres>) -> Arc<Self> {
        let health = Self::always_up();
        let weak = Arc::downgrade(&health);
        tokio::spawn(async move {
            let mut failures = 0u32;
            let mut tick = tokio::time::interval(PROBE_EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(health) = weak.upgrade() else { return };
                let probe = tokio::time::timeout(
                    PROBE_TIMEOUT,
                    sqlx::query_scalar::<_, bool>("SELECT pg_is_in_recovery()").fetch_one(&pool),
                )
                .await;
                let outcome = match probe {
                    Ok(Ok(in_recovery)) => Ok(in_recovery),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err(format!("no answer in {PROBE_TIMEOUT:?}")),
                };
                match outcome {
                    Ok(in_recovery) => {
                        failures = 0;
                        health.mark_up();
                        health.mark_writable(!in_recovery);
                    }
                    Err(error) => {
                        failures += 1;
                        if failures >= FAILURES_TO_DOWN {
                            health.mark_down(&error);
                        }
                    }
                }
            }
        });
        health
    }

    fn mark_up(&self) {
        if !self.up.swap(true, Ordering::AcqRel) {
            let since = self.down_since.swap(0, Ordering::AcqRel);
            info!(
                down_for_secs = now().saturating_sub(since),
                "The database answers again; registrations and calls are taken"
            );
        }
    }

    fn mark_down(&self, error: &str) {
        if self.up.swap(false, Ordering::AcqRel) {
            self.down_since.store(now(), Ordering::Release);
            self.outages.fetch_add(1, Ordering::Relaxed);
            warn!(
                error,
                "The database does not answer; new registrations and calls are refused with 503"
            );
        }
    }

    fn mark_writable(&self, writable: bool) {
        if self.writable.swap(writable, Ordering::AcqRel) != writable {
            if writable {
                info!("The database takes writes again; registrations are taken");
            } else {
                warn!(
                    "The database is a standby (in recovery) and refuses writes; \
                     new registrations are refused with 503"
                );
            }
        }
    }

    /// Whether the primary answered the last probes.
    pub fn is_up(&self) -> bool {
        self.up.load(Ordering::Acquire)
    }

    /// Whether it answered and takes writes: not a standby.
    pub fn is_writable(&self) -> bool {
        self.is_up() && self.writable.load(Ordering::Acquire)
    }

    /// What `/status` and `/ready` say under `database`.
    pub fn json(&self) -> serde_json::Value {
        let since = self.down_since.load(Ordering::Acquire);
        serde_json::json!({
            "up": self.is_up(),
            "writable": self.is_writable(),
            "down_for_secs": (since > 0).then(|| now().saturating_sub(since)),
            "outages": self.outages.load(Ordering::Relaxed),
        })
    }

    /// The series for `/metrics`.
    pub fn prometheus(&self, labels: &str) -> String {
        format!(
            "# HELP fcp_database_up Whether the primary database answered the last probes.\n\
             # TYPE fcp_database_up gauge\n\
             fcp_database_up{{{labels}}} {}\n\
             # HELP fcp_database_writable Whether the primary database takes writes (0 while it is a standby).\n\
             # TYPE fcp_database_writable gauge\n\
             fcp_database_writable{{{labels}}} {}\n\
             # HELP fcp_database_outages_total Times the primary has stopped answering.\n\
             # TYPE fcp_database_outages_total counter\n\
             fcp_database_outages_total{{{labels}}} {}\n",
            u8::from(self.is_up()),
            u8::from(self.is_writable()),
            self.outages.load(Ordering::Relaxed),
        )
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn down_and_up_again() {
        let health = PrimaryHealth::always_up();
        assert!(health.is_up());
        health.mark_down("refused");
        assert!(!health.is_up());
        assert_eq!(health.json()["up"], false);
        assert_eq!(health.json()["outages"], 1);
        assert!(health
            .prometheus("node=\"a\"")
            .contains("fcp_database_up{node=\"a\"} 0"));
        health.mark_up();
        assert!(health.is_up());
        assert!(health.json()["down_for_secs"].is_null());
        health.mark_down("refused");
        assert_eq!(health.json()["outages"], 2);
    }

    #[test]
    fn a_standby_is_up_and_not_writable() {
        let health = PrimaryHealth::always_up();
        assert!(health.is_writable());
        health.mark_writable(false);
        assert!(health.is_up());
        assert!(!health.is_writable());
        assert_eq!(health.json()["writable"], false);
        assert!(health
            .prometheus("node=\"a\"")
            .contains("fcp_database_writable{node=\"a\"} 0"));
        // Down is not writable either, whatever the last answer said.
        health.mark_writable(true);
        health.mark_down("refused");
        assert!(!health.is_writable());
        health.mark_up();
        assert!(health.is_writable());
    }
}
