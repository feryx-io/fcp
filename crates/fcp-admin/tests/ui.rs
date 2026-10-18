//! The admin UI as a person uses it (`docs/DEEP_TESTING.md` D7): an admin
//! is started on a throwaway database with authentication on, serving the
//! UI it embeds, and the Playwright suite in `crates/fcp-admin-ui/ui/e2e`
//! runs against it in Chromium — signing in, opening every page, and
//! making, changing and deleting the main objects through their forms.
//!
//! The suite is told where the admin is and who may sign in through
//! `FCP_UI_URL`, `FCP_UI_TOKEN` (the static token, for checking what a
//! form did through the API), `FCP_UI_ADMIN` (a super admin),
//! `FCP_UI_TENANT_ADMIN` and `FCP_UI_USER` (each `name:password`).
//!
//! Skipped without `FCP_TEST_DATABASE_URL`, and when the admin was built
//! without the UI (`npm run build` in `crates/fcp-admin-ui/ui` before
//! `cargo build`) or Playwright's Chromium is not installed — unless
//! `FCP_UI_TESTS=required`, as CI sets it.

use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Service, TestDatabase};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "ui-test-static-token-0123456789abcdef";
const PASSWORD: &str = "Ui-test-pass-2026!";

fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fcp-admin-ui/ui")
}

fn required() -> bool {
    std::env::var("FCP_UI_TESTS").is_ok_and(|v| v == "required")
}

/// Why the suite cannot run here, if it cannot.
fn cannot_run(index: &str) -> Option<String> {
    if index.contains("Admin UI not built") || !index.contains("<div id=\"root\"") {
        return Some(
            "the admin was built without the UI (npm run build in crates/fcp-admin-ui/ui, \
             then cargo build)"
                .into(),
        );
    }
    if !ui_dir().join("node_modules/@playwright/test").is_dir() {
        return Some("no node_modules (npm ci in crates/fcp-admin-ui/ui)".into());
    }
    let browsers = Command::new("npx")
        .args(["playwright", "install", "--dry-run", "chromium"])
        .current_dir(ui_dir())
        .output();
    let installed = browsers.is_ok_and(|o| {
        let text = String::from_utf8_lossy(&o.stdout);
        o.status.success()
            && text
                .lines()
                .filter_map(|l| l.trim().strip_prefix("Install location:"))
                .all(|dir| Path::new(dir.trim()).is_dir())
    });
    if !installed {
        return Some(
            "Playwright's Chromium is not installed (npx playwright install chromium)".into(),
        );
    }
    None
}

/// Make a user through the API; `name:password` for the suite.
async fn user(url: &str, name: &str, role: &str) -> String {
    let res = reqwest::Client::new()
        .post(format!("{url}/api/v1/users"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "username": name,
            "password": PASSWORD,
            "role": role,
            "email": format!("{name}@ui.test"),
        }))
        .send()
        .await
        .expect("POST /users");
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    assert_eq!(status, 201, "make {name}: {body}");
    format!("{name}:{PASSWORD}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_admin_ui_works_in_a_browser() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let http = free_port();
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "ui-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"

[storage]
database_url = "{db}"

# The webhook form's test points at loopback.
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

    let (code, index) = smoke::http_get(&format!("{url}/admin/"))
        .await
        .expect("GET /admin/");
    assert_eq!(code, 200, "{index}");
    if let Some(why) = cannot_run(&index) {
        assert!(!required(), "the UI suite cannot run: {why}");
        eprintln!("skipping: {why}");
        service.stop().await;
        db.drop_db().await;
        return;
    }

    let admin = user(&url, "root", "super_admin").await;
    let tenant_admin = user(&url, "ta", "tenant_admin").await;
    let person = user(&url, "pat", "end_user").await;

    let out = tokio::task::spawn_blocking(move || {
        Command::new("npx")
            .args(["playwright", "test"])
            .current_dir(ui_dir())
            .env("FCP_UI_URL", &url)
            .env("FCP_UI_TOKEN", TOKEN)
            .env("FCP_UI_ADMIN", admin)
            .env("FCP_UI_TENANT_ADMIN", tenant_admin)
            .env("FCP_UI_USER", person)
            .output()
            .expect("npx playwright test")
    })
    .await
    .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        let logs = service.kill().await;
        db.drop_db().await;
        panic!(
            "the UI suite failed against a live admin\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}\n--- admin ---\n{logs}"
        );
    }
    println!("{stdout}");
    service.stop().await;
    db.drop_db().await;
}
