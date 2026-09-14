//! Where the shipper sends records.

use std::sync::Arc;

use async_trait::async_trait;
use fcp_core::storage::CallDetailRecord;

use crate::processor::Processor;
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
/// own store. This is the deployment without Redis.
pub struct LocalSink {
    processor: Arc<Processor>,
}

impl LocalSink {
    pub fn new(processor: Arc<Processor>) -> Self {
        Self { processor }
    }
}

#[async_trait]
impl Sink for LocalSink {
    async fn ship(&self, records: Vec<CallDetailRecord>) -> Result<()> {
        self.processor
            .process(records)
            .await
            .map(|_| ())
            .map_err(|e| match e {
                Error::Store(inner) => Error::Store(inner),
                other => Error::Sink(other.to_string()),
            })
    }

    fn name(&self) -> &'static str {
        "local"
    }
}
