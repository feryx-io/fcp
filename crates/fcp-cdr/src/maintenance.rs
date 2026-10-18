//! The record table's upkeep (`docs/CDR_PIPELINE.md` §8–9): the coming
//! months' partitions and the hourly usage rollups. Run by `fcp-admin` and
//! by the processor in `fcp-jobs`; the store's locks keep each job to one
//! host at a time, so running it in more places costs nothing but a lock
//! attempt.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fcp_core::storage::CdrStore;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// How often the jobs run.
#[derive(Debug, Clone)]
pub struct MaintenanceConfig {
    /// How often the rollups are rebuilt.
    pub rollup_every: Duration,
    /// How far back each rebuild recomputes (a record written later than
    /// this after its hour is not in the rollups).
    pub rollup_window: Duration,
    /// How often the coming months' partitions are checked.
    pub partitions_every: Duration,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            rollup_every: Duration::from_secs(600),
            rollup_window: Duration::from_secs(48 * 3600),
            partitions_every: Duration::from_secs(6 * 3600),
        }
    }
}

/// The lock the scheduled rebuild takes, so one host runs it and the
/// others skip the round. Not the rebuild's own lock (`fcp:cdr-rollups`,
/// taken inside its transaction for any caller): an advisory lock held
/// by one connection blocks the same lock from another, so sharing the
/// name left the rebuild waiting on its own scheduler for as long as the
/// process lived (`docs/DEEP_TESTING.md` D6).
pub const ROLLUP_LOCK: &str = "fcp:cdr-rollup-schedule";

/// Start the upkeep loop; the first round runs at once.
pub fn spawn_maintenance(store: Arc<dyn CdrStore>, config: MaintenanceConfig) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_partitions: Option<Instant> = None;
        let mut interval = tokio::time::interval(config.rollup_every);
        loop {
            interval.tick().await;
            if last_partitions.is_none_or(|t| t.elapsed() >= config.partitions_every) {
                match store.ensure_cdr_partitions().await {
                    Ok(created) if !created.is_empty() => {
                        info!(partitions = ?created, "Created CDR partitions")
                    }
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "Could not create CDR partitions"),
                }
                last_partitions = Some(Instant::now());
            }
            rebuild_rollups(&*store, config.rollup_window).await;
        }
    })
}

/// One rollup rebuild, if no other host is running one.
pub async fn rebuild_rollups(store: &dyn CdrStore, window: Duration) {
    let lock = match store.try_maintenance_lock(ROLLUP_LOCK).await {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            debug!("Another host is rebuilding the usage rollups");
            return;
        }
        Err(e) => {
            warn!(error = %e, "Could not take the rollup lock");
            return;
        }
    };
    let started = Instant::now();
    match store.rebuild_usage_rollups(window).await {
        Ok(run) => info!(
            hours = run.hours,
            rows = run.rows,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "Rebuilt usage rollups"
        ),
        Err(e) => warn!(error = %e, "Usage rollup rebuild failed"),
    }
    lock.release().await;
}
