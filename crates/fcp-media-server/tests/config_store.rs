//! The media server's operational sections are stored in the database and
//! reconciled with its TOML at every start. Needs `FCP_TEST_DATABASE_URL`;
//! skipped otherwise.

use fcp_core::ConfigStore;
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Seed, Service, TestDatabase};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-media");

/// #367: with no `tokens` in the file, a random token was made at every
/// start, stored as the `auth` section and its version bumped, though the
/// section was never read back. The tokens are the file's alone now: none
/// is stored, a copy an earlier release stored is removed, and a file
/// with auth on and no token is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_auth_section_is_never_stored() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let seed = Seed::new(db.url(), "media-config.local").await;
    let storage = seed.storage();
    // What an earlier release left.
    storage
        .set_config(
            "media",
            "auth",
            "_system",
            serde_json::json!({"disable_auth": false, "tokens": ["media-file-token"]}),
            Some("seed"),
        )
        .await
        .unwrap();

    let dir = smoke::temp_dir();
    let config = |http: u16, auth: &str| {
        format!(
            r#"[service]
node_id = "media-config"
shutdown_timeout_secs = 5

[http]
bind_address = "127.0.0.1:{http}"

[recording]
base_dir = "{dir}/recordings"
root_jail = "{dir}"
prompts_dir = "{dir}/prompts"

[storage]
database_url = "{db}"

[auth]
{auth}

[logging]
level = "info"
format = "compact"
"#,
            dir = smoke::toml_path(dir.path()),
            db = db.url(),
        )
    };

    let http = free_port();
    Service::start(
        "media",
        BIN,
        &config(http, r#"tokens = ["media-file-token"]"#),
        &loopback(http, "/health"),
    )
    .await
    .stop()
    .await;
    assert!(
        storage
            .get_config("media", "auth", "_system")
            .await
            .unwrap()
            .is_none(),
        "the stored auth section is removed"
    );
    let media = storage
        .get_config("media", "media", "_system")
        .await
        .unwrap()
        .expect("the other sections are still stored");

    // Auth on and no token: refused, and nothing written.
    let refused = std::process::Command::new(BIN)
        .arg("--check-config")
        .arg("--config")
        .arg({
            let path = dir.path().join("no-token.toml");
            std::fs::write(&path, config(free_port(), "")).unwrap();
            path
        })
        .env_remove("FCP_MEDIA_AUTH_TOKENS")
        .output()
        .unwrap();
    assert!(!refused.status.success(), "a file with no token is refused");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("no auth.tokens"), "{stderr}");
    assert!(storage
        .get_config("media", "auth", "_system")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        storage
            .get_config("media", "media", "_system")
            .await
            .unwrap()
            .unwrap()
            .version,
        media.version
    );
    db.drop_db().await;
}
