//! The processor: normalize, rate, store. In phase 1 it runs in-process
//! behind the spool; phase 2 moves it into `fcp-jobs` behind a stream.

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use fcp_core::storage::{CallDetailRecord, CdrKind, CdrStore};
use tracing::{debug, error, info};

use crate::record::durations;
use crate::{Error, Result};

/// What rating a record produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Rating {
    pub cost: f64,
    pub currency: String,
    pub matched_prefix: Option<String>,
    pub billable_seconds: u32,
    pub rate_plan_id: Option<String>,
}

/// Rates and settles a record. `None` means the record is not ratable
/// (no plan, no dialable destination) and is stored without a cost.
#[async_trait]
pub trait Rater: Send + Sync {
    async fn settle(&self, record: &CallDetailRecord) -> Option<Rating>;
}

#[async_trait]
impl Rater for fcp_billing::BillingEngine {
    async fn settle(&self, record: &CallDetailRecord) -> Option<Rating> {
        if record.kind != CdrKind::Call {
            return None;
        }
        let settlement = fcp_billing::BillingEngine::settle(
            self,
            &record.tenant_id,
            record.call_id,
            &record.to_uri,
            record.call_duration_ms.unwrap_or(0),
        )
        .await?;
        Some(Rating {
            cost: settlement.rated.cost,
            currency: settlement.rated.currency,
            matched_prefix: settlement.rated.matched_prefix,
            billable_seconds: settlement.rated.billable_secs.min(u32::MAX as u64) as u32,
            rate_plan_id: settlement.rate_plan_id,
        })
    }
}

/// How a batch went.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Records handed to the store.
    pub stored: usize,
    /// Records the normalizer refused.
    pub rejected: usize,
}

/// Normalize, rate and store records.
pub struct Processor {
    store: Arc<dyn CdrStore>,
    rater: Option<Arc<dyn Rater>>,
}

impl Processor {
    pub fn new(store: Arc<dyn CdrStore>, rater: Option<Arc<dyn Rater>>) -> Self {
        Self { store, rater }
    }

    /// Process one batch. A record the normalizer refuses is logged and
    /// skipped; a store error stops the batch and is returned, so the
    /// caller retries it — the records already stored are skipped by the
    /// store on the replay.
    pub async fn process(&self, records: Vec<CallDetailRecord>) -> Result<Outcome> {
        let mut outcome = Outcome::default();
        for mut record in records {
            if let Err(e) = normalize(&mut record) {
                error!(cdr_id = %record.cdr_id, call_id = %record.call_id, error = %e, "CDR rejected");
                outcome.rejected += 1;
                continue;
            }
            if record.cost.is_none() {
                if let Some(rater) = &self.rater {
                    if let Some(rating) = rater.settle(&record).await {
                        record.cost = Some(rating.cost);
                        record.currency = Some(rating.currency);
                        record.matched_prefix = rating.matched_prefix;
                        record.billable_seconds = Some(rating.billable_seconds);
                        record.rate_plan_id = rating.rate_plan_id;
                        record.rated_at = Some(SystemTime::now());
                    }
                }
            }
            record.created_at = Some(SystemTime::now());
            let cdr_id = record.cdr_id;
            let call_id = record.call_id;
            self.store.insert_cdr(record).await.map_err(Error::Store)?;
            debug!(%cdr_id, %call_id, "CDR stored");
            outcome.stored += 1;
        }
        if outcome.stored > 0 || outcome.rejected > 0 {
            info!(
                stored = outcome.stored,
                rejected = outcome.rejected,
                "CDR batch processed"
            );
        }
        Ok(outcome)
    }
}

/// Validate a record and fill what can be derived: the end time, the
/// durations, the schema version.
pub fn normalize(record: &mut CallDetailRecord) -> Result<()> {
    if record.cdr_id.is_nil() {
        return Err(Error::Invalid("nil cdr_id".into()));
    }
    if record.from_uri.is_empty() {
        return Err(Error::Invalid("empty from_uri".into()));
    }
    if record.to_uri.is_empty() {
        return Err(Error::Invalid("empty to_uri".into()));
    }
    if record.sip_call_id.is_empty() {
        record.sip_call_id = format!("unknown-{}", record.call_id);
    }
    if record.end_time.is_none() {
        record.end_time = Some(SystemTime::now());
    }
    if let Some(answer) = record.answer_time {
        if answer < record.start_time {
            record.answer_time = Some(record.start_time);
        }
    }
    if let Some(end) = record.end_time {
        let floor = record.answer_time.unwrap_or(record.start_time);
        if end < floor {
            record.end_time = Some(floor);
        }
    }
    let (setup, talk, total) = durations(record.start_time, record.answer_time, record.end_time);
    record.setup_duration_ms = record.setup_duration_ms.or(setup);
    record.call_duration_ms = record.call_duration_ms.or(talk);
    record.total_duration_ms = record.total_duration_ms.or(total);
    if record.schema_version == 0 {
        record.schema_version = fcp_core::storage::CDR_SCHEMA_VERSION;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::record::RecordBuilder;
    use fcp_core::storage::{CdrQuery, CdrStore};
    use fcp_core::{CallId, TenantId};
    use std::sync::Mutex;
    use std::time::Duration;

    /// A store that remembers, and can be told to fail.
    #[derive(Default)]
    pub(crate) struct MemStore {
        pub records: Mutex<Vec<CallDetailRecord>>,
        pub fail: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl CdrStore for MemStore {
        async fn insert_cdr(&self, cdr: CallDetailRecord) -> anyhow::Result<()> {
            if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(anyhow::anyhow!("store down"));
            }
            let mut records = self.records.lock().unwrap();
            if !records.iter().any(|r| r.cdr_id == cdr.cdr_id) {
                records.push(cdr);
            }
            Ok(())
        }
        async fn get_cdr(&self, _: &CallId) -> anyhow::Result<Option<CallDetailRecord>> {
            Ok(None)
        }
        async fn query_cdrs(&self, _: CdrQuery) -> anyhow::Result<Vec<CallDetailRecord>> {
            Ok(self.records.lock().unwrap().clone())
        }
        async fn count_cdrs(&self, _: CdrQuery) -> anyhow::Result<usize> {
            Ok(self.records.lock().unwrap().len())
        }
    }

    struct FlatRater;

    #[async_trait]
    impl Rater for FlatRater {
        async fn settle(&self, record: &CallDetailRecord) -> Option<Rating> {
            if !record.to_uri.contains("+1") {
                return None;
            }
            Some(Rating {
                cost: 0.5,
                currency: "USD".into(),
                matched_prefix: Some("+1".into()),
                billable_seconds: 60,
                rate_plan_id: Some("plan".into()),
            })
        }
    }

    fn record(to: &str) -> CallDetailRecord {
        let start = SystemTime::now() - Duration::from_secs(30);
        RecordBuilder::new(CdrKind::Call, CallId::new(), TenantId::new())
            .from("sip:alice@x", None)
            .to(to, None)
            .started_at(start)
            .answered_at(Some(start + Duration::from_secs(5)))
            .ended_at(Some(start + Duration::from_secs(25)))
            .build()
    }

    #[tokio::test]
    async fn records_are_rated_stamped_and_stored_and_replays_store_nothing_twice() {
        let store = Arc::new(MemStore::default());
        let processor = Processor::new(store.clone(), Some(Arc::new(FlatRater)));
        let rated = record("sip:+15551234567@x");
        let unrated = record("sip:bob@x");
        let outcome = processor
            .process(vec![rated.clone(), unrated.clone()])
            .await
            .unwrap();
        assert_eq!(
            outcome,
            Outcome {
                stored: 2,
                rejected: 0
            }
        );

        let stored = store.records.lock().unwrap().clone();
        assert_eq!(stored.len(), 2);
        let r = stored.iter().find(|r| r.cdr_id == rated.cdr_id).unwrap();
        assert_eq!(r.cost, Some(0.5));
        assert_eq!(r.currency.as_deref(), Some("USD"));
        assert_eq!(r.billable_seconds, Some(60));
        assert_eq!(r.rate_plan_id.as_deref(), Some("plan"));
        assert!(r.rated_at.is_some());
        assert!(r.created_at.is_some());
        let u = stored.iter().find(|r| r.cdr_id == unrated.cdr_id).unwrap();
        assert_eq!(u.cost, None);
        assert!(u.rated_at.is_none());

        // The same batch again: nothing new.
        processor.process(vec![rated, unrated]).await.unwrap();
        assert_eq!(store.records.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_bad_record_is_rejected_and_the_rest_stored_but_a_store_error_fails_the_batch() {
        let store = Arc::new(MemStore::default());
        let processor = Processor::new(store.clone(), None);
        let mut bad = record("sip:bob@x");
        bad.from_uri.clear();
        let good = record("sip:bob@x");
        let outcome = processor.process(vec![bad, good]).await.unwrap();
        assert_eq!(
            outcome,
            Outcome {
                stored: 1,
                rejected: 1
            }
        );

        store.fail.store(true, std::sync::atomic::Ordering::Relaxed);
        let err = processor
            .process(vec![record("sip:bob@x")])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Store(_)), "{err}");
        assert_eq!(store.records.lock().unwrap().len(), 1);
    }

    #[test]
    fn normalize_fills_the_end_and_the_durations_and_clamps_a_backwards_clock() {
        let start = SystemTime::now() - Duration::from_secs(10);
        let mut r = record("sip:bob@x");
        r.end_time = None;
        r.total_duration_ms = None;
        r.call_duration_ms = None;
        r.answer_time = Some(start - Duration::from_secs(5));
        r.start_time = start;
        normalize(&mut r).unwrap();
        assert_eq!(r.answer_time, Some(start));
        assert!(r.end_time.is_some());
        assert!(r.total_duration_ms.unwrap() >= 9_000);
        assert!(r.call_duration_ms.is_some());
    }
}
