//! The SDKs against a real admin (`docs/DEVELOPER_SURFACE.md` §3, phase 4):
//! one is started on a throwaway database with authentication on, and each
//! SDK's own test suite runs against it — `npm test` in `sdk/typescript`,
//! `uv run --extra test pytest` in `sdk/python` — with `FCP_SDK_URL` and
//! `FCP_SDK_TOKEN` set.
//!
//! Skipped without `FCP_TEST_DATABASE_URL`. An SDK whose toolchain is not
//! on the `PATH` (`npm`, `uv`) or whose TypeScript dependencies are not
//! installed is skipped, unless `FCP_SDK_TESTS=required`, as CI sets it.

use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Service, TestDatabase};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "sdk-test-static-token-0123456789abcdef";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn required() -> bool {
    std::env::var("FCP_SDK_TESTS").is_ok_and(|v| v == "required")
}

fn on_path(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run one SDK's suite; `None` when it cannot run here and need not.
fn run(name: &str, dir: &Path, tool: &str, args: &[&str], url: &str) -> Option<()> {
    if !on_path(tool) {
        assert!(!required(), "{name}: `{tool}` is not on the PATH");
        eprintln!("{name}: `{tool}` not on the PATH; skipping");
        return None;
    }
    if name == "typescript" && !dir.join("node_modules").is_dir() {
        assert!(!required(), "{name}: run `npm ci` in {}", dir.display());
        eprintln!("{name}: no node_modules (npm ci); skipping");
        return None;
    }
    let out = Command::new(tool)
        .args(args)
        .current_dir(dir)
        .env("FCP_SDK_URL", url)
        .env("FCP_SDK_TOKEN", TOKEN)
        .output()
        .unwrap_or_else(|e| panic!("{name}: {tool} did not run: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the {name} SDK's tests failed against a live admin\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    // The live tests ran, not skipped.
    let text = format!("{stdout}{stderr}");
    assert!(
        !text.contains("# skipped 4") && !text.contains("4 skipped"),
        "{name}: the live tests were skipped:\n{text}"
    );
    Some(())
}

#[tokio::test]
async fn the_sdks_pass_their_tests_against_a_live_admin() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let http = free_port();
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "sdk-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"

[storage]
database_url = "{db}"

# The SDKs' webhook tests receive on loopback.
[webhooks]
allow_private_targets = true

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let service = Service::start("admin", BIN, &config, &loopback(http, "/api/v1/health")).await;
    let url = format!("http://127.0.0.1:{http}");
    let sdk = repo().join("sdk");

    let typescript = run(
        "typescript",
        &sdk.join("typescript"),
        "npm",
        &["test"],
        &url,
    );
    let python = run(
        "python",
        &sdk.join("python"),
        "uv",
        &["run", "--extra", "test", "pytest", "-q"],
        &url,
    );

    service.stop().await;
    db.drop_db().await;
    if typescript.is_none() && python.is_none() {
        eprintln!("no SDK toolchain here; nothing ran");
    }
}
