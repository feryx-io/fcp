//! What the cloud asks of the site (`docs/HYBRID_CONTROL_PLANE.md` §3.3):
//! collected on a long poll, since the site opens no port, and answered
//! on the link's own connection — a file the cloud wants sent up, a file
//! the cloud wants gone. Only what lies under `[site] file_roots` is
//! touched.

use anyhow::{Context, Result};
use fcp_core::site::{SiteRequest, SiteRequestKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, info, warn};

/// The header the answer carries its outcome in, as the cloud reads it.
const OUTCOME_HEADER: &str = "x-fcp-outcome";
const DETAIL_HEADER: &str = "x-fcp-detail";

pub struct Requester {
    client: reqwest::Client,
    link_url: String,
    secret: String,
    roots: Vec<PathBuf>,
    answered: AtomicU64,
    error: Mutex<Option<String>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RequestsStatus {
    pub answered: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Requester {
    pub fn new(
        client: reqwest::Client,
        link_url: &str,
        secret: String,
        roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            client,
            link_url: link_url.trim_end_matches('/').to_string(),
            secret,
            roots,
            answered: AtomicU64::new(0),
            error: Mutex::new(None),
        }
    }

    pub fn status(&self) -> RequestsStatus {
        RequestsStatus {
            answered: self.answered.load(Ordering::Relaxed),
            error: self.error.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }

    pub async fn run(&self) {
        loop {
            match self.step().await {
                Ok(()) => {
                    *self.error.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
                Err(e) => {
                    let text = format!("{e:#}");
                    warn!(error = %text, "Collecting the cloud's requests failed; trying again");
                    *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn step(&self) -> Result<()> {
        let response = self
            .client
            .get(format!("{}/link/v1/requests?wait_secs=25", self.link_url))
            .bearer_auth(&self.secret)
            .send()
            .await
            .context("reaching the cloud for its requests")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("the cloud refused the poll: {status} {body}");
        }
        let requests: Vec<SiteRequest> = response
            .json()
            .await
            .context("reading the cloud's requests")?;
        for request in requests {
            let (outcome, body) = self.execute(&request).await;
            let mut post = self
                .client
                .post(format!("{}/link/v1/requests/{}", self.link_url, request.id))
                .bearer_auth(&self.secret);
            post = match &outcome {
                Ok(()) => post.header(OUTCOME_HEADER, "ok"),
                Err(detail) => post
                    .header(OUTCOME_HEADER, "failed")
                    .header(DETAIL_HEADER, detail.replace(['\r', '\n'], " ")),
            };
            if let Some(bytes) = body {
                post = post.body(bytes);
            }
            let answered = post
                .send()
                .await
                .with_context(|| format!("answering request {}", request.id))?;
            if !answered.status().is_success() {
                warn!(request = %request.id, status = %answered.status(), "The cloud did not take the answer");
            } else {
                self.answered.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(())
    }

    /// The file a request names, when it lies under a root: an absolute
    /// path under one, or a relative path found under one of them.
    fn locate(&self, path: &str) -> Result<PathBuf, String> {
        let given = Path::new(path.strip_prefix("file://").unwrap_or(path));
        if given
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("the path climbs out of its directory".to_string());
        }
        if given.is_absolute() {
            return if fcp_core::compliance::under_file_root(given, &self.roots) {
                Ok(given.to_path_buf())
            } else {
                Err("not under a directory this link may reach ([site] file_roots)".to_string())
            };
        }
        for root in &self.roots {
            let candidate = root.join(given);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        match self.roots.first() {
            Some(root) => Ok(root.join(given)),
            None => Err("no [site] file_roots to look under".to_string()),
        }
    }

    async fn execute(&self, request: &SiteRequest) -> (Result<(), String>, Option<Vec<u8>>) {
        match &request.kind {
            SiteRequestKind::FetchFile { class, path } => match self.locate(path) {
                Ok(file) => match tokio::fs::read(&file).await {
                    Ok(bytes) => {
                        info!(request = %request.id, class, path = %file.display(), bytes = bytes.len(), "Sending a file up at the cloud's request");
                        (Ok(()), Some(bytes))
                    }
                    Err(e) => (Err(format!("{}: {e}", file.display())), None),
                },
                Err(e) => (Err(e), None),
            },
            SiteRequestKind::DeleteFile { class, path } => match self.locate(path) {
                Ok(file) => match tokio::fs::remove_file(&file).await {
                    Ok(()) => {
                        info!(request = %request.id, class, path = %file.display(), "Removed a file at the cloud's request");
                        (Ok(()), None)
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        debug!(request = %request.id, path = %file.display(), "A file the cloud asked to remove was gone already");
                        (Ok(()), None)
                    }
                    Err(e) => (Err(format!("{}: {e}", file.display())), None),
                },
                Err(e) => (Err(e), None),
            },
        }
    }
}
