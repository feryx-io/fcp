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
    routing::{get, post},
    Json, Router,
};
use axum_server::tls_rustls::RustlsConfig;
use base64::Engine;
use fcp_core::storage::TrunkStore;
use fcp_core::{DynStorage, Site, SiteHeartbeat, SiteStatus, SiteStore, SyncStore, TenantId};
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
    pub flows: Arc<dyn fcp_flow::storage::FlowStore>,
    pub ca: Arc<LinkCa>,
    pub events: Arc<dyn EventPublisher>,
    pub config: LinkConfig,
    /// The cloud's processor for the records sites ship up: it rates,
    /// stores and announces them as any node's (`docs/HYBRID_CONTROL_PLANE.md`
    /// §3.3).
    pub processor: Option<Arc<fcp_cdr::Processor>>,
    /// The events server's ingest, which a site's events are posted to
    /// under the site's name; none without `[events] server_url`.
    pub ingest: Option<crate::site_files::IngestPublisher>,
    /// What the cloud asks of sites, and their answers.
    pub requests: Arc<crate::site_files::SiteRequests>,
    /// Firmware images, which a site's link fetches for the targets it
    /// holds (`docs/PROVISIONING.md` §11).
    pub firmware: Option<Arc<dyn fcp_api_core::firmware_files::FirmwareFiles>>,
    /// The call managers, one of which sends a site's pushes with the
    /// publisher's keys (`docs/PUSH.md` §2.4).
    pub pushes: Option<Arc<crate::call_supervision::NodeCallSupervisor>>,
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
    provision_link_trunk(&state, &mut site).await?;
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

/// The site's link trunk (`docs/HYBRID_CONTROL_PLANE.md` §3.6): one
/// tenant trunk per site, the cloud's end, toward `[link] sip_host`, with
/// credentials minted here. Made at the first enrollment and kept across
/// re-enrollments with a fresh password, since the link that held the old
/// How a site's emergency call goes up its link: with the location the
/// site resolved for its caller (a PIDF-LO, which the cloud places the call
/// by) and its ELIN as the caller (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
const LINK_EMERGENCY_LOCATION: [fcp_core::e911::LocationDelivery; 2] = [
    fcp_core::e911::LocationDelivery::PidfLo,
    fcp_core::e911::LocationDelivery::Elin,
];

/// one is gone. Nothing is made when the cloud names no SIP address.
async fn provision_link_trunk(state: &LinkState, site: &mut Site) -> Result<(), LinkError> {
    let Some(sip_host) = state
        .config
        .sip_host
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty())
    else {
        info!(site_id = %site.id, "No [link] sip_host: the site gets no link trunk");
        return Ok(());
    };
    let password_bytes: [u8; 24] = rand::random();
    let password = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(password_bytes);
    let credentials = fcp_core::trunk::TrunkCredentials {
        username: site.id.to_string(),
        password,
        realm: Some(state.config.sip_realm.clone()),
    };
    let existing = match &site.trunk_id {
        Some(id) => state.storage.get_trunk(id).await?,
        None => None,
    };
    let now = SystemTime::now();
    let trunk = match existing {
        Some(mut trunk) => {
            trunk.hosts = vec![sip_host.to_string()];
            trunk.credentials = Some(credentials);
            trunk.emergency_location = LINK_EMERGENCY_LOCATION.to_vec();
            trunk.enabled = true;
            trunk.updated_at = now;
            trunk
        }
        None => {
            let mut trunk = fcp_core::Trunk::new(format!("link-{}", site.name), site.tenant_id);
            trunk.description = Some(format!("The link trunk of site {}", site.name));
            trunk.site_id = Some(site.id);
            trunk.link = Some(fcp_core::LinkEnd::Cloud);
            trunk.hosts = vec![sip_host.to_string()];
            trunk.transport.protocol = fcp_core::transport::TransportType::Tls;
            trunk.transport.port = None;
            trunk.transport.verify_tls = true;
            trunk.credentials = Some(credentials);
            trunk.register = true;
            // Refreshed often enough to keep the site's connection, and any
            // NAT binding on the way, alive between calls.
            trunk.register_expiry = Some(120);
            trunk.direction = fcp_core::TrunkDirection::Bidirectional;
            // After the site's own carriers, which are its first choice.
            trunk.priority = 100;
            trunk.emergency_capable = true;
            trunk.emergency_location = LINK_EMERGENCY_LOCATION.to_vec();
            trunk.failover.health_check_enabled = false;
            trunk.created_at = now;
            trunk.updated_at = now;
            trunk
        }
    };
    if let Err(e) = trunk.validate() {
        warn!(site_id = %site.id, error = %e, "The site's link trunk does not validate");
        return Err(LinkError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the site's link trunk could not be made".to_string(),
        ));
    }
    let trunk_id = trunk.id.clone();
    state.storage.put_trunk(trunk).await?;
    site.trunk_id = Some(trunk_id.clone());
    // The cloud's routing nodes re-read the tenants' trunks on this, so the
    // SBC knows the site before its first REGISTER.
    let event = PlatformEvent::System(SystemEvent::ConfigChanged {
        entity_type: "trunk".to_string(),
        entity_id: trunk_id.to_string(),
        tenant_id: site.tenant_id,
        timestamp: SystemTime::now(),
    });
    if let Err(e) = state.events.publish(event).await {
        warn!(error = %e, "Failed to publish the link trunk's change");
    }
    info!(site_id = %site.id, trunk_id = %trunk_id, sip_host, "The site's link trunk is provisioned");
    Ok(())
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
        .route("/link/v1/snapshot", get(snapshot))
        .route("/link/v1/changes", get(changes))
        .route("/link/v1/cdrs", post(cdrs))
        .route("/link/v1/records", post(records))
        .route("/link/v1/events", post(events))
        .route("/link/v1/requests", get(requests))
        .route("/link/v1/requests/:id", post(request_result))
        .route("/link/v1/firmware/:image", get(firmware_image))
        .route("/link/v1/brand-assets/:sha256", get(brand_asset))
        .route("/link/v1/push", post(push))
        // A file a site sends up is one body; a recording may be large.
        .layer(axum::extract::DefaultBodyLimit::max(SITE_UPLOAD_LIMIT))
        .layer(Extension(state))
}

/// The most a site may send up in one request: a batch, or one file.
pub const SITE_UPLOAD_LIMIT: usize = 512 << 20;

/// The header a link's answer to a request carries its outcome in:
/// `ok`, or `failed`, with [`REQUEST_DETAIL_HEADER`] saying why.
pub const REQUEST_OUTCOME_HEADER: &str = "x-fcp-outcome";
pub const REQUEST_DETAIL_HEADER: &str = "x-fcp-detail";

/// `POST /link/v1/push`: a push for one of the site's phone apps
/// (`docs/PUSH.md` §2.4). The site holds no keys; a call manager here does,
/// and sends it. The message is for the site's tenant or is refused. What
/// came of it goes back: a gone token is the site's to forget.
async fn push(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    Json(push): Json<fcp_core::push::RelayedPush>,
) -> Result<Response, LinkError> {
    let site = site_of(&state, &headers).await?;
    if push.tenant() != Some(site.tenant_id.to_string().as_str()) {
        return Err(LinkError(
            StatusCode::FORBIDDEN,
            "a site pushes only for its own tenant".to_string(),
        ));
    }
    let Some(pushes) = &state.pushes else {
        return Err(LinkError(
            StatusCode::SERVICE_UNAVAILABLE,
            "this cloud has no call manager to push through".to_string(),
        ));
    };
    match pushes.send_push(&push).await {
        Ok((status, body)) => {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            Ok((status, Json(body)).into_response())
        }
        Err(e) => {
            warn!(site_id = %site.id, error = %e, "A site's push could not be sent");
            Err(LinkError(StatusCode::SERVICE_UNAVAILABLE, e))
        }
    }
}

/// `POST /link/v1/cdrs`: the site's records, batch by batch. Each is
/// rated afresh here — the site rated with its copy of the plans and
/// charged nobody — stored under the site's tenant, idempotently by its
/// id, and announced as `cdr.written` naming the site.
async fn cdrs(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    Json(records): Json<Vec<fcp_core::storage::CallDetailRecord>>,
) -> Result<Json<fcp_core::ShippedAnswer>, LinkError> {
    let site = site_of(&state, &headers).await?;
    let Some(processor) = &state.processor else {
        return Err(LinkError(
            StatusCode::SERVICE_UNAVAILABLE,
            "this cloud does not process records".to_string(),
        ));
    };
    let mut answer = fcp_core::ShippedAnswer::default();
    let mut keyed = Vec::with_capacity(records.len());
    for (index, mut record) in records.into_iter().enumerate() {
        if record.tenant_id != site.tenant_id {
            answer.skipped += 1;
            continue;
        }
        record.site_id = Some(site.id);
        // The cloud bills: whatever the site's copy of the plans said is
        // set aside and the record rated here.
        record.cost = None;
        record.currency = None;
        record.rate_plan_id = None;
        record.matched_prefix = None;
        record.billable_seconds = None;
        record.rated_at = None;
        keyed.push((index, record));
    }
    if keyed.is_empty() {
        return Ok(Json(answer));
    }
    let outcome = processor
        .process_keyed(keyed)
        .await
        .map_err(|e| anyhow::anyhow!("processing a site's records: {e}"))?;
    answer.applied = outcome.stored.len();
    answer.skipped += outcome.duplicates.len();
    answer.rejected = outcome
        .rejected
        .into_iter()
        .map(|r| (r.key, r.reason))
        .collect();
    Ok(Json(answer))
}

/// `POST /link/v1/records`: the rows of the site's recordings, mailboxes
/// and messages as it has them, each marked as the site's, and the
/// deletions its log recorded, mirrored here.
async fn records(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    Json(batch): Json<fcp_core::ShippedRecords>,
) -> Result<Json<fcp_core::ShippedAnswer>, LinkError> {
    use fcp_core::storage::VoicemailStore;
    let site = site_of(&state, &headers).await?;
    let storage = &state.storage;
    let mut answer = fcp_core::ShippedAnswer::default();
    for put in batch.puts {
        let applied = match put.table.as_str() {
            "call_recordings" => {
                let mut row: fcp_core::recording::CallRecording =
                    serde_json::from_value(put.object)
                        .map_err(|e| bad_row(&put.table, &put.id, e))?;
                if row.tenant_id != site.tenant_id {
                    false
                } else if let Some(store) = storage.call_recording_store() {
                    row.site_id = Some(site.id);
                    store.put_call_recording(&row).await?;
                    true
                } else {
                    false
                }
            }
            "conference_recordings" => {
                let mut row: fcp_core::recording::Recording = serde_json::from_value(put.object)
                    .map_err(|e| bad_row(&put.table, &put.id, e))?;
                if row.tenant_id != site.tenant_id {
                    false
                } else if let Some(store) = storage.recording_store() {
                    row.site_id = Some(site.id);
                    store.put_recording(&row).await?;
                    true
                } else {
                    false
                }
            }
            "voicemail_mailboxes" => {
                let row: fcp_core::VoicemailMailbox = serde_json::from_value(put.object)
                    .map_err(|e| bad_row(&put.table, &put.id, e))?;
                if row.tenant_id != site.tenant_id {
                    false
                } else {
                    storage.put_mailbox(row).await?;
                    true
                }
            }
            // A contact written at the site: taken when newer than the
            // cloud's copy, for a user of the site's tenant
            // (docs/RECORD_AND_CONTACTS.md §3).
            "personal_contacts" => {
                let row: fcp_core::contact::PersonalContact = serde_json::from_value(put.object)
                    .map_err(|e| bad_row(&put.table, &put.id, e))?;
                let owner =
                    fcp_core::CredentialStore::get_user_by_id(storage.as_ref(), &row.owner_user_id)
                        .await?;
                match storage.contact_store() {
                    Some(store)
                        if row.tenant_id == site.tenant_id
                            && owner.is_some_and(|u| u.tenant_id == site.tenant_id) =>
                    {
                        let taken = store.put_contact_if_newer(&row).await?;
                        if taken {
                            let change = if row.deleted_at.is_some() {
                                "deleted"
                            } else if row.created_at == row.updated_at {
                                "created"
                            } else {
                                "updated"
                            };
                            fcp_api_core::v1::me_contacts::announce(
                                state.events.as_ref(),
                                &row,
                                change,
                            )
                            .await;
                        }
                        taken
                    }
                    _ => false,
                }
            }
            "voicemail_messages" => {
                let mut row: fcp_core::VoicemailMessage = serde_json::from_value(put.object)
                    .map_err(|e| bad_row(&put.table, &put.id, e))?;
                let owner = storage.get_mailbox(&row.mailbox_id).await?;
                if owner.is_none_or(|m| m.tenant_id != site.tenant_id) {
                    false
                } else {
                    row.site_id = Some(site.id);
                    storage.put_message(row).await?;
                    true
                }
            }
            _ => false,
        };
        if applied {
            answer.applied += 1;
        } else {
            answer.skipped += 1;
        }
    }
    if !batch.deletions.is_empty() {
        let actor = format!("site:{}", site.id);
        let deleter =
            fcp_core::compliance::Deleter::new(fcp_core::compliance::DeletionCause::Site, actor);
        let (applied, skipped) = fcp_core::compliance::deleting_as(
            deleter,
            mirror_deletions(storage, &site, batch.deletions),
        )
        .await?;
        answer.applied += applied;
        answer.skipped += skipped;
    }
    Ok(Json(answer))
}

fn bad_row(table: &str, id: &str, e: serde_json::Error) -> LinkError {
    LinkError(
        StatusCode::BAD_REQUEST,
        format!("{table} {id} does not parse: {e}"),
    )
}

/// Delete here what the site deleted, when the row is the site's.
async fn mirror_deletions(
    storage: &DynStorage,
    site: &Site,
    deletions: Vec<fcp_core::ShippedDeletion>,
) -> Result<(usize, usize)> {
    use fcp_core::storage::VoicemailStore;
    let (mut applied, mut skipped) = (0, 0);
    for deletion in deletions {
        let done = match deletion.class.as_str() {
            "call_recording" => match (
                storage.call_recording_store(),
                deletion.item.parse::<fcp_core::RecordingId>(),
            ) {
                (Some(store), Ok(id)) => match store.get_call_recording(&id).await? {
                    Some(row) if row.site_id == Some(site.id) => {
                        store.delete_call_recording(&id).await?
                    }
                    _ => false,
                },
                _ => false,
            },
            "conference_recording" => match (
                storage.recording_store(),
                deletion.item.parse::<fcp_core::RecordingId>(),
            ) {
                (Some(store), Ok(id)) => match store.get_recording(&id).await? {
                    Some(row) if row.site_id == Some(site.id) => {
                        store.delete_recording(&id).await?
                    }
                    _ => false,
                },
                _ => false,
            },
            "voicemail" => match storage.get_message(&deletion.item).await? {
                Some(row) if row.site_id == Some(site.id) => {
                    storage.delete_message(&deletion.item).await?;
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if done {
            applied += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((applied, skipped))
}

/// `POST /link/v1/events`: the site's events, forwarded to this cloud's
/// events server under the site's name, so webhooks fire here, once.
/// Only the site's tenant's events are taken; the rest are dropped.
async fn events(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    Json(batches): Json<Vec<fcp_core::ShippedEvents>>,
) -> Result<Json<fcp_core::ShippedAnswer>, LinkError> {
    let site = site_of(&state, &headers).await?;
    let Some(ingest) = &state.ingest else {
        return Err(LinkError(
            StatusCode::SERVICE_UNAVAILABLE,
            "this cloud has no events server to forward to ([events] server_url)".to_string(),
        ));
    };
    let mut answer = fcp_core::ShippedAnswer::default();
    for batch in batches {
        let mut kept = Vec::with_capacity(batch.events.len());
        for value in batch.events {
            let Ok(event) = serde_json::from_value::<PlatformEvent>(value) else {
                answer.skipped += 1;
                continue;
            };
            if event.tenant_id() == Some(&site.tenant_id) {
                kept.push(event);
            } else {
                answer.skipped += 1;
            }
        }
        let count = kept.len();
        ingest
            .publish_for_site(site.id, &batch.node_id, &kept)
            .await
            .map_err(|e| {
                warn!(site_id = %site.id, error = %e, "A site's events could not be forwarded");
                LinkError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the events server did not take the events".to_string(),
                )
            })?;
        answer.applied += count;
    }
    Ok(Json(answer))
}

#[derive(Debug, Deserialize)]
pub struct RequestsQuery {
    #[serde(default = "default_requests_wait")]
    pub wait_secs: u64,
    #[serde(default = "default_requests_limit")]
    pub limit: usize,
}

fn default_requests_wait() -> u64 {
    25
}

fn default_requests_limit() -> usize {
    20
}

/// `GET /link/v1/requests`: what the cloud asks of the site, held up to
/// `wait_secs` for something to ask.
async fn requests(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<RequestsQuery>,
) -> Result<Json<Vec<fcp_core::SiteRequest>>, LinkError> {
    let site = site_of(&state, &headers).await?;
    let pending = state
        .requests
        .wait_pending(
            site.id,
            Duration::from_secs(query.wait_secs.min(30)),
            query.limit.clamp(1, 100),
        )
        .await?;
    Ok(Json(pending))
}

/// `POST /link/v1/requests/:id`: the site's answer — the outcome in
/// [`REQUEST_OUTCOME_HEADER`], and for a fetch that succeeded, the file
/// as the body.
async fn request_result(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> Result<StatusCode, LinkError> {
    let site = site_of(&state, &headers).await?;
    let id: fcp_core::SiteRequestId = id
        .parse()
        .map_err(|_| LinkError(StatusCode::BAD_REQUEST, "not a request id".to_string()))?;
    let ok = headers
        .get(REQUEST_OUTCOME_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("ok"));
    let detail = headers
        .get(REQUEST_DETAIL_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let outcome = fcp_core::SiteRequestOutcome { ok, detail };
    let body = ok.then(|| body.to_vec());
    if state.requests.complete(site.id, &id, outcome, body).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(LinkError(
            StatusCode::NOT_FOUND,
            "no such request of this site".to_string(),
        ))
    }
}

// ── Configuration down: the snapshot and the feed (§3.2, phase 2) ────

/// One object of a tenant's slice, as the feed and the snapshot carry
/// it: the table the journal names, the row's id, and the object as the
/// storage crate serializes it (a `rate_plans` object is the plan with
/// its rates).
#[derive(Debug, Serialize)]
pub struct SyncItem {
    pub table: String,
    pub id: String,
    pub object: serde_json::Value,
}

/// `GET /link/v1/snapshot`: every synced object of the site's tenant,
/// and the journal position to follow from.
#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub protocol: u32,
    pub tenant_id: String,
    /// The journal's head when the snapshot began: every change after it
    /// is in the feed, and a change during the snapshot is replayed,
    /// which is harmless.
    pub cursor: i64,
    pub objects: Vec<SyncItem>,
}

/// One entry of the feed: what the journal said, with the object as it
/// is now — `None` when the row is gone, whatever the journal said.
#[derive(Debug, Serialize)]
pub struct Change {
    pub seq: i64,
    pub table: String,
    pub id: String,
    pub op: fcp_core::SyncOp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct Changes {
    pub protocol: u32,
    /// The last sequence number in `changes`, or `since` when there are none.
    pub cursor: i64,
    pub changes: Vec<Change>,
    /// Whether more may follow at once (the page was full).
    pub more: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChangesQuery {
    #[serde(default)]
    pub since: i64,
    /// How long to hold the request open waiting for a change, at most
    /// thirty seconds; zero answers at once.
    #[serde(default)]
    pub wait_secs: u64,
    #[serde(default = "default_changes_limit")]
    pub limit: usize,
}

fn default_changes_limit() -> usize {
    500
}

/// The site behind a request's secret, enrolled and enabled.
async fn site_of(state: &LinkState, headers: &HeaderMap) -> Result<Site, LinkError> {
    let Some(secret) = bearer(headers) else {
        return Err(LinkError(
            StatusCode::UNAUTHORIZED,
            "the request carries the site's secret as a bearer token".to_string(),
        ));
    };
    let Some(site) = state
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
    Ok(site)
}

fn item<T: Serialize>(table: &str, id: &dyn std::fmt::Display, object: &T) -> Result<SyncItem> {
    Ok(SyncItem {
        table: table.to_string(),
        id: id.to_string(),
        object: serde_json::to_value(object)?,
    })
}

/// A rate plan as the feed carries it: the plan with its rates.
async fn rate_plan_object(
    storage: &DynStorage,
    tenant: &TenantId,
    plan: fcp_core::billing::RatePlan,
) -> Result<serde_json::Value> {
    use fcp_core::billing::BillingStore;
    let rates = storage.list_rates(tenant, plan.id).await?;
    Ok(serde_json::json!({ "plan": plan, "rates": rates }))
}

/// Every synced object of a tenant, table by table.
async fn slice_of(state: &LinkState, tenant: &TenantId) -> Result<Vec<SyncItem>> {
    use fcp_core::billing::BillingStore;
    use fcp_core::storage::*;
    use fcp_core::CallingStore;
    use fcp_core::ClassOfServiceStore;
    use fcp_core::ScheduleStore;
    let storage = state.storage.as_ref();
    let mut items = Vec::new();
    if let Some(t) = storage.get_tenant(tenant).await? {
        items.push(item("tenants", &t.id, &t)?);
    }
    for u in storage
        .list_users_in_tenant(tenant, &fcp_core::storage::UserFilter::default())
        .await?
    {
        items.push(item("users", &u.id, &u)?);
    }
    for c in storage.list_calling_settings(tenant).await? {
        items.push(item("user_calling_settings", &c.user_id, &c)?);
    }
    for sched in storage.list_schedules(tenant).await? {
        items.push(item("schedules", &sched.id, &sched)?);
    }
    for class in storage.list_classes_of_service(tenant).await? {
        items.push(item("classes_of_service", &class.id, &class)?);
    }
    for d in storage.list_devices(tenant).await? {
        items.push(item("devices", &d.id, &d)?);
    }
    for e in storage.list_extensions(tenant).await? {
        items.push(item("extensions", &e.id, &e)?);
    }
    for b in storage.list_line_bindings(tenant).await? {
        items.push(item("line_bindings", &b.id, &b)?);
    }
    for n in storage.list_phone_numbers(tenant).await? {
        items.push(item("phone_numbers", &n.id, &n)?);
    }
    for r in storage.list_routing_rules(tenant).await? {
        items.push(item("routing_rules", &r.id, &r)?);
    }
    for r in storage.list_lcr_routes(tenant).await? {
        items.push(item("lcr_routes", &r.id, &r)?);
    }
    for t in storage.list_trunks(tenant).await? {
        items.push(item("trunks", &t.id, &t)?);
    }
    for q in storage.list_queue_configs(tenant).await? {
        items.push(item("queue_configs", &q.id, &q)?);
    }
    for f in state.flows.list_flows(Some(&tenant.to_string())).await? {
        items.push(item("flows", &f.id, &f)?);
    }
    if let Some(prompts) = storage.prompt_store() {
        for p in prompts.list_prompts(tenant, None, None).await? {
            items.push(item("prompts", &p.id, &p)?);
        }
    }
    if let Some(templates) = storage.config_template_store() {
        for c in templates.list_config_templates(tenant).await? {
            items.push(item("config_templates", &c.id, &c)?);
        }
    }
    for m in storage.list_meetings(tenant).await? {
        items.push(item("conference_meetings", &m.id, &m)?);
    }
    for l in storage.list_locations(tenant).await? {
        items.push(item("locations", &l.id, &l)?);
    }
    if let Some(emergency) = storage.emergency_store() {
        for n in emergency.list_emergency_networks(tenant).await? {
            items.push(item("emergency_networks", &n.id, &n)?);
        }
        if let Some(c) = emergency.get_emergency_config(tenant).await? {
            items.push(item("emergency_configs", tenant, &c)?);
        }
    }
    for plan in storage.list_rate_plans(tenant).await? {
        let id = plan.id;
        let object = rate_plan_object(storage, tenant, plan).await?;
        items.push(SyncItem {
            table: "rate_plans".to_string(),
            id: id.to_string(),
            object,
        });
    }
    // After the users: an identity names its user.
    if let Some(identities) = storage.identity_store() {
        for p in identities.list_identity_providers(tenant).await? {
            items.push(item("identity_providers", &p.id, &p)?);
        }
        for i in identities.list_tenant_identities(tenant).await? {
            items.push(item("user_identities", &i.row_id(), &i)?);
        }
    }
    // Firmware targets, each with its image (docs/PROVISIONING.md §11).
    for synced in synced_firmware_targets(storage, tenant).await? {
        items.push(item("firmware_targets", &synced.target.id, &synced)?);
    }
    // The brand, with what its assets are (docs/BRANDING.md).
    if let Some(synced) = synced_brand(storage, tenant).await? {
        items.push(item("tenant_brands", tenant, &synced)?);
    }
    // After the users, who own them (docs/RECORD_AND_CONTACTS.md §3).
    if let Some(contacts) = storage.contact_store() {
        for c in contacts.tenant_contacts(tenant).await? {
            items.push(item("personal_contacts", &c.id, &c)?);
        }
    }
    // After the users: a group's members name them (docs/USERS.md §5).
    if let Some(groups) = storage.group_store() {
        for g in groups.list_groups(tenant).await? {
            items.push(item("groups", &g.id, &g)?);
        }
        for m in groups.list_tenant_members(tenant).await? {
            items.push(item("group_members", &m.row_id(), &m)?);
        }
    }
    Ok(items)
}

/// One synced object as it is now, by the journal's table and row id;
/// `None` when it is gone, or belongs to another tenant.
async fn object_now(
    state: &LinkState,
    tenant: &TenantId,
    table: &str,
    id: &str,
) -> Result<Option<serde_json::Value>> {
    use fcp_core::billing::BillingStore;
    use fcp_core::storage::*;
    use fcp_core::CallingStore;
    use fcp_core::ClassOfServiceStore;
    use fcp_core::ScheduleStore;
    let storage = state.storage.as_ref();
    macro_rules! same_tenant {
        ($object:expr) => {
            match $object {
                Some(o) if o.tenant_id == *tenant => Some(serde_json::to_value(&o)?),
                _ => None,
            }
        };
    }
    Ok(match table {
        "tenants" => match id.parse::<TenantId>() {
            Ok(t) if t == *tenant => storage
                .get_tenant(&t)
                .await?
                .map(|t| serde_json::to_value(&t))
                .transpose()?,
            _ => None,
        },
        "users" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_user_by_id(&id).await?),
            Err(_) => None,
        },
        "schedules" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_schedule(&id).await?),
            Err(_) => None,
        },
        "classes_of_service" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_class_of_service(&id).await?),
            Err(_) => None,
        },
        "user_calling_settings" => match id.parse() {
            Ok(user_id) => same_tenant!(storage.get_calling_settings(tenant, &user_id).await?),
            Err(_) => None,
        },
        "devices" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_device(&id).await?),
            Err(_) => None,
        },
        "extensions" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_extension(&id).await?),
            Err(_) => None,
        },
        "line_bindings" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_line_binding(&id).await?),
            Err(_) => None,
        },
        "phone_numbers" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_phone_number(&id).await?),
            Err(_) => None,
        },
        "routing_rules" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_routing_rule(&id).await?),
            Err(_) => None,
        },
        "lcr_routes" => {
            let id: fcp_core::LcrRouteId =
                serde_json::from_value(serde_json::Value::String(id.to_string()))?;
            same_tenant!(storage.get_lcr_route(&id).await?)
        }
        "trunks" => match id.parse() {
            Ok(id) => match storage.get_trunk(&id).await? {
                Some(t) if t.tenant_id == Some(*tenant) => Some(serde_json::to_value(&t)?),
                _ => None,
            },
            Err(_) => None,
        },
        "queue_configs" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_queue_config(&id).await?),
            Err(_) => None,
        },
        "flows" => match state.flows.get_flow(id).await? {
            Some(f) if f.tenant_id.as_deref() == Some(tenant.to_string().as_str()) => {
                Some(serde_json::to_value(&f)?)
            }
            _ => None,
        },
        "prompts" => match (storage.prompt_store(), id.parse()) {
            (Some(prompts), Ok(id)) => same_tenant!(prompts.get_prompt(&id).await?),
            _ => None,
        },
        "config_templates" => match (storage.config_template_store(), id.parse()) {
            (Some(templates), Ok(id)) => {
                same_tenant!(templates.get_config_template(&id).await?)
            }
            _ => None,
        },
        "conference_meetings" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_meeting(&id).await?),
            Err(_) => None,
        },
        "locations" => match id.parse() {
            Ok(id) => same_tenant!(storage.get_location(&id).await?),
            Err(_) => None,
        },
        "emergency_networks" => match (storage.emergency_store(), id.parse()) {
            (Some(emergency), Ok(id)) => {
                same_tenant!(emergency.get_emergency_network(&id).await?)
            }
            _ => None,
        },
        "emergency_configs" => match storage.emergency_store() {
            Some(emergency) => emergency
                .get_emergency_config(tenant)
                .await?
                .map(|c| serde_json::to_value(&c))
                .transpose()?,
            None => None,
        },
        "rate_plans" | "rates" => match id.parse::<uuid::Uuid>() {
            Ok(plan_id) => match storage.get_rate_plan(tenant, plan_id).await? {
                Some(plan) => Some(rate_plan_object(storage, tenant, plan).await?),
                None => None,
            },
            Err(_) => None,
        },
        "identity_providers" => match (storage.identity_store(), id.parse()) {
            (Some(store), Ok(id)) => same_tenant!(store.get_identity_provider(&id).await?),
            _ => None,
        },
        "user_identities" => match (
            storage.identity_store(),
            fcp_core::identity::UserIdentity::parse_row_id(id),
        ) {
            (Some(store), Some((provider, subject))) => {
                same_tenant!(store.get_user_identity(&provider, &subject).await?)
            }
            _ => None,
        },
        "firmware_targets" => match id.parse::<uuid::Uuid>() {
            Ok(id) => synced_firmware_targets(storage, tenant)
                .await?
                .into_iter()
                .find(|s| s.target.id == id)
                .map(|s| serde_json::to_value(&s))
                .transpose()?,
            Err(_) => None,
        },
        "tenant_brands" if id == tenant.to_string() => synced_brand(storage, tenant)
            .await?
            .map(|s| serde_json::to_value(&s))
            .transpose()?,
        "groups" => match (storage.group_store(), id.parse()) {
            (Some(store), Ok(id)) => same_tenant!(store.get_group(&id).await?),
            _ => None,
        },
        "personal_contacts" => match storage.contact_store() {
            Some(store) => same_tenant!(store.get_contact(id).await?),
            None => None,
        },
        "group_members" => match (
            storage.group_store(),
            fcp_core::group::GroupMember::parse_row_id(id),
        ) {
            (Some(store), Some((group, user))) => {
                same_tenant!(store.get_member(&group, &user).await?)
            }
            _ => None,
        },
        _ => None,
    })
}

/// A tenant's brand, with the assets it names.
async fn synced_brand(
    storage: &DynStorage,
    tenant: &TenantId,
) -> Result<Option<fcp_core::branding::SyncedBrand>> {
    let Some(store) = storage.brand_store() else {
        return Ok(None);
    };
    let Some(brand) = store.get_brand(tenant).await? else {
        return Ok(None);
    };
    let mut assets = Vec::new();
    for hash in brand.asset_hashes() {
        if let Some(asset) = store.get_brand_asset(tenant, hash).await? {
            assets.push(asset);
        }
    }
    Ok(Some(fcp_core::branding::SyncedBrand { brand, assets }))
}

/// `GET /link/v1/brand-assets/:sha256` — an asset the site's tenant's
/// brand names, for the site to serve its softphones.
async fn brand_asset(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    axum::extract::Path(sha256): axum::extract::Path<String>,
) -> Result<Response, LinkError> {
    let site = site_of(&state, &headers).await?;
    let not_found = || {
        LinkError(
            StatusCode::NOT_FOUND,
            "No such asset for this site".to_string(),
        )
    };
    let named = synced_brand(&state.storage, &site.tenant_id)
        .await?
        .is_some_and(|s| s.assets.iter().any(|a| a.sha256 == sha256));
    if !named {
        return Err(not_found());
    }
    let (asset, bytes) = state
        .storage
        .brand_store()
        .ok_or_else(not_found)?
        .brand_asset_bytes(&sha256)
        .await?
        .ok_or_else(not_found)?;
    Ok((StatusCode::OK, [(header::CONTENT_TYPE, asset.mime)], bytes).into_response())
}

/// A tenant's firmware targets, each with the image it names.
async fn synced_firmware_targets(
    storage: &DynStorage,
    tenant: &TenantId,
) -> Result<Vec<fcp_core::firmware::SyncedFirmwareTarget>> {
    let Some(store) = storage.provisioning_store() else {
        return Ok(Vec::new());
    };
    let mut synced = Vec::new();
    for target in store.list_firmware_targets(tenant).await? {
        if let Some(image) = store.get_firmware_image(&target.image_id).await? {
            synced.push(fcp_core::firmware::SyncedFirmwareTarget { target, image });
        }
    }
    Ok(synced)
}

/// `GET /link/v1/firmware/:image` — an image the site's tenant targets,
/// streamed to the site's link, which caches it for the site's phones.
async fn firmware_image(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    axum::extract::Path(image): axum::extract::Path<String>,
) -> Result<Response, LinkError> {
    let site = site_of(&state, &headers).await?;
    let not_found = || {
        LinkError(
            StatusCode::NOT_FOUND,
            "No such image for this site".to_string(),
        )
    };
    let id = uuid::Uuid::parse_str(&image).map_err(|_| not_found())?;
    let synced = synced_firmware_targets(&state.storage, &site.tenant_id).await?;
    let image = synced
        .into_iter()
        .map(|s| s.image)
        .find(|i| i.id == id)
        .ok_or_else(not_found)?;
    let files = state.firmware.as_ref().ok_or_else(not_found)?;
    let key = format!("{}/{}", image.id.simple(), image.file_name);
    let (size, body) = files
        .open(&key)
        .await
        .map_err(|e| LinkError(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(not_found)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_LENGTH, size.to_string()),
            (
                header::HeaderName::from_static("x-fcp-sha256"),
                image.sha256,
            ),
        ],
        body,
    )
        .into_response())
}

async fn snapshot(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
) -> Result<Json<Snapshot>, LinkError> {
    let site = site_of(&state, &headers).await?;
    let cursor = state.storage.journal_head().await?;
    let objects = slice_of(&state, &site.tenant_id).await?;
    info!(site_id = %site.id, objects = objects.len(), cursor, "A site took a snapshot");
    Ok(Json(Snapshot {
        protocol: LINK_PROTOCOL,
        tenant_id: site.tenant_id.to_string(),
        cursor,
        objects,
    }))
}

async fn changes(
    Extension(state): Extension<LinkState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<ChangesQuery>,
) -> Result<Json<Changes>, LinkError> {
    let site = site_of(&state, &headers).await?;
    let limit = query.limit.clamp(1, 5000);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(query.wait_secs.min(30));
    let entries = loop {
        let entries = state
            .storage
            .journal_after(&site.tenant_id, query.since, limit)
            .await?;
        if !entries.is_empty() || tokio::time::Instant::now() >= deadline {
            break entries;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let more = entries.len() >= limit;
    let mut cursor = query.since;
    let mut changes = Vec::with_capacity(entries.len());
    for entry in entries {
        cursor = entry.seq;
        let table = if entry.table == "rates" {
            "rate_plans".to_string()
        } else {
            entry.table.clone()
        };
        let object = match entry.op {
            fcp_core::SyncOp::Delete => None,
            fcp_core::SyncOp::Put => {
                object_now(&state, &site.tenant_id, &entry.table, &entry.row_id).await?
            }
        };
        let op = if object.is_some() {
            fcp_core::SyncOp::Put
        } else {
            fcp_core::SyncOp::Delete
        };
        changes.push(Change {
            seq: entry.seq,
            table,
            id: entry.row_id,
            op,
            object,
        });
    }
    Ok(Json(Changes {
        protocol: LINK_PROTOCOL,
        cursor,
        changes,
        more,
    }))
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
