//! The shipper: drains the spool into a sink, committing the cursor only
//! after the sink acknowledged a batch.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::sink::Sink;
use crate::spool::Spool;

/// How the shipper paces itself.
#[derive(Debug, Clone)]
pub struct ShipperConfig {
    /// Records per batch.
    pub batch: usize,
    /// How long to wait for an append before looking again anyway.
    pub poll: Duration,
    /// The first delay after a sink failure; doubles up to `max_backoff`.
    pub backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for ShipperConfig {
    fn default() -> Self {
        Self {
            batch: 200,
            poll: Duration::from_secs(1),
            backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// Drains a spool into a sink until aborted.
pub struct Shipper;

impl Shipper {
    /// Start shipping. The task runs until the handle is aborted; records
    /// not yet shipped stay in the spool for the next start.
    pub fn spawn(spool: Spool, sink: Arc<dyn Sink>, config: ShipperConfig) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!(dir = %spool.dir().display(), sink = sink.name(), "CDR shipper started");
            let mut backoff = config.backoff;
            loop {
                let cursor = match spool.cursor() {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(error = %e, "CDR shipper could not read the spool cursor");
                        tokio::time::sleep(config.poll).await;
                        continue;
                    }
                };
                let reader = spool.clone();
                let batch = config.batch;
                let read =
                    tokio::task::spawn_blocking(move || reader.read_from(cursor, batch)).await;
                let (records, next) = match read {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => {
                        warn!(error = %e, "CDR shipper could not read the spool");
                        tokio::time::sleep(config.poll).await;
                        continue;
                    }
                    Err(e) => {
                        warn!(error = %e, "CDR shipper read task panicked");
                        tokio::time::sleep(config.poll).await;
                        continue;
                    }
                };
                if records.is_empty() {
                    tokio::select! {
                        _ = spool.notified() => {}
                        _ = tokio::time::sleep(config.poll) => {}
                    }
                    continue;
                }
                let count = records.len();
                match sink.ship(records).await {
                    Ok(()) => {
                        let committer = spool.clone();
                        match tokio::task::spawn_blocking(move || committer.commit(next)).await {
                            Ok(Ok(())) => debug!(count, ?next, "CDR batch shipped"),
                            Ok(Err(e)) => {
                                warn!(error = %e, "CDR shipper could not commit the cursor; the batch will ship again")
                            }
                            Err(e) => warn!(error = %e, "CDR shipper commit task panicked"),
                        }
                        backoff = config.backoff;
                    }
                    Err(e) => {
                        warn!(error = %e, count, retry_in = ?backoff, "CDR batch not shipped; retrying");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(config.max_backoff);
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor::tests::MemStore;
    use crate::processor::Processor;
    use crate::record::RecordBuilder;
    use crate::sink::LocalSink;
    use crate::spool::SpoolConfig;
    use fcp_core::storage::{CallDetailRecord, CdrKind};
    use fcp_core::{CallId, TenantId};
    use std::sync::atomic::Ordering;

    fn record() -> CallDetailRecord {
        RecordBuilder::new(CdrKind::Call, CallId::new(), TenantId::new())
            .from("sip:a@x", None)
            .to("sip:b@x", None)
            .build()
    }

    async fn wait_for(store: &MemStore, n: usize) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while store.records.lock().unwrap().len() < n {
            if tokio::time::Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        true
    }

    #[tokio::test]
    async fn records_reach_the_store_and_a_store_outage_delays_them_without_loss() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::open(dir.path(), SpoolConfig::default()).unwrap();
        let store = Arc::new(MemStore::default());
        let processor = Arc::new(Processor::new(store.clone(), None));
        let config = ShipperConfig {
            batch: 10,
            poll: Duration::from_millis(50),
            backoff: Duration::from_millis(20),
            max_backoff: Duration::from_millis(100),
        };
        let task = Shipper::spawn(spool.clone(), Arc::new(LocalSink::new(processor)), config);

        let first = record();
        spool.append(&first).unwrap();
        assert!(wait_for(&store, 1).await, "the first record lands");
        assert_eq!(store.records.lock().unwrap()[0].cdr_id, first.cdr_id);

        // The store goes away; records pile up in the spool.
        store.fail.store(true, Ordering::Relaxed);
        let ids: Vec<_> = (0..25)
            .map(|_| {
                let r = record();
                spool.append(&r).unwrap();
                r.cdr_id
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(store.records.lock().unwrap().len(), 1);
        assert!(spool.status().unwrap().pending_bytes > 0);

        // It comes back; everything lands, once, in order.
        store.fail.store(false, Ordering::Relaxed);
        assert!(wait_for(&store, 26).await, "the backlog lands");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let stored: Vec<_> = store
            .records
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.cdr_id)
            .collect();
        assert_eq!(stored.len(), 26);
        assert_eq!(&stored[1..], &ids[..]);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while spool.status().unwrap().pending_bytes > 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the cursor catches up"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        task.abort();
    }

    #[tokio::test]
    async fn a_spool_left_by_an_earlier_run_is_drained_on_start() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<_> = {
            let spool = Spool::open(dir.path(), SpoolConfig::default()).unwrap();
            (0..3)
                .map(|_| {
                    let r = record();
                    spool.append(&r).unwrap();
                    r.cdr_id
                })
                .collect()
        };
        let spool = Spool::open(dir.path(), SpoolConfig::default()).unwrap();
        let store = Arc::new(MemStore::default());
        let processor = Arc::new(Processor::new(store.clone(), None));
        let task = Shipper::spawn(
            spool,
            Arc::new(LocalSink::new(processor)),
            ShipperConfig::default(),
        );
        assert!(wait_for(&store, 3).await);
        let stored: Vec<_> = store
            .records
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.cdr_id)
            .collect();
        assert_eq!(stored, ids);
        task.abort();
    }
}
