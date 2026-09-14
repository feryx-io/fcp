//! Where the shipper sends records.

use std::sync::Arc;

use async_trait::async_trait;
use fcp_core::storage::CallDetailRecord;
use tracing::error;

use crate::processor::Processor;
use crate::spool::Spool;
use crate::{Error, Result};

/// Accepts a batch of records. `Ok` means every record in the batch is
/// safe with the sink and the spool may move past it; `Err` means none
/// may be assumed so, and the shipper will send the batch again.
#[async_trait]
pub trait Sink: Send + Sync {
    async fn ship(&self, records: Vec<CallDetailRecord>) -> Result<()>;

    /// A name for logs and metrics.
    fn name(&self) -> &'static str;
}

/// The in-process sink: the processor runs right here, against the node's
/// own store. Records the processor refuses are kept in the spool's
/// `dead.jsonl`.
pub struct LocalSink {
    processor: Arc<Processor>,
    dead_letters: Option<Spool>,
}

impl LocalSink {
    pub fn new(processor: Arc<Processor>) -> Self {
        Self {
            processor,
            dead_letters: None,
        }
    }

    /// Keep refused records in `spool`'s dead letters.
    pub fn with_dead_letters(mut self, spool: Spool) -> Self {
        self.dead_letters = Some(spool);
        self
    }
}

#[async_trait]
impl Sink for LocalSink {
    async fn ship(&self, records: Vec<CallDetailRecord>) -> Result<()> {
        let outcome = self.processor.process(records).await?;
        for rejection in outcome.rejected {
            match &self.dead_letters {
                Some(spool) => {
                    if let Err(e) = spool.dead_letter_record(&rejection.record, &rejection.reason) {
                        // Refuse the batch: it comes back and is tried again.
                        return Err(Error::Sink(format!("could not keep a dead letter: {e}")));
                    }
                }
                None => {
                    error!(cdr_id = %rejection.record.cdr_id, reason = %rejection.reason, record = %serde_json::to_string(&rejection.record).unwrap_or_default(), "CDR refused and no spool to keep it in");
                }
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "local"
    }
}
