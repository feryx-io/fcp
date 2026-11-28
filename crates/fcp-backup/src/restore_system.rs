//! The whole system restored from a point (`docs/SCHEDULED_BACKUPS.md`
//! B3c): the services drained and stopped, the database restored into a
//! new database and swapped in by rename, every volume made as the
//! snapshot holds it, the services started again (and migrating forward).
//!
//! Until the swap nothing is changed: a failure before it drops the new
//! database and starts the services on the old one. After it the old
//! database is kept, renamed, for going back by hand.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use sqlx::Connection;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::process::Command;
use tracing::{error, info, warn};

use crate::config::{BackupConfig, RestoreSection};
use crate::restic::{Repo, Snapshot};

/// One step and where it is.
#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub name: String,
    /// `waiting`, `running`, `done` or `failed`.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A restore of the whole system, as the page follows it.
#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: uuid::Uuid,
    /// `running`, `succeeded` or `failed`.
    pub state: String,
    pub run: String,
    pub point_time: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub steps: Vec<Step>,
    pub error: Option<String>,
    /// The database as it was, renamed, once the restored one is in.
    pub previous_database: Option<String>,
}

pub const STEPS: [&str; 5] = ["check", "stop", "database", "files", "start"];

impl Job {
    pub fn new(run: &str, point_time: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            state: "running".into(),
            run: run.to_string(),
            point_time: point_time.to_string(),
            started_at: now(),
            finished_at: None,
            steps: STEPS
                .iter()
                .map(|n| Step {
                    name: n.to_string(),
                    state: "waiting".into(),
                    detail: None,
                })
                .collect(),
            error: None,
            previous_database: None,
        }
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The job, shared with the API, and kept in `work_dir/restore.json` so it
/// is there to read after the admin comes back.
#[derive(Clone)]
pub struct Tracker {
    pub job: Arc<Mutex<Job>>,
    file: PathBuf,
}

impl Tracker {
    pub fn new(job: Job, work_dir: &Path) -> Self {
        let t = Self {
            job: Arc::new(Mutex::new(job)),
            file: work_dir.join("restore.json"),
        };
        t.save();
        t
    }

    fn update(&self, f: impl FnOnce(&mut Job)) {
        {
            let mut j = self.job.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut j);
        }
        self.save();
    }

    fn save(&self) {
        let j = self.job.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Ok(json) = serde_json::to_vec_pretty(&j) {
            let _ = std::fs::write(&self.file, json);
        }
    }

    fn step(&self, name: &str, state: &str, detail: Option<String>) {
        self.update(|j| {
            if let Some(s) = j.steps.iter_mut().find(|s| s.name == name) {
                s.state = state.into();
                s.detail = detail;
            }
        });
    }

    fn finish(&self, outcome: Result<(), String>) {
        self.update(|j| {
            j.finished_at = Some(now());
            match outcome {
                Ok(()) => j.state = "succeeded".into(),
                Err(e) => {
                    j.state = "failed".into();
                    j.error = Some(e);
                }
            }
        });
    }
}

/// The last restore written down, if any.
pub fn load_last(work_dir: &Path) -> Option<serde_json::Value> {
    let bytes = std::fs::read(work_dir.join("restore.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

// ── The services ───────────────────────────────────────────────────────────

/// What stops and starts the other services.
pub enum Services {
    /// The compose project's containers, through the Docker socket.
    Docker { socket: PathBuf },
    /// The operator's commands.
    Commands {
        stop: Vec<String>,
        start: Vec<String>,
    },
}

impl Services {
    pub fn of(config: &BackupConfig) -> Option<Self> {
        let r = &config.restore;
        if !r.stop_commands.is_empty() || !r.start_commands.is_empty() {
            return Some(Self::Commands {
                stop: r.stop_commands.clone(),
                start: r.start_commands.clone(),
            });
        }
        let socket = config.backup.docker_socket.clone();
        socket.exists().then_some(Self::Docker { socket })
    }
}

async fn docker(socket: &Path, method: &str, path: &str) -> Result<String> {
    // curl speaks HTTP over the socket; the image has it for its health check.
    let out = Command::new("curl")
        .args(["-sS", "--fail-with-body", "--max-time", "600", "-X", method])
        .arg("--unix-socket")
        .arg(socket)
        .arg(format!("http://localhost{path}"))
        .stdin(Stdio::null())
        .output()
        .await
        .context("curl")?;
    let body = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        bail!(
            "Docker {method} {path}: {} {}",
            body.trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(body)
}

/// This container's compose project, from its own labels.
async fn own_project(socket: &Path) -> Result<String> {
    // Compose makes a container's hostname its id; the compose file names
    // this one `fcp-backup`, for a hostname set otherwise.
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|h| h.trim().to_string())
        .unwrap_or_default();
    let mut last = None;
    for name in [host.as_str(), "fcp-backup"]
        .into_iter()
        .filter(|n| !n.is_empty())
    {
        match docker(socket, "GET", &format!("/containers/{name}/json")).await {
            Ok(body) => {
                let me: serde_json::Value = serde_json::from_str(&body)?;
                return me["Config"]["Labels"]["com.docker.compose.project"]
                    .as_str()
                    .map(str::to_string)
                    .context("this container is not part of a compose project");
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no container to find this one by")))
        .context("finding this container's compose project; set [restore] compose_project")
}

/// A running container to stop and start again.
#[derive(Debug, Clone)]
pub struct Stopped {
    pub id: String,
    pub service: String,
}

async fn stop_services(services: &Services, restore: &RestoreSection) -> Result<Vec<Stopped>> {
    match services {
        Services::Commands { stop, .. } => {
            for c in stop {
                shell(c).await?;
            }
            Ok(Vec::new())
        }
        Services::Docker { socket } => {
            let project = match &restore.compose_project {
                Some(p) => p.clone(),
                None => own_project(socket).await?,
            };
            let filter =
                serde_json::json!({ "label": [format!("com.docker.compose.project={project}")] });
            let list: Vec<serde_json::Value> = serde_json::from_str(
                &docker(
                    socket,
                    "GET",
                    &format!(
                        "/containers/json?filters={}",
                        urlencode(&filter.to_string())
                    ),
                )
                .await?,
            )?;
            let mut stopped = Vec::new();
            let running: Vec<Stopped> = list
                .iter()
                .filter_map(|c| {
                    let service = c["Labels"]["com.docker.compose.service"]
                        .as_str()?
                        .to_string();
                    (!restore.keep_running.contains(&service)).then(|| Stopped {
                        id: c["Id"].as_str().unwrap_or_default().to_string(),
                        service,
                    })
                })
                .collect();
            // All at once: each drains for up to `drain_secs`.
            let mut handles = Vec::new();
            for c in running {
                let socket = socket.clone();
                let t = restore.drain_secs;
                handles.push(tokio::spawn(async move {
                    let r =
                        docker(&socket, "POST", &format!("/containers/{}/stop?t={t}", c.id)).await;
                    (c, r)
                }));
            }
            let mut failed = Vec::new();
            for h in handles {
                let (c, r) = h.await.context("stopping")?;
                match r {
                    Ok(_) => stopped.push(c),
                    Err(e) => failed.push(format!("{}: {e:#}", c.service)),
                }
            }
            if !failed.is_empty() {
                // What did stop is started again by the caller.
                start_services(services, &stopped).await.ok();
                bail!("could not stop {}", failed.join("; "));
            }
            Ok(stopped)
        }
    }
}

async fn start_services(services: &Services, stopped: &[Stopped]) -> Result<()> {
    match services {
        Services::Commands { start, .. } => {
            for c in start {
                shell(c).await?;
            }
            Ok(())
        }
        Services::Docker { socket } => {
            let mut failed = Vec::new();
            for c in stopped {
                if let Err(e) = docker(socket, "POST", &format!("/containers/{}/start", c.id)).await
                {
                    failed.push(format!("{}: {e:#}", c.service));
                }
            }
            if failed.is_empty() {
                Ok(())
            } else {
                bail!("could not start {}", failed.join("; "))
            }
        }
    }
}

async fn shell(command: &str) -> Result<()> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| command.to_string())?;
    if !out.status.success() {
        bail!(
            "`{command}` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ── The database ───────────────────────────────────────────────────────────

/// The database's name and a URL to the server's `postgres` database.
pub fn database_parts(url: &str) -> Result<(String, String)> {
    let mut u = url::Url::parse(url).context("[storage] database_url")?;
    let name = u.path().trim_start_matches('/').to_string();
    if name.is_empty() {
        bail!("[storage] database_url names no database");
    }
    u.set_path("/postgres");
    Ok((name, u.to_string()))
}

fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `snapshot`'s `/fcp.dump` into a new database `target`.
async fn restore_into(
    repo: &Repo,
    restore: &RestoreSection,
    snapshot: &str,
    admin_url: &str,
    db_url: &str,
    target: &str,
) -> Result<()> {
    let mut conn = sqlx::PgConnection::connect(admin_url)
        .await
        .context("the server's postgres database")?;
    sqlx::query(&format!("CREATE DATABASE {}", ident(target)))
        .execute(&mut conn)
        .await
        .context("CREATE DATABASE")?;
    let mut target_url = url::Url::parse(db_url)?;
    target_url.set_path(&format!("/{target}"));
    let mut dump = repo
        .command(&["dump", snapshot, "/fcp.dump"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("restic dump")?;
    let out = dump.stdout.take().context("restic's output")?;
    let out: Stdio = out.try_into().context("piping the dump")?;
    let restored = Command::new(&restore.pg_restore)
        .args([
            "--no-owner",
            "--no-privileges",
            "--exit-on-error",
            "--dbname",
        ])
        .arg(target_url.as_str())
        .stdin(out)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .context("pg_restore")?;
    let dumped = dump.wait_with_output().await.context("restic dump")?;
    if !dumped.status.success() {
        bail!(
            "restic dump failed: {}",
            String::from_utf8_lossy(&dumped.stderr).trim()
        );
    }
    if !restored.status.success() {
        bail!(
            "pg_restore failed: {}",
            String::from_utf8_lossy(&restored.stderr).trim()
        );
    }
    Ok(())
}

async fn drop_database(admin_url: &str, name: &str) {
    if let Ok(mut conn) = sqlx::PgConnection::connect(admin_url).await {
        let _ = sqlx::query(&format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            ident(name)
        ))
        .execute(&mut conn)
        .await;
    }
}

/// `restored` becomes `name`; `name` becomes `previous`.
async fn swap(admin_url: &str, name: &str, restored: &str, previous: &str) -> Result<()> {
    let mut conn = sqlx::PgConnection::connect(admin_url).await?;
    sqlx::query(&format!(
        "ALTER DATABASE {} ALLOW_CONNECTIONS false",
        ident(name)
    ))
    .execute(&mut conn)
    .await
    .context("closing the database to new connections")?;
    let swapped = async {
        sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()")
            .bind(name)
            .execute(&mut conn)
            .await?;
        // A terminated backend takes a moment to go.
        for _ in 0..50 {
            let left: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = $1")
                .bind(name)
                .fetch_one(&mut conn)
                .await?;
            if left == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        sqlx::query(&format!("ALTER DATABASE {} RENAME TO {}", ident(name), ident(previous)))
            .execute(&mut conn)
            .await
            .context("renaming the database aside")?;
        sqlx::query(&format!("ALTER DATABASE {} RENAME TO {}", ident(restored), ident(name)))
            .execute(&mut conn)
            .await
            .context("renaming the restored database in")?;
        anyhow::Ok(())
    }
    .await;
    // Whichever name the old one has now, it takes connections again.
    for db in [name, previous] {
        let _ = sqlx::query(&format!(
            "ALTER DATABASE {} ALLOW_CONNECTIONS true",
            ident(db)
        ))
        .execute(&mut conn)
        .await;
    }
    swapped
}

// ── The restore ────────────────────────────────────────────────────────────

/// What a restore is asked to do, checked before anything is stopped.
pub struct Plan {
    pub database_snapshot: String,
    pub files: Option<Snapshot>,
    /// The master key to write into the restored key ring, when the
    /// snapshot does not hold one.
    pub master_key: Option<String>,
}

/// Run it, step by step, into `tracker`.
pub async fn run(
    config: BackupConfig,
    repo: Repo,
    plan: Plan,
    services: Services,
    tracker: Tracker,
) {
    let outcome = carry_out(&config, &repo, &plan, &services, &tracker).await;
    match &outcome {
        Ok(()) => info!("Backups: the system was restored"),
        Err(e) => error!(error = %e, "Backups: the restore failed"),
    }
    tracker.finish(outcome);
}

async fn carry_out(
    config: &BackupConfig,
    repo: &Repo,
    plan: &Plan,
    services: &Services,
    tracker: &Tracker,
) -> Result<(), String> {
    tracker.step("check", "done", None);
    let (name, admin_url) =
        database_parts(&config.storage.database_url).map_err(|e| format!("{e:#}"))?;
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let restored = format!("{name}_restore_{stamp}");
    let previous = format!("{name}_before_{stamp}");

    tracker.step(
        "stop",
        "running",
        Some(format!(
            "draining for up to {} s",
            config.restore.drain_secs
        )),
    );
    let stopped = match stop_services(services, &config.restore).await {
        Ok(s) => s,
        Err(e) => {
            tracker.step("stop", "failed", Some(format!("{e:#}")));
            return Err(format!("nothing was changed: {e:#}"));
        }
    };
    tracker.step(
        "stop",
        "done",
        Some(if stopped.is_empty() {
            "stopped".into()
        } else {
            stopped
                .iter()
                .map(|s| s.service.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }),
    );

    // The database, into a new one and swapped in.
    tracker.step("database", "running", Some(format!("into {restored}")));
    let db = async {
        restore_into(
            repo,
            &config.restore,
            &plan.database_snapshot,
            &admin_url,
            &config.storage.database_url,
            &restored,
        )
        .await?;
        swap(&admin_url, &name, &restored, &previous).await
    }
    .await;
    if let Err(e) = db {
        drop_database(&admin_url, &restored).await;
        tracker.step("database", "failed", Some(format!("{e:#}")));
        let started = start_services(services, &stopped).await;
        tracker.step(
            "start",
            if started.is_ok() { "done" } else { "failed" },
            started.err().map(|e| format!("{e:#}")),
        );
        return Err(format!(
            "nothing was changed, and the services are started again: {e:#}"
        ));
    }
    tracker.update(|j| j.previous_database = Some(previous.clone()));
    tracker.step(
        "database",
        "done",
        Some(format!("the old database kept as {previous}")),
    );

    // The volumes, as the snapshot holds them.
    tracker.step("files", "running", None);
    let files = restore_volumes(config, repo, plan).await;
    match &files {
        Ok(n) => tracker.step("files", "done", Some(format!("{n} volume(s)"))),
        Err(e) => tracker.step("files", "failed", Some(format!("{e:#}"))),
    }

    tracker.step("start", "running", None);
    let started = start_services(services, &stopped).await;
    tracker.step(
        "start",
        if started.is_ok() { "done" } else { "failed" },
        started.as_ref().err().map(|e| format!("{e:#}")),
    );
    match (files, started) {
        (Ok(_), Ok(())) => Ok(()),
        (Err(e), _) => Err(format!(
            "the database was restored, the files were not: {e:#} (the old database is {previous})"
        )),
        (Ok(_), Err(e)) => Err(format!("restored, but {e:#}")),
    }
}

async fn restore_volumes(config: &BackupConfig, repo: &Repo, plan: &Plan) -> Result<usize> {
    let Some(files) = &plan.files else {
        return Ok(0);
    };
    let key_ring = config.backup.key_ring.clone();
    let mut n = 0;
    for (volume, path) in files.volumes() {
        let path = PathBuf::from(&path);
        let holds_key = key_ring.as_ref().is_some_and(|k| k.starts_with(&path));
        // The certs volume keeps a key the snapshot left out; every other
        // volume is made exactly as the snapshot holds it.
        let mut args: Vec<String> = vec![
            "restore".into(),
            files.id.clone(),
            "--target".into(),
            "/".into(),
            "--include".into(),
            path.display().to_string(),
            "--overwrite".into(),
            "always".into(),
        ];
        if !holds_key {
            args.push("--delete".into());
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        repo.run(&refs)
            .await
            .with_context(|| format!("restoring {volume}"))?;
        n += 1;
    }
    if let (Some(key), Some(k)) = (&plan.master_key, &key_ring) {
        if let Some(parent) = k.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(k, format!("{}\n", key.trim()))
            .await
            .context("writing the master key")?;
        warn!("Backups: the master key was written from the key file given");
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    /// The Docker path against a compose project of two containers: the
    /// one kept running is left alone, the other drained, stopped and
    /// started again. Needs Docker (`FCP_DOCKER_TESTS=1`).
    #[tokio::test]
    async fn the_compose_services_are_stopped_and_started_through_the_socket() {
        if std::env::var("FCP_DOCKER_TESTS").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("compose.yml"),
            "services:\n  postgres:\n    image: alpine:3.20\n    command: sleep 600\n  worker:\n    image: alpine:3.20\n    command: sleep 600\n",
        )
        .unwrap();
        let project = format!("fcpbk{}", std::process::id());
        let compose = |args: &[&str]| {
            let ok = std::process::Command::new("docker")
                .args(["compose", "-p", &project, "-f"])
                .arg(dir.path().join("compose.yml"))
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "docker compose {args:?}");
        };
        compose(&["up", "-d"]);
        let socket = std::path::PathBuf::from("/var/run/docker.sock");
        let restore = super::RestoreSection {
            drain_secs: 1,
            keep_running: vec!["postgres".into()],
            compose_project: Some(project.clone()),
            ..Default::default()
        };
        let services = super::Services::Docker {
            socket: socket.clone(),
        };
        let running = |service: &str| {
            let out = std::process::Command::new("docker")
                .args(["compose", "-p", &project, "-f"])
                .arg(dir.path().join("compose.yml"))
                .args(["ps", "-q", "--status", "running", service])
                .output()
                .unwrap();
            !String::from_utf8_lossy(&out.stdout).trim().is_empty()
        };
        let stopped = super::stop_services(&services, &restore).await.unwrap();
        assert_eq!(
            stopped
                .iter()
                .map(|s| s.service.as_str())
                .collect::<Vec<_>>(),
            ["worker"]
        );
        assert!(!running("worker"));
        assert!(running("postgres"));
        super::start_services(&services, &stopped).await.unwrap();
        assert!(running("worker"));
        compose(&["down", "-t", "0"]);
    }

    #[test]
    fn the_database_and_the_server_are_named_from_the_url() {
        let (name, admin) =
            super::database_parts("postgresql://fcp:pw@postgres:5432/fcp?sslmode=disable").unwrap();
        assert_eq!(name, "fcp");
        assert_eq!(
            admin,
            "postgresql://fcp:pw@postgres:5432/postgres?sslmode=disable"
        );
        assert!(super::database_parts("postgresql://fcp@postgres:5432").is_err());
        assert_eq!(super::ident("a\"b"), "\"a\"\"b\"");
    }
}
