//! What a node hands its records to.

use std::sync::Arc;

use fcp_core::storage::CallDetailRecord;
use tracing::{error, warn};

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
    direct: Option<Arc<Processor>>,
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
        Self {
            spool: Some(spool),
            direct: fallback,
        }
    }

    /// No spool: records go straight to `processor`, each on its own task.
    pub fn direct(processor: Arc<Processor>) -> Self {
        Self {
            spool: None,
            direct: Some(processor),
        }
    }

    /// Whether records have anywhere to go.
    pub fn is_configured(&self) -> bool {
        self.spool.is_some() || self.direct.is_some()
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
                tokio::spawn(async move {
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
