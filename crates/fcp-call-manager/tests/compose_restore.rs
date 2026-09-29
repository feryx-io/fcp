//! A backup of the compose stack restored into a fresh one
//! (`docs/BACKUP_RESTORE_UPGRADE.md` phase 3, `docs/BACKUP_AND_RESTORE.md`
//! §§3–4), with the commands `scripts/backup.sh` and `scripts/restore.sh`
//! run — `pg_dump`, `tar`, `sha256sum`, `pg_restore` — against the
//! binaries' own directories, since there is no Docker here. The stack's
//! database services start from the compose file's configurations
//! (`fcp_testkit::compose`) with encryption at rest on; a user is made
//! through the admin API, so their SIP credentials are sealed under the
//! stack's data encryption key, and they register and call a room. Then
//! the backup: the dump, a tar per stateful directory, the manifest with
//! the KEK id the key ring names, the checksums. Then everything goes —
//! the database dropped, the directories deleted — and the backup is
//! restored into a new database and a new directory. Without the key —
//! a fresh master key beside the restored certificates — every service
//! that opens the database refuses to start, naming the KEK it wanted.
//! With it, the stack comes up, the old user registers with the sealed
//! password, the same data encryption key is active, and the room takes
//! a call. The compose file's ports are used as they are, so this is the
//! one test in this binary.

use fcp_testkit::certs::ClusterCa;
use fcp_testkit::compose::ComposeStack;
use fcp_testkit::smoke::{http_request, loopback, trust_ca, Smoke};
use fcp_testkit::{bins, cluster, Phone, Seed, Service, TestDatabase};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const REALM: &str = "fcp.local"; // the compose call manager's realm
const ROOM: &str = "8000";
/// One the admin API's password policy accepts.
const PASSWORD: &str = "Alice-restores-2026!";
const A_WHILE: Duration = Duration::from_secs(20);

/// What the test runs: the events server the call manager posts to,
/// the admin that makes the user, the call manager, the room, and the
/// job worker that takes the call's record off the Redis stream and
/// stores it — so the record is in the dump, and so nothing is left on
/// the stream for another test's processor to find.
const STACK: [&str; 5] = [
    "fcp-events-server",
    "fcp-admin",
    "fcp-call-manager",
    "fcp-conference",
    "fcp-jobs",
];

/// Every compose service that opens Postgres under the master key
/// (`FCP_MASTER_KEY_FILE` in its compose environment).
const DATABASE_SERVICES: [&str; 10] = [
    "fcp-admin",
    "fcp-events-server",
    "fcp-call-manager",
    "fcp-conference",
    "fcp-ivr",
    "fcp-voicemail",
    "fcp-presence",
    "fcp-sbc",
    "fcp-jobs",
    "fcp-registrar",
];

/// The stateful directories under the test's root, as the compose
/// volumes they stand for: the certs volume, and the call manager's
/// `/app/data`.
const VOLUMES: [(&str, &str); 2] = [
    ("fcp-certs", "var-lib-fcp/certs"),
    ("call-manager-data", "app-data"),
];

async fn start(stack: &ComposeStack, name: &str, dir: &Path, db: &str, redis: &str) -> Service {
    let svc = stack.service(name);
    let config = stack.config_for(name, dir, redis);
    let env = stack.env_for(name, db, dir);
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let bin = bins::path(svc.binary, svc.package);
    Service::start_with(name, bin, &config, &svc.health_url, &env_refs).await
}

async fn stop_all(services: Vec<Service>) {
    for service in services.into_iter().rev() {
        let log = service.stop().await;
        assert!(!log.contains("panicked"), "a service panicked:\n{log}");
    }
}

async fn request(
    method: &str,
    url: &str,
    token: &str,
    body: Option<&str>,
) -> (u16, serde_json::Value) {
    let bearer = format!("Bearer {token}");
    let tenant = fcp_core::TenantId::nil().to_string();
    let (code, text) = http_request(
        method,
        url,
        &[
            ("Authorization", &bearer),
            ("x-fcp-tenant-id", &tenant),
            ("Content-Type", "application/json"),
        ],
        body,
    )
    .await
    .unwrap_or_else(|e| panic!("{method} {url}: {e}"));
    (
        code,
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
    )
}

/// A Postgres client tool: from the directory `FCP_TEST_PG_BIN` names,
/// else the PATH. The dump and the restore are the client's, so it must
/// be the server's major version or newer.
fn pg_tool(name: &str) -> PathBuf {
    let path = match std::env::var("FCP_TEST_PG_BIN") {
        Ok(dir) => PathBuf::from(dir).join(name),
        Err(_) => PathBuf::from(name),
    };
    let version = Command::new(&path).arg("--version").output();
    assert!(
        version.as_ref().is_ok_and(|o| o.status.success()),
        "{name} is needed for the restore test (set FCP_TEST_PG_BIN to the directory that has it): {:?}",
        version.map(|o| String::from_utf8_lossy(&o.stderr).to_string())
    );
    path
}

fn run(cmd: &mut Command) -> String {
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("{:?}: {e}", cmd.get_program()));
    assert!(
        out.status.success(),
        "{:?} {:?} failed ({}):\n{}{}",
        cmd.get_program(),
        cmd.get_args().collect::<Vec<_>>(),
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The certificates the compose stack's `certs` service makes into its
/// volume, from a CA of this test's, and beside them a master key as
/// `openssl rand -base64 32` writes it.
fn make_certs(certs_dir: &Path) -> ClusterCa {
    std::fs::create_dir_all(certs_dir).expect("certs dir");
    let ca = ClusterCa::new(certs_dir);
    for service in [
        "fcp-call-manager",
        "fcp-registrar",
        "fcp-conference",
        "fcp-sbc",
        "fcp-media",
        "fcp-webrtc-gw",
        "fcp-proxy",
        "fcp-admin",
    ] {
        ca.node(service);
    }
    write_master_key(certs_dir);
    ca
}

fn write_master_key(certs_dir: &Path) {
    let mut key = uuid::Uuid::new_v4().as_bytes().to_vec();
    key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    std::fs::write(
        certs_dir.join("master.key"),
        format!(
            "{}\n",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key)
        ),
    )
    .expect("master key");
}

/// The KEK id a local master key file has: `local:` and the first eight
/// bytes of the SHA-256 of the key, as `fcp_crypto::LocalKek` derives it
/// and as `restore.sh` computes it with `base64 -d | sha256sum`.
fn local_kek_id(master_key_file: &Path) -> String {
    let encoded = std::fs::read_to_string(master_key_file).expect("master.key");
    let key = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded.trim())
        .expect("master.key is base64");
    let digest = Sha256::digest(&key);
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("local:{hex}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_backup_restores_into_a_fresh_stack_and_refuses_without_the_key() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the compose restore test");
        return;
    };
    let Some(redis) = cluster::private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the compose restore test");
        return;
    };
    let pg_dump = pg_tool("pg_dump");
    let pg_restore = pg_tool("pg_restore");
    let stack = ComposeStack::load();
    let admin_token = stack.dotenv("FCP_ADMIN_AUTH_TOKEN");

    // ── The stack before: a user made sealed, a call carried ─────────
    let dir = tempfile::tempdir().expect("tempdir");
    let certs_dir = dir.path().join("var-lib-fcp").join("certs");
    let ca = make_certs(&certs_dir);
    trust_ca(&ca.ca_path);
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting(ROOM, None, None).await;

    let mut services = Vec::new();
    for name in STACK {
        services.push(start(&stack, name, dir.path(), db.url(), &redis).await);
    }

    // Encryption at rest is on, and the key ring names this key.
    let (code, encryption) = request(
        "GET",
        &loopback(8080, "/api/v1/system/encryption"),
        &admin_token,
        None,
    )
    .await;
    assert_eq!(code, 200, "{encryption}");
    let kek_id = local_kek_id(&certs_dir.join("master.key"));
    assert_eq!(
        encryption["kek_id"].as_str(),
        Some(kek_id.as_str()),
        "the key ring names the key in the certs directory: {encryption}"
    );
    let dek_before = encryption["active_key_id"]
        .as_str()
        .expect("an active DEK")
        .to_string();

    // A user through the admin API: their digest credentials are
    // written sealed under that DEK (`docs/ENCRYPTION.md`), which is
    // what the restore must be able to open.
    let alice = serde_json::json!({
        "username": "alice",
        "password": PASSWORD,
        "display_name": "alice",
        "extension": "1001",
    })
    .to_string();
    let (code, created) = request(
        "POST",
        &loopback(8080, "/api/v1/users"),
        &admin_token,
        Some(alice.as_str()),
    )
    .await;
    assert_eq!(code, 201, "{created}");

    let cm_sip: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let phone = Phone::new("alice", PASSWORD, REALM, cm_sip)
        .await
        .expect("phone binds");
    assert_eq!(phone.register().await.expect("register"), 200);
    let call = phone.call(ROOM).await.expect("INVITE sent");
    call.answered(A_WHILE)
        .await
        .expect("the conference server admits the caller");
    assert_eq!(call.hangup().await.expect("BYE"), 200);
    drop(phone);

    // The call's record, shipped over the stream and stored by the job
    // worker: what the dump must carry.
    let cdr_id = stored_call_record(&admin_token, 1).await;

    // A file where the call manager writes its recordings, standing in
    // for one: what a row would name, and what the tar must bring back.
    let recording = Path::new("app-data/recordings/call-restore-proof.wav");
    std::fs::create_dir_all(dir.path().join(recording).parent().unwrap()).unwrap();
    std::fs::write(dir.path().join(recording), b"RIFF....WAVE").unwrap();

    // ── The backup: backup.sh's commands against the directories ─────
    let backup = tempfile::tempdir().expect("backup dir");
    let out = backup.path();
    let dump = out.join("fcp.dump");
    run(Command::new(&pg_dump)
        .args(["-Fc", "--no-owner", "--no-privileges", "-f"])
        .arg(&dump)
        .arg(db.url()));
    assert!(
        std::fs::metadata(&dump).unwrap().len() > 0,
        "pg_dump wrote nothing"
    );
    for (volume, subdir) in VOLUMES {
        run(Command::new("tar")
            .arg("-C")
            .arg(dir.path().join(subdir))
            .arg("-czf")
            .arg(out.join(format!("{volume}.tar.gz")))
            .arg("."));
    }
    let volumes: Vec<&str> = VOLUMES.iter().map(|(v, _)| *v).collect();
    std::fs::write(
        out.join("manifest"),
        format!(
            "fcp_backup = 1\ntaken_at = {}\nstack_version = {}\nstack_version_from = test\nproject = test\ndatabase = fcp.dump\nkek_id = {kek_id}\nvolumes = {}\nvolumes_absent =\n",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            env!("CARGO_PKG_VERSION"),
            volumes.join(" "),
        ),
    )
    .unwrap();
    let mut files = vec!["fcp.dump".to_string(), "manifest".to_string()];
    files.extend(volumes.iter().map(|v| format!("{v}.tar.gz")));
    let sums = run(Command::new("sha256sum").current_dir(out).args(&files));
    std::fs::write(out.join("SHA256SUMS"), sums).unwrap();

    // ── Everything goes ──────────────────────────────────────────────
    stop_all(services).await;
    db.drop_db().await;
    drop(dir);

    // ── The restore: restore.sh's commands into a new database and
    // directory, the checksums verified first ────────────────────────
    run(Command::new("sha256sum")
        .current_dir(out)
        .args(["-c", "--quiet", "SHA256SUMS"]));
    let restored = tempfile::tempdir().expect("restored dir");
    for (volume, subdir) in VOLUMES {
        let target = restored.path().join(subdir);
        std::fs::create_dir_all(&target).unwrap();
        run(Command::new("tar")
            .arg("-C")
            .arg(&target)
            .arg("-xzf")
            .arg(out.join(format!("{volume}.tar.gz"))));
    }
    let restored_certs = restored.path().join("var-lib-fcp").join("certs");
    assert_eq!(
        local_kek_id(&restored_certs.join("master.key")),
        kek_id,
        "the restored master key fingerprints to the manifest's KEK id"
    );
    assert!(
        restored.path().join(recording).is_file(),
        "the recording is back at the path its row would name"
    );
    let db = TestDatabase::create().await.expect("a second database");
    run(Command::new(&pg_restore)
        .args(["--no-owner", "--no-privileges", "--exit-on-error", "-d"])
        .arg(db.url())
        .arg(&dump));

    // ── Without the key: every database service refuses ──────────────
    // The restored certificates beside a master key that is not the one
    // the key ring names.
    let wrong = tempfile::tempdir().expect("wrong-key dir");
    let wrong_certs = wrong.path().join("var-lib-fcp").join("certs");
    std::fs::create_dir_all(&wrong_certs).unwrap();
    for entry in std::fs::read_dir(&restored_certs).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), wrong_certs.join(entry.file_name())).unwrap();
    }
    write_master_key(&wrong_certs);
    let wrong_kek = local_kek_id(&wrong_certs.join("master.key"));
    assert_ne!(wrong_kek, kek_id);
    for name in DATABASE_SERVICES {
        let svc = stack.service(name);
        let mut smoke = Smoke::new(bins::path(svc.binary, svc.package))
            .config_toml(stack.config_for(name, wrong.path(), &redis))
            .health_url(svc.health_url.clone())
            .startup_timeout(Duration::from_secs(60));
        for (k, v) in stack.env_for(name, db.url(), wrong.path()) {
            smoke = smoke.env(k, v);
        }
        let report = smoke.run().await;
        assert!(
            report.status.is_some_and(|s| !s.success()),
            "{name} did not refuse to start with the wrong master key: {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            report.failure,
            report.stdout,
            report.stderr
        );
        report.assert_logged(&format!("wrapped by KEK {kek_id}"));
        report.assert_logged(&format!("configured with KEK {wrong_kek}"));
    }

    // ── With the key: the stack is back ──────────────────────────────
    let mut services = Vec::new();
    for name in STACK {
        services.push(start(&stack, name, restored.path(), db.url(), &redis).await);
    }
    let (code, encryption) = request(
        "GET",
        &loopback(8080, "/api/v1/system/encryption"),
        &admin_token,
        None,
    )
    .await;
    assert_eq!(code, 200, "{encryption}");
    assert_eq!(
        encryption["active_key_id"].as_str(),
        Some(dek_before.as_str()),
        "the same data encryption key is active, unwrapped by the restored master key: {encryption}"
    );
    let (code, user) = request(
        "GET",
        &loopback(8080, "/api/v1/users/alice"),
        &admin_token,
        None,
    )
    .await;
    assert_eq!(code, 200, "the dump carried the user: {user}");
    let (code, cdrs) = request("GET", &loopback(8080, "/api/v1/cdrs"), &admin_token, None).await;
    assert_eq!(code, 200, "{cdrs}");
    assert!(
        cdrs["data"]
            .as_array()
            .is_some_and(|list| list.iter().any(|r| r["cdr_id"] == cdr_id.as_str())),
        "the dump carried the call's record {cdr_id}: {cdrs}"
    );

    // The old user registers with the sealed password, and calls.
    let phone = Phone::new("alice", PASSWORD, REALM, cm_sip)
        .await
        .expect("phone binds");
    assert_eq!(
        phone.register().await.expect("register"),
        200,
        "the sealed credentials open under the restored key"
    );
    let call = phone.call(ROOM).await.expect("INVITE sent");
    call.answered(A_WHILE)
        .await
        .expect("the restored stack carries a call");
    assert_eq!(call.hangup().await.expect("BYE"), 200);
    // And the restored stack's own pipeline stores this call's record too
    // — which also leaves nothing of ours on the stream.
    stored_call_record(&admin_token, 2).await;

    stop_all(services).await;
    db.drop_db().await;
}

/// The id of the newest of the tenant's call records once `n` of them
/// are stored: the job worker took them off the stream.
async fn stored_call_record(admin_token: &str, n: usize) -> String {
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let (code, cdrs) = request("GET", &loopback(8080, "/api/v1/cdrs"), admin_token, None).await;
        assert_eq!(code, 200, "{cdrs}");
        let calls: Vec<&serde_json::Value> = cdrs["data"]
            .as_array()
            .map(|list| list.iter().filter(|r| r["kind"] == "call").collect())
            .unwrap_or_default();
        if calls.len() >= n {
            return calls
                .iter()
                // RFC 3339 in UTC to the second sorts as the time does.
                .max_by_key(|r| r["start_time"].as_str().unwrap_or("").to_string())
                .and_then(|r| r["cdr_id"].as_str())
                .expect("a record has a cdr_id")
                .to_string();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{} of {n} call records reached the database: {cdrs}",
            calls.len()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
