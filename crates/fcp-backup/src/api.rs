//! `fcp-backup`'s own API, for the admin (`docs/SCHEDULED_BACKUPS.md` B3a):
//! a destination's snapshots, and a snapshot made a download in
//! `backup.sh`'s layout, which `restore.sh` restores on any host. Every
//! route needs `[http] token`.

use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tracing::{info, warn};

use crate::restic::Snapshot;
use crate::runner::{self, Ctx};

/// How long a built download waits to be fetched.
const DOWNLOAD_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone)]
pub struct ApiState {
    pub ctx: Arc<Ctx>,
    pub token: String,
    pub downloads: Arc<Mutex<HashMap<uuid::Uuid, Download>>>,
    /// The restore of the whole system under way or last run (B3c).
    pub restore: Arc<Mutex<Option<crate::restore_system::Tracker>>>,
}

pub struct Download {
    pub token: String,
    pub state: DownloadState,
    pub dir: PathBuf,
    pub made: Instant,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DownloadState {
    Building,
    Ready { file: String, size: u64 },
    Failed { error: String },
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/v1/destinations/:id/snapshots", get(snapshots))
        .route("/v1/downloads", post(start_download))
        .route("/v1/downloads/:id", get(download_state))
        .route("/v1/downloads/:id/file", get(download_file))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/files/check", post(check_files))
        .route("/v1/files/find", post(find_files))
        .route("/v1/restores/files", post(restore_files))
        .route("/v1/restores/system", post(restore_system))
        .route("/v1/restores/system/current", get(restore_state))
        .with_state(state)
}

fn authorized(state: &ApiState, headers: &HeaderMap) -> Result<(), Refused> {
    let given = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !state.token.is_empty() && constant_eq(given.as_bytes(), state.token.as_bytes()) {
        Ok(())
    } else {
        Err(Refused(
            StatusCode::UNAUTHORIZED,
            "a bearer token is needed".into(),
        ))
    }
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn error(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// A refusal, small enough to return (`Response` is not).
struct Refused(StatusCode, String);

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        error(self.0, self.1)
    }
}

/// A run's two snapshots, as one restorable point.
#[derive(Debug, Serialize)]
pub struct Point {
    /// The run that made it (`bkr-<uuid>`, or the tag's value).
    pub run: String,
    pub time: String,
    pub release: Option<String>,
    pub kek_id: Option<String>,
    pub host: String,
    pub database: Option<String>,
    pub files: Option<String>,
    /// The volumes the files hold.
    pub volumes: Vec<String>,
    /// Whether the files hold the master key.
    pub holds_key: Option<bool>,
}

fn points(snapshots: &[Snapshot]) -> Vec<Point> {
    let mut by_run: Vec<Point> = Vec::new();
    for s in snapshots {
        let Some(run) = s.tag("run") else { continue };
        let i = match by_run.iter().position(|p| p.run == run) {
            Some(i) => i,
            None => {
                by_run.push(Point {
                    run: run.to_string(),
                    time: s.time.clone(),
                    release: s.tag("release").map(str::to_string),
                    kek_id: s.tag("kek").map(str::to_string),
                    host: s.hostname.clone(),
                    database: None,
                    files: None,
                    volumes: Vec::new(),
                    holds_key: None,
                });
                by_run.len() - 1
            }
        };
        let p = &mut by_run[i];
        if s.tags.iter().any(|t| t == "part:db") {
            p.database = Some(s.id.clone());
        }
        if s.tags.iter().any(|t| t == "part:files") {
            p.files = Some(s.id.clone());
            p.volumes = s.volumes().into_iter().map(|(n, _)| n).collect();
        }
    }
    by_run.sort_by(|a, b| b.time.cmp(&a.time));
    by_run
}

async fn destination(
    state: &ApiState,
    id: &str,
) -> Result<fcp_core::backup::BackupDestination, Refused> {
    let id: uuid::Uuid = id
        .trim_start_matches("bkd-")
        .parse()
        .map_err(|_| Refused(StatusCode::NOT_FOUND, "no such destination".into()))?;
    match state.ctx.store.get_backup_destination(&id).await {
        Ok(Some(d)) => Ok(d),
        Ok(None) => Err(Refused(StatusCode::NOT_FOUND, "no such destination".into())),
        Err(e) => Err(Refused(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))),
    }
}

async fn snapshots(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    let mut dest = match destination(&state, &id).await {
        Ok(d) => d,
        Err(r) => return r.into_response(),
    };
    let repo = match runner::repo_for(&state.ctx, &mut dest, false).await {
        Ok(r) => r,
        Err(e) => return error(StatusCode::CONFLICT, format!("{e:#}")),
    };
    match repo.snapshots().await {
        Ok(list) => Json(points(&list)).into_response(),
        Err(e) => error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

#[derive(Debug, Deserialize)]
struct DownloadRequest {
    destination_id: String,
    /// The run whose snapshots are put together.
    run: String,
    /// The master key (base64), when the snapshot does not hold it and this
    /// system's key is another; from the key file the admin was given.
    #[serde(default)]
    master_key: Option<String>,
}

async fn start_download(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<DownloadRequest>,
) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    let dest = match destination(&state, &req.destination_id).await {
        Ok(d) => d,
        Err(r) => return r.into_response(),
    };
    // Old downloads go.
    let expired: Vec<PathBuf> = {
        let mut map = state.downloads.lock().unwrap_or_else(|e| e.into_inner());
        let gone: Vec<uuid::Uuid> = map
            .iter()
            .filter(|(_, d)| d.made.elapsed() > DOWNLOAD_TTL)
            .map(|(id, _)| *id)
            .collect();
        gone.into_iter()
            .filter_map(|id| map.remove(&id))
            .map(|d| d.dir)
            .collect()
    };
    for dir in expired {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
    let id = uuid::Uuid::new_v4();
    let token = {
        let mut b = [0u8; 24];
        getrandom(&mut b);
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    };
    let dir = state
        .ctx
        .config
        .backup
        .work_dir
        .join("downloads")
        .join(id.to_string());
    state
        .downloads
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            id,
            Download {
                token: token.clone(),
                state: DownloadState::Building,
                dir: dir.clone(),
                made: Instant::now(),
            },
        );
    let s = state.clone();
    tokio::spawn(async move {
        let outcome = build(&s.ctx, dest, &req.run, req.master_key, &dir).await;
        let next = match outcome {
            Ok((file, size)) => {
                info!(run = %req.run, size, "Backups: a download is ready");
                DownloadState::Ready { file, size }
            }
            Err(e) => {
                warn!(run = %req.run, error = %format!("{e:#}"), "Backups: a download could not be built");
                let _ = tokio::fs::remove_dir_all(&dir).await;
                DownloadState::Failed {
                    error: format!("{e:#}"),
                }
            }
        };
        if let Some(d) = s
            .downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&id)
        {
            d.state = next;
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "id": id, "token": token, "state": "building" })),
    )
        .into_response()
}

/// Randomness for a download's token.
fn getrandom(buf: &mut [u8]) {
    for chunk in buf.chunks_mut(16) {
        let u = uuid::Uuid::new_v4();
        let bytes = u.as_bytes();
        for (d, s) in chunk.iter_mut().zip(bytes.iter()) {
            *d = *s;
        }
    }
}

async fn download_state(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    match state
        .downloads
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
    {
        Some(d) => Json(d.state.clone()).into_response(),
        None => error(StatusCode::NOT_FOUND, "no such download".into()),
    }
}

#[derive(Deserialize)]
struct FileQuery {
    token: String,
}

/// The built download, streamed once and removed after.
async fn download_file(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<FileQuery>,
) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    let (file, dir) = {
        let mut map = state.downloads.lock().unwrap_or_else(|e| e.into_inner());
        let Some(d) = map.get(&id) else {
            return error(StatusCode::NOT_FOUND, "no such download".into());
        };
        if !constant_eq(d.token.as_bytes(), q.token.as_bytes()) {
            return error(StatusCode::NOT_FOUND, "no such download".into());
        }
        let DownloadState::Ready { file, .. } = &d.state else {
            return error(StatusCode::CONFLICT, "the download is not ready".into());
        };
        let out = (d.dir.join(file), d.dir.clone(), file.clone());
        map.remove(&id);
        (out.0, (out.1, out.2))
    };
    let handle = match tokio::fs::File::open(&file).await {
        Ok(f) => f,
        Err(e) => return error(StatusCode::GONE, format!("{e}")),
    };
    let size = handle.metadata().await.map(|m| m.len()).unwrap_or(0);
    let (dir, name) = dir;
    // Removed once read through (or after the TTL by the next download).
    let stream = tokio_util::io::ReaderStream::new(handle);
    let cleanup = Cleanup(Some(dir));
    let body = Body::from_stream(futures_util::StreamExt::map(stream, move |chunk| {
        let _keep = &cleanup;
        chunk
    }));
    (
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/x-tar".to_string(),
            ),
            (axum::http::header::CONTENT_LENGTH, size.to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        body,
    )
        .into_response()
}

/// Removes a download's directory when the stream that read it is done.
struct Cleanup(Option<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

async fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.output().await.with_context(|| what.to_string())?;
    if !out.status.success() {
        bail!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Put a run's snapshots together in `backup.sh`'s layout under `dir`, and
/// tar it: the file's name and size.
async fn build(
    ctx: &Ctx,
    mut dest: fcp_core::backup::BackupDestination,
    run_id: &str,
    master_key: Option<String>,
    dir: &std::path::Path,
) -> Result<(String, u64)> {
    let run_id = run_id.trim_start_matches("bkr-");
    let repo = runner::repo_for(ctx, &mut dest, false).await?;
    let all = repo.snapshots().await?;
    let point = points(&all)
        .into_iter()
        .find(|p| p.run == run_id)
        .context("no such snapshot")?;
    let db = point
        .database
        .clone()
        .context("the snapshot holds no database")?;
    let files = all.iter().find(|s| Some(&s.id) == point.files.as_ref());
    let stamp = point
        .time
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(14)
        .collect::<String>();
    let name = format!("fcp-backup-{stamp}");
    let bundle = dir.join(&name);
    tokio::fs::create_dir_all(&bundle).await?;
    repo.dump_to(&db, "/fcp.dump", &bundle.join("fcp.dump"))
        .await?;

    // The files restored apart, each volume tarred from its root.
    let restored = dir.join("restore");
    let mut taken: Vec<String> = Vec::new();
    let mut absent: Vec<String> = Vec::new();
    let volumes = files.map(Snapshot::volumes).unwrap_or_default();
    if let Some(files) = files {
        tokio::fs::create_dir_all(&restored).await?;
        repo.restore(&files.id, &restored, &[], true).await?;
    }
    let key_name = ctx
        .config
        .backup
        .key_ring
        .as_ref()
        .and_then(|k| k.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "master.key".into());
    let mut certs_seen = false;
    for (vol, path) in &volumes {
        let src = restored.join(path.trim_start_matches('/'));
        if !src.exists() {
            absent.push(vol.clone());
            continue;
        }
        if vol == "fcp-certs" {
            certs_seen = true;
            put_key(
                ctx,
                &src.join(&key_name),
                master_key.as_deref(),
                point.kek_id.as_deref(),
            )
            .await?;
        }
        run(
            Command::new("tar")
                .arg("-C")
                .arg(&src)
                .arg("-czf")
                .arg(bundle.join(format!("{vol}.tar.gz")))
                .arg("."),
            "tar",
        )
        .await?;
        taken.push(vol.clone());
    }
    // restore.sh needs the key: a certs volume of the key alone when the
    // snapshot had none.
    if !certs_seen {
        let certs = dir.join("certs-only");
        tokio::fs::create_dir_all(&certs).await?;
        put_key(
            ctx,
            &certs.join(&key_name),
            master_key.as_deref(),
            point.kek_id.as_deref(),
        )
        .await?;
        run(
            Command::new("tar")
                .arg("-C")
                .arg(&certs)
                .arg("-czf")
                .arg(bundle.join("fcp-certs.tar.gz"))
                .arg("."),
            "tar",
        )
        .await?;
        taken.push("fcp-certs".into());
    }
    let manifest = format!(
        "fcp_backup = 1\ntaken_at = {}\nstack_version = {}\nstack_version_from = snapshot\n\
         project = {}\ndatabase = fcp.dump\nkek_id = {}\nvolumes ={}\nvolumes_absent ={}\n",
        point.time,
        point.release.clone().unwrap_or_default(),
        point.host,
        point.kek_id.clone().unwrap_or_default(),
        taken.iter().map(|v| format!(" {v}")).collect::<String>(),
        absent.iter().map(|v| format!(" {v}")).collect::<String>(),
    );
    tokio::fs::write(bundle.join("manifest"), manifest).await?;
    run(
        Command::new("sh")
            .arg("-c")
            .arg("sha256sum fcp.dump manifest *.tar.gz > SHA256SUMS")
            .current_dir(&bundle),
        "sha256sum",
    )
    .await?;
    let _ = tokio::fs::remove_dir_all(&restored).await;
    let _ = tokio::fs::remove_dir_all(dir.join("certs-only")).await;
    let file = format!("{name}.tar");
    run(
        Command::new("tar")
            .arg("-C")
            .arg(dir)
            .arg("-cf")
            .arg(dir.join(&file))
            .arg(&name),
        "tar",
    )
    .await?;
    let _ = tokio::fs::remove_dir_all(&bundle).await;
    let size = tokio::fs::metadata(dir.join(&file)).await?.len();
    Ok((file, size))
}

/// The master key into a restored certs directory: kept if the snapshot
/// had it, else the one given, else this system's own when its KEK is the
/// snapshot's; else the download cannot be made.
async fn put_key(
    _ctx: &Ctx,
    at: &std::path::Path,
    given: Option<&str>,
    kek_id: Option<&str>,
) -> Result<()> {
    if at.exists() {
        return Ok(());
    }
    let own = fcp_crypto::EncryptionSettings::from_env()
        .ok()
        .and_then(|s| s.exportable_master_key());
    let key = match (given, own) {
        (Some(k), _) => k.trim().to_string(),
        (None, Some((k, id))) if Some(id.as_str()) == kek_id || kek_id.is_none() => k,
        _ => bail!(
            "needs the master key: the snapshot does not hold it and this system's key is another (KEK {})",
            kek_id.unwrap_or("unknown")
        ),
    };
    if let Some(parent) = at.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(at, format!("{key}\n")).await?;
    Ok(())
}

// ── File restores (B3b) ────────────────────────────────────────────────────

async fn capabilities(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    let mut caps = crate::files::capabilities(&state.ctx.config.backup);
    caps.full_restore =
        caps.file_restores && crate::restore_system::Services::of(&state.ctx.config).is_some();
    Json(caps).into_response()
}

#[derive(Debug, Deserialize)]
struct PathsRequest {
    paths: Vec<String>,
}

/// Whether each path (as its service wrote it) is here.
async fn check_files(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<PathsRequest>,
) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    Json(crate::files::check(&state.ctx.config.backup, &req.paths)).into_response()
}

#[derive(Debug, Deserialize)]
struct FindRequest {
    destination_id: String,
    paths: Vec<String>,
}

/// The snapshots that hold each path, newest first.
async fn find_files(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<FindRequest>,
) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    let mut dest = match destination(&state, &req.destination_id).await {
        Ok(d) => d,
        Err(r) => return r.into_response(),
    };
    let repo = match runner::repo_for(&state.ctx, &mut dest, false).await {
        Ok(r) => r,
        Err(e) => return error(StatusCode::CONFLICT, format!("{e:#}")),
    };
    match crate::files::find(&repo, &state.ctx.config.backup, &req.paths).await {
        Ok(found) => Json(found).into_response(),
        Err(e) => error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

#[derive(Debug, Deserialize)]
struct RestoreFilesRequest {
    destination_id: String,
    /// restic's id of a files snapshot.
    snapshot: String,
    /// Paths as their services wrote them (a tenant's rows)…
    #[serde(default)]
    service_paths: Vec<String>,
    /// …or a volume and paths in it (a platform administrator's); no
    /// paths is the whole volume.
    #[serde(default)]
    volume: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Restored {
    path: String,
    /// Whether it is there now.
    present: bool,
}

/// Put files back beside what is there: nothing present is overwritten.
async fn restore_files(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<RestoreFilesRequest>,
) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    let config = &state.ctx.config.backup;
    let caps = crate::files::capabilities(config);
    if !caps.file_restores {
        return error(
            StatusCode::CONFLICT,
            "restores are off: start fcp-backup with docker-compose.restore.yml".into(),
        );
    }
    // (what the caller named, where it is here)
    let mut targets: Vec<(String, std::path::PathBuf)> = Vec::new();
    for p in &req.service_paths {
        match crate::files::map(config, p) {
            Some(here) => targets.push((p.clone(), here)),
            None => return error(StatusCode::BAD_REQUEST, format!("{p} is not held here")),
        }
    }
    if let Some(volume) = &req.volume {
        let rels = if req.paths.is_empty() {
            vec![String::new()]
        } else {
            req.paths.clone()
        };
        for rel in rels {
            match crate::files::in_volume(config, volume, &rel) {
                Ok(here) => targets.push((format!("{volume}:{rel}"), here)),
                Err(e) => return error(StatusCode::BAD_REQUEST, format!("{e:#}")),
            }
        }
    }
    if targets.is_empty() {
        return error(StatusCode::BAD_REQUEST, "nothing to restore".into());
    }
    let mut dest = match destination(&state, &req.destination_id).await {
        Ok(d) => d,
        Err(r) => return r.into_response(),
    };
    let repo = match runner::repo_for(&state.ctx, &mut dest, false).await {
        Ok(r) => r,
        Err(e) => return error(StatusCode::CONFLICT, format!("{e:#}")),
    };
    let snapshot = req.snapshot.trim();
    if snapshot.is_empty() || !snapshot.chars().all(|c| c.is_ascii_hexdigit()) {
        return error(StatusCode::BAD_REQUEST, "no such snapshot".into());
    }
    let here: Vec<std::path::PathBuf> = targets.iter().map(|(_, h)| h.clone()).collect();
    match crate::files::restore(&repo, snapshot, &here).await {
        Ok(after) => {
            info!(snapshot, count = targets.len(), "Backups: files put back");
            let out: Vec<Restored> = targets
                .iter()
                .zip(after)
                .map(|((named, _), (_, present))| Restored {
                    path: named.clone(),
                    present,
                })
                .collect();
            Json(out).into_response()
        }
        Err(e) => error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

// ── The whole system (B3c) ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SystemRestoreRequest {
    destination_id: String,
    run: String,
    /// The point's time, typed back.
    confirm: String,
    /// The master key, when the snapshot holds none and this system's is
    /// another.
    #[serde(default)]
    master_key: Option<String>,
}

/// Restore the whole system from a point: checked here, then run in the
/// background; follow it at `/v1/restores/system/current`.
async fn restore_system(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<SystemRestoreRequest>,
) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    let config = state.ctx.config.clone();
    let caps = crate::files::capabilities(&config.backup);
    let Some(services) = crate::restore_system::Services::of(&config) else {
        return error(
            StatusCode::CONFLICT,
            "restores of the whole system are off: start fcp-backup with docker-compose.restore.yml".into(),
        );
    };
    if !caps.file_restores {
        return error(
            StatusCode::CONFLICT,
            format!(
                "the volumes are read-only here ({})",
                caps.read_only.join(", ")
            ),
        );
    }
    if let Some(t) = state
        .restore
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        if t.job.lock().unwrap_or_else(|e| e.into_inner()).state == "running" {
            return error(StatusCode::CONFLICT, "a restore is under way".into());
        }
    }
    let mut dest = match destination(&state, &req.destination_id).await {
        Ok(d) => d,
        Err(r) => return r.into_response(),
    };
    let repo = match runner::repo_for(&state.ctx, &mut dest, false).await {
        Ok(r) => r,
        Err(e) => return error(StatusCode::CONFLICT, format!("{e:#}")),
    };
    let all = match repo.snapshots().await {
        Ok(a) => a,
        Err(e) => return error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    };
    let run_id = req.run.trim_start_matches("bkr-");
    let Some(point) = points(&all).into_iter().find(|p| p.run == run_id) else {
        return error(StatusCode::NOT_FOUND, "no such snapshot".into());
    };
    if req.confirm.trim() != point.time {
        return error(
            StatusCode::BAD_REQUEST,
            "type the snapshot's time back exactly to confirm".into(),
        );
    }
    let Some(database_snapshot) = point.database.clone() else {
        return error(
            StatusCode::BAD_REQUEST,
            "the snapshot holds no database".into(),
        );
    };
    let files = all
        .iter()
        .find(|s| Some(&s.id) == point.files.as_ref())
        .cloned();
    // The key: the snapshot's own, else the one given, else this system's
    // when it is the snapshot's.
    let holds_key = match (&files, &config.backup.key_ring) {
        (Some(f), Some(k)) => repo
            .run(&["ls", &f.id, &k.display().to_string()])
            .await
            .map(|out| out.lines().any(|l| l.trim() == k.display().to_string()))
            .unwrap_or(false),
        _ => false,
    };
    let own = fcp_crypto::EncryptionSettings::from_env()
        .ok()
        .and_then(|s| s.exportable_master_key());
    let master_key = if holds_key {
        None
    } else if let Some(k) = req.master_key.filter(|k| !k.trim().is_empty()) {
        Some(k)
    } else {
        match (&own, point.kek_id.as_deref()) {
            (Some((_, id)), Some(kek)) if id == kek => None,
            (Some(_), None) => None,
            _ => {
                return error(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "needs the master key: the snapshot does not hold it and this system's key is another (KEK {})",
                        point.kek_id.as_deref().unwrap_or("unknown")
                    ),
                )
            }
        }
    };
    let job = crate::restore_system::Job::new(&format!("bkr-{}", point.run), &point.time);
    let tracker = crate::restore_system::Tracker::new(job.clone(), &config.backup.work_dir);
    *state.restore.lock().unwrap_or_else(|e| e.into_inner()) = Some(tracker.clone());
    warn!(run = %point.run, "Backups: the whole system is being restored");
    tokio::spawn(crate::restore_system::run(
        config,
        repo,
        crate::restore_system::Plan {
            database_snapshot,
            files,
            master_key,
        },
        services,
        tracker,
    ));
    (StatusCode::ACCEPTED, Json(job)).into_response()
}

/// The restore under way, or the last one (kept in `work_dir`).
async fn restore_state(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorized(&state, &headers) {
        return r.into_response();
    }
    if let Some(t) = state
        .restore
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        return Json(t.job.lock().unwrap_or_else(|e| e.into_inner()).clone()).into_response();
    }
    match crate::restore_system::load_last(&state.ctx.config.backup.work_dir) {
        Some(j) => Json(j).into_response(),
        None => error(StatusCode::NOT_FOUND, "no restore has run".into()),
    }
}
