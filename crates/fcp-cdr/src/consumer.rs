//! The processor's loop behind a durable source of records — the Redis
//! stream in `fcp-jobs` (`docs/CDR_PIPELINE.md` §4.2–4.3). The source is a
//! trait so the loop is tested without Redis.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use fcp_core::storage::CallDetailRecord;
use serde::Serialize;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::processor::Processor;

/// One entry read from the source: its id, for acknowledgement, and the
/// record, or why it could not be decoded along with its raw text.
#[derive(Debug, Clone)]
pub struct SourcedRecord {
    pub id: String,
    pub record: std::result::Result<CallDetailRecord, Undecodable>,
}

/// An entry that is not a record this build can read.
#[derive(Debug, Clone)]
pub struct Undecodable {
    pub raw: String,
    pub reason: String,
}

/// How much the source holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SourceStatus {
    /// Entries in the stream.
    pub length: u64,
    /// Entries delivered to a consumer and not yet acknowledged.
    pub pending: u64,
    /// Entries in the dead-letter stream.
    pub dead_letters: u64,
}

/// A durable, acknowledged source of records.
#[async_trait]
pub trait RecordSource: Send + Sync {
    /// Prepare the source (create the consumer group).
    async fn prepare(&self) -> anyhow::Result<()>;

    /// Up to `count` entries not yet delivered to any consumer, waiting up
    /// to `block` for the first.
    async fn read(&self, count: usize, block: Duration) -> anyhow::Result<Vec<SourcedRecord>>;

    /// Entries delivered to a consumer that has not acknowledged them for
    /// `idle` (it died mid-batch), taken over by this one.
    async fn claim_stale(&self, idle: Duration, count: usize)
        -> anyhow::Result<Vec<SourcedRecord>>;

    /// Mark entries done.
    async fn ack(&self, ids: &[String]) -> anyhow::Result<()>;

    /// Keep an entry that will never be stored where a person will look.
    async fn dead_letter(&self, id: &str, raw: &str, reason: &str) -> anyhow::Result<()>;

    /// Drop entries that are acknowledged and older than the replay window.
    /// Returns how many were dropped.
    async fn trim(&self) -> anyhow::Result<u64>;

    async fn status(&self) -> anyhow::Result<SourceStatus>;
}

/// How the consumer paces itself.
#[derive(Debug, Clone)]
pub struct ConsumerConfig {
    /// Entries per batch.
    pub batch: usize,
    /// How long a read waits for the first entry.
    pub batch_wait: Duration,
    /// An entry unacknowledged this long belongs to a dead consumer.
    pub claim_idle: Duration,
    /// How often to look for such entries.
    pub claim_every: Duration,
    /// How often to trim the source.
    pub trim_every: Duration,
    /// First delay after the store failed a batch; doubles up to
    /// `max_backoff`.
    pub backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for ConsumerConfig {
    fn default() -> Self {
        Self {
            batch: 500,
            batch_wait: Duration::from_millis(200),
            claim_idle: Duration::from_secs(60),
            claim_every: Duration::from_secs(30),
            trim_every: Duration::from_secs(300),
            backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// A running consumer.
pub struct Consumer {
    stop: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl Consumer {
    /// Start consuming `source` into `processor`. Needs a tokio runtime.
    pub fn spawn(
        source: Arc<dyn RecordSource>,
        processor: Arc<Processor>,
        config: ConsumerConfig,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(run(source, processor, config, Arc::clone(&stop)));
        Self { stop, task }
    }

    /// Finish the batch in hand and stop. Unacknowledged entries stay in
    /// the source for the next consumer.
    pub async fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.task.await;
    }
}

async fn run(
    source: Arc<dyn RecordSource>,
    processor: Arc<Processor>,
    config: ConsumerConfig,
    stop: Arc<AtomicBool>,
) {
    let mut backoff = config.backoff;
    while let Err(e) = source.prepare().await {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        warn!(error = %e, retry_in = ?backoff, "CDR source not ready");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(config.max_backoff);
    }
    info!(batch = config.batch, "CDR consumer started");

    // Claim at once: a restart after a crash picks up its own entries.
    let mut last_claim = Instant::now() - config.claim_every;
    let mut last_trim = Instant::now();
    backoff = config.backoff;

    while !stop.load(Ordering::Relaxed) {
        let entries = if last_claim.elapsed() >= config.claim_every {
            last_claim = Instant::now();
            match source.claim_stale(config.claim_idle, config.batch).await {
                Ok(claimed) if !claimed.is_empty() => {
                    warn!(
                        count = claimed.len(),
                        "Claimed CDR entries a consumer left unacknowledged"
                    );
                    Ok(claimed)
                }
                Ok(_) => source.read(config.batch, config.batch_wait).await,
                Err(e) => Err(e),
            }
        } else {
            source.read(config.batch, config.batch_wait).await
        };
        let entries = match entries {
            Ok(entries) => {
                backoff = config.backoff;
                entries
            }
            Err(e) => {
                warn!(error = %e, retry_in = ?backoff, "CDR source read failed");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(config.max_backoff);
                continue;
            }
        };

        if !entries.is_empty() {
            handle_batch(&*source, &processor, entries, &config, &stop).await;
        }

        if last_trim.elapsed() >= config.trim_every {
            last_trim = Instant::now();
            match source.trim().await {
                Ok(0) => {}
                Ok(dropped) => info!(dropped, "Trimmed the CDR stream"),
                Err(e) => warn!(error = %e, "CDR stream trim failed"),
            }
        }
    }
    info!("CDR consumer stopped");
}

/// Process one batch to the end: undecodable entries are dead-lettered,
/// the rest go to the processor, which is retried with backoff for as long
/// as the store is away. Entries are acknowledged only once their records
/// are stored or kept as dead letters.
async fn handle_batch(
    source: &dyn RecordSource,
    processor: &Processor,
    entries: Vec<SourcedRecord>,
    config: &ConsumerConfig,
    stop: &AtomicBool,
) {
    let mut done: Vec<String> = Vec::with_capacity(entries.len());
    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry.record {
            Ok(record) => records.push((entry.id, record)),
            Err(bad) => {
                error!(id = %entry.id, reason = %bad.reason, "CDR stream entry does not decode; dead-lettered");
                if keep_dead(source, &entry.id, &bad.raw, &bad.reason).await {
                    processor.stats().record_rejected(1);
                    done.push(entry.id);
                }
            }
        }
    }

    let mut backoff = config.backoff;
    loop {
        match processor.process_keyed(records.clone()).await {
            Ok(outcome) => {
                done.extend(outcome.stored.iter().cloned());
                done.extend(outcome.duplicates.iter().cloned());
                for rejection in &outcome.rejected {
                    let raw = serde_json::to_string(&rejection.record).unwrap_or_default();
                    if keep_dead(source, &rejection.key, &raw, &rejection.reason).await {
                        done.push(rejection.key.clone());
                    }
                }
                break;
            }
            Err(e) => {
                if stop.load(Ordering::Relaxed) {
                    // Leave the batch unacknowledged for whoever comes next.
                    warn!(error = %e, "CDR batch not stored before stopping");
                    return;
                }
                warn!(error = %e, records = records.len(), retry_in = ?backoff, "CDR batch not stored; retrying");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(config.max_backoff);
            }
        }
    }

    if let Err(e) = source.ack(&done).await {
        // Unacknowledged entries are claimed and replayed; the replay
        // stores nothing twice.
        warn!(error = %e, count = done.len(), "CDR acknowledgement failed; the entries will be replayed");
    }
}

async fn keep_dead(source: &dyn RecordSource, id: &str, raw: &str, reason: &str) -> bool {
    match source.dead_letter(id, raw, reason).await {
        Ok(()) => true,
        Err(e) => {
            // Not acknowledged: it comes back and is tried again.
            error!(id = %id, error = %e, "Could not keep a CDR dead letter");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor::tests::{record, MemStore};
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    /// A source in memory with Redis's delivery rules: an entry read is
    /// pending until acknowledged, and a pending entry idle long enough can
    /// be claimed.
    #[derive(Default)]
    struct MemSource {
        undelivered: Mutex<VecDeque<SourcedRecord>>,
        pending: Mutex<HashMap<String, (Instant, SourcedRecord)>>,
        acked: Mutex<Vec<String>>,
        dead: Mutex<Vec<(String, String)>>,
        fail_reads: AtomicBool,
    }

    impl MemSource {
        fn push(&self, id: &str, record: std::result::Result<CallDetailRecord, Undecodable>) {
            self.undelivered.lock().unwrap().push_back(SourcedRecord {
                id: id.into(),
                record,
            });
        }
    }

    #[async_trait]
    impl RecordSource for MemSource {
        async fn prepare(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn read(&self, count: usize, block: Duration) -> anyhow::Result<Vec<SourcedRecord>> {
            if self.fail_reads.load(Ordering::Relaxed) {
                return Err(anyhow::anyhow!("redis away"));
            }
            let mut out = Vec::new();
            {
                let mut q = self.undelivered.lock().unwrap();
                while out.len() < count {
                    let Some(e) = q.pop_front() else { break };
                    self.pending
                        .lock()
                        .unwrap()
                        .insert(e.id.clone(), (Instant::now(), e.clone()));
                    out.push(e);
                }
            }
            if out.is_empty() {
                tokio::time::sleep(block.min(Duration::from_millis(20))).await;
            }
            Ok(out)
        }
        async fn claim_stale(
            &self,
            idle: Duration,
            count: usize,
        ) -> anyhow::Result<Vec<SourcedRecord>> {
            let mut pending = self.pending.lock().unwrap();
            let mut out = Vec::new();
            for (at, e) in pending.values_mut() {
                if out.len() < count && at.elapsed() >= idle {
                    *at = Instant::now();
                    out.push(e.clone());
                }
            }
            Ok(out)
        }
        async fn ack(&self, ids: &[String]) -> anyhow::Result<()> {
            let mut pending = self.pending.lock().unwrap();
            for id in ids {
                pending.remove(id);
            }
            self.acked.lock().unwrap().extend(ids.iter().cloned());
            Ok(())
        }
        async fn dead_letter(&self, id: &str, _raw: &str, reason: &str) -> anyhow::Result<()> {
            self.dead.lock().unwrap().push((id.into(), reason.into()));
            Ok(())
        }
        async fn trim(&self) -> anyhow::Result<u64> {
            Ok(0)
        }
        async fn status(&self) -> anyhow::Result<SourceStatus> {
            Ok(SourceStatus {
                length: self.undelivered.lock().unwrap().len() as u64,
                pending: self.pending.lock().unwrap().len() as u64,
                dead_letters: self.dead.lock().unwrap().len() as u64,
            })
        }
    }

    fn fast() -> ConsumerConfig {
        ConsumerConfig {
            batch: 10,
            batch_wait: Duration::from_millis(10),
            claim_idle: Duration::from_millis(100),
            claim_every: Duration::from_millis(50),
            trim_every: Duration::from_secs(60),
            backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(50),
        }
    }

    async fn until(what: &str, mut f: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn records_are_stored_then_acknowledged_and_bad_entries_dead_lettered() {
        let source = Arc::new(MemSource::default());
        let store = Arc::new(MemStore::default());
        let processor = Arc::new(Processor::new(store.clone(), None));
        source.push("1-0", Ok(record("sip:bob@x")));
        source.push(
            "2-0",
            Err(Undecodable {
                raw: "{nope".into(),
                reason: "not JSON".into(),
            }),
        );
        source.push("3-0", Ok(record("sip:refuse@x")));
        source.push("4-0", Ok(record("sip:carol@x")));

        let consumer = Consumer::spawn(source.clone(), processor.clone(), fast());
        until("all four acknowledged", || {
            source.acked.lock().unwrap().len() == 4
        })
        .await;
        consumer.stop().await;

        assert_eq!(store.records.lock().unwrap().len(), 2);
        let dead: Vec<_> = source
            .dead
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(dead, vec!["2-0", "3-0"]);
        assert!(source.pending.lock().unwrap().is_empty());
        let stats = processor.stats().snapshot();
        assert_eq!((stats.stored, stats.rejected), (2, 2));
    }

    #[tokio::test]
    async fn a_store_outage_holds_the_batch_unacknowledged_until_the_store_returns() {
        let source = Arc::new(MemSource::default());
        let store = Arc::new(MemStore::default());
        store.fail.store(true, Ordering::Relaxed);
        let processor = Arc::new(Processor::new(store.clone(), None));
        for i in 0..5 {
            source.push(&format!("{i}-0"), Ok(record("sip:bob@x")));
        }
        let consumer = Consumer::spawn(source.clone(), processor, fast());
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(source.acked.lock().unwrap().is_empty());
        assert_eq!(source.pending.lock().unwrap().len(), 5);

        store.fail.store(false, Ordering::Relaxed);
        until("the batch lands", || {
            source.acked.lock().unwrap().len() == 5
        })
        .await;
        consumer.stop().await;
        assert_eq!(store.records.lock().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn entries_a_dead_consumer_left_pending_are_claimed_and_stored_once() {
        let source = Arc::new(MemSource::default());
        let store = Arc::new(MemStore::default());
        let first = record("sip:bob@x");
        let second = record("sip:carol@x");
        source.push("1-0", Ok(first.clone()));
        source.push("2-0", Ok(second.clone()));
        // A consumer reads both, stores the first, and dies before acking.
        let delivered = source.read(10, Duration::ZERO).await.unwrap();
        assert_eq!(delivered.len(), 2);
        store.insert_cdr_for_test(first.clone());

        let processor = Arc::new(Processor::new(store.clone(), None));
        let consumer = Consumer::spawn(source.clone(), processor.clone(), fast());
        until("the claimed entries are acknowledged", || {
            source.acked.lock().unwrap().len() == 2
        })
        .await;
        consumer.stop().await;
        assert_eq!(store.records.lock().unwrap().len(), 2, "stored once each");
        let stats = processor.stats().snapshot();
        assert_eq!((stats.stored, stats.duplicates), (1, 1));
    }

    #[tokio::test]
    async fn a_source_that_cannot_be_read_is_retried() {
        let source = Arc::new(MemSource::default());
        source.fail_reads.store(true, Ordering::Relaxed);
        source.push("1-0", Ok(record("sip:bob@x")));
        let store = Arc::new(MemStore::default());
        let consumer = Consumer::spawn(
            source.clone(),
            Arc::new(Processor::new(store.clone(), None)),
            ConsumerConfig {
                claim_every: Duration::from_secs(60),
                ..fast()
            },
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        source.fail_reads.store(false, Ordering::Relaxed);
        until("the record lands", || {
            store.records.lock().unwrap().len() == 1
        })
        .await;
        consumer.stop().await;
    }
}
