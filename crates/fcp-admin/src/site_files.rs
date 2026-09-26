//! What the cloud asks of a site, and the site's answers
//! (`docs/HYBRID_CONTROL_PLANE.md` §3.3). A site opens no port, so a
//! request — the bytes of a file the site holds, or its removal — is
//! written down here, collected by the site's link on its long poll, and
//! answered by the link on its own connection. A route that needs the
//! bytes waits for that answer; a deletion is done once it is queued.

use anyhow::Result;
use fcp_core::compliance::SiteFileRemover;
use fcp_core::site::{SiteRequest, SiteRequestKind, SiteRequestOutcome};
use fcp_core::{DynStorage, SiteId, SiteRequestId, SiteStore, TenantId};
use fcp_events::{EventPublisher, PlatformEvent};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::sync::{oneshot, Notify};
use tracing::{debug, info, warn};

/// How long a route waits for a site to send a file up.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// The routes waiting for a fetch to be answered, by request id.
type FetchWaiters = Mutex<HashMap<String, oneshot::Sender<Result<Vec<u8>, String>>>>;

pub struct SiteRequests {
    storage: Arc<DynStorage>,
    waiters: FetchWaiters,
    /// Wakes a site's long poll when something is queued for it.
    wakers: Mutex<HashMap<SiteId, Arc<Notify>>>,
    fetch_timeout: Duration,
}

impl std::fmt::Debug for SiteRequests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let waiting = self.waiters.lock().map(|w| w.len()).unwrap_or_default();
        f.debug_struct("SiteRequests")
            .field("waiting", &waiting)
            .finish()
    }
}

impl SiteRequests {
    pub fn new(storage: Arc<DynStorage>) -> Self {
        Self {
            storage,
            waiters: Mutex::new(HashMap::new()),
            wakers: Mutex::new(HashMap::new()),
            fetch_timeout: FETCH_TIMEOUT,
        }
    }

    fn waker(&self, site: SiteId) -> Arc<Notify> {
        let mut wakers = self.wakers.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(wakers.entry(site).or_default())
    }

    /// The site, if it is enrolled and its link is up: what a request
    /// needs to be answered at all.
    async fn linked_site(&self, site: SiteId) -> Result<fcp_core::Site, String> {
        let found = self
            .storage
            .get_site(&site)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("site {site} does not exist"))?;
        if !found.link_up {
            return Err(format!("site {}'s link is down", found.name));
        }
        Ok(found)
    }

    /// Queue `request` and wake the site's poll.
    async fn queue(&self, request: SiteRequest) -> Result<(), String> {
        let site = request.site_id;
        self.storage
            .put_site_request(request)
            .await
            .map_err(|e| e.to_string())?;
        self.waker(site).notify_waiters();
        Ok(())
    }

    /// Ask `site` for `path` and wait for the bytes.
    pub async fn fetch(
        &self,
        site: SiteId,
        tenant: TenantId,
        class: &str,
        path: &str,
    ) -> Result<Vec<u8>, String> {
        let found = self.linked_site(site).await?;
        if found.tenant_id != tenant {
            return Err(format!("site {site} is not tenant {tenant}'s"));
        }
        let request = SiteRequest::new(
            site,
            tenant,
            SiteRequestKind::FetchFile {
                class: class.to_string(),
                path: path.to_string(),
            },
        );
        let id = request.id.to_string();
        let (tx, rx) = oneshot::channel();
        self.waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), tx);
        if let Err(e) = self.queue(request.clone()).await {
            self.waiters
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(e);
        }
        debug!(site_id = %site, request = %id, path, "Asked the site for a file");
        match tokio::time::timeout(self.fetch_timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err("the request was dropped".to_string()),
            Err(_) => {
                self.waiters
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                // Nobody waits for it any more: the link is not asked again.
                let mut request = request;
                request.done_at = Some(SystemTime::now());
                request.outcome = Some(SiteRequestOutcome {
                    ok: false,
                    detail: Some("the site did not answer in time".to_string()),
                });
                let _ = self.storage.put_site_request(request).await;
                Err(format!(
                    "site {} did not answer within {} s",
                    found.name,
                    self.fetch_timeout.as_secs()
                ))
            }
        }
    }

    /// Ask `site` to remove `path`; done once queued.
    pub async fn queue_delete(
        &self,
        site: SiteId,
        tenant: Option<TenantId>,
        class: &str,
        path: &str,
    ) -> Result<(), String> {
        let found = self
            .storage
            .get_site(&site)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("site {site} does not exist"))?;
        let request = SiteRequest::new(
            site,
            tenant.unwrap_or(found.tenant_id),
            SiteRequestKind::DeleteFile {
                class: class.to_string(),
                path: path.to_string(),
            },
        );
        self.queue(request).await
    }

    /// The requests waiting for `site`, holding up to `wait` for one.
    /// Each is marked collected; one the link never answers is handed
    /// out again on its next poll, and answered idempotently.
    pub async fn wait_pending(
        &self,
        site: SiteId,
        wait: Duration,
        limit: usize,
    ) -> Result<Vec<SiteRequest>> {
        let deadline = tokio::time::Instant::now() + wait;
        let waker = self.waker(site);
        loop {
            let mut pending = self.storage.pending_site_requests(&site, limit).await?;
            if !pending.is_empty() {
                let now = SystemTime::now();
                for request in &mut pending {
                    request.taken_at = Some(now);
                    self.storage.put_site_request(request.clone()).await?;
                }
                return Ok(pending);
            }
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return Ok(Vec::new());
            }
            let _ = tokio::time::timeout(left.min(Duration::from_secs(5)), waker.notified()).await;
        }
    }

    /// The site answered `id`: the outcome, and the bytes for a fetch.
    /// `Ok(false)` when the request is not `site`'s.
    pub async fn complete(
        &self,
        site: SiteId,
        id: &SiteRequestId,
        outcome: SiteRequestOutcome,
        body: Option<Vec<u8>>,
    ) -> Result<bool> {
        let Some(mut request) = self.storage.get_site_request(id).await? else {
            return Ok(false);
        };
        if request.site_id != site {
            return Ok(false);
        }
        request.done_at = Some(SystemTime::now());
        request.outcome = Some(outcome.clone());
        self.storage.put_site_request(request.clone()).await?;
        let waiter = self
            .waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id.to_string());
        match (&request.kind, waiter) {
            (SiteRequestKind::FetchFile { .. }, Some(tx)) => {
                let answer = if outcome.ok {
                    Ok(body.unwrap_or_default())
                } else {
                    Err(outcome
                        .detail
                        .unwrap_or_else(|| "the site could not send the file".to_string()))
                };
                let _ = tx.send(answer);
            }
            (SiteRequestKind::FetchFile { .. }, None) => {
                debug!(request = %id, "A site answered a fetch nobody waits for any more");
            }
            (SiteRequestKind::DeleteFile { path, .. }, _) => {
                if outcome.ok {
                    info!(site_id = %site, path, "A site removed a file at the cloud's request");
                } else {
                    warn!(site_id = %site, path, detail = ?outcome.detail, "A site could not remove a file at the cloud's request");
                }
            }
        }
        Ok(true)
    }
}

#[async_trait::async_trait]
impl fcp_api_core::site_files::SiteFiles for SiteRequests {
    async fn fetch(
        &self,
        site: SiteId,
        tenant: TenantId,
        class: &str,
        path: &str,
    ) -> Result<Vec<u8>, String> {
        SiteRequests::fetch(self, site, tenant, class, path).await
    }

    async fn remove(
        &self,
        site: SiteId,
        tenant: TenantId,
        class: &str,
        path: &str,
    ) -> Result<(), String> {
        self.queue_delete(site, Some(tenant), class, path).await
    }
}

#[async_trait::async_trait]
impl SiteFileRemover for SiteRequests {
    async fn remove_at_site(
        &self,
        site: SiteId,
        tenant: Option<TenantId>,
        class: &str,
        path: &str,
    ) -> Result<(), String> {
        self.queue_delete(site, tenant, class, path).await
    }
}

/// The events server's `/ingest`, as a publisher: what this admin says
/// about records it stores for a site (`cdr.written`), and the site's own
/// events, go onto the cloud's stream through it, under the site's name.
#[derive(Clone)]
pub struct IngestPublisher {
    client: reqwest::Client,
    /// The `/ingest` URL.
    url: String,
    token: Option<String>,
    node_id: String,
}

impl IngestPublisher {
    pub fn new(url: String, token: Option<String>, node_id: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("an HTTP client"),
            url: url.trim_end_matches('/').to_string(),
            token,
            node_id,
        }
    }

    /// Post `events` as `site`'s, under `node_id` (the site's node).
    pub async fn publish_for_site(
        &self,
        site: SiteId,
        node_id: &str,
        events: &[PlatformEvent],
    ) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut request = self
            .client
            .post(format!("{}/batch", self.url))
            .header(fcp_events::NODE_ID_HEADER, node_id)
            .header(fcp_events::ORIGIN_SITE_HEADER, site.to_string())
            .json(events);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            anyhow::bail!("the events server answered {}", response.status());
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl EventPublisher for IngestPublisher {
    async fn publish(&self, event: PlatformEvent) -> Result<()> {
        let mut request = self
            .client
            .post(&self.url)
            .header(fcp_events::NODE_ID_HEADER, &self.node_id)
            .json(&event);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            anyhow::bail!("the events server answered {}", response.status());
        }
        Ok(())
    }
}
