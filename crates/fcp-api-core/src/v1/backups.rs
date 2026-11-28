//! Backups from the admin (`docs/SCHEDULED_BACKUPS.md` B1): a platform
//! administrator's destinations, their schedules and retention, a test,
//! "Back up now", and the runs `fcp-backup` makes.
//!
//! The admin never touches the host: it writes destinations and asks for
//! runs; the `fcp-backup` service claims and runs them.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use fcp_core::backup::{
    BackupDestination, BackupRetention, BackupRun, BackupRunKind, BackupRunState, BackupSchedule,
    BackupStore, BackupTarget,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::SystemTime;

use crate::auth::AuthIdentity;
use crate::error::ApiError;
use crate::state::ApiState;

const DESTINATION_PREFIX: &str = "bkd-";
const RUN_PREFIX: &str = "bkr-";

fn require_platform(identity: &AuthIdentity) -> Result<(), ApiError> {
    if identity.is_platform_admin() {
        Ok(())
    } else {
        Err(ApiError::Forbidden("Platform admin access required".into()))
    }
}

fn store(state: &ApiState) -> Result<&Arc<dyn BackupStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.backup_store())
        .ok_or_else(|| ApiError::ServiceUnavailable("Backups are not kept here".into()))
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError::Internal(e.to_string())
}

fn parse_id(prefix: &str, id: &str) -> Result<uuid::Uuid, ApiError> {
    id.strip_prefix(prefix)
        .unwrap_or(id)
        .parse()
        .map_err(|_| ApiError::NotFound("No such backup destination or run".into()))
}

/// A destination's target, its secrets never shown.
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TargetView {
    Sftp {
        host: String,
        port: u16,
        user: String,
        path: String,
        has_private_key: bool,
        /// The server's host key as pinned by the first test.
        host_key: Option<String>,
    },
    S3 {
        endpoint: String,
        bucket: String,
        region: Option<String>,
        prefix: Option<String>,
        access_key_id: String,
        has_secret_access_key: bool,
    },
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ScheduleView {
    /// A daily backup at `HH:MM`, UTC; unset, none is scheduled.
    pub time_of_day: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetentionView {
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
}

/// A backup destination.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DestinationView {
    /// `bkd-<uuid>`.
    pub id: String,
    pub name: String,
    pub target: TargetView,
    pub has_password: bool,
    pub schedule: ScheduleView,
    pub keep: RetentionView,
    pub include_key: bool,
    pub enabled: bool,
    /// The repository's address as restic names it, for restoring by hand.
    pub repository: String,
    /// The last run of a backup, if any.
    pub last_run: Option<RunView>,
    /// Shown once, when a destination is made without a password and one
    /// is generated: keep it, it opens every snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A run of a destination.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RunView {
    /// `bkr-<uuid>`.
    pub id: String,
    pub destination_id: String,
    pub kind: BackupRunKind,
    pub state: BackupRunState,
    pub trigger: String,
    pub requested_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub snapshots: Vec<String>,
    pub bytes_added: u64,
    pub message: Option<String>,
    pub node: Option<String>,
}

impl RunView {
    fn of(r: &BackupRun) -> Self {
        let t = crate::time::format_system_time;
        Self {
            id: format!("{RUN_PREFIX}{}", r.id),
            destination_id: format!("{DESTINATION_PREFIX}{}", r.destination_id),
            kind: r.kind,
            state: r.state,
            trigger: r.trigger.clone(),
            requested_at: t(r.requested_at),
            started_at: r.started_at.map(t),
            finished_at: r.finished_at.map(t),
            snapshots: r.snapshots.clone(),
            bytes_added: r.bytes_added,
            message: r.message.clone(),
            node: r.node.clone(),
        }
    }
}

fn view(d: &BackupDestination, last_run: Option<&BackupRun>) -> DestinationView {
    let t = crate::time::format_system_time;
    DestinationView {
        id: format!("{DESTINATION_PREFIX}{}", d.id),
        name: d.name.clone(),
        target: match &d.target {
            BackupTarget::Sftp {
                host,
                port,
                user,
                path,
                private_key,
                host_key,
            } => TargetView::Sftp {
                host: host.clone(),
                port: *port,
                user: user.clone(),
                path: path.clone(),
                has_private_key: private_key.as_deref().is_some_and(|k| !k.is_empty()),
                host_key: host_key.clone(),
            },
            BackupTarget::S3 {
                endpoint,
                bucket,
                region,
                prefix,
                access_key_id,
                secret_access_key,
            } => TargetView::S3 {
                endpoint: endpoint.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
                prefix: prefix.clone(),
                access_key_id: access_key_id.clone(),
                has_secret_access_key: secret_access_key.as_deref().is_some_and(|k| !k.is_empty()),
            },
        },
        has_password: d.password.as_deref().is_some_and(|p| !p.is_empty()),
        schedule: ScheduleView {
            time_of_day: d.schedule.daily_at.clone(),
        },
        keep: RetentionView {
            daily: d.keep.daily,
            weekly: d.keep.weekly,
            monthly: d.keep.monthly,
        },
        include_key: d.include_key,
        enabled: d.enabled,
        repository: d.target.repository(),
        last_run: last_run.map(RunView::of),
        password: None,
        created_at: t(d.created_at),
        updated_at: t(d.updated_at),
    }
}

/// A destination's target to keep; a secret left out on a change keeps the
/// one kept.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetRequest {
    Sftp {
        host: String,
        #[serde(default)]
        port: Option<u16>,
        user: String,
        path: String,
        /// An OpenSSH private key; write-only.
        #[serde(default)]
        private_key: Option<String>,
    },
    S3 {
        endpoint: String,
        bucket: String,
        #[serde(default)]
        region: Option<String>,
        #[serde(default)]
        prefix: Option<String>,
        access_key_id: String,
        /// Write-only.
        #[serde(default)]
        secret_access_key: Option<String>,
    },
}

/// A destination to make or change.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DestinationRequest {
    pub name: String,
    pub target: TargetRequest,
    /// The repository password; write-only. Left out when a destination is
    /// made, one is generated and shown once.
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub schedule: Option<ScheduleView>,
    #[serde(default)]
    pub keep: Option<RetentionView>,
    #[serde(default)]
    pub include_key: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

fn text(what: &str, v: &str, max: usize) -> Result<String, ApiError> {
    let v = v.trim();
    if v.is_empty() || v.len() > max || v.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(format!(
            "{what} is 1 to {max} characters"
        )));
    }
    Ok(v.to_string())
}

fn secret(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The destination a request describes, keeping what `kept` has where the
/// request leaves a secret out.
fn destination_from(
    req: DestinationRequest,
    kept: Option<&BackupDestination>,
) -> Result<(BackupDestination, Option<String>), ApiError> {
    let name = text("A name", &req.name, 100)?;
    let target = match req.target {
        TargetRequest::Sftp {
            host,
            port,
            user,
            path,
            private_key,
        } => {
            let host = text("The host", &host, 255)?;
            if host.contains(['/', ' ', '@']) {
                return Err(ApiError::BadRequest(
                    "The host is a name or an address".into(),
                ));
            }
            let kept_target = kept.map(|k| &k.target);
            let (kept_key, kept_host_key) = match kept_target {
                Some(BackupTarget::Sftp {
                    private_key,
                    host_key,
                    host: kept_host,
                    port: kept_port,
                    ..
                }) => (
                    private_key.clone(),
                    // A pinned host key holds only while the server is the same.
                    (kept_host == &host && *kept_port == port.unwrap_or(22))
                        .then(|| host_key.clone())
                        .flatten(),
                ),
                _ => (None, None),
            };
            let private_key = secret(private_key).or(kept_key);
            if let Some(k) = &private_key {
                if !k.contains("PRIVATE KEY-----") || k.len() > 16 * 1024 {
                    return Err(ApiError::BadRequest(
                        "The private key is an OpenSSH or PEM private key".into(),
                    ));
                }
            } else {
                return Err(ApiError::BadRequest(
                    "An SFTP destination has a private key".into(),
                ));
            }
            BackupTarget::Sftp {
                host,
                port: port.unwrap_or(22),
                user: text("The user", &user, 64)?,
                path: text("The path", &path, 1024)?,
                private_key,
                host_key: kept_host_key,
            }
        }
        TargetRequest::S3 {
            endpoint,
            bucket,
            region,
            prefix,
            access_key_id,
            secret_access_key,
        } => {
            let endpoint = text("The endpoint", &endpoint, 512)?;
            let url = url::Url::parse(&endpoint)
                .map_err(|_| ApiError::BadRequest("The endpoint is a URL".into()))?;
            if !matches!(url.scheme(), "https" | "http") {
                return Err(ApiError::BadRequest(
                    "The endpoint is https (or http)".into(),
                ));
            }
            let kept_secret = match kept.map(|k| &k.target) {
                Some(BackupTarget::S3 {
                    secret_access_key, ..
                }) => secret_access_key.clone(),
                _ => None,
            };
            let secret_access_key = secret(secret_access_key).or(kept_secret);
            if secret_access_key.is_none() {
                return Err(ApiError::BadRequest(
                    "An S3 destination has a secret access key".into(),
                ));
            }
            BackupTarget::S3 {
                endpoint,
                bucket: text("The bucket", &bucket, 255)?,
                region: region
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty()),
                prefix: prefix
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty()),
                access_key_id: text("The access key id", &access_key_id, 256)?,
                secret_access_key,
            }
        }
    };
    let daily_at = req
        .schedule
        .and_then(|s| s.time_of_day)
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty());
    if let Some(d) = &daily_at {
        if chrono::NaiveTime::parse_from_str(d, "%H:%M").is_err() {
            return Err(ApiError::BadRequest("time_of_day is HH:MM (UTC)".into()));
        }
    }
    let keep = req
        .keep
        .map_or(BackupRetention::default(), |k| BackupRetention {
            daily: k.daily,
            weekly: k.weekly,
            monthly: k.monthly,
        });
    if keep.daily + keep.weekly + keep.monthly == 0
        || keep.daily > 1000
        || keep.weekly > 1000
        || keep.monthly > 1000
    {
        return Err(ApiError::BadRequest(
            "Keep some snapshots: up to 1000 daily, weekly and monthly".into(),
        ));
    }
    let (password, generated) = match (secret(req.password), kept.and_then(|k| k.password.clone()))
    {
        (Some(p), _) => {
            if p.len() < 12 || p.len() > 1024 {
                return Err(ApiError::BadRequest(
                    "A repository password is 12 to 1024 characters".into(),
                ));
            }
            (p, None)
        }
        (None, Some(kept)) => (kept, None),
        (None, None) => {
            use rand::RngCore;
            let mut bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut bytes);
            let p = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
            (p.clone(), Some(p))
        }
    };
    let now = SystemTime::now();
    Ok((
        BackupDestination {
            id: kept.map(|k| k.id).unwrap_or_else(uuid::Uuid::new_v4),
            name,
            target,
            password: Some(password),
            schedule: BackupSchedule {
                daily_at,
                cron: None,
            },
            keep,
            include_key: req.include_key,
            enabled: req.enabled,
            created_at: kept.map(|k| k.created_at).unwrap_or(now),
            updated_at: now,
        },
        generated,
    ))
}

async fn name_taken(
    store: &Arc<dyn BackupStore>,
    name: &str,
    except: Option<uuid::Uuid>,
) -> Result<bool, ApiError> {
    Ok(store
        .list_backup_destinations()
        .await
        .map_err(internal)?
        .iter()
        .any(|d| d.name.eq_ignore_ascii_case(name) && Some(d.id) != except))
}

async fn last_backup(
    store: &Arc<dyn BackupStore>,
    id: &uuid::Uuid,
) -> Result<Option<BackupRun>, ApiError> {
    Ok(store
        .list_backup_runs(Some(id), 20)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|r| r.kind == BackupRunKind::Backup))
}

/// GET /system/backups/destinations
#[utoipa::path(
    get,
    path = "/api/v1/system/backups/destinations",
    tag = "backups",
    operation_id = "backups_list_destinations",
    responses(
        (status = 200, description = "OK", body = crate::page::Page<DestinationView>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_destinations(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Result<Json<crate::page::Page<DestinationView>>, ApiError> {
    require_platform(&identity)?;
    let store = store(&state)?;
    let mut list = Vec::new();
    for d in store.list_backup_destinations().await.map_err(internal)? {
        let last = last_backup(store, &d.id).await?;
        list.push(view(&d, last.as_ref()));
    }
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}

/// POST /system/backups/destinations
#[utoipa::path(
    post,
    path = "/api/v1/system/backups/destinations",
    tag = "backups",
    operation_id = "backups_create_destination",
    request_body = DestinationRequest,
    responses(
        (status = 201, description = "Made; a generated password is shown this once", body = DestinationView),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_destination(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Json(req): Json<DestinationRequest>,
) -> Result<(StatusCode, Json<DestinationView>), ApiError> {
    require_platform(&identity)?;
    let store = store(&state)?;
    let (d, generated) = destination_from(req, None)?;
    if name_taken(store, &d.name, None).await? {
        return Err(ApiError::Conflict(format!(
            "A destination is named {}",
            d.name
        )));
    }
    store.put_backup_destination(&d).await.map_err(internal)?;
    let mut v = view(&d, None);
    v.password = generated;
    Ok((StatusCode::CREATED, Json(v)))
}

/// PUT /system/backups/destinations/{id}
#[utoipa::path(
    put,
    path = "/api/v1/system/backups/destinations/{id}",
    tag = "backups",
    operation_id = "backups_update_destination",
    params(("id" = String, Path, description = "The destination")),
    request_body = DestinationRequest,
    responses(
        (status = 200, description = "OK", body = DestinationView),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_destination(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
    Json(req): Json<DestinationRequest>,
) -> Result<Json<DestinationView>, ApiError> {
    require_platform(&identity)?;
    let store = store(&state)?;
    let id = parse_id(DESTINATION_PREFIX, &id)?;
    let kept = store
        .get_backup_destination(&id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError::NotFound("No such backup destination".into()))?;
    let (d, _) = destination_from(req, Some(&kept))?;
    if name_taken(store, &d.name, Some(id)).await? {
        return Err(ApiError::Conflict(format!(
            "A destination is named {}",
            d.name
        )));
    }
    store.put_backup_destination(&d).await.map_err(internal)?;
    let last = last_backup(store, &d.id).await?;
    Ok(Json(view(&d, last.as_ref())))
}

/// DELETE /system/backups/destinations/{id} — what is stored there stays.
#[utoipa::path(
    delete,
    path = "/api/v1/system/backups/destinations/{id}",
    tag = "backups",
    operation_id = "backups_delete_destination",
    params(("id" = String, Path, description = "The destination")),
    responses(
        (status = 204, description = "Removed; the snapshots at the destination are not touched"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_destination(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_platform(&identity)?;
    let id = parse_id(DESTINATION_PREFIX, &id)?;
    if store(&state)?
        .delete_backup_destination(&id)
        .await
        .map_err(internal)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("No such backup destination".into()))
    }
}

async fn request(
    state: &ApiState,
    id: &str,
    kind: BackupRunKind,
) -> Result<(StatusCode, Json<RunView>), ApiError> {
    let store = store(state)?;
    let id = parse_id(DESTINATION_PREFIX, id)?;
    store
        .get_backup_destination(&id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError::NotFound("No such backup destination".into()))?;
    let run = BackupRun {
        id: uuid::Uuid::new_v4(),
        destination_id: id,
        kind,
        state: BackupRunState::Requested,
        trigger: match kind {
            BackupRunKind::Backup => "now".into(),
            BackupRunKind::Test => "test".into(),
        },
        requested_at: SystemTime::now(),
        started_at: None,
        finished_at: None,
        snapshots: Vec::new(),
        bytes_added: 0,
        message: None,
        node: None,
    };
    store.request_backup_run(&run).await.map_err(internal)?;
    Ok((StatusCode::ACCEPTED, Json(RunView::of(&run))))
}

/// POST /system/backups/destinations/{id}/test — reach it, pin its host
/// key, write and remove a probe; the run says how it went.
#[utoipa::path(
    post,
    path = "/api/v1/system/backups/destinations/{id}/test",
    tag = "backups",
    operation_id = "backups_test_destination",
    params(("id" = String, Path, description = "The destination")),
    responses(
        (status = 202, description = "Asked for; follow the run", body = RunView),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn test_destination(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<RunView>), ApiError> {
    require_platform(&identity)?;
    request(&state, &id, BackupRunKind::Test).await
}

/// POST /system/backups/destinations/{id}/run — back up now.
#[utoipa::path(
    post,
    path = "/api/v1/system/backups/destinations/{id}/run",
    tag = "backups",
    operation_id = "backups_run_destination",
    params(("id" = String, Path, description = "The destination")),
    responses(
        (status = 202, description = "Asked for; follow the run", body = RunView),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn run_destination(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<RunView>), ApiError> {
    require_platform(&identity)?;
    request(&state, &id, BackupRunKind::Backup).await
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RunsQuery {
    /// One destination's runs (`bkd-<uuid>`).
    pub destination: Option<String>,
    /// At most this many, newest first (100 by default, 1000 at most).
    pub limit: Option<usize>,
}

/// GET /system/backups/runs
#[utoipa::path(
    get,
    path = "/api/v1/system/backups/runs",
    tag = "backups",
    operation_id = "backups_list_runs",
    params(RunsQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<RunView>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_runs(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Query(q): Query<RunsQuery>,
) -> Result<Json<crate::page::Page<RunView>>, ApiError> {
    require_platform(&identity)?;
    let destination = q
        .destination
        .as_deref()
        .map(|d| parse_id(DESTINATION_PREFIX, d))
        .transpose()?;
    let runs = store(&state)?
        .list_backup_runs(destination.as_ref(), q.limit.unwrap_or(100).clamp(1, 1000))
        .await
        .map_err(internal)?;
    let list: Vec<RunView> = runs.iter().map(RunView::of).collect();
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}

/// GET /system/backups/runs/{id}
#[utoipa::path(
    get,
    path = "/api/v1/system/backups/runs/{id}",
    tag = "backups",
    operation_id = "backups_get_run",
    params(("id" = String, Path, description = "The run")),
    responses(
        (status = 200, description = "OK", body = RunView),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_run(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> Result<Json<RunView>, ApiError> {
    require_platform(&identity)?;
    let id = parse_id(RUN_PREFIX, &id)?;
    let run = store(&state)?
        .get_backup_run(&id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError::NotFound("No such backup run".into()))?;
    Ok(Json(RunView::of(&run)))
}

// ── The master key, downloaded once (B2) ───────────────────────────────────

/// Whether the master key can be downloaded here, and when it last was.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct KeyStatus {
    /// `local`, `aws-kms` or `none`.
    pub provider: String,
    /// A local key can be downloaded; a KMS key never leaves the KMS.
    pub exportable: bool,
    pub kek_id: Option<String>,
    /// When this KEK was last downloaded; until then the page asks for it.
    pub last_exported_at: Option<String>,
    pub last_exported_by: Option<String>,
}

fn encryption(state: &ApiState) -> Result<fcp_crypto::EncryptionSettings, ApiError> {
    match &state.encryption {
        Some(s) => Ok(s.clone()),
        None => fcp_crypto::EncryptionSettings::from_env()
            .map_err(|e| ApiError::ServiceUnavailable(format!("Encryption settings: {e}"))),
    }
}

/// GET /system/backups/key
#[utoipa::path(
    get,
    path = "/api/v1/system/backups/key",
    tag = "backups",
    operation_id = "backups_key_status",
    responses(
        (status = 200, description = "OK", body = KeyStatus),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn key_status(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Result<Json<KeyStatus>, ApiError> {
    require_platform(&identity)?;
    let settings = encryption(&state)?;
    let export = settings.exportable_master_key();
    let kek_id = export.as_ref().map(|(_, id)| id.clone());
    let last = match &kek_id {
        Some(id) => store(&state)?.last_key_export(id).await.map_err(internal)?,
        None => None,
    };
    Ok(Json(KeyStatus {
        provider: settings.provider_name().to_string(),
        exportable: export.is_some(),
        kek_id,
        last_exported_at: last
            .as_ref()
            .map(|(at, _)| crate::time::format_system_time(*at)),
        last_exported_by: last.map(|(_, by)| by),
    }))
}

/// A passphrase to wrap the key in.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyExportRequest {
    /// At least 12 characters; kept nowhere, needed to open the file.
    pub passphrase: String,
}

/// POST /system/backups/key — the local master key and its KEK id, as an
/// age file wrapped in the passphrase (`age -d` opens it). Recorded and
/// audited; the key itself is kept nowhere new.
#[utoipa::path(
    post,
    path = "/api/v1/system/backups/key",
    tag = "backups",
    operation_id = "backups_export_key",
    request_body = KeyExportRequest,
    responses(
        (status = 200, description = "The key, an ASCII-armored age file", content_type = "application/octet-stream", body = Vec<u8>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn export_key(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Json(req): Json<KeyExportRequest>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    use std::io::Write;
    require_platform(&identity)?;
    if req.passphrase.chars().count() < 12 || req.passphrase.len() > 1024 {
        return Err(ApiError::BadRequest(
            "A passphrase is 12 to 1024 characters".into(),
        ));
    }
    let (key, kek_id) = encryption(&state)?.exportable_master_key().ok_or_else(|| {
        ApiError::BadRequest(
            "This deployment's master key is not local: a KMS key never leaves the KMS".into(),
        )
    })?;
    let plain = serde_json::json!({
        "format": "fcp-master-key-v1",
        "kek_id": kek_id,
        // As FCP_MASTER_KEY takes it, or the file FCP_MASTER_KEY_FILE names.
        "master_key": key,
    })
    .to_string();
    let encryptor =
        age::Encryptor::with_user_passphrase(age::secrecy::SecretString::from(req.passphrase));
    let mut armored = Vec::new();
    {
        let armor =
            age::armor::ArmoredWriter::wrap_output(&mut armored, age::armor::Format::AsciiArmor)
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        let mut w = encryptor
            .wrap_output(armor)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        w.write_all(plain.as_bytes())
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        w.finish()
            .and_then(|armor| armor.finish())
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    store(&state)?
        .record_key_export(&kek_id, &identity.actor())
        .await
        .map_err(internal)?;
    tracing::warn!(actor = %identity.actor(), kek_id = %kek_id, "The master key was downloaded from the backup page");
    let name = format!("fcp-master-key-{}.age", kek_id.replace(':', "-"));
    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/octet-stream".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (axum::http::header::CACHE_CONTROL, "no-store".to_string()),
        ],
        armored,
    )
        .into_response())
}

// ── Snapshots and downloads, through fcp-backup's API (B3a) ────────────────

fn service(state: &ApiState) -> Result<&crate::BackupService, ApiError> {
    state.backup_service.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "The backup service is not set up here: [backup] url on the admin".into(),
        )
    })
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_default()
}

/// What the backup service answered, as an error of ours where it failed.
async fn answer(r: reqwest::Response) -> Result<serde_json::Value, ApiError> {
    let status = r.status();
    let body: serde_json::Value = r.json().await.unwrap_or_default();
    if status.is_success() {
        return Ok(body);
    }
    let said = body["error"]
        .as_str()
        .unwrap_or("the backup service refused")
        .to_string();
    Err(match status.as_u16() {
        404 => ApiError::NotFound(said),
        409 => ApiError::Conflict(said),
        400 => ApiError::BadRequest(said),
        _ => ApiError::ServiceUnavailable(format!("The backup service: {said}")),
    })
}

/// A run's snapshots, as one point a restore or a download takes.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SnapshotPoint {
    /// The run that made it (`bkr-<uuid>`).
    pub run: String,
    pub time: String,
    pub release: Option<String>,
    pub kek_id: Option<String>,
    pub host: String,
    /// restic's ids of the database's snapshot and the files'.
    pub database: Option<String>,
    pub files: Option<String>,
    /// The volumes the files hold.
    pub volumes: Vec<String>,
}

/// GET /system/backups/destinations/{id}/snapshots
#[utoipa::path(
    get,
    path = "/api/v1/system/backups/destinations/{id}/snapshots",
    tag = "backups",
    operation_id = "backups_list_snapshots",
    params(("id" = String, Path, description = "The destination")),
    responses(
        (status = 200, description = "OK, newest first", body = crate::page::Page<SnapshotPoint>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_snapshots(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> Result<Json<crate::page::Page<SnapshotPoint>>, ApiError> {
    require_platform(&identity)?;
    let id = parse_id(DESTINATION_PREFIX, &id)?;
    let svc = service(&state)?;
    let r = client()
        .get(format!(
            "{}/v1/destinations/{DESTINATION_PREFIX}{id}/snapshots",
            svc.url
        ))
        .bearer_auth(&svc.token)
        .send()
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("The backup service: {e}")))?;
    let body = answer(r).await?;
    let mut list: Vec<SnapshotPoint> =
        serde_json::from_value(body).map_err(|e| ApiError::Internal(e.to_string()))?;
    for p in &mut list {
        p.run = format!("{RUN_PREFIX}{}", p.run.trim_start_matches(RUN_PREFIX));
    }
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}

/// What a download needs when the snapshot holds no master key and this
/// system's is another: the key file downloaded from this page, and its
/// passphrase.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadRequest {
    #[serde(default)]
    pub key_file: Option<String>,
    #[serde(default)]
    pub passphrase: Option<String>,
}

/// A download being made.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DownloadView {
    pub id: String,
    /// `building`, `ready` or `failed`.
    pub state: String,
    pub size: Option<u64>,
    pub error: Option<String>,
    /// Where the browser fetches it, once, when ready: the link is its own
    /// credential, good for an hour.
    pub file_url: Option<String>,
}

/// Open a key file downloaded from this page with its passphrase: the
/// master key, base64.
fn open_key_file(file: &str, passphrase: &str) -> Result<String, ApiError> {
    use std::io::Read;
    let refused = || ApiError::BadRequest("The key file did not open with that passphrase".into());
    let reader = age::armor::ArmoredReader::new(file.as_bytes());
    let decryptor = age::Decryptor::new(reader).map_err(|_| refused())?;
    let identity =
        age::scrypt::Identity::new(age::secrecy::SecretString::from(passphrase.to_string()));
    let mut plain = String::new();
    decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| refused())?
        .read_to_string(&mut plain)
        .map_err(|_| refused())?;
    let v: serde_json::Value = serde_json::from_str(&plain).map_err(|_| refused())?;
    v["master_key"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(refused)
}

/// POST /system/backups/destinations/{id}/snapshots/{run}/download — the
/// point made a `backup.sh`-layout tar by the backup service, for
/// `restore.sh` on any host. Audited.
#[utoipa::path(
    post,
    path = "/api/v1/system/backups/destinations/{id}/snapshots/{run}/download",
    tag = "backups",
    operation_id = "backups_start_download",
    params(
        ("id" = String, Path, description = "The destination"),
        ("run" = String, Path, description = "The point's run"),
    ),
    request_body = DownloadRequest,
    responses(
        (status = 202, description = "Being made; follow it", body = DownloadView),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn start_download(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path((id, run)): Path<(String, String)>,
    Json(req): Json<DownloadRequest>,
) -> Result<(StatusCode, Json<DownloadView>), ApiError> {
    require_platform(&identity)?;
    let id = parse_id(DESTINATION_PREFIX, &id)?;
    let run = parse_id(RUN_PREFIX, &run)?;
    let master_key = match (req.key_file.as_deref(), req.passphrase.as_deref()) {
        (Some(f), Some(p)) if !f.trim().is_empty() => Some(open_key_file(f, p)?),
        (Some(f), None) if !f.trim().is_empty() => {
            return Err(ApiError::BadRequest(
                "A key file needs its passphrase".into(),
            ))
        }
        _ => None,
    };
    let svc = service(&state)?;
    let r = client()
        .post(format!("{}/v1/downloads", svc.url))
        .bearer_auth(&svc.token)
        .json(&serde_json::json!({
            "destination_id": format!("{DESTINATION_PREFIX}{id}"),
            "run": run.to_string(),
            "master_key": master_key,
        }))
        .send()
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("The backup service: {e}")))?;
    let body = answer(r).await?;
    tracing::warn!(actor = %identity.actor(), run = %run, "A backup's download was asked for");
    let did = body["id"].as_str().unwrap_or_default().to_string();
    let token = body["token"].as_str().unwrap_or_default().to_string();
    Ok((
        StatusCode::ACCEPTED,
        Json(DownloadView {
            id: format!("{did}.{token}"),
            state: "building".into(),
            size: None,
            error: None,
            file_url: None,
        }),
    ))
}

fn split_download(id: &str) -> Result<(String, String), ApiError> {
    id.split_once('.')
        .filter(|(i, t)| !i.is_empty() && !t.is_empty())
        .map(|(i, t)| (i.to_string(), t.to_string()))
        .ok_or_else(|| ApiError::NotFound("No such download".into()))
}

/// GET /system/backups/downloads/{id}
#[utoipa::path(
    get,
    path = "/api/v1/system/backups/downloads/{id}",
    tag = "backups",
    operation_id = "backups_download_state",
    params(("id" = String, Path, description = "The download")),
    responses(
        (status = 200, description = "OK", body = DownloadView),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn download_state(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> Result<Json<DownloadView>, ApiError> {
    require_platform(&identity)?;
    let (did, token) = split_download(&id)?;
    let svc = service(&state)?;
    let r = client()
        .get(format!("{}/v1/downloads/{did}", svc.url))
        .bearer_auth(&svc.token)
        .send()
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("The backup service: {e}")))?;
    let body = answer(r).await?;
    let st = body["state"].as_str().unwrap_or("failed").to_string();
    Ok(Json(DownloadView {
        file_url: (st == "ready").then(|| format!("/backups/downloads/{did}/{token}")),
        size: body["size"].as_u64(),
        error: body["error"].as_str().map(str::to_string),
        state: st,
        id,
    }))
}

/// GET /backups/downloads/{id}/{token} — the tar, streamed once from the
/// backup service; the link is the credential (a browser saving a file
/// sends no bearer token).
pub async fn serve_download(
    State(state): State<ApiState>,
    Path((id, token)): Path<(String, String)>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let svc = service(&state)?;
    let id: uuid::Uuid = id
        .parse()
        .map_err(|_| ApiError::NotFound("No such download".into()))?;
    let r = reqwest::Client::new()
        .get(format!("{}/v1/downloads/{id}/file", svc.url))
        .query(&[("token", token.as_str())])
        .bearer_auth(&svc.token)
        .send()
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("The backup service: {e}")))?;
    if !r.status().is_success() {
        return Err(ApiError::NotFound(
            "No such download, or it was fetched".into(),
        ));
    }
    let mut headers = axum::http::HeaderMap::new();
    for h in [
        axum::http::header::CONTENT_TYPE,
        axum::http::header::CONTENT_LENGTH,
        axum::http::header::CONTENT_DISPOSITION,
    ] {
        if let Some(v) = r.headers().get(h.as_str()) {
            if let Ok(v) = axum::http::HeaderValue::from_bytes(v.as_bytes()) {
                headers.insert(h, v);
            }
        }
    }
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    let body = axum::body::Body::from_stream(r.bytes_stream());
    Ok((StatusCode::OK, headers, body).into_response())
}
