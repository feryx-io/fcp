//! The link: what a site's `fcp-link` enrolls to and heartbeats at
//! (`docs/HYBRID_CONTROL_PLANE.md` §3, phase 1).
//!
//! Two listeners. The **enroll** listener serves TLS with this process's
//! own certificate; a link connects once, presents the one-time token a
//! tenant administrator minted at `POST /v1/sites`, and a certificate
//! signing request, and receives a certificate signed by the **link CA**
//! this process keeps in `[link] ca_dir` (made on first start), the CA's
//! certificate, and the secret it will heartbeat with. The **link**
//! listener requires a client certificate from that CA: a connection
//! without one never reaches a handler. On it, a heartbeat is the site's
//! secret in `Authorization: Bearer` and what the link found at the site;
//! the cloud records it, and says `system.site_link_changed` when a
//! site's link comes after none, or, from the sweep, when none came for
//! `heartbeat_stale_secs`.

use anyhow::{Context, Result};
use axum::{
    extract::Extension,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use axum_server::tls_rustls::RustlsConfig;
use base64::Engine;
use fcp_core::{DynStorage, Site, SiteHeartbeat, SiteStatus, SiteStore};
use fcp_events::{EventPublisher, PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

use crate::config::LinkConfig;

/// The link protocol this cloud speaks.
pub const LINK_PROTOCOL: u32 = 1;

/// The link CA: a certificate and key on disk, made once, that every
/// site's certificate is signed by.
pub struct LinkCa {
    /// The CA certificate as sites are given it.
    pub cert_pem: String,
    issuer: rcgen::Certificate,
    key: rcgen::KeyPair,
}

impl LinkCa {
    /// `link-ca.crt` and `link-ca.key` in `dir`, made when absent.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating the link CA directory {}", dir.display()))?;
        let cert_path = dir.join("link-ca.crt");
        let key_path = dir.join("link-ca.key");
        if cert_path.is_file() && key_path.is_file() {
            let cert_pem = std::fs::read_to_string(&cert_path)?;
            let key_pem = std::fs::read_to_string(&key_path)?;
            let key = rcgen::KeyPair::from_pem(&key_pem).context("reading link-ca.key")?;
            let params = rcgen::CertificateParams::from_ca_cert_pem(&cert_pem)
                .context("reading link-ca.crt")?;
            // The same subject and key as the file sites trust; only the
            // signing identity is needed from here.
            let issuer = params
                .self_signed(&key)
                .context("rebuilding the link CA issuer")?;
            info!(dir = %dir.display(), "Link CA loaded");
            return Ok(Self {
                cert_pem,
                issuer,
                key,
            });
        }
        let key = rcgen::KeyPair::generate().context("generating the link CA key")?;
        let mut params = rcgen::CertificateParams::default();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "FCP link CA");
        params.not_before = rcgen::date_time_ymd(2026, 1, 1);
        params.not_after = rcgen::date_time_ymd(2046, 1, 1);
        let issuer = params.self_signed(&key).context("signing the link CA")?;
        let cert_pem = issuer.pem();
        std::fs::write(&cert_path, &cert_pem)?;
        std::fs::write(&key_path, key.serialize_pem())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }
        info!(dir = %dir.display(), "Link CA made");
        Ok(Self {
            cert_pem,
            issuer,
            key,
        })
    }

    /// Sign a site's request: the certificate names the site by id,
    /// lasts `days`, and carries a fresh serial. Returns the PEM, the
    /// serial (hex) and when it ends.
    pub fn sign(
        &self,
        csr_pem: &str,
        site_id: &str,
        days: u32,
    ) -> Result<(String, String, SystemTime)> {
        let mut csr = rcgen::CertificateSigningRequestParams::from_pem(csr_pem)
            .context("reading the certificate signing request")?;
        csr.params.distinguished_name = rcgen::DistinguishedName::new();
        csr.params
            .distinguished_name
            .push(rcgen::DnType::CommonName, site_id);
        csr.params.subject_alt_names = Vec::new();
        csr.params.is_ca = rcgen::IsCa::ExplicitNoCa;
        csr.params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        csr.params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        let serial: [u8; 16] = rand::random();
        csr.params.serial_number = Some(rcgen::SerialNumber::from_slice(&serial));
        let now = SystemTime::now();
        let ends = now + Duration::from_secs(u64::from(days) * 86_400);
        csr.params.not_before = now.into();
        csr.params.not_after = ends.into();
        let cert = csr
            .signed_by(&self.issuer, &self.key)
            .context("signing the site's certificate")?;
        let serial_hex: String = serial.iter().map(|b| format!("{b:02x}")).collect();
        Ok((cert.pem(), serial_hex, ends))
    }
}

/// What the link handlers share.
#[derive(Clone)]
pub struct LinkState {
    pub storage: Arc<DynStorage>,
    pub ca: Arc<LinkCa>,
    pub events: Arc<dyn EventPublisher>,
    pub config: LinkConfig,
}

/// `POST /link/v1/enroll`.
#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    /// The one-time token from `POST /v1/sites`.
    pub token: String,
    /// The link's certificate signing request (PEM).
    pub csr_pem: String,
    #[serde(default)]
    pub link_version: String,
}

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub site_id: String,
    pub tenant_id: String,
    pub name: String,
    /// The site's certificate, signed by the link CA.
    pub certificate_pem: String,
    /// The link CA, for the link to verify nothing yet: kept for later
    /// phases, and so the site's operator can see what signed them.
    pub ca_pem: String,
    /// The secret every heartbeat carries. Shown this once.
    pub secret: String,
    pub heartbeat_interval_secs: u64,
    pub protocol: u32,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatResponse {
    pub ok: bool,
    pub heartbeat_interval_secs: u64,
    pub protocol: u32,
}

/// One error shape for the link: a status and a sentence.
struct LinkError(StatusCode, String);

impl IntoResponse for LinkError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for LinkError {
    fn from(e: anyhow::Error) -> Self {
        warn!(error = %e, "Link request failed");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the cloud could not complete the request".to_string(),
        )
    }
}

fn sha256_hex(input: &str) -> String {
    Sha256::digest(input.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn secs_between(from: Option<SystemTime>, to: SystemTime) -> Option<u64> {
    from.and_then(|f| to.duration_since(f).ok())
        .map(|d| d.as_secs())
}

async fn enroll(
    Extension(state): Extension<LinkState>,
    Json(req): Json<EnrollRequest>,
) -> Result<Json<EnrollResponse>, LinkError> {
    let token = req.token.trim();
    if token.is_empty() {
        return Err(LinkError(
            StatusCode::BAD_REQUEST,
            "an enrollment token is required".to_string(),
        ));
    }
    let Some(spent) = state
        .storage
        .consume_enrollment_token(&sha256_hex(token))
        .await?
    else {
        return Err(LinkError(
            StatusCode::FORBIDDEN,
            "the enrollment token is unknown, spent or expired".to_string(),
        ));
    };
    let Some(mut site) = state.storage.get_site(&spent.site_id).await? else {
        return Err(LinkError(
            StatusCode::FORBIDDEN,
            "the token's site no longer exists".to_string(),
        ));
    };
    if site.status == SiteStatus::Disabled {
        return Err(LinkError(
            StatusCode::FORBIDDEN,
            "the site is disabled; enable it in the cloud first".to_string(),
        ));
    }
    let (certificate_pem, serial, ends) = state
        .ca
        .sign(&req.csr_pem, &site.id.to_string(), state.config.certificate_days)
        .map_err(|e| {
            warn!(site_id = %site.id, error = %e, "A site's certificate signing request was refused");
            LinkError(
                StatusCode::BAD_REQUEST,
                "the certificate signing request could not be signed".to_string(),
            )
        })?;
    let secret_bytes: [u8; 32] = rand::random();
    let secret = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret_bytes);
    let now = SystemTime::now();
    site.status = SiteStatus::Enrolled;
    site.enrolled_at = Some(now);
    site.certificate_serial = Some(serial);
    site.certificate_expires_at = Some(ends);
    site.secret_hash = Some(sha256_hex(&secret));
    site.updated_at = now;
    state.storage.put_site(site.clone()).await?;
    info!(
        site_id = %site.id,
        tenant_id = %site.tenant_id,
        name = %site.name,
        link_version = %req.link_version,
        "A site enrolled"
    );
    Ok(Json(EnrollResponse {
        site_id: site.id.to_string(),
        tenant_id: site.tenant_id.to_string(),
        name: site.name,
        certificate_pem,
        ca_pem: state.ca.cert_pem.clone(),
        secret,
        heartbeat_interval_secs: state.config.heartbeat_interval_secs,
        protocol: LINK_PROTOCOL,
    }))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

async fn heartbeat(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    Json(beat): Json<SiteHeartbeat>,
) -> Result<Json<HeartbeatResponse>, LinkError> {
    let Some(secret) = bearer(&headers) else {
        return Err(LinkError(
            StatusCode::UNAUTHORIZED,
            "a heartbeat carries the site's secret as a bearer token".to_string(),
        ));
    };
    let Some(mut site) = state
        .storage
        .get_site_by_secret_hash(&sha256_hex(secret))
        .await?
    else {
        return Err(LinkError(
            StatusCode::UNAUTHORIZED,
            "no site has this secret".to_string(),
        ));
    };
    if site.status == SiteStatus::Disabled {
        return Err(LinkError(
            StatusCode::FORBIDDEN,
            "the site is disabled".to_string(),
        ));
    }
    let now = SystemTime::now();
    let came_up = !site.link_up;
    let lasted = if came_up {
        secs_between(site.link_changed_at, now)
    } else {
        None
    };
    site.last_seen_at = Some(now);
    site.last_heartbeat = Some(beat);
    site.link_up = true;
    if came_up {
        site.link_changed_at = Some(now);
    }
    site.updated_at = now;
    state.storage.put_site(site.clone()).await?;
    if came_up {
        announce(&state, &site, true, lasted).await;
    }
    Ok(Json(HeartbeatResponse {
        ok: true,
        heartbeat_interval_secs: state.config.heartbeat_interval_secs,
        protocol: LINK_PROTOCOL,
    }))
}

async fn announce(state: &LinkState, site: &Site, up: bool, lasted_secs: Option<u64>) {
    if up {
        info!(site_id = %site.id, name = %site.name, "A site's link is up");
    } else {
        warn!(site_id = %site.id, name = %site.name, "A site's link is down: no heartbeat");
    }
    let event = PlatformEvent::System(SystemEvent::SiteLinkChanged {
        site_id: site.id.to_string(),
        tenant_id: site.tenant_id,
        name: site.name.clone(),
        up,
        lasted_secs,
        timestamp: SystemTime::now(),
    });
    if let Err(e) = state.events.publish(event).await {
        warn!(error = %e, "Failed to publish SiteLinkChanged");
    }
}

/// Mark the sites whose link went quiet, every third of the stale time.
pub fn spawn_sweep(state: LinkState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let stale = Duration::from_secs(state.config.heartbeat_stale_secs);
        let mut tick = tokio::time::interval(stale / 3);
        loop {
            tick.tick().await;
            let sites = match state.storage.list_all_sites().await {
                Ok(sites) => sites,
                Err(e) => {
                    warn!(error = %e, "The site sweep could not list sites");
                    continue;
                }
            };
            let now = SystemTime::now();
            for mut site in sites {
                let quiet = site
                    .last_seen_at
                    .and_then(|seen| now.duration_since(seen).ok())
                    .is_none_or(|since| since > stale);
                if site.link_up && quiet {
                    let lasted = secs_between(site.link_changed_at, now);
                    site.link_up = false;
                    site.link_changed_at = Some(now);
                    site.updated_at = now;
                    if let Err(e) = state.storage.put_site(site.clone()).await {
                        warn!(site_id = %site.id, error = %e, "The site sweep could not save a site");
                        continue;
                    }
                    announce(&state, &site, false, lasted).await;
                }
            }
        }
    })
}

/// The enroll listener's routes.
pub fn enroll_router(state: LinkState) -> Router {
    Router::new()
        .route("/link/v1/enroll", post(enroll))
        .layer(Extension(state))
}

/// The link listener's routes.
pub fn link_router(state: LinkState) -> Router {
    Router::new()
        .route("/link/v1/heartbeat", post(heartbeat))
        .layer(Extension(state))
}

fn read_pem_certs(path: &Path) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let pem = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let certs = rustls_pemfile::certs(&mut pem.as_slice())
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("parsing certificates in {}", path.display()))?;
    anyhow::ensure!(!certs.is_empty(), "{} holds no certificate", path.display());
    Ok(certs)
}

fn read_pem_key(path: &Path) -> Result<rustls::pki_types::PrivateKeyDer<'static>> {
    let pem = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    rustls_pemfile::private_key(&mut pem.as_slice())
        .with_context(|| format!("parsing the key in {}", path.display()))?
        .with_context(|| format!("{} holds no private key", path.display()))
}

/// TLS for the enroll listener: this process's certificate, no client
/// certificate asked for.
pub fn enroll_tls(config: &LinkConfig) -> Result<RustlsConfig> {
    let certs = read_pem_certs(Path::new(config.cert.as_deref().unwrap_or_default()))?;
    let key = read_pem_key(Path::new(config.key.as_deref().unwrap_or_default()))?;
    let server = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("the enroll listener's certificate")?;
    Ok(RustlsConfig::from_config(Arc::new(server)))
}

/// TLS for the link listener: this process's certificate, and a client
/// certificate from the link CA required of every connection.
pub fn link_tls(config: &LinkConfig, ca: &LinkCa) -> Result<RustlsConfig> {
    let certs = read_pem_certs(Path::new(config.cert.as_deref().unwrap_or_default()))?;
    let key = read_pem_key(Path::new(config.key.as_deref().unwrap_or_default()))?;
    let mut roots = rustls::RootCertStore::empty();
    let ca_certs = rustls_pemfile::certs(&mut ca.cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("parsing the link CA")?;
    for cert in ca_certs {
        roots
            .add(cert)
            .context("adding the link CA to the trust store")?;
    }
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .context("the link listener's client verifier")?;
    let server = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .context("the link listener's certificate")?;
    Ok(RustlsConfig::from_config(Arc::new(server)))
}

/// Start both listeners and the sweep. Returns when either listener ends.
pub async fn serve(state: LinkState) -> Result<()> {
    let enroll_tls = enroll_tls(&state.config)?;
    let link_tls = link_tls(&state.config, &state.ca)?;
    let enroll_bind = state.config.enroll_bind;
    let link_bind = state.config.bind;
    let _sweep = spawn_sweep(state.clone());
    info!(
        enroll = %enroll_bind,
        link = %link_bind,
        ca_dir = %state.config.ca_dir,
        "Link listeners starting: enroll over TLS, heartbeats under mutual TLS"
    );
    let enroll_app = enroll_router(state.clone());
    let link_app = link_router(state);
    tokio::select! {
        r = axum_server::bind_rustls(enroll_bind, enroll_tls).serve(enroll_app.into_make_service()) => {
            r.context("the enroll listener")?;
        }
        r = axum_server::bind_rustls(link_bind, link_tls).serve(link_app.into_make_service()) => {
            r.context("the link listener")?;
        }
    }
    Ok(())
}

/// Where the link CA is, for the process's own use.
pub fn ca_dir(config: &LinkConfig) -> PathBuf {
    PathBuf::from(&config.ca_dir)
}

/// rustls needs a crypto provider installed before its first
/// configuration; once is enough, and a second install is refused
/// harmlessly.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
