//! Presence analytics: in-memory history store and event consumer.

use anyhow::Result;
use async_trait::async_trait;
use fcp_core::storage::{PresenceHistoryStore, PresenceSummary, StoredPresenceChange};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory presence history store backed by a ring buffer.
///
/// When the buffer reaches `max_entries`, the oldest records are evicted.
pub struct InMemoryPresenceHistory {
    entries: RwLock<VecDeque<StoredPresenceChange>>,
    max_entries: usize,
}

impl InMemoryPresenceHistory {
    /// Create a new in-memory history store with the given capacity.
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: RwLock::new(VecDeque::with_capacity(max_entries.min(1024))),
            max_entries,
        }
    }
}

#[async_trait]
impl PresenceHistoryStore for InMemoryPresenceHistory {
    async fn record_change(&self, change: StoredPresenceChange) -> Result<()> {
        let mut entries = self.entries.write().await;
        if entries.len() >= self.max_entries {
            entries.pop_front();
        }
        entries.push_back(change);
        Ok(())
    }

    async fn query_history(
        &self,
        tenant_id: &fcp_core::TenantId,
        aor: &str,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        limit: usize,
    ) -> Result<Vec<StoredPresenceChange>> {
        let entries = self.entries.read().await;
        let results: Vec<StoredPresenceChange> = entries
            .iter()
            .filter(|e| &e.tenant_id == tenant_id)
            .filter(|e| e.aor == aor)
            .filter(|e| from_epoch.is_none_or(|f| e.timestamp_epoch >= f))
            .filter(|e| to_epoch.is_none_or(|t| e.timestamp_epoch <= t))
            .rev() // most recent first
            .take(limit)
            .cloned()
            .collect();
        Ok(results)
    }

    async fn query_summary(
        &self,
        tenant_id: &fcp_core::TenantId,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
    ) -> Result<Vec<PresenceSummary>> {
        let entries = self.entries.read().await;

        // Group changes by AOR
        let mut by_aor: HashMap<String, Vec<&StoredPresenceChange>> = HashMap::new();
        for entry in entries.iter() {
            if &entry.tenant_id == tenant_id
                && from_epoch.is_none_or(|f| entry.timestamp_epoch >= f)
                && to_epoch.is_none_or(|t| entry.timestamp_epoch <= t)
            {
                by_aor.entry(entry.aor.clone()).or_default().push(entry);
            }
        }

        let mut summaries = Vec::new();
        for (aor, changes) in by_aor {
            let total_changes = changes.len() as u64;
            let mut time_open: u64 = 0;
            let mut time_busy: u64 = 0;
            let mut time_away: u64 = 0;
            let mut time_closed: u64 = 0;

            // Walk consecutive pairs to compute durations
            for window in changes.windows(2) {
                let current = &window[0];
                let next = &window[1];
                let duration = next.timestamp_epoch.saturating_sub(current.timestamp_epoch);

                match current.new_state.as_str() {
                    "open" => time_open += duration,
                    "busy" => time_busy += duration,
                    "away" => time_away += duration,
                    "closed" => time_closed += duration,
                    _ => {}
                }
            }

            summaries.push(PresenceSummary {
                aor,
                total_changes,
                time_in_open_secs: time_open,
                time_in_busy_secs: time_busy,
                time_in_away_secs: time_away,
                time_in_closed_secs: time_closed,
            });
        }

        Ok(summaries)
    }
}

/// Spawns a background task that consumes presence events from the event bus
/// and records state changes in the history store.
pub fn spawn_analytics_consumer(
    event_bus: Arc<dyn fcp_events::EventSubscriber>,
    history_store: Arc<dyn PresenceHistoryStore>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let filter =
            fcp_events::EventFilter::new().types(vec![fcp_events::filter::EventType::Presence]);

        let mut stream = match event_bus.subscribe(filter).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "Failed to subscribe to presence events for analytics");
                return;
            }
        };

        while let Some(event) = stream.next().await {
            if let fcp_events::PlatformEvent::Presence(fcp_events::PresenceEvent::StatusChanged {
                aor,
                old_state,
                new_state,
                tenant_id,
                timestamp,
            }) = event
            {
                let epoch = timestamp
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let change = StoredPresenceChange {
                    id: uuid::Uuid::new_v4().to_string(),
                    aor,
                    old_state,
                    new_state,
                    tenant_id,
                    timestamp_epoch: epoch,
                };

                if let Err(e) = history_store.record_change(change).await {
                    tracing::warn!(error = %e, "Failed to record presence change in analytics");
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_record_and_query() {
        let store = InMemoryPresenceHistory::new(100);
        let tenant_id = fcp_core::TenantId::new();
        let change = StoredPresenceChange {
            id: "1".to_string(),
            aor: "alice@example.com".to_string(),
            old_state: "closed".to_string(),
            new_state: "open".to_string(),
            tenant_id,
            timestamp_epoch: 1000,
        };

        store.record_change(change).await.unwrap();

        let results = store
            .query_history(&tenant_id, "alice@example.com", None, None, 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].new_state, "open");
    }

    #[tokio::test]
    async fn test_ring_buffer_eviction() {
        let store = InMemoryPresenceHistory::new(3);
        let tenant_id = fcp_core::TenantId::new();

        for i in 0..5 {
            store
                .record_change(StoredPresenceChange {
                    id: i.to_string(),
                    aor: "bob@example.com".to_string(),
                    old_state: "closed".to_string(),
                    new_state: "open".to_string(),
                    tenant_id,
                    timestamp_epoch: i as u64,
                })
                .await
                .unwrap();
        }

        let results = store
            .query_history(&tenant_id, "bob@example.com", None, None, 100)
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
        // Oldest should be id "2" (0 and 1 evicted)
        assert_eq!(results.last().unwrap().id, "2");
    }

    #[tokio::test]
    async fn test_query_with_time_range() {
        let store = InMemoryPresenceHistory::new(100);
        let tenant_id = fcp_core::TenantId::new();

        for i in 0..10 {
            store
                .record_change(StoredPresenceChange {
                    id: i.to_string(),
                    aor: "alice@example.com".to_string(),
                    old_state: "closed".to_string(),
                    new_state: "open".to_string(),
                    tenant_id,
                    timestamp_epoch: i * 100,
                })
                .await
                .unwrap();
        }

        let results = store
            .query_history(&tenant_id, "alice@example.com", Some(300), Some(700), 100)
            .await
            .unwrap();
        assert_eq!(results.len(), 5); // epochs 300, 400, 500, 600, 700
    }

    #[tokio::test]
    async fn test_summary() {
        let store = InMemoryPresenceHistory::new(100);
        let tenant_id = fcp_core::TenantId::new();

        let changes = [
            ("closed", "open", 1000),
            ("open", "busy", 1060),
            ("busy", "open", 1090),
            ("open", "closed", 1200),
        ];

        for (i, (old, new, ts)) in changes.iter().enumerate() {
            store
                .record_change(StoredPresenceChange {
                    id: i.to_string(),
                    aor: "alice@example.com".to_string(),
                    old_state: old.to_string(),
                    new_state: new.to_string(),
                    tenant_id,
                    timestamp_epoch: *ts,
                })
                .await
                .unwrap();
        }

        let summaries = store.query_summary(&tenant_id, None, None).await.unwrap();
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.total_changes, 4);
        assert_eq!(s.time_in_open_secs, 60 + 110); // open→busy(60s) + open→closed(110s)
        assert_eq!(s.time_in_busy_secs, 30); // busy→open(30s)
    }

    #[tokio::test]
    async fn test_history_is_tenant_scoped() {
        let store = InMemoryPresenceHistory::new(100);
        let tenant_a = fcp_core::TenantId::new();
        let tenant_b = fcp_core::TenantId::new();

        for tenant_id in [tenant_a, tenant_b] {
            store
                .record_change(StoredPresenceChange {
                    id: tenant_id.to_string(),
                    aor: "alice@example.com".to_string(),
                    old_state: "closed".to_string(),
                    new_state: "open".to_string(),
                    tenant_id,
                    timestamp_epoch: 1000,
                })
                .await
                .unwrap();
        }

        let results = store
            .query_history(&tenant_a, "alice@example.com", None, None, 10)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tenant_id, tenant_a);
    }
}
