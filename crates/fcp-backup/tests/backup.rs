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

/// The service, set to back up `files` from `db`.
async fn service(db: &TestDatabase, restic: &Path, files: &Path, work: &Path) -> Service {
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "backup-test"

[http]
bind_address = "127.0.0.1:{http}"

[storage]
database_url = "{db}"

[backup]
restic = "{restic}"
pg_dump = "{pg_dump}"
work_dir = "{work}"
poll_secs = 1

[backup.paths]
recordings = "{files}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        restic = restic.display(),
        pg_dump = pg_dump().display(),
        work = work.display(),
        files = files.display(),
    );
    Service::start(
        "fcp-backup",
        BIN,
        &config,
        &fcp_testkit::smoke::loopback(http, "/health"),
    )
    .await
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
    let svc = service(&db, &restic, files.path(), work.path()).await;

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
    let svc = service(&db, &restic, files.path(), work.path()).await;
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
