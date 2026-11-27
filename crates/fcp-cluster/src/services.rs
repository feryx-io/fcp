//! Every process says it is there (`docs/SERVICES.md` §2, phase S1).
//!
//! A [`ServiceBeacon`] writes the process's [`ServiceRecord`] to
//! `fcp:service:<id>` every five seconds with a fifteen-second TTL: what it
//! is, where, which release, its state, the checks of what it depends on and
//! its load. It is apart from cluster membership (`fcp:cluster:node:<id>`):
//! a beacon changes nothing about how calls are placed or handed over, so
//! every binary with a Redis runs one. The admin reads them for its Services
//! page (`ServiceBeacon::read_all`).

pub use fcp_core::service_health::ServiceRecord;
use fcp_core::service_health::{ServiceHealth, ServiceState};
use fcp_storage_redis::RedisStorage;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// How often a beacon is written.
pub const SERVICE_INTERVAL_SECS: u64 = 5;
/// How long a beacon lasts unless written again.
pub const SERVICE_TTL_SECS: u64 = 15;
/// How long one check may take before it counts as failing.
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// Who a process is, for its beacon.
#[derive(Debug, Clone)]
pub struct ServiceIdentity {
    pub id: String,
    pub kind: String,
    pub version: String,
    pub region: Option<String>,
    pub member: bool,
    pub http_address: Option<String>,
    pub sip_address: Option<String>,
    pub public_url: Option<String>,
}

impl ServiceIdentity {
    /// A process of `kind` named `id`, on this crate's release unless
    /// [`Self::version`] says otherwise.
    pub fn new(id: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            region: None,
            member: false,
            http_address: None,
            sip_address: None,
            public_url: None,
        }
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    pub fn region(mut self, region: Option<String>) -> Self {
        self.region = region;
        self
    }

    pub fn member(mut self, member: bool) -> Self {
        self.member = member;
        self
    }

    pub fn http(mut self, address: impl ToString) -> Self {
        self.http_address = Some(address.to_string());
        self
    }

    pub fn sip(mut self, address: impl ToString) -> Self {
        self.sip_address = Some(address.to_string());
        self
    }

    pub fn public_url(mut self, url: Option<String>) -> Self {
        self.public_url = url;
        self
    }
}

type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// A question a service asks of something it depends on.
pub type CheckFn = Arc<dyn Fn() -> BoxFut<Result<(), String>> + Send + Sync>;
/// The numbers a service reports as its load.
pub type LoadFn = Arc<dyn Fn() -> BoxFut<BTreeMap<String, u64>> + Send + Sync>;
/// Whether the service is draining.
pub type DrainingFn = Arc<dyn Fn() -> bool + Send + Sync>;

/// One check a beacon runs on its own timer.
#[derive(Clone)]
pub struct Check {
    name: String,
    required: bool,
    run: CheckFn,
}

impl Check {
    pub fn new<F, Fut>(name: impl Into<String>, required: bool, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        Self {
            name: name.into(),
            required,
            run: Arc::new(move || Box::pin(f())),
        }
    }

    /// The database, asked one cheap question by `probe`; required.
    pub fn database<F, Fut>(probe: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        Self::new("database", true, probe)
    }

    /// The database through a [`crate::DatabaseProbe`], the cluster's own
    /// form of the question.
    pub fn database_probe(probe: crate::DatabaseProbe) -> Self {
        Self::new("database", true, move || {
            let probe = Arc::clone(&probe);
            async move {
                if probe().await {
                    Ok(())
                } else {
                    Err("the database did not answer".to_string())
                }
            }
        })
    }

    /// The database, asked for a tenant that does not exist: the cheapest
    /// question every binary's store can answer.
    pub fn tenants(store: Arc<dyn fcp_core::storage::TenantStore>) -> Self {
        let nobody = fcp_core::TenantId::from_uuid(uuid::Uuid::nil());
        Self::database(move || {
            let store = Arc::clone(&store);
            async move {
                store
                    .get_tenant(&nobody)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        })
    }

    /// A service this one calls over HTTP, answered by a 2xx from `url`
    /// (its `/health`). Not required: the process serves what it can
    /// without it.
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        let url = url.into();
        let client = reqwest::Client::builder()
            .timeout(CHECK_TIMEOUT)
            // The deployment's own services, often on a private CA.
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap_or_default();
        Self::new(name, false, move || {
            let client = client.clone();
            let url = url.clone();
            async move {
                match client.get(&url).send().await {
                    Ok(r) if r.status().is_success() => Ok(()),
                    Ok(r) => Err(format!("{} answered {}", url, r.status())),
                    Err(e) => Err(format!("{}: {}", url, short_error(&e))),
                }
            }
        })
    }

    /// A service this one calls, named by any of its URLs: its `/health`
    /// at that URL's origin (`health_url`).
    pub fn health(name: impl Into<String>, url: &str) -> Self {
        Self::http(name, health_url(url))
    }
}

/// Where a service named by any of its URLs answers its health: the
/// URL's origin and `/health`. `[events] server_url` is the events
/// server's ingest endpoint (`http://host:8081/ingest`), whose health is
/// at `http://host:8081/health`, not under it.
pub fn health_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut u) => {
            u.set_path("/health");
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => format!("{}/health", url.trim_end_matches('/')),
    }
}

fn short_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "no answer".to_string()
    } else if e.is_connect() {
        "connection refused or unreachable".to_string()
    } else {
        e.to_string()
    }
}

/// Writes this process's [`ServiceRecord`] until shut down.
pub struct ServiceBeacon {
    identity: ServiceIdentity,
    checks: Vec<Check>,
    load: Option<LoadFn>,
    draining: Option<DrainingFn>,
    health: ServiceHealth,
    interval: Duration,
    ttl: u64,
}

impl ServiceBeacon {
    pub fn new(identity: ServiceIdentity) -> Self {
        Self {
            identity,
            checks: Vec::new(),
            load: None,
            draining: None,
            health: ServiceHealth::new(),
            interval: Duration::from_secs(SERVICE_INTERVAL_SECS),
            ttl: SERVICE_TTL_SECS,
        }
    }

    pub fn check(mut self, check: Check) -> Self {
        self.checks.push(check);
        self
    }

    pub fn load<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = BTreeMap<String, u64>> + Send + 'static,
    {
        self.load = Some(Arc::new(move || Box::pin(f())));
        self
    }

    pub fn draining<F>(mut self, f: F) -> Self
    where
        F: Fn() -> bool + Send + Sync + 'static,
    {
        self.draining = Some(Arc::new(f));
        self
    }

    /// For tests: beat faster.
    pub fn with_intervals(mut self, interval: Duration, ttl_secs: u64) -> Self {
        self.interval = interval;
        self.ttl = ttl_secs;
        self
    }

    /// The checks as last run, for `/ready`.
    pub fn health(&self) -> ServiceHealth {
        self.health.clone()
    }

    /// Start beating into the Redis at `config`. The connection is made in
    /// the beacon's own task and made again while Redis is away (the
    /// `redis` check fails meanwhile), so a process never waits on it.
    pub fn start(self, config: fcp_storage_redis::RedisConfig) -> BeaconHandle {
        // The process's log lines go to the same Redis, under its id
        // (`docs/LOG_VIEWER.md`).
        crate::logging::start_shipping(&self.identity, config.clone());
        let (shutdown, shutdown_rx) = watch::channel(false);
        let health = self.health.clone();
        let task = tokio::spawn(self.run(config, shutdown_rx));
        BeaconHandle {
            shutdown,
            task: tokio::sync::Mutex::new(Some(task)),
            health,
        }
    }

    async fn run(
        self,
        config: fcp_storage_redis::RedisConfig,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let started_at = unix_now();
        let host = host_name();
        let mut storage: Option<RedisStorage> = None;
        let mut told_unreachable = false;
        info!(
            id = %self.identity.id,
            kind = %self.identity.kind,
            "Service beacon starting"
        );
        let mut tick = tokio::time::interval(self.interval);
        loop {
            let stopping = tokio::select! {
                _ = tick.tick() => false,
                _ = shutdown.changed() => true,
            };

            if !stopping {
                self.run_checks().await;
                if let Some(load) = &self.load {
                    if let Ok(numbers) = tokio::time::timeout(CHECK_TIMEOUT, load()).await {
                        self.health.set_load(numbers);
                    }
                }
            }

            if storage.is_none() {
                match tokio::time::timeout(CHECK_TIMEOUT, RedisStorage::new(config.clone())).await {
                    Ok(Ok(s)) => storage = Some(s),
                    Ok(Err(e)) => self.redis_failed(&mut told_unreachable, format!("{e:#}")),
                    Err(_) => self.redis_failed(&mut told_unreachable, "no answer".into()),
                }
            }

            if let Some(s) = &storage {
                let state = if stopping {
                    ServiceState::Stopping
                } else if self.draining.as_ref().is_some_and(|d| d()) {
                    ServiceState::Draining
                } else if self.health.degraded() {
                    ServiceState::Degraded
                } else {
                    ServiceState::Serving
                };
                // A record that lands is Redis answering; one that does not
                // is put right below.
                self.health.record("redis", true, Ok(()), unix_now());
                let record = self.record(&host, started_at, state);
                match tokio::time::timeout(CHECK_TIMEOUT, write(s, &record, self.ttl)).await {
                    Ok(Ok(())) => told_unreachable = false,
                    Ok(Err(e)) => self.redis_failed(&mut told_unreachable, e),
                    Err(_) => self.redis_failed(&mut told_unreachable, "no answer".into()),
                }
                // A level set from the admin's Logs page, for a while
                // (`docs/LOG_VIEWER.md` §2); none is the configured one.
                if !stopping {
                    let key = s.key(&crate::logging::level_key(&self.identity.id));
                    let mut conn = s.connection();
                    if let Ok(Ok(level)) = tokio::time::timeout(
                        CHECK_TIMEOUT,
                        redis::cmd("GET")
                            .arg(&key)
                            .query_async::<Option<String>>(&mut conn),
                    )
                    .await
                    {
                        crate::logging::set_level(level.as_deref());
                    }
                }
            }

            if stopping {
                break;
            }
        }
    }

    fn redis_failed(&self, told: &mut bool, error: String) {
        if !*told {
            warn!(id = %self.identity.id, error = %error, "Service beacon cannot reach Redis");
            *told = true;
        }
        self.health.record("redis", true, Err(error), unix_now());
    }

    async fn run_checks(&self) {
        let runs = self.checks.iter().map(|c| {
            let run = Arc::clone(&c.run);
            async move {
                let outcome = match tokio::time::timeout(CHECK_TIMEOUT, run()).await {
                    Ok(outcome) => outcome,
                    Err(_) => Err("no answer".to_string()),
                };
                (c, outcome)
            }
        });
        let now = unix_now();
        for (c, outcome) in futures::future::join_all(runs).await {
            if let Err(e) = &outcome {
                debug!(check = %c.name, error = %e, "Service check failing");
            }
            self.health.record(&c.name, c.required, outcome, now);
        }
    }

    fn record(&self, host: &str, started_at: u64, state: ServiceState) -> ServiceRecord {
        let i = &self.identity;
        ServiceRecord {
            id: i.id.clone(),
            kind: i.kind.clone(),
            host: host.to_string(),
            version: i.version.clone(),
            started_at,
            region: i.region.clone(),
            member: i.member,
            http_address: i.http_address.clone(),
            sip_address: i.sip_address.clone(),
            public_url: i.public_url.clone(),
            state,
            checks: self.health.checks(),
            load: self.health.load(),
            beat_at: unix_now(),
        }
    }

    /// Every beacon in the Redis behind `storage`, by id.
    pub async fn read_all(storage: &RedisStorage) -> anyhow::Result<Vec<ServiceRecord>> {
        let mut conn = storage.connection();
        let pattern = storage.key("service:*");
        let mut keys: Vec<String> = Vec::new();
        let mut cursor: u64 = 0;
        loop {
            let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(200)
                .query_async(&mut conn)
                .await?;
            keys.extend(batch);
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let values: Vec<Option<String>> =
            redis::cmd("MGET").arg(&keys).query_async(&mut conn).await?;
        let mut records: Vec<ServiceRecord> = values
            .into_iter()
            .flatten()
            .filter_map(|v| serde_json::from_str(&v).ok())
            .collect();
        records.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(records)
    }
}

/// A running beacon.
pub struct BeaconHandle {
    shutdown: watch::Sender<bool>,
    task: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    health: ServiceHealth,
}

impl BeaconHandle {
    pub fn health(&self) -> ServiceHealth {
        self.health.clone()
    }

    /// Say `stopping` once and stop.
    pub async fn shutdown(&self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.task.lock().await.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        }
    }
}

async fn write(storage: &RedisStorage, record: &ServiceRecord, ttl: u64) -> Result<(), String> {
    let key = storage.key(&format!("service:{}", record.id));
    let json = serde_json::to_string(record).map_err(|e| e.to_string())?;
    let mut conn = storage.connection();
    redis::cmd("SET")
        .arg(&key)
        .arg(json)
        .arg("EX")
        .arg(ttl)
        .query_async::<()>(&mut conn)
        .await
        .map_err(|e| e.to_string())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The machine's name: `HOSTNAME` (a container's), else `/etc/hostname`.
fn host_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::health_url;

    #[test]
    fn health_is_asked_at_the_origin() {
        assert_eq!(
            health_url("http://127.0.0.1:8081/ingest"),
            "http://127.0.0.1:8081/health"
        );
        assert_eq!(
            health_url("http://fcp-tts:8093/"),
            "http://fcp-tts:8093/health"
        );
        assert_eq!(
            health_url("https://events.example/ingest?x=1"),
            "https://events.example/health"
        );
    }
}
