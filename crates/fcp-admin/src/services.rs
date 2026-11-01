//! The admin's record of every service (`docs/SERVICES.md` §2, phase S2).
//!
//! Every process writes a beacon to Redis (`fcp_cluster::ServiceBeacon`);
//! a site's link carries its site's beacons up on its heartbeat. A beacon
//! lasts fifteen seconds, so this keeps what was seen in Postgres
//! (`service_instances`): a service whose beacon goes is "down since", not
//! gone, until it comes back, is forgotten by hand, or has been down for
//! `[services] forget_after_days`. Coming up and going down are said once,
//! as `system.service_up` and `system.service_down`, by whichever admin
//! holds the sweep's lock; nothing is said down while Redis cannot be read.
//!
//! Routes (platform administrators only): `GET /system/services`,
//! `GET /system/services/summary`, `GET /system/services/{id}` and
//! `DELETE /system/services/{id}` (a site's with `?site_id=`).

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use fcp_api_core::error::ApiError;
use fcp_api_core::page::{Order, Page, PageQuery};
use fcp_core::service_health::{ServiceRecord, ServiceState};
use fcp_core::service_registry::{ServiceInstance, ServiceInstanceStore};
use fcp_events::{EventPublisher, PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// How often the admin looks at the beacons.
const SWEEP_EVERY: Duration = Duration::from_secs(fcp_cluster::SERVICE_INTERVAL_SECS);
/// The sweep's lock outlives a missed tick or two, not a dead admin.
const LOCK_SECS: u64 = 15;

/// What changed for one service at a look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// First seen, or back; how long it was down when it was.
    Up { down_secs: Option<u64> },
    /// Its beacon went, or said it was stopping.
    Down,
}

/// Compare what was known in one scope (the platform's, or a site's) with
/// the beacons there now. Every live service comes back updated; every
/// known one whose beacon is gone comes back down, once. A beacon that says
/// `stopping` is down at once, stopped on purpose.
pub fn reconcile(
    known: &[ServiceInstance],
    site_id: Option<&str>,
    live: &[ServiceRecord],
    now: SystemTime,
) -> Vec<(ServiceInstance, Option<Change>)> {
    let in_scope = |i: &&ServiceInstance| i.site_id.as_deref() == site_id;
    let by_id: HashMap<&str, &ServiceInstance> = known
        .iter()
        .filter(in_scope)
        .map(|i| (i.id.as_str(), i))
        .collect();
    let mut out = Vec::new();
    for record in live {
        let stopping = record.state == ServiceState::Stopping;
        let before = by_id.get(record.id.as_str()).copied();
        let was_up = before.is_some_and(|b| b.up);
        let change = match (before, stopping) {
            (_, true) if was_up => Some(Change::Down),
            (_, true) => None,
            (None, false) => Some(Change::Up { down_secs: None }),
            (Some(b), false) if !b.up => Some(Change::Up {
                down_secs: b
                    .down_since
                    .and_then(|d| now.duration_since(d).ok())
                    .map(|d| d.as_secs()),
            }),
            (Some(_), false) => None,
        };
        let instance = ServiceInstance {
            id: record.id.clone(),
            site_id: site_id.map(str::to_string),
            kind: record.kind.clone(),
            host: record.host.clone(),
            version: record.version.clone(),
            first_seen: before.map(|b| b.first_seen).unwrap_or(now),
            last_seen: now,
            up: !stopping,
            down_since: if stopping {
                before.and_then(|b| b.down_since).or(Some(now))
            } else {
                None
            },
            stopped: stopping,
            last: serde_json::to_value(record).unwrap_or_default(),
        };
        out.push((instance, change));
    }
    for (id, before) in by_id {
        if before.up && !live.iter().any(|r| r.id == id) {
            let mut gone = before.clone();
            gone.up = false;
            gone.down_since = Some(now);
            out.push((gone, Some(Change::Down)));
        }
    }
    out
}

/// What the last look at Redis found.
#[derive(Debug, Clone, Default)]
struct Sweep {
    /// When Redis was last read; none before the first.
    read_at: Option<SystemTime>,
    /// Why it could not be read, until it can.
    error: Option<String>,
}

/// The record, the events and the last sweep.
pub struct ServiceRegistry {
    store: Arc<dyn ServiceInstanceStore>,
    events: Arc<dyn EventPublisher>,
    sweep: RwLock<Sweep>,
    /// One look at a time, so two never say the same change.
    looking: tokio::sync::Mutex<()>,
}

impl ServiceRegistry {
    pub fn new(store: Arc<dyn ServiceInstanceStore>, events: Arc<dyn EventPublisher>) -> Self {
        Self {
            store,
            events,
            sweep: RwLock::new(Sweep::default()),
            looking: tokio::sync::Mutex::new(()),
        }
    }

    /// Take in what one scope's beacons say now: the platform's from this
    /// admin's Redis, a site's from its link's heartbeat.
    pub async fn observe(
        &self,
        site_id: Option<&str>,
        live: &[ServiceRecord],
    ) -> anyhow::Result<()> {
        let _one = self.looking.lock().await;
        let known = self.store.list_service_instances().await?;
        let now = SystemTime::now();
        for (instance, change) in reconcile(&known, site_id, live, now) {
            self.store.put_service_instance(&instance).await?;
            if let Some(change) = change {
                self.announce(&instance, change, now).await;
            }
        }
        Ok(())
    }

    async fn announce(&self, i: &ServiceInstance, change: Change, now: SystemTime) {
        let event = match change {
            Change::Up { down_secs } => {
                info!(service_id = %i.id, kind = %i.kind, site_id = ?i.site_id, "A service is up");
                SystemEvent::ServiceUp {
                    service_id: i.id.clone(),
                    kind: i.kind.clone(),
                    host: i.host.clone(),
                    version: i.version.clone(),
                    site_id: i.site_id.clone(),
                    down_secs,
                    timestamp: now,
                }
            }
            Change::Down => {
                warn!(
                    service_id = %i.id, kind = %i.kind, site_id = ?i.site_id,
                    stopped = i.stopped, "A service is down"
                );
                SystemEvent::ServiceDown {
                    service_id: i.id.clone(),
                    kind: i.kind.clone(),
                    host: i.host.clone(),
                    version: i.version.clone(),
                    site_id: i.site_id.clone(),
                    stopped: i.stopped,
                    last_seen: secs(i.last_seen),
                    timestamp: now,
                }
            }
        };
        if let Err(e) = self.events.publish(PlatformEvent::System(event)).await {
            warn!(error = %e, "Failed to publish a service's change");
        }
    }

    fn swept(&self, outcome: Result<(), String>) {
        if let Ok(mut s) = self.sweep.write() {
            match outcome {
                Ok(()) => {
                    s.read_at = Some(SystemTime::now());
                    s.error = None;
                }
                Err(e) => s.error = Some(e),
            }
        }
    }

    fn sweep_state(&self) -> Sweep {
        self.sweep.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// The platform's Redis cannot be read now (or never was).
    fn redis_unreadable(&self) -> bool {
        let s = self.sweep_state();
        s.error.is_some() || s.read_at.is_none()
    }

    /// Look at this admin's Redis every five seconds while holding the
    /// sweep's lock there, and forget long-down services hourly.
    pub fn spawn_sweep(
        self: Arc<Self>,
        redis: fcp_storage_redis::RedisConfig,
        node_id: String,
        forget_after: Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut storage: Option<fcp_storage_redis::RedisStorage> = None;
            let mut tick = tokio::time::interval(SWEEP_EVERY);
            let mut last_forget = SystemTime::UNIX_EPOCH;
            loop {
                tick.tick().await;
                if storage.is_none() {
                    match fcp_storage_redis::RedisStorage::new(redis.clone()).await {
                        Ok(s) => storage = Some(s),
                        Err(e) => {
                            self.swept(Err(format!("{e:#}")));
                            continue;
                        }
                    }
                }
                let Some(s) = &storage else { continue };
                match hold_lock(s, &node_id).await {
                    Ok(true) => {}
                    // Another admin sweeps; this one reads what it wrote.
                    Ok(false) => {
                        self.swept(Ok(()));
                        continue;
                    }
                    Err(e) => {
                        self.swept(Err(e.to_string()));
                        continue;
                    }
                }
                match fcp_cluster::ServiceBeacon::read_all(s).await {
                    Ok(live) => {
                        self.swept(Ok(()));
                        if let Err(e) = self.observe(None, &live).await {
                            warn!(error = %e, "The service sweep could not keep its record");
                        }
                    }
                    // Nothing is said down while the beacons cannot be read.
                    Err(e) => self.swept(Err(e.to_string())),
                }
                let now = SystemTime::now();
                if now
                    .duration_since(last_forget)
                    .is_ok_and(|d| d >= Duration::from_secs(3600))
                {
                    last_forget = now;
                    match self
                        .store
                        .forget_service_instances_down_before(now - forget_after)
                        .await
                    {
                        Ok(0) => {}
                        Ok(n) => info!(forgotten = n, "Forgot services long down"),
                        Err(e) => warn!(error = %e, "Could not forget services long down"),
                    }
                }
            }
        })
    }
}

/// Take or keep the sweep's lock; whether this admin holds it.
async fn hold_lock(
    storage: &fcp_storage_redis::RedisStorage,
    node_id: &str,
) -> anyhow::Result<bool> {
    let key = storage.key("services:sweep");
    let mut conn = storage.connection();
    let taken: Option<String> = redis::cmd("SET")
        .arg(&key)
        .arg(node_id)
        .arg("NX")
        .arg("EX")
        .arg(LOCK_SECS)
        .query_async(&mut conn)
        .await?;
    if taken.is_some() {
        return Ok(true);
    }
    let holder: Option<String> = redis::cmd("GET").arg(&key).query_async(&mut conn).await?;
    if holder.as_deref() == Some(node_id) {
        let _: () = redis::cmd("EXPIRE")
            .arg(&key)
            .arg(LOCK_SECS)
            .query_async(&mut conn)
            .await?;
        return Ok(true);
    }
    Ok(false)
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

// ── The API ───────────────────────────────────────────────────────────

/// One check, as the page shows it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ServiceCheckView {
    pub name: String,
    pub required: bool,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// When it came to its current answer.
    #[serde(with = "fcp_api_core::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub since: SystemTime,
}

/// One service.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ServiceView {
    pub id: String,
    /// The site it runs at; null for the platform's own.
    pub site_id: Option<String>,
    pub site_name: Option<String>,
    pub kind: String,
    pub host: String,
    pub version: String,
    /// `serving`, `degraded`, `draining`, `stopping`, `down` (its beacon
    /// went), `stopped` (it said it was stopping), or `unknown` (Redis, or
    /// the site's link, cannot be read now).
    pub state: String,
    #[serde(with = "fcp_api_core::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub first_seen: SystemTime,
    #[serde(with = "fcp_api_core::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub last_seen: SystemTime,
    #[serde(with = "fcp_api_core::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub down_since: Option<SystemTime>,
    /// When the process started, from its last beacon.
    #[serde(with = "fcp_api_core::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub started_at: Option<SystemTime>,
    pub region: Option<String>,
    /// Also a cluster member.
    pub member: bool,
    pub http_address: Option<String>,
    pub sip_address: Option<String>,
    pub public_url: Option<String>,
    pub checks: Vec<ServiceCheckView>,
    pub load: BTreeMap<String, u64>,
}

/// Every service in one number each, for the dashboard.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ServicesSummary {
    pub total: u64,
    /// Serving.
    pub serving: u64,
    /// Degraded or draining.
    pub attention: u64,
    /// Down or stopped.
    pub down: u64,
    pub unknown: u64,
    /// Whether this admin can read the platform's Redis now.
    pub redis_reachable: bool,
    /// Why it cannot, when it cannot.
    pub redis_error: Option<String>,
    /// When the beacons were last read.
    #[serde(with = "fcp_api_core::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub read_at: Option<SystemTime>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ServicesQuery {
    /// Only this kind (`voicemail`, …).
    pub kind: Option<String>,
    /// Only this state (`down`, `degraded`, …).
    pub state: Option<String>,
    /// Only this site's; `platform` for the platform's own.
    pub site_id: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ServiceQuery {
    /// The site the service runs at; absent for the platform's own.
    pub site_id: Option<String>,
}

/// What the routes need.
#[derive(Clone)]
pub struct ServicesApiState {
    pub registry: Option<Arc<ServiceRegistry>>,
    pub storage: Arc<fcp_core::DynStorage>,
}

/// The platform's services routes, under `/api/v1`.
pub fn services_router<S>(state: ServicesApiState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/system/services", get(list_services))
        .route("/system/services/summary", get(summary))
        .route(
            "/system/services/:id",
            get(get_service).delete(forget_service),
        )
        .layer(Extension(state))
}

fn registry(state: &ServicesApiState) -> Result<&Arc<ServiceRegistry>, ApiError> {
    state.registry.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "Services are not kept here: add [cluster] redis_url to the admin's configuration"
                .to_string(),
        )
    })
}

/// Every service as the page shows it.
async fn views(state: &ServicesApiState) -> Result<Vec<ServiceView>, ApiError> {
    use fcp_core::SiteStore;
    let registry = registry(state)?;
    let instances = registry
        .store
        .list_service_instances()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read services: {e}")))?;
    let sites: HashMap<String, (String, bool)> = state
        .storage
        .list_all_sites()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read sites: {e}")))?
        .into_iter()
        .map(|s| (s.id.to_string(), (s.name, s.link_up)))
        .collect();
    let platform_unknown = registry.redis_unreadable();
    Ok(instances
        .into_iter()
        .map(|i| {
            let site = i.site_id.as_ref().and_then(|id| sites.get(id));
            let unknown = match &i.site_id {
                None => platform_unknown,
                Some(_) => !site.is_some_and(|(_, up)| *up),
            };
            view(i, site.map(|(name, _)| name.clone()), unknown)
        })
        .collect())
}

fn view(i: ServiceInstance, site_name: Option<String>, unknown: bool) -> ServiceView {
    let record: Option<ServiceRecord> = serde_json::from_value(i.last.clone()).ok();
    let state = if !i.up {
        if i.stopped { "stopped" } else { "down" }.to_string()
    } else if unknown {
        "unknown".to_string()
    } else {
        record
            .as_ref()
            .and_then(|r| serde_json::to_value(r.state).ok())
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string())
    };
    ServiceView {
        id: i.id,
        site_id: i.site_id,
        site_name,
        kind: i.kind,
        host: i.host,
        version: i.version,
        state,
        first_seen: i.first_seen,
        last_seen: i.last_seen,
        down_since: i.down_since,
        started_at: record.as_ref().map(|r| at(r.started_at)),
        region: record.as_ref().and_then(|r| r.region.clone()),
        member: record.as_ref().is_some_and(|r| r.member),
        http_address: record.as_ref().and_then(|r| r.http_address.clone()),
        sip_address: record.as_ref().and_then(|r| r.sip_address.clone()),
        public_url: record.as_ref().and_then(|r| r.public_url.clone()),
        checks: record
            .as_ref()
            .map(|r| {
                r.checks
                    .iter()
                    .map(|c| ServiceCheckView {
                        name: c.name.clone(),
                        required: c.required,
                        ok: c.ok,
                        error: c.error.clone(),
                        since: at(c.since),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        load: record.map(|r| r.load).unwrap_or_default(),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/system/services",
    tag = "services",
    operation_id = "services_list",
    params(ServicesQuery, PageQuery),
    responses(
        (status = 200, description = "Every service the platform has seen, its own and its sites'", body = Page<ServiceView>),
        (status = "default", description = "An error", body = fcp_api_core::error::ErrorResponse),
    ),
)]
async fn list_services(
    Extension(state): Extension<ServicesApiState>,
    Query(filter): Query<ServicesQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<ServiceView>>, ApiError> {
    let all = views(&state).await?;
    let kept: Vec<ServiceView> = all
        .into_iter()
        .filter(|v| filter.kind.as_ref().is_none_or(|k| &v.kind == k))
        .filter(|v| filter.state.as_ref().is_none_or(|s| &v.state == s))
        .filter(|v| match filter.site_id.as_deref() {
            None => true,
            Some("platform") => v.site_id.is_none(),
            Some(site) => v.site_id.as_deref() == Some(site),
        })
        .collect();
    Ok(Json(fcp_api_core::page::paginate(
        kept,
        &page,
        Order::Ascending,
        |v| format!("{}/{}", v.site_id.as_deref().unwrap_or(""), v.id),
    )?))
}

#[utoipa::path(
    get,
    path = "/api/v1/system/services/summary",
    tag = "services",
    operation_id = "services_summary",
    responses(
        (status = 200, description = "Every service in one number each", body = ServicesSummary),
        (status = "default", description = "An error", body = fcp_api_core::error::ErrorResponse),
    ),
)]
async fn summary(
    Extension(state): Extension<ServicesApiState>,
) -> Result<Json<ServicesSummary>, ApiError> {
    let all = views(&state).await?;
    let sweep = registry(&state)?.sweep_state();
    let count = |states: &[&str]| {
        all.iter()
            .filter(|v| states.contains(&v.state.as_str()))
            .count() as u64
    };
    Ok(Json(ServicesSummary {
        total: all.len() as u64,
        serving: count(&["serving"]),
        attention: count(&["degraded", "draining", "stopping"]),
        down: count(&["down", "stopped"]),
        unknown: count(&["unknown"]),
        redis_reachable: sweep.error.is_none() && sweep.read_at.is_some(),
        redis_error: sweep.error,
        read_at: sweep.read_at,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/system/services/{id}",
    tag = "services",
    operation_id = "services_get",
    params(("id" = String, Path, description = "The service's id"), ServiceQuery),
    responses(
        (status = 200, description = "One service", body = ServiceView),
        (status = "default", description = "An error", body = fcp_api_core::error::ErrorResponse),
    ),
)]
async fn get_service(
    Extension(state): Extension<ServicesApiState>,
    Path(id): Path<String>,
    Query(q): Query<ServiceQuery>,
) -> Result<Json<ServiceView>, ApiError> {
    views(&state)
        .await?
        .into_iter()
        .find(|v| v.id == id && v.site_id == q.site_id)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("No service {id}")))
}

#[utoipa::path(
    delete,
    path = "/api/v1/system/services/{id}",
    tag = "services",
    operation_id = "services_forget",
    params(("id" = String, Path, description = "The service's id"), ServiceQuery),
    responses(
        (status = 204, description = "Forgotten"),
        (status = "default", description = "An error; 409 for a service that is up", body = fcp_api_core::error::ErrorResponse),
    ),
)]
async fn forget_service(
    Extension(state): Extension<ServicesApiState>,
    Path(id): Path<String>,
    Query(q): Query<ServiceQuery>,
) -> Result<StatusCode, ApiError> {
    let registry = registry(&state)?;
    let known = registry
        .store
        .list_service_instances()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read services: {e}")))?;
    let Some(found) = known.iter().find(|i| i.id == id && i.site_id == q.site_id) else {
        return Err(ApiError::NotFound(format!("No service {id}")));
    };
    if found.up {
        return Err(ApiError::Conflict(format!(
            "{id} is up; only a service that is down can be forgotten"
        )));
    }
    registry
        .store
        .delete_service_instance(q.site_id.as_deref(), &id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to forget {id}: {e}")))?;
    info!(service_id = %id, site_id = ?q.site_id, "A service was forgotten");
    Ok(StatusCode::NO_CONTENT)
}

/// The services routes' part of the contract (`crate::api_docs`).
#[derive(utoipa::OpenApi)]
#[openapi(paths(list_services, summary, get_service, forget_service))]
pub struct ServicesDoc;

// ── Metrics ───────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl fcp_api_core::MetricsSource for ServiceRegistry {
    async fn render(&self) -> String {
        let Ok(instances) = self.store.list_service_instances().await else {
            return String::new();
        };
        let mut out = String::new();
        out.push_str("# HELP fcp_service_up A service's beacon is there (1) or not (0)\n");
        out.push_str("# TYPE fcp_service_up gauge\n");
        for i in &instances {
            out.push_str(&format!(
                "fcp_service_up{{id=\"{}\",kind=\"{}\",site=\"{}\"}} {}\n",
                label(&i.id),
                label(&i.kind),
                label(i.site_id.as_deref().unwrap_or("")),
                u8::from(i.up)
            ));
        }
        out.push_str(
            "# HELP fcp_service_check_failing A service's check is failing (1) or not (0)\n",
        );
        out.push_str("# TYPE fcp_service_check_failing gauge\n");
        for i in instances.iter().filter(|i| i.up) {
            let Ok(record) = serde_json::from_value::<ServiceRecord>(i.last.clone()) else {
                continue;
            };
            for c in &record.checks {
                out.push_str(&format!(
                    "fcp_service_check_failing{{id=\"{}\",kind=\"{}\",site=\"{}\",check=\"{}\"}} {}\n",
                    label(&i.id),
                    label(&i.kind),
                    label(i.site_id.as_deref().unwrap_or("")),
                    label(&c.name),
                    u8::from(!c.ok)
                ));
            }
        }
        out
    }
}

/// A Prometheus label value, escaped.
fn label(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, state: ServiceState) -> ServiceRecord {
        ServiceRecord {
            id: id.into(),
            kind: "voicemail".into(),
            host: "h".into(),
            version: "1".into(),
            started_at: 0,
            region: None,
            member: false,
            http_address: None,
            sip_address: None,
            public_url: None,
            state,
            checks: Vec::new(),
            load: BTreeMap::new(),
            beat_at: 0,
        }
    }

    fn apply(known: &mut Vec<ServiceInstance>, out: &[(ServiceInstance, Option<Change>)]) {
        for (i, _) in out {
            known.retain(|k| !(k.id == i.id && k.site_id == i.site_id));
            known.push(i.clone());
        }
    }

    #[test]
    fn a_service_is_seen_lost_back_and_stopped() {
        let t0 = UNIX_EPOCH + Duration::from_secs(1_000);
        let mut known = Vec::new();

        let out = reconcile(&known, None, &[record("vm", ServiceState::Serving)], t0);
        assert_eq!(out[0].1, Some(Change::Up { down_secs: None }), "first seen");
        apply(&mut known, &out);

        let out = reconcile(&known, None, &[record("vm", ServiceState::Degraded)], t0);
        assert_eq!(out[0].1, None, "still up: nothing said");
        apply(&mut known, &out);

        let t1 = t0 + Duration::from_secs(5);
        let out = reconcile(&known, None, &[], t1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, Some(Change::Down));
        assert_eq!(out[0].0.down_since, Some(t1));
        assert!(!out[0].0.stopped, "lost, not stopped");
        apply(&mut known, &out);
        assert!(
            reconcile(&known, None, &[], t1).is_empty(),
            "said down once"
        );

        let t2 = t1 + Duration::from_secs(60);
        let out = reconcile(&known, None, &[record("vm", ServiceState::Serving)], t2);
        assert_eq!(
            out[0].1,
            Some(Change::Up {
                down_secs: Some(60)
            })
        );
        assert_eq!(out[0].0.first_seen, t0, "first seen is kept");
        apply(&mut known, &out);

        let out = reconcile(&known, None, &[record("vm", ServiceState::Stopping)], t2);
        assert_eq!(out[0].1, Some(Change::Down), "stopping is down at once");
        assert!(out[0].0.stopped);
        apply(&mut known, &out);
        let out = reconcile(&known, None, &[], t2 + Duration::from_secs(15));
        assert!(out.is_empty(), "its beacon expiring says nothing more");
    }

    #[test]
    fn scopes_are_apart() {
        let t0 = UNIX_EPOCH + Duration::from_secs(1_000);
        let mut known = Vec::new();
        let out = reconcile(&known, None, &[record("cm", ServiceState::Serving)], t0);
        apply(&mut known, &out);
        // A site's heartbeat without the platform's call manager says
        // nothing about it.
        let out = reconcile(
            &known,
            Some("site-1"),
            &[record("cm", ServiceState::Serving)],
            t0,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.site_id.as_deref(), Some("site-1"));
        assert_eq!(out[0].1, Some(Change::Up { down_secs: None }));
        apply(&mut known, &out);
        assert_eq!(known.len(), 2);
        assert!(reconcile(&known, Some("site-1"), &[], t0)
            .iter()
            .all(|(i, _)| i.site_id.as_deref() == Some("site-1")));
    }
}
