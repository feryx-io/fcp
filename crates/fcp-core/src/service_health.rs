//! What a running service says about itself (`docs/SERVICES.md` §2): its
//! state, the checks of what it depends on, and its load. The service
//! beacon in `fcp-cluster` writes it to Redis for the admin's Services
//! page; `/ready` answers from the same checks, so a load balancer and the
//! page agree.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Where a service stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    /// Taking work, every check passing.
    Serving,
    /// Taking work, a check failing.
    Degraded,
    /// Finishing what it has, taking nothing new.
    Draining,
    /// Shutting down on purpose: its last word, so the admin can tell a
    /// service stopped from one lost.
    Stopping,
}

/// One thing a service depends on, as it last found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CheckResult {
    /// `database`, `redis`, or the dependency's kind (`tts`, `events`, …).
    pub name: String,
    /// A failing required check makes `/ready` answer 503; any other
    /// failing check only makes the service degraded.
    pub required: bool,
    pub ok: bool,
    /// Why it failed, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// When the check came to its current answer (Unix seconds).
    pub since: u64,
}

/// What one running process says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ServiceRecord {
    /// `[service] node_id`, unique among the deployment's processes.
    pub id: String,
    /// What it is: `call_manager`, `sbc`, `voicemail`, `conference`, `ivr`,
    /// `media`, `presence`, `tts`, `webrtc_gateway`, `events`, `jobs`,
    /// `api_gateway`, `registrar`, `admin`, `link`.
    pub kind: String,
    /// The machine or container it runs on.
    pub host: String,
    pub version: String,
    /// Unix seconds.
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Whether it is also a cluster member (`[cluster] enabled`).
    #[serde(default)]
    pub member: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sip_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    pub state: ServiceState,
    #[serde(default)]
    pub checks: Vec<CheckResult>,
    /// Numbers the kind has: `calls`, `rooms`, `sessions`, `jobs_queued`, …
    #[serde(default)]
    pub load: BTreeMap<String, u64>,
    /// When this record was written (Unix seconds).
    pub beat_at: u64,
}

/// The checks a service last ran and its load, shared between the beacon
/// that writes them and the `/ready` handler that answers from them.
#[derive(Debug, Clone, Default)]
pub struct ServiceHealth {
    inner: Arc<RwLock<HealthInner>>,
}

#[derive(Debug, Default)]
struct HealthInner {
    checks: Vec<CheckResult>,
    load: BTreeMap<String, u64>,
}

impl ServiceHealth {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a check's answer, keeping `since` while the answer is
    /// unchanged.
    pub fn record(&self, name: &str, required: bool, outcome: Result<(), String>, now: u64) {
        let Ok(mut inner) = self.inner.write() else {
            return;
        };
        let ok = outcome.is_ok();
        let error = outcome.err();
        match inner.checks.iter_mut().find(|c| c.name == name) {
            Some(c) => {
                if c.ok != ok {
                    c.since = now;
                }
                c.ok = ok;
                c.error = error;
                c.required = required;
            }
            None => inner.checks.push(CheckResult {
                name: name.to_string(),
                required,
                ok,
                error,
                since: now,
            }),
        }
    }

    pub fn set_load(&self, load: BTreeMap<String, u64>) {
        if let Ok(mut inner) = self.inner.write() {
            inner.load = load;
        }
    }

    pub fn checks(&self) -> Vec<CheckResult> {
        self.inner
            .read()
            .map(|i| i.checks.clone())
            .unwrap_or_default()
    }

    pub fn load(&self) -> BTreeMap<String, u64> {
        self.inner
            .read()
            .map(|i| i.load.clone())
            .unwrap_or_default()
    }

    /// Whether every required check passes. Before any check has run the
    /// service is taken as ready, as it was before checks existed.
    pub fn ready(&self) -> bool {
        self.checks().iter().all(|c| c.ok || !c.required)
    }

    /// Whether any check fails.
    pub fn degraded(&self) -> bool {
        self.checks().iter().any(|c| !c.ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_keeps_its_since_until_its_answer_changes() {
        let h = ServiceHealth::new();
        assert!(h.ready() && !h.degraded(), "nothing checked yet");
        h.record("tts", false, Ok(()), 10);
        h.record("tts", false, Ok(()), 15);
        assert_eq!(h.checks()[0].since, 10);
        h.record("tts", false, Err("refused".into()), 20);
        assert_eq!(h.checks()[0].since, 20);
        assert_eq!(h.checks()[0].error.as_deref(), Some("refused"));
        assert!(h.ready(), "a dependency the service can do without");
        assert!(h.degraded());
        h.record("database", true, Err("timed out".into()), 25);
        assert!(!h.ready());
        h.record("database", true, Ok(()), 30);
        h.record("tts", false, Ok(()), 30);
        assert!(h.ready() && !h.degraded());
        assert_eq!(h.checks()[0].error, None);
    }
}
