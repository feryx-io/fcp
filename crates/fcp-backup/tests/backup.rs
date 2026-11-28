//! `fcp-backup` against a real SFTP server and a real database
//! (`docs/SCHEDULED_BACKUPS.md` B1): a destination tested (its host key
//! pinned, its repository made), then backed up — the database streamed as
//! one snapshot, the files as another — and what was stored read back with
//! restic as an operator would. An S3-compatible destination too, when
//! `FCP_TEST_S3_*` name one (Garage locally).
//!
//! Needs restic (`FCP_RESTIC`, `~/.cache/fcp-restic/restic` or on the
//! PATH), `/usr/sbin/sshd`, `pg_dump` (`FCP_TEST_PG_BIN` or the PATH) and
//! `FCP_TEST_DATABASE_URL`; skips without them unless
//! `FCP_BACKUP_TESTS=required`.

use fcp_core::backup::{
    BackupDestination, BackupRetention, BackupRun, BackupRunKind, BackupRunState, BackupSchedule,
    BackupStore, BackupTarget,
};
use fcp_testkit::db::TestDatabase;
use fcp_testkit::service::Service;
use fcp_testkit::smoke::free_port;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-backup");

fn required() -> bool {
    std::env::var("FCP_BACKUP_TESTS").as_deref() == Ok("required")
}

fn skip(why: &str) -> Option<()> {
    if required() {
        panic!("FCP_BACKUP_TESTS=required: {why}");
    }
    eprintln!("skipping: {why}");
    None
}

fn restic() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("FCP_RESTIC") {
        return Some(p.into());
    }
    let cached =
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/fcp-restic/restic");
    if cached.exists() {
        return Some(cached);
    }
    Command::new("restic")
        .arg("version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| "restic".into())
}

fn pg_dump() -> PathBuf {
    match std::env::var("FCP_TEST_PG_BIN") {
        Ok(dir) => Path::new(&dir).join("pg_dump"),
        Err(_) => "pg_dump".into(),
    }
}

/// An OpenSSH server of our own on a free port, as this user, serving SFTP
/// from `root`.
struct Sftp {
    child: std::process::Child,
    port: u16,
    user: String,
    client_key: String,
    _dir: tempfile::TempDir,
}

impl Drop for Sftp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn keygen(path: &Path) {
    let ok = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .status()
        .expect("ssh-keygen")
        .success();
    assert!(ok, "ssh-keygen");
}

fn start_sftp() -> Option<Sftp> {
    let sshd = Path::new("/usr/sbin/sshd");
    let sftp_server = [
        "/usr/lib/openssh/sftp-server",
        "/usr/libexec/openssh/sftp-server",
        "/usr/lib/ssh/sftp-server",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists());
    let (true, Some(sftp_server)) = (sshd.exists(), sftp_server) else {
        skip("no sshd or sftp-server here")?;
        return None;
    };
    let dir = tempfile::tempdir().unwrap();
    let host_key = dir.path().join("host_key");
    let client = dir.path().join("client_key");
    keygen(&host_key);
    keygen(&client);
    let authorized = dir.path().join("authorized_keys");
    std::fs::copy(dir.path().join("client_key.pub"), &authorized).unwrap();
    let port = free_port();
    let config = dir.path().join("sshd_config");
    std::fs::write(
        &config,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\n\
             PasswordAuthentication no\nKbdInteractiveAuthentication no\nStrictModes no\n\
             UsePAM no\nPidFile {}\nSubsystem sftp {sftp_server}\n",
            host_key.display(),
            authorized.display(),
            dir.path().join("sshd.pid").display(),
        ),
    )
    .unwrap();
    let child = Command::new(sshd)
        .args(["-D", "-e", "-f"])
        .arg(&config)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("sshd");
    // Listening.
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let user = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_string();
    Some(Sftp {
        child,
        port,
        user,
        client_key: std::fs::read_to_string(client).unwrap(),
        _dir: dir,
    })
}

async fn wait_for(store: &Arc<dyn BackupStore>, id: &uuid::Uuid) -> BackupRun {
    for _ in 0..240 {
        if let Some(r) = store.get_backup_run(id).await.unwrap() {
            if matches!(r.state, BackupRunState::Succeeded | BackupRunState::Failed) {
                return r;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("the run never finished");
}

fn request(dest: uuid::Uuid, kind: BackupRunKind) -> BackupRun {
    BackupRun {
        id: uuid::Uuid::new_v4(),
        destination_id: dest,
        kind,
        state: BackupRunState::Requested,
        trigger: "now".into(),
        requested_at: SystemTime::now(),
        started_at: None,
        finished_at: None,
        snapshots: Vec::new(),
        bytes_added: 0,
        message: None,
        node: None,
    }
}

/// The service, set to back up `files` (and `certs`, its master key left
/// out) from `db`; the port its API answers on.
async fn service(
    db: &TestDatabase,
    restic: &Path,
    files: &Path,
    certs: &Path,
    work: &Path,
) -> (Service, u16) {
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "backup-test"

[http]
bind_address = "127.0.0.1:{http}"
token = "backup-api-token"

[storage]
database_url = "{db}"

[backup]
restic = "{restic}"
pg_dump = "{pg_dump}"
work_dir = "{work}"
poll_secs = 1
key_ring = "{certs}/master.key"

[backup.paths]
recordings = "{files}"
fcp-certs = "{certs}"

[backup.service_paths]
"/srv/recordings" = "{files}"

[restore]
pg_restore = "{pg_restore}"
stop_commands = ["echo stopped >> {work}/services.log"]
start_commands = ["echo started >> {work}/services.log"]

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        restic = restic.display(),
        pg_dump = pg_dump().display(),
        pg_restore = pg_dump().with_file_name("pg_restore").display(),
        work = work.display(),
        files = files.display(),
        certs = certs.display(),
    );
    let svc = Service::start(
        "fcp-backup",
        BIN,
        &config,
        &fcp_testkit::smoke::loopback(http, "/health"),
    )
    .await;
    (svc, http)
}

fn certs_dir() -> tempfile::TempDir {
    let certs = tempfile::tempdir().unwrap();
    std::fs::write(
        certs.path().join("ca.pem"),
        "-----BEGIN CERTIFICATE-----\nca\n",
    )
    .unwrap();
    std::fs::write(certs.path().join("master.key"), "the-running-key\n").unwrap();
    certs
}

/// The database and a recording, backed up over SFTP and read back.
#[tokio::test]
async fn the_database_and_the_files_go_to_an_sftp_server() {
    let Some(restic) = restic() else {
        return skip("no restic").unwrap_or(());
    };
    let Some(db) = TestDatabase::create().await else {
        return skip("FCP_TEST_DATABASE_URL is not set").unwrap_or(());
    };
    let Some(sftp) = start_sftp() else { return };
    let storage = Arc::new(
        fcp_storage_postgres::PostgresStorage::new(fcp_storage_postgres::PostgresConfig::new(
            db.url().to_string(),
        ))
        .await
        .unwrap(),
    );
    let store: Arc<dyn BackupStore> = storage.clone();
    let files = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(files.path().join("tenant-a")).unwrap();
    std::fs::write(
        files.path().join("tenant-a/call-1.wav"),
        b"RIFF recorded audio",
    )
    .unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();

    let now = SystemTime::now();
    let dest = BackupDestination {
        id: uuid::Uuid::new_v4(),
        name: "Off site".into(),
        target: BackupTarget::Sftp {
            host: "127.0.0.1".into(),
            port: sftp.port,
            user: sftp.user.clone(),
            path: repo_dir.path().join("repo").display().to_string(),
            private_key: Some(sftp.client_key.clone()),
            host_key: None,
        },
        password: Some("a long repository password".into()),
        schedule: BackupSchedule {
            daily_at: None,
            cron: None,
        },
        keep: BackupRetention::default(),
        include_key: false,
        enabled: true,
        created_at: now,
        updated_at: now,
    };
    store.put_backup_destination(&dest).await.unwrap();
    let certs = certs_dir();
    let (svc, http) = service(&db, &restic, files.path(), certs.path(), work.path()).await;

    // A backup before the host key is pinned is refused.
    let early = request(dest.id, BackupRunKind::Backup);
    store.request_backup_run(&early).await.unwrap();
    let early = wait_for(&store, &early.id).await;
    assert_eq!(early.state, BackupRunState::Failed, "{early:?}");
    assert!(early
        .message
        .as_deref()
        .unwrap_or("")
        .contains("test the destination first"));

    // The test pins the host key and makes the repository.
    let test = request(dest.id, BackupRunKind::Test);
    store.request_backup_run(&test).await.unwrap();
    let test = wait_for(&store, &test.id).await;
    assert_eq!(test.state, BackupRunState::Succeeded, "{test:?}");
    assert!(
        test.message
            .as_deref()
            .unwrap_or("")
            .contains("repository was made"),
        "{test:?}"
    );
    let pinned = store
        .get_backup_destination(&dest.id)
        .await
        .unwrap()
        .unwrap();
    let BackupTarget::Sftp { host_key, .. } = &pinned.target else {
        unreachable!()
    };
    assert!(
        host_key.as_deref().unwrap_or("").contains("ssh-ed25519"),
        "{host_key:?}"
    );

    // Backed up: two snapshots, the dump and the files.
    let run = request(dest.id, BackupRunKind::Backup);
    store.request_backup_run(&run).await.unwrap();
    let run = wait_for(&store, &run.id).await;
    assert_eq!(run.state, BackupRunState::Succeeded, "{run:?}");
    assert_eq!(run.snapshots.len(), 2, "{run:?}");
    assert!(run.bytes_added > 0);
    assert_eq!(run.node.as_deref(), Some("backup-test"));

    // Read back as an operator would: the dump is a pg_dump archive, the
    // recording is there.
    let known = work.path().join("known_hosts").join(dest.id.to_string());
    let key = work.path().join("keys").join(dest.id.to_string());
    let restic_cmd = |args: &[&str]| {
        let out = Command::new(&restic)
            .env("RESTIC_REPOSITORY", pinned.target.repository())
            .env("RESTIC_PASSWORD", "a long repository password")
            .arg("-o")
            .arg(format!(
                "sftp.command=ssh -i {} -p {} -o UserKnownHostsFile={} -o StrictHostKeyChecking=yes -o BatchMode=yes {}@127.0.0.1 -s sftp",
                key.display(),
                sftp.port,
                known.display(),
                sftp.user
            ))
            .arg("--no-cache")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "restic {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    };
    let dump = restic_cmd(&["dump", &run.snapshots[0], "/fcp.dump"]);
    assert!(dump.starts_with(b"PGDMP"), "a pg_dump archive");
    let listed = String::from_utf8(restic_cmd(&["ls", &run.snapshots[1]])).unwrap();
    assert!(listed.contains("call-1.wav"), "{listed}");
    let snapshots: Vec<serde_json::Value> =
        serde_json::from_slice(&restic_cmd(&["snapshots", "--json"])).unwrap();
    assert_eq!(snapshots.len(), 2);
    assert!(snapshots.iter().all(|s| s["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == &format!("run:{}", run.id))));

    // The snapshot leaves the master key out (Q4) but keeps the rest of
    // the certs volume.
    let listing = String::from_utf8(restic_cmd(&["ls", &run.snapshots[1]])).unwrap();
    assert!(listing.contains("ca.pem"), "{listing}");
    assert!(!listing.contains("master.key"), "{listing}");

    // B3a: the point listed through the service's API, then a download in
    // backup.sh's layout.
    let client = reqwest::Client::new();
    let api = |path: &str| format!("http://127.0.0.1:{http}{path}");
    let unauthorized = client
        .get(api(&format!("/v1/destinations/{}/snapshots", dest.id)))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    let points: serde_json::Value = client
        .get(api(&format!("/v1/destinations/bkd-{}/snapshots", dest.id)))
        .bearer_auth("backup-api-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(points.as_array().map(Vec::len), Some(1), "{points}");
    assert_eq!(points[0]["run"], run.id.to_string());
    assert!(points[0]["volumes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "fcp-certs"));
    let download = |master_key: Option<&str>| {
        let client = client.clone();
        let body = serde_json::json!({
            "destination_id": format!("bkd-{}", dest.id),
            "run": format!("bkr-{}", run.id),
            "master_key": master_key,
        });
        let url = api("/v1/downloads");
        async move {
            let started: serde_json::Value = client
                .post(url)
                .bearer_auth("backup-api-token")
                .json(&body)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let id = started["id"].as_str().unwrap().to_string();
            let token = started["token"].as_str().unwrap().to_string();
            for _ in 0..240 {
                let state: serde_json::Value = client
                    .get(format!("http://127.0.0.1:{http}/v1/downloads/{id}"))
                    .bearer_auth("backup-api-token")
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                if state["state"] != "building" {
                    return (state, id, token);
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            panic!("the download never finished");
        }
    };
    // Without a key: this system has none to lend.
    let (state, _, _) = download(None).await;
    assert_eq!(state["state"], "failed", "{state}");
    assert!(
        state["error"]
            .as_str()
            .unwrap()
            .contains("needs the master key"),
        "{state}"
    );
    // With the key from the file the admin was given.
    let (state, id, token) = download(Some("dGhlLWtleS1naXZlbg==")).await;
    assert_eq!(state["state"], "ready", "{state}");
    let tar = client
        .get(api(&format!("/v1/downloads/{id}/file?token={token}")))
        .bearer_auth("backup-api-token")
        .send()
        .await
        .unwrap();
    assert_eq!(tar.status(), 200);
    let bytes = tar.bytes().await.unwrap();
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("b.tar"), &bytes).unwrap();
    let untar = Command::new("tar")
        .arg("-C")
        .arg(out.path())
        .arg("-xf")
        .arg(out.path().join("b.tar"))
        .output()
        .unwrap();
    assert!(
        untar.status.success(),
        "{} bytes: {} | {:?}",
        bytes.len(),
        String::from_utf8_lossy(&untar.stderr),
        &bytes[..bytes.len().min(120)]
    );
    let bundle = std::fs::read_dir(out.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.is_dir())
        .expect("the bundle's directory");
    // restore.sh's checks: the checksums, the manifest, the key, the volumes.
    let sums = Command::new("sha256sum")
        .arg("-c")
        .arg("SHA256SUMS")
        .current_dir(&bundle)
        .output()
        .unwrap();
    assert!(
        sums.status.success(),
        "{}",
        String::from_utf8_lossy(&sums.stdout)
    );
    let manifest = std::fs::read_to_string(bundle.join("manifest")).unwrap();
    assert!(manifest.contains("fcp_backup = 1"), "{manifest}");
    assert!(manifest.contains("database = fcp.dump"), "{manifest}");
    assert!(
        manifest.contains(" fcp-certs") && manifest.contains(" recordings"),
        "{manifest}"
    );
    assert!(std::fs::read(bundle.join("fcp.dump"))
        .unwrap()
        .starts_with(b"PGDMP"));
    let key = Command::new("tar")
        .arg("-xzOf")
        .arg(bundle.join("fcp-certs.tar.gz"))
        .arg("./master.key")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&key.stdout).trim(),
        "dGhlLWtleS1naXZlbg=="
    );
    let rec = Command::new("tar")
        .arg("-tzf")
        .arg(bundle.join("recordings.tar.gz"))
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&rec.stdout).contains("./tenant-a/call-1.wav"),
        "{}",
        String::from_utf8_lossy(&rec.stdout)
    );
    // Fetched once.
    let again = client
        .get(api(&format!("/v1/downloads/{id}/file?token={token}")))
        .bearer_auth("backup-api-token")
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 404);

    // B3b: a lost recording found and put back by the path its row names;
    // a file changed since is left as it is.
    let caps: serde_json::Value = client
        .get(api("/v1/capabilities"))
        .bearer_auth("backup-api-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(caps["file_restores"], true, "{caps}");
    let lost = files.path().join("tenant-a/call-1.wav");
    let kept_before = std::fs::read(&lost).unwrap();
    std::fs::remove_file(&lost).unwrap();
    let post = |path: &str, body: serde_json::Value| {
        let client = client.clone();
        let url = api(path);
        async move {
            let r = client
                .post(url)
                .bearer_auth("backup-api-token")
                .json(&body)
                .send()
                .await
                .unwrap();
            let status = r.status();
            (
                status,
                r.json::<serde_json::Value>().await.unwrap_or_default(),
            )
        }
    };
    let (status, checked) = post(
        "/v1/files/check",
        serde_json::json!({ "paths": ["/srv/recordings/tenant-a/call-1.wav", "/etc/passwd"] }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(checked[0]["held_here"], true);
    assert_eq!(checked[0]["present"], false);
    assert_eq!(checked[1]["held_here"], false);
    let (_, found) = post(
        "/v1/files/find",
        serde_json::json!({
            "destination_id": format!("bkd-{}", dest.id),
            "paths": ["/srv/recordings/tenant-a/call-1.wav"],
        }),
    )
    .await;
    let snap = found["/srv/recordings/tenant-a/call-1.wav"][0]["snapshot"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(snap.get(..8), run.snapshots[1].get(..8), "{found}");
    // Something else written since the snapshot is not overwritten.
    let changed = files.path().join("tenant-a/changed.wav");
    std::fs::write(&changed, b"newer").unwrap();
    let (status, refused) = post(
        "/v1/restores/files",
        serde_json::json!({
            "destination_id": format!("bkd-{}", dest.id), "snapshot": snap,
            "service_paths": ["/etc/passwd"],
        }),
    )
    .await;
    assert_eq!(status, 400, "{refused}");
    let (status, restored) = post(
        "/v1/restores/files",
        serde_json::json!({
            "destination_id": format!("bkd-{}", dest.id), "snapshot": snap,
            "service_paths": ["/srv/recordings/tenant-a/call-1.wav"],
        }),
    )
    .await;
    assert_eq!(status, 200, "{restored}");
    assert_eq!(restored[0]["present"], true);
    assert_eq!(std::fs::read(&lost).unwrap(), kept_before);
    // The whole volume, by a platform administrator: what is there stays.
    std::fs::remove_file(&lost).unwrap();
    let (status, restored) = post(
        "/v1/restores/files",
        serde_json::json!({
            "destination_id": format!("bkd-{}", dest.id), "snapshot": snap, "volume": "recordings",
        }),
    )
    .await;
    assert_eq!(status, 200, "{restored}");
    assert!(lost.exists());
    assert_eq!(std::fs::read(&changed).unwrap(), b"newer");

    // B3c: the whole system back as the point holds it. What came after
    // the backup goes: a table, a new file, a change.
    {
        let mut conn = <sqlx::PgConnection as sqlx::Connection>::connect(db.url())
            .await
            .unwrap();
        sqlx::query("CREATE TABLE after_the_backup (x INT)")
            .execute(&mut conn)
            .await
            .unwrap();
    }
    std::fs::write(files.path().join("tenant-a/after.wav"), b"later").unwrap();
    std::fs::write(&lost, b"changed since").unwrap();
    let restore_body = |confirm: &str, key: Option<&str>| {
        serde_json::json!({
            "destination_id": format!("bkd-{}", dest.id),
            "run": format!("bkr-{}", run.id),
            "confirm": confirm,
            "master_key": key,
        })
    };
    let (status, said) = post(
        "/v1/restores/system",
        restore_body("not the time", Some("a2V5")),
    )
    .await;
    assert_eq!(status, 400, "{said}");
    let point_time = points[0]["time"].as_str().unwrap().to_string();
    // No key to give the restored key ring: refused before anything stops.
    let (status, said) = post("/v1/restores/system", restore_body(&point_time, None)).await;
    assert_eq!(status, 400, "{said}");
    assert!(
        said["error"]
            .as_str()
            .unwrap()
            .contains("needs the master key"),
        "{said}"
    );
    assert!(!work.path().join("services.log").exists());
    let (status, job) = post(
        "/v1/restores/system",
        restore_body(&point_time, Some("dGhlLWtleS1naXZlbg==")),
    )
    .await;
    assert_eq!(status, 202, "{job}");
    let mut job = job;
    for _ in 0..240 {
        job = client
            .get(api("/v1/restores/system/current"))
            .bearer_auth("backup-api-token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if job["state"] != "running" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(job["state"], "succeeded", "{job}");
    assert!(
        job["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["state"] == "done"),
        "{job}"
    );
    let previous = job["previous_database"].as_str().unwrap().to_string();
    assert_eq!(
        std::fs::read_to_string(work.path().join("services.log")).unwrap(),
        "stopped\nstarted\n"
    );
    {
        let mut conn = <sqlx::PgConnection as sqlx::Connection>::connect(db.url())
            .await
            .unwrap();
        let gone: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('after_the_backup')::text")
                .fetch_one(&mut conn)
                .await
                .unwrap();
        assert_eq!(gone, None, "the restored database is the point's");
        let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM backup_runs")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert!(runs >= 2, "the point's own rows are there");
    }
    assert!(
        !files.path().join("tenant-a/after.wav").exists(),
        "a file made after is gone"
    );
    assert_eq!(std::fs::read(&lost).unwrap(), kept_before);
    assert!(certs.path().join("ca.pem").exists());
    assert_eq!(
        std::fs::read_to_string(certs.path().join("master.key"))
            .unwrap()
            .trim(),
        "dGhlLWtleS1naXZlbg=="
    );
    // The service goes on against the restored database.
    let after = request(dest.id, BackupRunKind::Test);
    store.request_backup_run(&after).await.unwrap();
    let after = wait_for(&store, &after.id).await;
    assert_eq!(after.state, BackupRunState::Succeeded, "{after:?}");
    {
        let (_, admin) = ((), {
            let mut u = url::Url::parse(db.url()).unwrap();
            u.set_path("/postgres");
            u.to_string()
        });
        let mut conn = <sqlx::PgConnection as sqlx::Connection>::connect(&admin)
            .await
            .unwrap();
        sqlx::query(&format!(
            "DROP DATABASE IF EXISTS \"{previous}\" WITH (FORCE)"
        ))
        .execute(&mut conn)
        .await
        .unwrap();
    }

    svc.kill().await;
    drop(sftp);
    db.drop_db().await;
}

/// An S3-compatible bucket, when the environment names one.
#[tokio::test]
async fn the_database_goes_to_an_s3_compatible_bucket() {
    let var = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
    let (Some(endpoint), Some(bucket), Some(key_id), Some(secret)) = (
        var("FCP_TEST_S3_ENDPOINT"),
        var("FCP_TEST_S3_BUCKET"),
        var("FCP_TEST_S3_ACCESS_KEY_ID"),
        var("FCP_TEST_S3_SECRET_ACCESS_KEY"),
    ) else {
        eprintln!("FCP_TEST_S3_* not set; skipping");
        return;
    };
    let Some(restic) = restic() else {
        return skip("no restic").unwrap_or(());
    };
    let Some(db) = TestDatabase::create().await else {
        return skip("FCP_TEST_DATABASE_URL is not set").unwrap_or(());
    };
    let storage = Arc::new(
        fcp_storage_postgres::PostgresStorage::new(fcp_storage_postgres::PostgresConfig::new(
            db.url().to_string(),
        ))
        .await
        .unwrap(),
    );
    let store: Arc<dyn BackupStore> = storage.clone();
    let files = tempfile::tempdir().unwrap();
    std::fs::write(files.path().join("prompt.wav"), b"RIFF a prompt").unwrap();
    let work = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let dest = BackupDestination {
        id: uuid::Uuid::new_v4(),
        name: "Bucket".into(),
        target: BackupTarget::S3 {
            endpoint,
            bucket,
            region: var("FCP_TEST_S3_REGION"),
            prefix: Some(format!("restic-test-{}", uuid::Uuid::new_v4().simple())),
            access_key_id: key_id,
            secret_access_key: Some(secret),
        },
        password: Some("another long repository password".into()),
        schedule: BackupSchedule {
            daily_at: None,
            cron: None,
        },
        keep: BackupRetention::default(),
        include_key: false,
        enabled: true,
        created_at: now,
        updated_at: now,
    };
    store.put_backup_destination(&dest).await.unwrap();
    let certs = certs_dir();
    let (svc, _) = service(&db, &restic, files.path(), certs.path(), work.path()).await;
    for kind in [BackupRunKind::Test, BackupRunKind::Backup] {
        let run = request(dest.id, kind);
        store.request_backup_run(&run).await.unwrap();
        let run = wait_for(&store, &run.id).await;
        assert_eq!(run.state, BackupRunState::Succeeded, "{kind:?}: {run:?}");
        if kind == BackupRunKind::Backup {
            assert_eq!(run.snapshots.len(), 2, "{run:?}");
        }
    }
    svc.kill().await;
    db.drop_db().await;
}
