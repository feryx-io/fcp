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
                    if let Err(e) = processor.process(vec![record]).await {
                        error!(%cdr_id, %call_id, error = %e, "CDR lost: no spool and the store refused it");
                    }
                });
            }
            None => {
                warn!(cdr_id = %record.cdr_id, call_id = %record.call_id, "CDR dropped: no spool and no store configured")
            }
        }
    }
}
