//! Speech spending caps enforced at the server (`docs/SPEECH_CAPS.md` C1).
//!
//! Every [`crate::usage::FLUSH`], after the usage is written, the caps and
//! this month's synthesized characters per tenant and provider are read
//! back (this deployment's and its sites'); between reads, what this
//! server synthesizes is added on. So a cap is known to within a flush of
//! every server's speech: soft, a spending guard rather than billing.

use fcp_core::speech::{effective_cap, month_of, CapUnit, SpeechCap, SpeechUsageStore};
use fcp_core::TenantId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Default)]
struct Snapshot {
    caps: Vec<SpeechCap>,
    /// The month these totals are for.
    month: Option<SystemTime>,
    /// In each unit's base: characters, milliseconds heard.
    used: HashMap<(TenantId, String, CapUnit), u64>,
}

/// A cap reached: its limit (in its unit) and the month's count (in the
/// unit's base).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reached {
    pub cap: u64,
    pub used: u64,
}

pub struct Caps {
    store: Arc<dyn SpeechUsageStore>,
    now: Box<dyn Fn() -> SystemTime + Send + Sync>,
    snapshot: Mutex<Snapshot>,
}

fn tenant_id(tenant: &str) -> Option<TenantId> {
    tenant
        .parse::<TenantId>()
        .or_else(|_| format!("tenant-{tenant}").parse())
        .ok()
}

impl Caps {
    pub fn new(store: Arc<dyn SpeechUsageStore>) -> Self {
        Self::with_clock(store, Box::new(SystemTime::now))
    }

    pub fn with_clock(
        store: Arc<dyn SpeechUsageStore>,
        now: Box<dyn Fn() -> SystemTime + Send + Sync>,
    ) -> Self {
        Self {
            store,
            now,
            snapshot: Mutex::default(),
        }
    }

    /// Read the caps and the month's totals again; on a failure the last
    /// ones stand.
    pub async fn refresh(&self) {
        let (month, _) = month_of((self.now)());
        let caps = match self.store.all_speech_caps().await {
            Ok(caps) => caps,
            Err(e) => {
                tracing::warn!(error = %e, "Speech caps could not be read; the last ones stand");
                return;
            }
        };
        // Nothing capped: no totals to read.
        let used = if caps.is_empty() {
            HashMap::new()
        } else {
            match self.store.used_since(month).await {
                Ok(rows) => rows
                    .into_iter()
                    .map(|(t, p, u, n)| ((t, p, u), n))
                    .collect(),
                Err(e) => {
                    tracing::warn!(error = %e, "Speech usage could not be read for the caps; the last stands");
                    return;
                }
            }
        };
        let mut s = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        *s = Snapshot {
            caps,
            month: Some(month),
            used,
        };
    }

    /// What this server used since the last read, in `unit`'s base.
    pub fn add(&self, tenant: &str, provider: &str, unit: CapUnit, amount: u64) {
        let Some(tenant) = tenant_id(tenant) else {
            return;
        };
        let (month, _) = month_of((self.now)());
        let mut s = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        if s.caps.is_empty() {
            return;
        }
        if s.month != Some(month) {
            // A new month: last month's totals count for nothing.
            s.used.clear();
            s.month = Some(month);
        }
        *s.used
            .entry((tenant, provider.to_string(), unit))
            .or_default() += amount;
    }

    /// Whether `tenant` has reached its cap on `provider` this month.
    pub fn reached(&self, tenant: &str, provider: &str, unit: CapUnit) -> Option<Reached> {
        let tenant = tenant_id(tenant)?;
        let (month, _) = month_of((self.now)());
        let s = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        let cap = effective_cap(&s.caps, &tenant, provider, unit)?;
        let used = if s.month == Some(month) {
            s.used
                .get(&(tenant, provider.to_string(), unit))
                .copied()
                .unwrap_or(0)
        } else {
            0
        };
        (used >= unit.base(cap)).then_some(Reached { cap, used })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::speech::{SpeechCapKind, SpeechUsage};
    use std::time::Duration;

    #[derive(Default)]
    struct Store {
        caps: Mutex<Vec<SpeechCap>>,
        used: Mutex<Vec<(TenantId, String, CapUnit, u64)>>,
    }

    #[async_trait::async_trait]
    impl SpeechUsageStore for Store {
        async fn add_speech_usage(&self, _: &[SpeechUsage]) -> anyhow::Result<()> {
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
        async fn all_speech_caps(&self) -> anyhow::Result<Vec<SpeechCap>> {
            Ok(self.caps.lock().unwrap().clone())
        }
        async fn used_since(
            &self,
            _: SystemTime,
        ) -> anyhow::Result<Vec<(TenantId, String, CapUnit, u64)>> {
            Ok(self.used.lock().unwrap().clone())
        }
    }

    #[tokio::test]
    async fn a_cap_is_reached_by_the_month_s_count_and_this_server_s_since() {
        let store = Arc::new(Store::default());
        let t = TenantId::new();
        store.caps.lock().unwrap().push(SpeechCap {
            tenant_id: None,
            provider: "openai".into(),
            kind: SpeechCapKind::Platform,
            unit: CapUnit::Characters,
            monthly_characters: Some(100),
        });
        store
            .used
            .lock()
            .unwrap()
            .push((t, "openai".into(), CapUnit::Characters, 90));
        let clock = Arc::new(Mutex::new(SystemTime::now()));
        let c = clock.clone();
        let caps = Caps::with_clock(store.clone(), Box::new(move || *c.lock().unwrap()));
        let tenant = t.to_string();

        assert_eq!(
            caps.reached(&tenant, "openai", CapUnit::Characters),
            None,
            "nothing read yet"
        );
        caps.refresh().await;
        assert_eq!(caps.reached(&tenant, "openai", CapUnit::Characters), None);
        caps.add(&tenant, "openai", CapUnit::Characters, 10);
        assert_eq!(
            caps.reached(&tenant, "openai", CapUnit::Characters),
            Some(Reached {
                cap: 100,
                used: 100
            })
        );
        assert_eq!(caps.reached(&tenant, "piper", CapUnit::Characters), None);
        assert_eq!(
            caps.reached(&TenantId::new().to_string(), "openai", CapUnit::Characters),
            None
        );

        // The month turns: nothing counted yet.
        let (_, next) = month_of(*clock.lock().unwrap());
        *clock.lock().unwrap() = next + Duration::from_secs(1);
        assert_eq!(caps.reached(&tenant, "openai", CapUnit::Characters), None);

        // The cap removed: gone at the next read.
        *clock.lock().unwrap() = next - Duration::from_secs(1);
        store.caps.lock().unwrap().clear();
        caps.refresh().await;
        assert_eq!(caps.reached(&tenant, "openai", CapUnit::Characters), None);
    }
}
