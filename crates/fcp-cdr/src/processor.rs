//! The processor (`docs/CDR_PIPELINE.md` §4.3): normalize, enrich, rate,
//! store, settle, announce. The same processor runs in-process behind a
//! node's spool and in `fcp-jobs` behind the stream.

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use fcp_core::storage::{CallDetailRecord, CdrKind, CdrRejected, CdrStore};
use fcp_events::{CdrEvent, EventPublisher, PlatformEvent};
use tracing::{debug, error, info, warn};

use crate::enrich::Enricher;
use crate::record::durations;
use crate::stats::ProcessorStats;
use crate::{Error, Result};

pub use fcp_billing::Rating;

/// Rates records and settles their charges.
#[async_trait]
pub trait Rater: Send + Sync {
    /// The record's rating, writing nothing. `None` when the record is not
    /// ratable: not a call record, no dialable destination, no plan.
    async fn rate(&self, record: &CallDetailRecord) -> Option<Rating>;

    /// Charge the rating to the tenant's balance and announce it. Called
    /// for every rated record in a stored batch, replays included: the
    /// ledger charges a call once however often this is called.
    async fn settle(&self, record: &CallDetailRecord, rating: &Rating);
}

#[async_trait]
impl Rater for fcp_billing::BillingEngine {
    async fn rate(&self, record: &CallDetailRecord) -> Option<Rating> {
        if record.kind != CdrKind::Call {
            return None;
        }
        fcp_billing::BillingEngine::rate(
            self,
            &record.tenant_id,
            record.call_id,
            &record.to_uri,
            record.call_duration_ms.unwrap_or(0),
        )
        .await
    }

    async fn settle(&self, record: &CallDetailRecord, rating: &Rating) {
        self.settle_rated(&record.tenant_id, record.call_id, rating)
            .await;
    }
}

/// A record the processor would not store, and why.
#[derive(Debug, Clone)]
pub struct Rejection<K> {
    pub key: K,
    pub record: CallDetailRecord,
    pub reason: String,
}

/// How a batch went. Every input is in exactly one of `stored`,
/// `duplicates` or `rejected`.
#[derive(Debug, Clone)]
pub struct Outcome<K> {
    /// Keys of records stored by this batch.
    pub stored: Vec<K>,
    /// Keys of records that were stored already (a replay).
    pub duplicates: Vec<K>,
    /// Records refused by the normalizer or by the store.
    pub rejected: Vec<Rejection<K>>,
}

impl<K> Outcome<K> {
    fn new() -> Self {
        Self {
            stored: Vec::new(),
            duplicates: Vec::new(),
            rejected: Vec::new(),
        }
    }

    /// Every key, whatever became of its record.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.stored
            .iter()
            .chain(self.duplicates.iter())
            .chain(self.rejected.iter().map(|r| &r.key))
    }
}

/// Normalize, enrich, rate, store, settle and announce records.
pub struct Processor {
    store: Arc<dyn CdrStore>,
    rater: Option<Arc<dyn Rater>>,
    enricher: Option<Arc<dyn Enricher>>,
    events: Option<Arc<dyn EventPublisher>>,
    stats: Arc<ProcessorStats>,
}

impl Processor {
    pub fn new(store: Arc<dyn CdrStore>, rater: Option<Arc<dyn Rater>>) -> Self {
        Self {
            store,
            rater,
            enricher: None,
            events: None,
            stats: Arc::new(ProcessorStats::default()),
        }
    }

    /// Fill what the node could not know (the call's direction).
    pub fn with_enricher(mut self, enricher: Arc<dyn Enricher>) -> Self {
        self.enricher = Some(enricher);
        self
    }

    /// Announce every stored record as `cdr.written`.
    pub fn with_events(mut self, events: Arc<dyn EventPublisher>) -> Self {
        self.events = Some(events);
        self
    }

    /// What the processor has done since it started.
    pub fn stats(&self) -> &Arc<ProcessorStats> {
        &self.stats
    }

    /// Process records that need no key of their own.
    pub async fn process(&self, records: Vec<CallDetailRecord>) -> Result<Outcome<usize>> {
        self.process_keyed(records.into_iter().enumerate().collect())
            .await
    }

    /// Process one batch. Records the normalizer or the store refuse are in
    /// the outcome's `rejected`, for the caller to keep somewhere a person
    /// will look; an error means the store was unavailable and nothing may
    /// be assumed stored, so the caller retries the batch — the records
    /// that did land are duplicates on the replay, and charges and
    /// announcements are repeated safely.
    pub async fn process_keyed<K: Clone + Send>(
        &self,
        entries: Vec<(K, CallDetailRecord)>,
    ) -> Result<Outcome<K>> {
        let started = Instant::now();
        let mut outcome = Outcome::new();

        // 1–3: normalize, enrich, rate.
        let mut ready: Vec<(K, CallDetailRecord, Option<Rating>)> =
            Vec::with_capacity(entries.len());
        for (key, mut record) in entries {
            if let Err(e) = normalize(&mut record) {
                error!(cdr_id = %record.cdr_id, call_id = %record.call_id, error = %e, "CDR rejected by the normalizer");
                outcome.rejected.push(Rejection {
                    key,
                    record,
                    reason: e.to_string(),
                });
                continue;
            }
            if let Some(enricher) = &self.enricher {
                enricher.enrich(&mut record).await;
            }
            let mut rating = None;
            if record.cost.is_none() {
                if let Some(rater) = &self.rater {
                    rating = rater.rate(&record).await;
                    if let Some(r) = &rating {
                        record.cost = Some(r.rated.cost);
                        record.currency = Some(r.rated.currency.clone());
                        record.matched_prefix = r.rated.matched_prefix.clone();
                        record.billable_seconds =
                            Some(r.rated.billable_secs.min(u64::from(u32::MAX)) as u32);
                        record.rate_plan_id = Some(r.rate_plan_id.clone());
                        record.rated_at = Some(SystemTime::now());
                    }
                }
            }
            record.created_at = Some(SystemTime::now());
            ready.push((key, record, rating));
        }

        // 4: store, the whole batch at once; a refused record is found by
        // storing the batch one record at a time.
        let new_ids = match self
            .store
            .insert_cdrs(ready.iter().map(|(_, r, _)| r.clone()).collect())
            .await
        {
            Ok(ids) => ids,
            Err(e) if e.downcast_ref::<CdrRejected>().is_some() => {
                warn!(error = %e, records = ready.len(), "CDR batch refused; storing its records one at a time");
                let mut ids = Vec::new();
                let mut kept = Vec::with_capacity(ready.len());
                for (key, record, rating) in ready {
                    match self.store.insert_cdrs(vec![record.clone()]).await {
                        Ok(stored) => {
                            ids.extend(stored);
                            kept.push((key, record, rating));
                        }
                        Err(e) if e.downcast_ref::<CdrRejected>().is_some() => {
                            error!(cdr_id = %record.cdr_id, call_id = %record.call_id, error = %e, "CDR rejected by the store");
                            outcome.rejected.push(Rejection {
                                key,
                                record,
                                reason: e.to_string(),
                            });
                        }
                        Err(e) => {
                            self.stats.record_batch_error();
                            return Err(Error::Store(e));
                        }
                    }
                }
                ready = kept;
                ids
            }
            Err(e) => {
                self.stats.record_batch_error();
                return Err(Error::Store(e));
            }
        };

        // 5–6: settle and announce every stored record, replays included.
        for (key, record, rating) in ready {
            if let (Some(rater), Some(rating)) = (&self.rater, &rating) {
                rater.settle(&record, rating).await;
            }
            self.announce(&record).await;
            self.stats.observe_lag(&record);
            if new_ids.contains(&record.cdr_id) {
                debug!(cdr_id = %record.cdr_id, call_id = %record.call_id, "CDR stored");
                outcome.stored.push(key);
            } else {
                debug!(cdr_id = %record.cdr_id, call_id = %record.call_id, "CDR already stored");
                outcome.duplicates.push(key);
            }
        }

        self.stats.record_batch(
            outcome.stored.len(),
            outcome.duplicates.len(),
            outcome.rejected.len(),
            started.elapsed(),
        );
        if !outcome.stored.is_empty() || !outcome.rejected.is_empty() {
            info!(
                stored = outcome.stored.len(),
                duplicates = outcome.duplicates.len(),
                rejected = outcome.rejected.len(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "CDR batch processed"
            );
        }
        Ok(outcome)
    }

    async fn announce(&self, record: &CallDetailRecord) {
        let Some(events) = &self.events else {
            return;
        };
        let event = PlatformEvent::Cdr(CdrEvent::Written {
            cdr_id: record.cdr_id,
            call_id: record.call_id,
            tenant_id: record.tenant_id,
            kind: record.kind,
            disposition: record.disposition,
            start_time: record.start_time,
            end_time: record.end_time.unwrap_or(record.start_time),
            duration_ms: record.call_duration_ms.unwrap_or(0),
            cost: record.cost,
            currency: record.currency.clone(),
            node_id: record.node_id.clone(),
            site_id: record.site_id.map(|s| s.to_string()),
            timestamp: record.created_at.unwrap_or_else(SystemTime::now),
        });
        if let Err(e) = events.publish(event).await {
            warn!(cdr_id = %record.cdr_id, error = %e, "Failed to announce cdr.written");
        }
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
    if record.schema_version > fcp_core::storage::CDR_SCHEMA_VERSION {
        return Err(Error::Invalid(format!(
            "schema version {} is newer than this build reads ({})",
            record.schema_version,
            fcp_core::storage::CDR_SCHEMA_VERSION
        )));
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
    use fcp_billing::{Decimal, RatedCall};
    use fcp_core::storage::CdrQuery;
    use fcp_core::{CallId, TenantId};
    use rust_decimal_macros::dec;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    /// A store that remembers, can be told to fail, and refuses records
    /// whose `to_uri` is `sip:refuse@x`.
    #[derive(Default)]
    pub(crate) struct MemStore {
        pub records: Mutex<Vec<CallDetailRecord>>,
        pub fail: AtomicBool,
    }

    impl MemStore {
        /// Put a record in as if an earlier run had stored it.
        pub fn insert_cdr_for_test(&self, record: CallDetailRecord) {
            self.records.lock().unwrap().push(record);
        }
    }

    #[async_trait]
    impl CdrStore for MemStore {
        async fn insert_cdr(&self, cdr: CallDetailRecord) -> anyhow::Result<()> {
            self.insert_cdrs(vec![cdr]).await.map(|_| ())
        }
        async fn insert_cdrs(
            &self,
            cdrs: Vec<CallDetailRecord>,
        ) -> anyhow::Result<Vec<uuid::Uuid>> {
            if self.fail.load(Ordering::Relaxed) {
                return Err(anyhow::anyhow!("store down"));
            }
            if let Some(bad) = cdrs.iter().find(|c| c.to_uri == "sip:refuse@x") {
                return Err(anyhow::Error::new(CdrRejected(format!(
                    "{} refused",
                    bad.cdr_id
                ))));
            }
            let mut records = self.records.lock().unwrap();
            let mut new = Vec::new();
            for cdr in cdrs {
                if !records.iter().any(|r| r.cdr_id == cdr.cdr_id) {
                    new.push(cdr.cdr_id);
                    records.push(cdr);
                }
            }
            Ok(new)
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

    /// Rates `+1` destinations at 0.5 and counts charges once per call.
    #[derive(Default)]
    pub(crate) struct FlatRater {
        pub charged: Mutex<HashSet<CallId>>,
        pub settles: Mutex<usize>,
    }

    #[async_trait]
    impl Rater for FlatRater {
        async fn rate(&self, record: &CallDetailRecord) -> Option<Rating> {
            if record.kind != CdrKind::Call || !record.to_uri.contains("+1") {
                return None;
            }
            Some(Rating {
                rated: RatedCall {
                    cost: dec!(0.5),
                    currency: "USD".into(),
                    matched_prefix: Some("+1".into()),
                    billable_secs: 60,
                },
                rate_plan_id: "plan".into(),
                destination: "1".into(),
            })
        }
        async fn settle(&self, record: &CallDetailRecord, _rating: &Rating) {
            *self.settles.lock().unwrap() += 1;
            self.charged.lock().unwrap().insert(record.call_id);
        }
    }

    /// Keeps what it is given.
    #[derive(Default)]
    pub(crate) struct Recorder(pub Mutex<Vec<PlatformEvent>>);

    #[async_trait]
    impl EventPublisher for Recorder {
        async fn publish(&self, event: PlatformEvent) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    pub(crate) fn record(to: &str) -> CallDetailRecord {
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
    async fn records_are_rated_stored_settled_and_announced() {
        let store = Arc::new(MemStore::default());
        let rater = Arc::new(FlatRater::default());
        let events = Arc::new(Recorder::default());
        let processor =
            Processor::new(store.clone(), Some(rater.clone())).with_events(events.clone());
        let rated = record("sip:+15551234567@x");
        let unrated = record("sip:bob@x");
        let outcome = processor
            .process(vec![rated.clone(), unrated.clone()])
            .await
            .unwrap();
        assert_eq!(outcome.stored, vec![0, 1]);
        assert!(outcome.duplicates.is_empty() && outcome.rejected.is_empty());

        let stored = store.records.lock().unwrap().clone();
        let r = stored.iter().find(|r| r.cdr_id == rated.cdr_id).unwrap();
        assert_eq!(r.cost, Some(dec!(0.5)));
        assert_eq!(r.currency.as_deref(), Some("USD"));
        assert_eq!(r.billable_seconds, Some(60));
        assert_eq!(r.rate_plan_id.as_deref(), Some("plan"));
        assert!(r.rated_at.is_some() && r.created_at.is_some());
        let u = stored.iter().find(|r| r.cdr_id == unrated.cdr_id).unwrap();
        assert_eq!(u.cost, None);

        assert_eq!(*rater.settles.lock().unwrap(), 1);
        let announced: Vec<_> = events
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|e| match e {
                PlatformEvent::Cdr(CdrEvent::Written { cdr_id, cost, .. }) => (*cdr_id, *cost),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            announced,
            vec![
                (rated.cdr_id, Some(dec!(0.5))),
                (unrated.cdr_id, None::<Decimal>)
            ]
        );
    }

    #[tokio::test]
    async fn a_replay_is_a_duplicate_that_still_settles_and_announces() {
        let store = Arc::new(MemStore::default());
        let rater = Arc::new(FlatRater::default());
        let events = Arc::new(Recorder::default());
        let processor =
            Processor::new(store.clone(), Some(rater.clone())).with_events(events.clone());
        let rated = record("sip:+15551234567@x");
        processor.process(vec![rated.clone()]).await.unwrap();
        let outcome = processor.process(vec![rated.clone()]).await.unwrap();
        assert!(outcome.stored.is_empty());
        assert_eq!(outcome.duplicates, vec![0]);
        assert_eq!(store.records.lock().unwrap().len(), 1);
        // Settled twice, charged once: the ledger's rule, not the processor's.
        assert_eq!(*rater.settles.lock().unwrap(), 2);
        assert_eq!(rater.charged.lock().unwrap().len(), 1);
        assert_eq!(events.0.lock().unwrap().len(), 2);
        let stats = processor.stats().snapshot();
        assert_eq!((stats.stored, stats.duplicates), (1, 1));
    }

    #[tokio::test]
    async fn a_refused_record_is_found_and_the_rest_of_its_batch_stored() {
        let store = Arc::new(MemStore::default());
        let processor = Processor::new(store.clone(), None);
        let mut bad_normal = record("sip:bob@x");
        bad_normal.from_uri.clear();
        let refused = record("sip:refuse@x");
        let good = record("sip:carol@x");
        let outcome = processor
            .process_keyed(vec![("a", bad_normal), ("b", refused), ("c", good)])
            .await
            .unwrap();
        assert_eq!(outcome.stored, vec!["c"]);
        let rejected: Vec<_> = outcome.rejected.iter().map(|r| r.key).collect();
        assert_eq!(rejected, vec!["a", "b"]);
        assert!(outcome.rejected[1].reason.contains("refused"));
        assert_eq!(outcome.keys().count(), 3);
        assert_eq!(store.records.lock().unwrap().len(), 1);
        assert_eq!(processor.stats().snapshot().rejected, 2);
    }

    #[tokio::test]
    async fn a_store_outage_fails_the_batch_and_charges_nothing() {
        let store = Arc::new(MemStore::default());
        let rater = Arc::new(FlatRater::default());
        let processor = Processor::new(store.clone(), Some(rater.clone()));
        store.fail.store(true, Ordering::Relaxed);
        let err = processor
            .process(vec![record("sip:+15551234567@x")])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Store(_)), "{err}");
        assert_eq!(*rater.settles.lock().unwrap(), 0);
        assert_eq!(processor.stats().snapshot().batch_errors, 1);
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

    #[test]
    fn normalize_refuses_a_record_from_a_newer_build() {
        let mut r = record("sip:bob@x");
        r.schema_version = fcp_core::storage::CDR_SCHEMA_VERSION + 1;
        assert!(normalize(&mut r).is_err());
    }
}
