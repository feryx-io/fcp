//! The link itself: enroll once, then heartbeat, and keep the site's
//! configuration when it has the site's database.

use anyhow::{Context, Result};
use fcp_core::{SiteHeartbeat, SiteNode, SiteSync, TenantId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

use crate::config::LinkFileConfig;
use crate::sync::{SiteStores, Syncer};

/// The link protocol this link speaks.
pub const LINK_PROTOCOL: u32 = 1;

/// What enrollment gave, kept in the state directory as `site.json`
/// beside `link.key`, `link.crt` and `link-ca.crt`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Enrollment {
    pub site_id: String,
    pub tenant_id: String,
    pub name: String,
    /// The secret every heartbeat carries.
    pub secret: String,
    pub enrolled_at: u64,
    pub heartbeat_interval_secs: u64,
}

/// What the cloud answers an enrollment with.
#[derive(Debug, Deserialize)]
struct EnrollResponse {
    site_id: String,
    tenant_id: String,
    name: String,
    certificate_pem: String,
    ca_pem: String,
    secret: String,
    heartbeat_interval_secs: u64,
    #[allow(dead_code)]
    protocol: u32,
}

#[derive(Debug, Deserialize)]
struct HeartbeatResponse {
    #[allow(dead_code)]
    ok: bool,
    heartbeat_interval_secs: u64,
}

/// What `/status` shows.
#[derive(Debug, Clone, Serialize, Default)]
pub struct LinkStatus {
    pub enrolled: bool,
    pub site_id: Option<String>,
    pub tenant_id: Option<String>,
    pub name: Option<String>,
    pub last_heartbeat_ok_at: Option<u64>,
    pub last_heartbeat_error: Option<String>,
    pub heartbeats_sent: u64,
    pub heartbeat_interval_secs: u64,
    pub version: String,
    pub protocol: u32,
    /// Where the site's configuration stands; absent without a site
    /// database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<SiteSync>,
    /// Objects written since start.
    pub sync_applied: u64,
    /// Where the site's records stand on their way up
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3); absent without a site
    /// database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ship: Option<crate::ship::ShipStatus>,
    /// The site's events forwarded; absent without a site Redis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<crate::events::EventsStatus>,
    /// The cloud's requests answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests: Option<crate::requests::RequestsStatus>,
}

struct Inner {
    enrollment: Option<Enrollment>,
    last_ok: Option<SystemTime>,
    last_error: Option<String>,
    sent: u64,
    interval: Duration,
}

pub struct Link {
    config: LinkFileConfig,
    state_dir: PathBuf,
    inner: Mutex<Inner>,
    reader: Option<fcp_cluster::ClusterReader>,
    /// The site's stores, when `[site] database_url` is set.
    stores: Option<SiteStores>,
    /// The sync, once enrolled and the stores are known.
    sync: Mutex<Option<Arc<Syncer>>>,
    /// The site's Redis, for what the heartbeat reads: registrations,
    /// services, parked calls, nodes.
    redis: Option<fcp_storage_redis::RedisStorage>,
    /// The same Redis on a connection of its own, for the event stream.
    /// Its reader blocks (`XREADGROUP … BLOCK`), and on the shared,
    /// multiplexed connection every heartbeat read waited behind it: two
    /// seconds each, a heartbeat built in six, and the link's `cloud`
    /// check saying no heartbeat was answered while the cloud kept
    /// receiving them (#378).
    events_redis: Option<fcp_storage_redis::RedisStorage>,
    ship: Mutex<Option<Arc<crate::ship::Shipper>>>,
    events: Mutex<Option<Arc<crate::events::EventForwarder>>>,
    /// The site's Redis for its log lines, when `[site] forward_logs`.
    logs_redis: Option<fcp_storage_redis::RedisStorage>,
    logs: Mutex<Option<Arc<crate::logs::LogForwarder>>>,
    requests: Mutex<Option<Arc<crate::requests::Requester>>>,
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl Link {
    pub async fn new(config: LinkFileConfig) -> Result<Self> {
        let state_dir = PathBuf::from(&config.link.state_dir);
        std::fs::create_dir_all(&state_dir)
            .with_context(|| format!("creating the state directory {}", state_dir.display()))?;
        let enrollment = load_enrollment(&state_dir)?;
        if let Some(e) = &enrollment {
            info!(site_id = %e.site_id, name = %e.name, "Enrolled already; the state directory has the certificate");
        }
        let redis = match &config.site.redis_url {
            Some(url) => Some(
                fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
                    url: url.clone(),
                    key_prefix: "fcp:".to_string(),
                })
                .await
                .context("connecting to the site's Redis")?,
            ),
            None => None,
        };
        let events_redis = match (&redis, config.site.forward_events) {
            (Some(redis), true) => Some(
                redis
                    .detached()
                    .await
                    .context("connecting to the site's Redis for its events")?,
            ),
            _ => None,
        };
        let logs_redis = if config.site.forward_logs {
            redis.clone()
        } else {
            None
        };
        let reader = redis.clone().map(|storage| {
            fcp_cluster::ClusterReader::new(storage, config.site.heartbeat_ttl_secs)
        });
        let stores = match &config.site.database_url {
            Some(url) => {
                let stores = crate::sync::open_site(url, config.site.max_connections).await?;
                info!("The site's database is open; its configuration will follow the cloud's");
                Some(stores)
            }
            None => {
                info!("No site database ([site] database_url); this link carries no configuration");
                None
            }
        };
        let interval = Duration::from_secs(
            enrollment
                .as_ref()
                .map(|e| e.heartbeat_interval_secs)
                .unwrap_or(config.link.heartbeat_interval_secs)
                .max(1),
        );
        Ok(Self {
            config,
            state_dir,
            inner: Mutex::new(Inner {
                enrollment,
                last_ok: None,
                last_error: None,
                sent: 0,
                interval,
            }),
            reader,
            stores,
            sync: Mutex::new(None),
            redis,
            events_redis,
            ship: Mutex::new(None),
            events: Mutex::new(None),
            logs_redis,
            logs: Mutex::new(None),
            requests: Mutex::new(None),
        })
    }

    fn syncer(&self) -> Option<Arc<Syncer>> {
        self.sync.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn status(&self) -> LinkStatus {
        let syncer = self.syncer();
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        LinkStatus {
            enrolled: inner.enrollment.is_some(),
            site_id: inner.enrollment.as_ref().map(|e| e.site_id.clone()),
            tenant_id: inner.enrollment.as_ref().map(|e| e.tenant_id.clone()),
            name: inner.enrollment.as_ref().map(|e| e.name.clone()),
            last_heartbeat_ok_at: inner.last_ok.map(secs),
            last_heartbeat_error: inner.last_error.clone(),
            heartbeats_sent: inner.sent,
            heartbeat_interval_secs: inner.interval.as_secs(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: LINK_PROTOCOL,
            sync: syncer.as_ref().map(|s| s.state()),
            sync_applied: syncer.as_ref().map(|s| s.applied()).unwrap_or(0),
            ship: self
                .ship
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|s| s.status()),
            events: self
                .events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|s| s.status()),
            requests: self
                .requests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|s| s.status()),
        }
    }

    /// The site's identity and secret, once enrolled.
    fn enrolled(&self) -> Option<(String, TenantId, fcp_core::SiteId)> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let e = inner.enrollment.as_ref()?;
        let tenant = match e.tenant_id.parse() {
            Ok(t) => t,
            Err(err) => {
                warn!(error = %err, "The enrollment's tenant id does not parse");
                return None;
            }
        };
        let site = match e.site_id.parse() {
            Ok(s) => s,
            Err(err) => {
                warn!(error = %err, "The enrollment's site id does not parse");
                return None;
            }
        };
        Some((e.secret.clone(), tenant, site))
    }

    /// Start what runs for as long as the link does, once enrolled:
    /// records up, the site's events, the cloud's requests
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
    fn start_records_up(&self) {
        let Some((secret, tenant, _site)) = self.enrolled() else {
            return;
        };
        let link_url = self.config.link.link_url.clone();
        // Shipping: with a site database.
        if let Some(stores) = &self.stores {
            let mut slot = self.ship.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                match self.client(true).and_then(|client| {
                    crate::ship::Shipper::new(
                        stores.clone(),
                        client,
                        &link_url,
                        secret.clone(),
                        tenant,
                        &self.state_dir,
                        self.config.site.ship_batch,
                        Duration::from_secs(self.config.site.ship_interval_secs.max(1)),
                    )
                }) {
                    Ok(shipper) => {
                        let shipper = Arc::new(shipper);
                        let run = Arc::clone(&shipper);
                        tokio::spawn(async move { run.run().await });
                        *slot = Some(shipper);
                    }
                    Err(e) => warn!(error = %e, "Records cannot be shipped"),
                }
            }
        }
        // Events: with a site Redis.
        if let Some(redis) = &self.events_redis {
            let mut slot = self.events.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                match self.client(true) {
                    Ok(client) => {
                        let consumer = fcp_storage_redis::event_bus::RedisStreamConsumer::new(
                            redis.clone(),
                            redis.key("event_stream"),
                            crate::events::CONSUMER_GROUP.to_string(),
                            self.config.service.node_id.clone(),
                        );
                        let forwarder = Arc::new(crate::events::EventForwarder::new(
                            consumer,
                            client,
                            &link_url,
                            secret.clone(),
                            tenant,
                        ));
                        let run = Arc::clone(&forwarder);
                        tokio::spawn(async move { run.run().await });
                        *slot = Some(forwarder);
                    }
                    Err(e) => warn!(error = %e, "Events cannot be forwarded"),
                }
            }
        }
        // The site's warnings and errors, up to the cloud's Logs page
        // (`docs/LOG_VIEWER.md` §5).
        if let Some(redis) = &self.logs_redis {
            let mut slot = self.logs.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                match self.client(true) {
                    Ok(client) => {
                        let forwarder = Arc::new(crate::logs::LogForwarder::new(
                            redis.clone(),
                            client,
                            &link_url,
                            secret.clone(),
                        ));
                        let run = Arc::clone(&forwarder);
                        tokio::spawn(async move { run.run().await });
                        *slot = Some(forwarder);
                    }
                    Err(e) => warn!(error = %e, "The site's warnings cannot be carried up"),
                }
            }
        }
        // The cloud's requests: always, with the files under the roots.
        {
            let mut slot = self.requests.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                // A file may take a while to send; the poll itself waits
                // up to 25 s.
                match self.client_with(true, Duration::from_secs(600)) {
                    Ok(client) => {
                        let roots = self
                            .config
                            .site
                            .file_roots
                            .iter()
                            .map(PathBuf::from)
                            .collect();
                        let requester = Arc::new(crate::requests::Requester::new(
                            client, &link_url, secret, roots,
                        ));
                        let run = Arc::clone(&requester);
                        tokio::spawn(async move { run.run().await });
                        *slot = Some(requester);
                    }
                    Err(e) => warn!(error = %e, "The cloud's requests cannot be collected"),
                }
            }
        }
    }

    /// Start following the cloud's configuration, once, when enrolled
    /// and the site's database is open.
    fn start_sync(&self) {
        let Some(stores) = &self.stores else {
            return;
        };
        let mut slot = self.sync.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return;
        }
        let (secret, tenant, site) = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let Some(e) = &inner.enrollment else {
                return;
            };
            (e.secret.clone(), e.tenant_id.clone(), e.site_id.clone())
        };
        let site: fcp_core::SiteId = match site.parse() {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "The enrollment's site id does not parse; no sync");
                return;
            }
        };
        let tenant: TenantId = match tenant.parse() {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "The enrollment's tenant id does not parse; no sync");
                return;
            }
        };
        // The change feed is a long poll: the cloud holds it up to
        // `feed_wait_secs` before answering an idle one, so the request
        // may take that long on top of the usual timeout — shorter, and
        // every quiet stretch ended as a sync error.
        let client = match self.client_with(
            true,
            Duration::from_secs(
                self.config.link.request_timeout_secs + self.config.site.feed_wait_secs,
            ),
        ) {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "No client for the sync");
                return;
            }
        };
        let secret_for_firmware = secret.clone();
        match Syncer::new(
            stores.clone(),
            client,
            &self.config.link.link_url,
            secret,
            tenant,
            site,
            &self.state_dir,
            self.config.site.feed_wait_secs,
        ) {
            Ok(syncer) => {
                // The site's firmware, fetched for the targets it holds
                // (docs/PROVISIONING.md §11).
                let syncer = match (
                    &self.config.site.firmware_dir,
                    self.client_with(true, Duration::from_secs(6 * 3600)),
                ) {
                    (Some(dir), Ok(download)) => {
                        let cache = Arc::new(crate::firmware::FirmwareCache::new(
                            download,
                            &self.config.link.link_url,
                            secret_for_firmware,
                            tenant,
                            Arc::clone(&stores.storage),
                            dir,
                        ));
                        let run = Arc::clone(&cache);
                        tokio::spawn(async move { run.run().await });
                        syncer.with_firmware(cache)
                    }
                    (Some(_), Err(e)) => {
                        warn!(error = %e, "No client for the site's firmware");
                        syncer
                    }
                    (None, _) => syncer,
                };
                let syncer = Arc::new(syncer);
                let run = Arc::clone(&syncer);
                tokio::spawn(async move { run.run().await });
                *slot = Some(syncer);
            }
            Err(e) => warn!(error = %e, "The sync could not start"),
        }
    }

    /// Enroll if the state directory holds no certificate, then heartbeat
    /// forever. An enrollment that fails is retried; the token is the
    /// cloud's to refuse.
    pub async fn run(&self) {
        loop {
            let enrolled = self
                .inner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .enrollment
                .is_some();
            if !enrolled {
                match self.enroll().await {
                    Ok(()) => {}
                    Err(e) => {
                        warn!(error = %e, "Enrollment failed; trying again in 30 s");
                        self.note_error(e.to_string());
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        continue;
                    }
                }
            }
            self.start_sync();
            self.start_records_up();
            let interval = match self.heartbeat().await {
                Ok(interval) => interval,
                Err(e) => {
                    warn!(error = %e, "Heartbeat failed");
                    self.note_error(e.to_string());
                    self.inner
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .interval
                }
            };
            tokio::time::sleep(interval).await;
        }
    }

    fn note_error(&self, error: String) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_error = Some(error);
    }

    fn token(&self) -> Result<String> {
        if let Some(t) = &self.config.link.enrollment_token {
            let t = t.trim();
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
        if let Some(path) = &self.config.link.enrollment_token_file {
            let t = std::fs::read_to_string(path)
                .with_context(|| format!("reading the enrollment token from {path}"))?;
            let t = t.trim();
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
        anyhow::bail!(
            "not enrolled and no enrollment token: set link.enrollment_token, \
             link.enrollment_token_file or FCP_LINK_ENROLLMENT_TOKEN"
        )
    }

    /// A client for the cloud: the system's roots plus `cloud_ca`, and the
    /// site's certificate when it has one.
    fn client(&self, with_identity: bool) -> Result<reqwest::Client> {
        self.client_with(
            with_identity,
            Duration::from_secs(self.config.link.request_timeout_secs),
        )
    }

    fn client_with(&self, with_identity: bool, timeout: Duration) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder().timeout(timeout).use_rustls_tls();
        if let Some(ca) = &self.config.link.cloud_ca {
            let pem = std::fs::read(ca).with_context(|| format!("reading link.cloud_ca {ca}"))?;
            for cert in reqwest::Certificate::from_pem_bundle(&pem)
                .with_context(|| format!("parsing link.cloud_ca {ca}"))?
            {
                builder = builder.add_root_certificate(cert);
            }
        }
        if with_identity {
            let mut pem = std::fs::read(self.state_dir.join("link.crt"))
                .context("reading link.crt from the state directory")?;
            pem.extend_from_slice(
                &std::fs::read(self.state_dir.join("link.key"))
                    .context("reading link.key from the state directory")?,
            );
            let identity =
                reqwest::Identity::from_pem(&pem).context("the site's certificate and key")?;
            builder = builder.identity(identity);
        }
        builder.build().context("building the HTTP client")
    }

    async fn enroll(&self) -> Result<()> {
        let token = self.token()?;
        let key = rcgen::KeyPair::generate().context("generating the site's key")?;
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new())
            .context("certificate request parameters")?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, &self.config.service.node_id);
        let csr = params
            .serialize_request(&key)
            .context("the certificate signing request")?
            .pem()
            .context("the certificate signing request as PEM")?;
        let url = format!(
            "{}/link/v1/enroll",
            self.config.link.cloud_url.trim_end_matches('/')
        );
        info!(url = %url, "Enrolling");
        let response = self
            .client(false)?
            .post(&url)
            .json(&serde_json::json!({
                "token": token,
                "csr_pem": csr,
                "link_version": env!("CARGO_PKG_VERSION"),
            }))
            .send()
            .await
            .context("reaching the cloud's enroll listener")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("the cloud refused the enrollment: {status} {body}");
        }
        let answer: EnrollResponse = response
            .json()
            .await
            .context("reading the enrollment answer")?;
        write_private(
            &self.state_dir.join("link.key"),
            key.serialize_pem().as_bytes(),
        )?;
        std::fs::write(self.state_dir.join("link.crt"), &answer.certificate_pem)?;
        std::fs::write(self.state_dir.join("link-ca.crt"), &answer.ca_pem)?;
        let enrollment = Enrollment {
            site_id: answer.site_id,
            tenant_id: answer.tenant_id,
            name: answer.name,
            secret: answer.secret,
            enrolled_at: secs(SystemTime::now()),
            heartbeat_interval_secs: answer.heartbeat_interval_secs.max(1),
        };
        write_private(
            &self.state_dir.join("site.json"),
            serde_json::to_string_pretty(&enrollment)?.as_bytes(),
        )?;
        info!(
            site_id = %enrollment.site_id,
            tenant_id = %enrollment.tenant_id,
            name = %enrollment.name,
            "Enrolled; the certificate and secret are in the state directory"
        );
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.interval = Duration::from_secs(enrollment.heartbeat_interval_secs);
        inner.enrollment = Some(enrollment);
        inner.last_error = None;
        Ok(())
    }

    /// What the site looks like now: this link, and the cluster's nodes
    /// when the site's Redis is known.
    async fn look(&self) -> SiteHeartbeat {
        let mut nodes = Vec::new();
        if let Some(reader) = &self.reader {
            match reader.list_nodes().await {
                Ok(list) => {
                    for node in list {
                        nodes.push(SiteNode {
                            node_id: node.node_id,
                            node_type: node.node_type,
                            version: node.version,
                            healthy: node.status == fcp_cluster::ClusterNodeStatus::Healthy,
                        });
                    }
                }
                Err(e) => warn!(error = %e, "Could not list the site's nodes"),
            }
        }
        SiteHeartbeat {
            link_version: env!("CARGO_PKG_VERSION").to_string(),
            link_protocol: LINK_PROTOCOL,
            nodes,
            sync: self.syncer().map(|s| s.state()),
            registrations: self.registrations().await,
            services: self.services().await,
            parked: self.parked().await,
            backups: self.backups().await,
        }
    }

    /// The site's own backups, from its database
    /// (`docs/SCHEDULED_BACKUPS.md` B5): a site whose backups stopped is
    /// seen from the cloud.
    async fn backups(&self) -> Vec<fcp_core::backup::BackupStatus> {
        let Some(store) = self
            .stores
            .as_ref()
            .and_then(|s| s.storage.backup_store().cloned())
        else {
            return Vec::new();
        };
        match store.backup_status().await {
            Ok(list) => list,
            Err(e) => {
                warn!(error = %e, "Could not read the site's backups");
                Vec::new()
            }
        }
    }

    /// The calls parked at the site, from its Redis, for the cloud's
    /// board (`docs/PARKING.md` phase 4).
    async fn parked(&self) -> Vec<fcp_core::storage::ClusterParkedCall> {
        let (Some(redis), Some((_, tenant, _))) = (self.redis.as_ref(), self.enrolled()) else {
            return Vec::new();
        };
        let store = fcp_storage_redis::parking::RedisParkingStore::new(redis.clone());
        match store.list(&tenant).await {
            Ok(calls) => calls.iter().map(|c| c.to_cluster()).collect(),
            Err(e) => {
                warn!(error = %e, "Could not read the site's parked calls");
                Vec::new()
            }
        }
    }

    /// Every service at the site, from the beacons in its Redis
    /// (`docs/SERVICES.md`), for the cloud's record.
    async fn services(&self) -> Vec<fcp_core::service_health::ServiceRecord> {
        let Some(redis) = self.redis.as_ref() else {
            return Vec::new();
        };
        match fcp_cluster::ServiceBeacon::read_all(redis).await {
            Ok(records) => records,
            Err(e) => {
                warn!(error = %e, "Could not read the site's services");
                Vec::new()
            }
        }
    }

    /// Who is registered at the site, from its Redis, as the cloud's
    /// pages show it (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
    async fn registrations(&self) -> Option<fcp_core::SiteRegistrations> {
        use fcp_core::storage::RegistrationStore;
        let redis = self.redis.as_ref()?;
        let (_, tenant, _) = self.enrolled()?;
        let store = fcp_storage_redis::registration::RedisRegistrationStore::new(redis.clone());
        let rows = match store.get_all_registrations_in_tenant(&tenant).await {
            Ok(rows) => rows,
            Err(e) => {
                warn!(error = %e, "Could not read the site's registrations");
                return None;
            }
        };
        let mut by_aor: std::collections::BTreeMap<String, (u32, Option<u64>)> =
            std::collections::BTreeMap::new();
        for row in &rows {
            let entry = by_aor.entry(row.aor.clone()).or_insert((0, None));
            entry.0 += 1;
            let since = secs(row.registered_at);
            entry.1 = Some(entry.1.map_or(since, |s| s.min(since)));
        }
        let total_users = by_aor.len() as u32;
        let users = by_aor
            .into_iter()
            .take(fcp_core::SITE_REGISTRATIONS_CAP)
            .map(|(aor, (contacts, since))| fcp_core::SiteRegistration {
                aor,
                contacts,
                since,
            })
            .collect();
        Some(fcp_core::SiteRegistrations {
            users,
            total_users,
            total_contacts: rows.len() as u32,
        })
    }

    /// What the site's call managers must carry to `/push`.
    pub fn push_token(&self) -> Option<&str> {
        self.config.site.push_token.as_deref().map(str::trim)
    }

    /// A site's push, sent up to the cloud (`POST /link/v1/push`): the
    /// cloud's status and answer, as they are.
    pub async fn push(
        &self,
        push: &fcp_core::push::RelayedPush,
    ) -> Result<(u16, serde_json::Value)> {
        let secret = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .enrollment
            .as_ref()
            .map(|e| e.secret.clone())
            .context("not enrolled")?;
        let url = format!(
            "{}/link/v1/push",
            self.config.link.link_url.trim_end_matches('/')
        );
        let response = self
            .client(true)?
            .post(&url)
            .bearer_auth(secret)
            .json(push)
            .send()
            .await
            .context("reaching the cloud's link listener")?;
        let status = response.status().as_u16();
        let body = response
            .json::<serde_json::Value>()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        Ok((status, body))
    }

    async fn heartbeat(&self) -> Result<Duration> {
        let secret = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .enrollment
            .as_ref()
            .map(|e| e.secret.clone())
            .context("not enrolled")?;
        let beat = self.look().await;
        let url = format!(
            "{}/link/v1/heartbeat",
            self.config.link.link_url.trim_end_matches('/')
        );
        let response = self
            .client(true)?
            .post(&url)
            .bearer_auth(secret)
            .json(&beat)
            .send()
            .await
            .context("reaching the cloud's link listener")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("the cloud refused the heartbeat: {status} {body}");
        }
        let answer: HeartbeatResponse = response.json().await.context("reading the answer")?;
        let interval = Duration::from_secs(answer.heartbeat_interval_secs.max(1));
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.last_ok = Some(SystemTime::now());
        inner.last_error = None;
        inner.sent += 1;
        inner.interval = interval;
        Ok(interval)
    }
}

fn load_enrollment(state_dir: &Path) -> Result<Option<Enrollment>> {
    let path = state_dir.join("site.json");
    if !path.is_file() || !state_dir.join("link.crt").is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let enrollment: Enrollment =
        serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(enrollment))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
