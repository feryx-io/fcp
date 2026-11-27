//! Speech usage counted at the server (`docs/TENANT_SPEECH.md` S4): each
//! tenant's requests and characters per provider and hour, kept in memory
//! and added to the database every [`FLUSH`], so speaking never waits on a
//! write.

use fcp_core::speech::{SpeechUsage, SpeechUsageStore};
use fcp_core::TenantId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How often counts are written.
pub const FLUSH: Duration = Duration::from_secs(15);

type Key = (TenantId, String, u64);

#[derive(Default, Clone, Copy)]
struct Counts {
    requests: u64,
    characters: u64,
    synthesized: u64,
}

pub struct UsageCounter {
    store: Arc<dyn SpeechUsageStore>,
    pending: Mutex<HashMap<Key, Counts>>,
    /// The caps the counts are held to (`docs/SPEECH_CAPS.md`).
    pub caps: crate::caps::Caps,
}

impl UsageCounter {
    pub fn new(store: Arc<dyn SpeechUsageStore>) -> Arc<Self> {
        Arc::new(Self {
            caps: crate::caps::Caps::new(store.clone()),
            store,
            pending: Mutex::default(),
        })
    }

    /// One request of `characters` for `tenant` with `provider`;
    /// `synthesized` when the provider spoke it rather than the cache.
    pub fn count(&self, tenant: &str, provider: &str, characters: usize, synthesized: bool) {
        let Ok(tenant) = tenant
            .parse::<TenantId>()
            .or_else(|_| format!("tenant-{tenant}").parse())
        else {
            return;
        };
        let hour = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / 3600
            * 3600;
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let c = pending
            .entry((tenant, provider.to_string(), hour))
            .or_default();
        c.requests += 1;
        c.characters += characters as u64;
        if synthesized {
            c.synthesized += characters as u64;
        }
        drop(pending);
        if synthesized {
            self.caps
                .add(&tenant.to_string(), provider, characters as u64);
        }
    }

    /// Write what was counted; on a failure it is kept for the next try.
    pub async fn flush(&self) {
        let taken: Vec<(Key, Counts)> = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.drain().collect()
        };
        if taken.is_empty() {
            return;
        }
        let rows: Vec<SpeechUsage> = taken
            .iter()
            .map(|((tenant, provider, hour), c)| SpeechUsage {
                tenant_id: *tenant,
                provider: provider.clone(),
                hour: UNIX_EPOCH + Duration::from_secs(*hour),
                requests: c.requests,
                characters: c.characters,
                synthesized_characters: c.synthesized,
                site_id: None,
            })
            .collect();
        if let Err(e) = self.store.add_speech_usage(&rows).await {
            tracing::warn!(error = %e, rows = rows.len(), "Speech usage could not be written; kept for the next try");
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            for (key, c) in taken {
                let p = pending.entry(key).or_default();
                p.requests += c.requests;
                p.characters += c.characters;
                p.synthesized += c.synthesized;
            }
        }
    }

    /// Flush every [`FLUSH`] for as long as the server runs, and read the
    /// caps and the month's totals back after each.
    pub fn spawn(self: &Arc<Self>) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(FLUSH);
            tick.tick().await;
            me.caps.refresh().await;
            loop {
                tick.tick().await;
                me.flush().await;
                me.caps.refresh().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Kept(Mutex<Vec<SpeechUsage>>, Mutex<bool>);

    #[async_trait::async_trait]
    impl SpeechUsageStore for Kept {
        async fn add_speech_usage(&self, usage: &[SpeechUsage]) -> anyhow::Result<()> {
            if *self.1.lock().unwrap() {
                anyhow::bail!("down");
            }
            self.0.lock().unwrap().extend_from_slice(usage);
            Ok(())
        }
        async fn speech_usage(
            &self,
            _: &TenantId,
            _: SystemTime,
            _: SystemTime,
        ) -> anyhow::Result<Vec<SpeechUsage>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn counts_are_summed_per_tenant_and_provider_and_kept_through_a_failure() {
        let kept = Arc::new(Kept::default());
        let counter = UsageCounter::new(kept.clone());
        let t = TenantId::new();
        counter.count(&t.as_uuid().to_string(), "piper", 10, true);
        counter.count(&t.to_string(), "piper", 5, false);
        counter.count(&t.to_string(), "account:x", 3, true);
        counter.count("not a tenant", "piper", 99, true);

        *kept.1.lock().unwrap() = true;
        counter.flush().await;
        assert!(kept.0.lock().unwrap().is_empty());
        *kept.1.lock().unwrap() = false;
        counter.flush().await;

        let mut rows = kept.0.lock().unwrap().clone();
        rows.sort_by(|a, b| a.provider.cmp(&b.provider));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].provider, "piper");
        assert_eq!(
            (
                rows[1].requests,
                rows[1].characters,
                rows[1].synthesized_characters
            ),
            (2, 15, 10)
        );
        assert_eq!((rows[0].requests, rows[0].characters), (1, 3));
        counter.flush().await;
        assert_eq!(kept.0.lock().unwrap().len(), 2, "nothing written twice");
    }
}
