//! The subject request worker (`docs/COMPLIANCE_FOLLOWUPS.md` §6.2): one
//! admin replica at a time, under the lock `fcp:subject-requests`, runs the
//! requests tenants file, removes bundles past their time, and announces
//! each request it finishes.

use crate::config::{AdminConfig, ComplianceConfig};
use fcp_core::subject_erasure::{DeadLetterStore, MediaRemover};
use fcp_core::subject_request::{
    ExportClass, ExportSettings, LocalMedia, MediaFetcher, SubjectRow, WorkerAccess,
};
use fcp_core::{DynStorage, TenantId};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

/// The lock that keeps the worker to one replica.
pub const LOCK: &str = "fcp:subject-requests";

/// Files for a bundle: the directories this host mounts, and conference
/// recordings through the conference server, which fetches them from the
/// node that holds them.
pub struct AdminMedia {
    local: LocalMedia,
    conference: Option<(String, Option<String>)>,
    http: reqwest::Client,
    /// Files a site holds, fetched and removed through its link
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
    sites: Option<Arc<crate::site_files::SiteRequests>>,
}

impl AdminMedia {
    pub fn new(config: &AdminConfig) -> Self {
        let roots = [
            config.call_recordings_dir.as_deref(),
            config.voicemail_recordings_dir.as_deref(),
            config.voicemail_greetings_dir.as_deref(),
            config.profile_pictures_dir.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(String::from)
        .chain(config.retention.extra_file_roots.iter().cloned())
        .map(PathBuf::from)
        .collect();
        Self {
            local: LocalMedia {
                roots,
                profile_pictures_dir: config.profile_pictures_dir.as_deref().map(PathBuf::from),
            },
            conference: config
                .conference
                .server_url
                .clone()
                .map(|url| (url, config.conference.auth_token.clone())),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            sites: None,
        }
    }

    /// Reach files sites hold through their links.
    pub fn with_sites(mut self, sites: Option<Arc<crate::site_files::SiteRequests>>) -> Self {
        self.sites = sites;
        self
    }

    /// The site a row's file is at, when it is at one.
    fn site_of(row: &SubjectRow) -> Option<fcp_core::SiteId> {
        row.data["site_id"]
            .as_str()
            .and_then(|s| s.parse::<fcp_core::SiteId>().ok())
    }

    /// The class name a site's link knows the file by.
    fn class_name(class: ExportClass) -> &'static str {
        match class {
            ExportClass::CallRecordings => "call_recording",
            ExportClass::ConferenceRecordings => "conference_recording",
            ExportClass::VoicemailMessages => "voicemail",
            _ => "file",
        }
    }

    async fn fetch_from_conference_server(
        &self,
        tenant: &TenantId,
        row: &SubjectRow,
        dest: &Path,
    ) -> Result<u64, String> {
        use tokio::io::AsyncWriteExt;
        let (base, token) = self
            .conference
            .as_ref()
            .ok_or("no conference server configured on this host")?;
        let path = row.file.as_deref().ok_or("no file")?;
        let name = path.rsplit('/').next().unwrap_or(path);
        let node = row.data["node_id"].as_str().unwrap_or_default();
        let url = format!(
            "{}/recordings/{}?node={}",
            base.trim_end_matches('/'),
            urlencoding::encode(name),
            urlencoding::encode(node)
        );
        let mut request = self.http.get(&url).header(
            fcp_api_core::auth::PLATFORM_TENANT_HEADER,
            tenant.to_string(),
        );
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let mut response = request.send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "the conference server answered {}",
                response.status()
            ));
        }
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| e.to_string())?;
        let mut written = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            written += chunk.len() as u64;
        }
        file.flush().await.map_err(|e| e.to_string())?;
        Ok(written)
    }
}

#[async_trait::async_trait]
impl MediaFetcher for AdminMedia {
    async fn fetch(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        row: &SubjectRow,
        dest: &Path,
    ) -> Result<u64, String> {
        // A file a site holds comes through the site's link, whatever its
        // class; the report says so when the site cannot be reached.
        if let Some(site) = Self::site_of(row) {
            let sites = self
                .sites
                .as_ref()
                .ok_or_else(|| format!("held at site {site}, and no link runs here"))?;
            let path = row.file.as_deref().ok_or("no file")?;
            let bytes = sites
                .fetch(site, *tenant, Self::class_name(class), path)
                .await
                .map_err(|e| format!("site {site}: {e}"))?;
            let size = bytes.len() as u64;
            tokio::fs::write(dest, bytes)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(size);
        }
        if class == ExportClass::ConferenceRecordings {
            return self.fetch_from_conference_server(tenant, row, dest).await;
        }
        self.local.fetch(tenant, class, row, dest).await
    }
}

#[async_trait::async_trait]
impl MediaRemover for AdminMedia {
    async fn remove(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        row: &SubjectRow,
    ) -> Result<(), String> {
        // A file a site holds is the site's to remove, asked through its
        // link; done once the request is queued.
        if let Some(site) = Self::site_of(row) {
            let sites = self
                .sites
                .as_ref()
                .ok_or_else(|| format!("held at site {site}, and no link runs here"))?;
            let path = row.file.as_deref().ok_or("no file")?;
            return sites
                .queue_delete(site, Some(*tenant), Self::class_name(class), path)
                .await
                .map_err(|e| format!("site {site}: {e}"));
        }
        if class != ExportClass::ConferenceRecordings {
            return self.local.remove(tenant, class, row).await;
        }
        // Through the conference server, which deletes the file on the node
        // holding it.
        let (base, token) = self
            .conference
            .as_ref()
            .ok_or("no conference server configured on this host")?;
        let path = row.file.as_deref().ok_or("no file")?;
        let name = path.rsplit('/').next().unwrap_or(path);
        let node = row.data["node_id"].as_str().unwrap_or_default();
        let url = format!(
            "{}/recordings/{}?node={}",
            base.trim_end_matches('/'),
            urlencoding::encode(name),
            urlencoding::encode(node)
        );
        let mut request = self.http.delete(&url).header(
            fcp_api_core::auth::PLATFORM_TENANT_HEADER,
            tenant.to_string(),
        );
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|e| e.to_string())?;
        match response.status() {
            s if s.is_success() || s == reqwest::StatusCode::NOT_FOUND => Ok(()),
            s => Err(format!("the conference server answered {s}")),
        }
    }
}

/// The CDR stream's dead letters, when this host is given Redis.
async fn dead_letters(
    config: &AdminConfig,
) -> Option<fcp_storage_redis::cdr_stream::RedisCdrStream> {
    let url = config.cluster.redis_url.clone()?;
    match fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
        url,
        key_prefix: config.cluster.key_prefix.clone(),
    })
    .await
    {
        Ok(storage) => Some(fcp_storage_redis::cdr_stream::RedisCdrStream::new(
            storage,
            format!("fcp-admin-{}", config.service.node_id),
            Duration::from_secs(24 * 3600),
        )),
        Err(e) => {
            warn!(error = %e, "Subject requests: Redis unreachable; dead letters will not be searched");
            None
        }
    }
}

/// Run the worker until the process ends.
pub fn spawn(
    storage: Arc<DynStorage>,
    config: &AdminConfig,
    api_state: fcp_api_core::ApiState,
    sites: Option<Arc<crate::site_files::SiteRequests>>,
) {
    let settings = settings(&config.compliance);
    let media = AdminMedia::new(config).with_sites(sites);
    let every = Duration::from_secs(config.compliance.poll_secs.max(5));
    let config = config.clone();
    tokio::spawn(async move {
        let dead = dead_letters(&config).await;
        let access = WorkerAccess {
            media: &media,
            remover: &media,
            dead_letters: dead.as_ref().map(|d| d as &dyn DeadLetterStore),
        };
        let mut interval = tokio::time::interval(every);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let lock = {
                use fcp_core::storage::CdrStore;
                match storage.try_maintenance_lock(LOCK).await {
                    Ok(Some(lock)) => lock,
                    Ok(None) => continue,
                    Err(e) => {
                        warn!(error = %e, "Subject requests: could not take the lock");
                        continue;
                    }
                }
            };
            match fcp_core::subject_request::run_subject_requests(&storage, access, &settings).await
            {
                Ok(finished) => {
                    for request in finished {
                        let overdue = request
                            .completed_at
                            .is_some_and(|done| done > request.due_at);
                        api_state
                            .publish_event(fcp_events::PlatformEvent::Compliance(
                                fcp_events::ComplianceEvent::SubjectRequestCompleted {
                                    request_id: request.id.to_string(),
                                    tenant_id: request.tenant_id,
                                    kind: request.kind.as_str().to_string(),
                                    status: request.status.as_str().to_string(),
                                    overdue,
                                    timestamp: request.completed_at.unwrap_or_else(SystemTime::now),
                                },
                            ))
                            .await;
                    }
                }
                Err(e) => warn!(error = %e, "Subject requests: the round failed"),
            }
            match fcp_core::subject_request::remove_expired_bundles(&storage, SystemTime::now())
                .await
            {
                Ok(0) => {}
                Ok(n) => info!(removed = n, "Subject requests: expired bundles removed"),
                Err(e) => warn!(error = %e, "Subject requests: expired bundles not removed"),
            }
            lock.release().await;
        }
    });
}

pub fn settings(config: &ComplianceConfig) -> ExportSettings {
    ExportSettings {
        exports_dir: PathBuf::from(&config.exports_dir),
        bundle_ttl: Duration::from_secs(u64::from(config.export_ttl_days.max(1)) * 86_400),
    }
}
