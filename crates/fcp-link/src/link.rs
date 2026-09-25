//! The link itself: enroll once, then heartbeat.

use anyhow::{Context, Result};
use fcp_core::{SiteHeartbeat, SiteNode};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

use crate::config::LinkFileConfig;

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
        let reader = match &config.site.redis_url {
            Some(url) => {
                let storage =
                    fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
                        url: url.clone(),
                        key_prefix: "fcp:".to_string(),
                    })
                    .await
                    .context("connecting to the site's Redis")?;
                Some(fcp_cluster::ClusterReader::new(
                    storage,
                    config.site.heartbeat_ttl_secs,
                ))
            }
            None => None,
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
        })
    }

    pub fn status(&self) -> LinkStatus {
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
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.config.link.request_timeout_secs))
            .use_rustls_tls();
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
        }
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
