//! Backend health, probed in the background.
//!
//! Each configured backend is asked its health path every
//! `health_check_interval` (`docs/DEPLOYMENT_GAPS.md` §1.4); the last
//! answer is what `GET /gateway/status` reports, with how old it is. A
//! status request never probes a backend itself, so a slow backend cannot
//! make the gateway's own status slow, and an unreachable one is seen by
//! the probe rather than by whoever asks next.

use crate::config::{BackendKind, BackendService, GatewayConfig};
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The last probe of one backend.
#[derive(Debug, Clone)]
struct Probe {
    result: Value,
    at: Instant,
}

type Slot = Arc<RwLock<Option<Probe>>>;

/// The last probe of every configured backend. Cheap to clone; every
/// clone reads the same probes.
#[derive(Debug, Clone, Default)]
pub struct BackendHealth {
    admin: Slot,
    events: Option<Slot>,
    jobs: Option<Slot>,
}

impl BackendHealth {
    /// Slots for the backends `config` names, each empty until its first
    /// probe lands.
    pub fn new(config: &GatewayConfig) -> Self {
        Self {
            admin: Slot::default(),
            events: config.backends.events.as_ref().map(|_| Slot::default()),
            jobs: config.backends.jobs.as_ref().map(|_| Slot::default()),
        }
    }

    /// Start one probe loop per backend: an immediate probe, then one
    /// every `health_check_interval`.
    pub fn spawn_probes(&self, client: reqwest::Client, config: Arc<GatewayConfig>) {
        spawn_probe(
            client.clone(),
            config.backends.admin.clone(),
            BackendKind::Admin,
            Arc::clone(&self.admin),
        );
        if let (Some(backend), Some(slot)) = (&config.backends.events, &self.events) {
            spawn_probe(
                client.clone(),
                backend.clone(),
                BackendKind::Events,
                Arc::clone(slot),
            );
        }
        if let (Some(backend), Some(slot)) = (&config.backends.jobs, &self.jobs) {
            spawn_probe(client, backend.clone(), BackendKind::Jobs, Arc::clone(slot));
        }
    }

    /// What `/gateway/status` says: `operational` while every configured
    /// backend's last probe was healthy, `degraded` otherwise (a backend
    /// not yet probed counts as not healthy).
    pub fn report(&self, config: &GatewayConfig) -> Value {
        let admin = report_slot(&self.admin, &config.backends.admin);
        let events = match (&self.events, &config.backends.events) {
            (Some(slot), Some(backend)) => report_slot(slot, backend),
            _ => Value::Null,
        };
        let jobs = match (&self.jobs, &config.backends.jobs) {
            (Some(slot), Some(backend)) => report_slot(slot, backend),
            _ => Value::Null,
        };
        let healthy = |v: &Value| v.is_null() || v["status"] == "healthy";
        let status = if healthy(&admin) && healthy(&events) && healthy(&jobs) {
            "operational"
        } else {
            "degraded"
        };
        json!({
            "status": status,
            "service": "fcp-api-gateway",
            "version": env!("CARGO_PKG_VERSION"),
            "backends": {
                "admin": admin,
                "events": events,
                "jobs": jobs,
            }
        })
    }
}

fn report_slot(slot: &Slot, backend: &BackendService) -> Value {
    match slot.read().ok().and_then(|p| p.clone()) {
        Some(probe) => {
            let mut v = probe.result;
            v["checked_secs_ago"] = json!(probe.at.elapsed().as_secs());
            v
        }
        None => json!({
            "url": backend.url,
            "status": "unknown",
            "reason": "not probed yet",
        }),
    }
}

fn spawn_probe(client: reqwest::Client, backend: BackendService, kind: BackendKind, slot: Slot) {
    tokio::spawn(async move {
        let interval = backend.health_check_interval.max(Duration::from_secs(1));
        loop {
            let result = probe(&client, &backend, kind).await;
            if let Ok(mut s) = slot.write() {
                *s = Some(Probe {
                    result,
                    at: Instant::now(),
                });
            }
            tokio::time::sleep(interval).await;
        }
    });
}

/// One probe of `backend` at its health path for `kind`.
pub async fn probe(client: &reqwest::Client, backend: &BackendService, kind: BackendKind) -> Value {
    let path = backend.health_check_path(kind);
    let url = format!("{}{}", backend.url.trim_end_matches('/'), path);
    let start = Instant::now();
    let checked_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();

    match client
        .get(&url)
        .timeout(backend.connection_timeout)
        .send()
        .await
    {
        Ok(resp) => {
            let response_time_ms = start.elapsed().as_millis() as u64;
            let code = resp.status().as_u16();
            let status = if resp.status().is_success() {
                "healthy"
            } else {
                "unhealthy"
            };
            json!({
                "url": backend.url,
                "path": path,
                "status": status,
                "http_status": code,
                "response_time_ms": response_time_ms,
                "checked_at": checked_at,
            })
        }
        Err(e) => {
            let response_time_ms = start.elapsed().as_millis() as u64;
            let status = if e.is_timeout() || e.is_connect() {
                "unreachable"
            } else {
                "unhealthy"
            };
            json!({
                "url": backend.url,
                "path": path,
                "status": status,
                "error": e.to_string(),
                "response_time_ms": response_time_ms,
                "checked_at": checked_at,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backend_not_yet_probed_makes_the_gateway_degraded() {
        let config = GatewayConfig::default();
        let health = BackendHealth::new(&config);
        let report = health.report(&config);
        assert_eq!(report["status"], "degraded");
        assert_eq!(report["backends"]["admin"]["status"], "unknown");
        assert!(report["backends"]["events"].is_null());
    }

    #[test]
    fn every_healthy_probe_makes_the_gateway_operational() {
        let config = GatewayConfig::default();
        let health = BackendHealth::new(&config);
        *health.admin.write().unwrap() = Some(Probe {
            result: json!({"status": "healthy"}),
            at: Instant::now(),
        });
        let report = health.report(&config);
        assert_eq!(report["status"], "operational");
        assert_eq!(report["backends"]["admin"]["checked_secs_ago"], 0);
    }
}
