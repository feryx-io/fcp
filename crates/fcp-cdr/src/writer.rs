//! What a node hands its records to.

use std::sync::Arc;

use fcp_core::storage::CallDetailRecord;
use tracing::{error, warn};

use crate::open::OpenRecords;
use crate::processor::Processor;
use crate::spool::Spool;

/// Takes a finished record off the call path. With a spool, `write` is a
/// synchronous append; without one, the record goes to the processor on a
/// task of its own, which is the MVP's durability. With both, the
/// processor is the fallback for a spool append that fails (a full disk),
/// so a record is never dropped for want of a place to put it.
#[derive(Clone, Default)]
pub struct CdrWriter {
    spool: Option<Spool>,
    /// The records of calls still up, beside the spool.
    open: Option<OpenRecords>,
    direct: Option<Arc<Processor>>,
    /// Records handed to the processor directly and not yet stored.
    in_flight: Arc<std::sync::atomic::AtomicUsize>,
}

impl std::fmt::Debug for CdrWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdrWriter")
            .field("spool", &self.spool)
            .field("direct", &self.direct.is_some())
            .finish()
    }
}

impl CdrWriter {
    /// Records go to `spool`; `fallback` takes them if the spool cannot.
    pub fn spooled(spool: Spool, fallback: Option<Arc<Processor>>) -> Self {
        let open = match OpenRecords::at(spool.dir()) {
            Ok(open) => Some(open),
            Err(e) => {
                warn!(error = %e, "No open records: a call up when this node stops will have no record");
                None
            }
        };
        Self {
            open,
            spool: Some(spool),
            direct: fallback,
            in_flight: Arc::default(),
        }
    }

    /// No spool: records go straight to `processor`, each on its own task.
    pub fn direct(processor: Arc<Processor>) -> Self {
        Self {
            open: None,
            spool: None,
            direct: Some(processor),
            in_flight: Arc::default(),
        }
    }

    /// Whether records have anywhere to go.
    pub fn is_configured(&self) -> bool {
        self.spool.is_some() || self.direct.is_some()
    }

    /// Wait, up to `timeout`, for the records already written to leave the
    /// node: the direct writes to be stored, the spool to be shipped and
    /// synced. Called by a node stopping gracefully, so the last calls'
    /// records do not wait for its next start (or its disk). Returns
    /// whether everything left.
    pub async fn flush(&self, timeout: std::time::Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let direct_done = self.in_flight.load(std::sync::atomic::Ordering::Relaxed) == 0;
            let spool_done = match &self.spool {
                Some(spool) => spool
                    .status()
                    .map(|s| s.pending_bytes == 0)
                    .unwrap_or(false),
                None => true,
            };
            if direct_done && spool_done {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                warn!(
                    direct_pending = self.in_flight.load(std::sync::atomic::Ordering::Relaxed),
                    "CDRs still on the node at shutdown; the spool keeps them for the next start"
                );
                if let Some(spool) = &self.spool {
                    let _ = spool.sync();
                }
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        if let Some(spool) = &self.spool {
            let _ = spool.sync();
        }
        true
    }

    /// The spool, for status.
    pub fn spool(&self) -> Option<&Spool> {
        self.spool.as_ref()
    }

    /// The in-process processor records go to (or fall back to), for
    /// status.
    pub fn processor(&self) -> Option<&Arc<Processor>> {
        self.direct.as_ref()
    }

    /// The node's part of a status route: what the spool holds and what
    /// the in-process processor has done.
    pub fn status(&self) -> serde_json::Value {
        let spool = self.spool.as_ref().map(|spool| match spool.status() {
            Ok(s) => serde_json::json!({
                "dir": spool.dir().display().to_string(),
                "pending_bytes": s.pending_bytes,
                "segments": s.segments,
                "oldest_pending_age_seconds": s.oldest_pending_age.map(|d| d.as_secs_f64()),
            }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        });
        serde_json::json!({
            "spool": spool,
            "processor": self.direct.as_ref().map(|p| p.stats().snapshot()),
        })
    }

    /// The node's part of `/metrics`, with `labels` on every sample.
    pub fn prometheus(&self, labels: &str) -> String {
        let mut out = String::new();
        if let Some(Ok(status)) = self.spool.as_ref().map(|s| s.status()) {
            for (name, help, value) in [
                (
                    "cdr_spool_pending_bytes",
                    "Bytes of records spooled and not yet shipped.",
                    status.pending_bytes as f64,
                ),
                (
                    "cdr_spool_segments",
                    "Spool segment files on disk.",
                    status.segments as f64,
                ),
                (
                    "cdr_spool_oldest_age_seconds",
                    "How long the oldest unshipped record has waited.",
                    status
                        .oldest_pending_age
                        .map(|d| d.as_secs_f64())
                        .unwrap_or(0.0),
                ),
            ] {
                crate::stats::family(&mut out, name, "gauge", help);
                out.push_str(&format!("{name}{{{labels}}} {value}\n"));
            }
        }
        if let Some(processor) = &self.direct {
            out.push_str(&processor.stats().snapshot().prometheus(labels));
        }
        out
    }

    /// Keep `record` as what is written if this node stops before the
    /// call ends (`crate::open`). Only with a spool; a file write, so off
    /// the call path.
    pub fn keep_open(&self, record: &CallDetailRecord) {
        if let Some(open) = &self.open {
            open.put(record);
        }
    }

    /// Whether this writer keeps open records.
    pub fn keeps_open(&self) -> bool {
        self.open.is_some()
    }

    /// The call's record was written: its open record goes.
    pub fn close_open(&self, call_id: &fcp_core::CallId) {
        if let Some(open) = &self.open {
            open.close(call_id);
        }
    }

    /// Take the record. Never blocks on storage; never panics.
    pub fn write(&self, record: CallDetailRecord) {
        if let Some(spool) = &self.spool {
            match spool.append(&record) {
                Ok(()) => return,
                Err(e) => {
                    error!(cdr_id = %record.cdr_id, call_id = %record.call_id, error = %e, "CDR spool append failed; writing the record directly");
                }
            }
        }
        match &self.direct {
            Some(processor) => {
                let processor = Arc::clone(processor);
                let in_flight = Arc::clone(&self.in_flight);
                in_flight.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tokio::spawn(async move {
                    let _done = InFlight(in_flight);
                    let cdr_id = record.cdr_id;
                    let call_id = record.call_id;
                    match processor.process(vec![record]).await {
                        Ok(outcome) => {
                            for rejection in outcome.rejected {
                                error!(%cdr_id, %call_id, reason = %rejection.reason, record = %serde_json::to_string(&rejection.record).unwrap_or_default(), "CDR refused and no spool to keep it in");
                            }
                        }
                        Err(e) => {
                            error!(%cdr_id, %call_id, error = %e, "CDR lost: no spool and the store is unavailable");
                        }
                    }
                });
            }
            None => {
                warn!(cdr_id = %record.cdr_id, call_id = %record.call_id, "CDR dropped: no spool and no store configured")
            }
        }
    }
}

/// Counts a direct write down when its task ends, however it ends.
struct InFlight(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor::tests::{record, MemStore};
    use crate::spool::SpoolConfig;
    use std::time::Duration;

    #[tokio::test]
    async fn flush_waits_for_direct_writes_and_reports_an_unshipped_spool() {
        let store = Arc::new(MemStore::default());
        let direct = CdrWriter::direct(Arc::new(Processor::new(store.clone(), None)));
        direct.write(record("sip:bob@x"));
        assert!(direct.flush(Duration::from_secs(2)).await);
        assert_eq!(store.records.lock().unwrap().len(), 1);

        // A spool nothing ships from cannot be flushed.
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::open(dir.path(), SpoolConfig::default()).unwrap();
        let spooled = CdrWriter::spooled(spool, None);
        spooled.write(record("sip:bob@x"));
        assert!(!spooled.flush(Duration::from_millis(150)).await);
    }
}
