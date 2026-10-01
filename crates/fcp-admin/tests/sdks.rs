//! The SDKs against a real admin (`docs/DEVELOPER_SURFACE.md` §3, phase 4):
//! one is started on a throwaway database with authentication on, and each
//! SDK's own test suite runs against it — `npm test` in `sdk/typescript`,
//! `uv run --extra test pytest` in `sdk/python` — with `FCP_SDK_URL` and
//! `FCP_SDK_TOKEN` set.
//!
//! The suites run the developer guide's examples too
//! (`docs/DEVELOPER_GUIDE.md`, phase 8b), one of which acts on a live
//! call: a call manager runs beside the admin, and before each SDK's suite
//! two testkit phones are put on a call whose id is `FCP_SDK_CALL_ID`. The
//! example ends it, and the phones are told.
//!
//! Skipped without `FCP_TEST_DATABASE_URL`. An SDK whose toolchain is not
//! on the `PATH` (`npm`, `uv`) or whose TypeScript dependencies are not
//! installed is skipped, unless `FCP_SDK_TESTS=required`, as CI sets it.

use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "sdk-test-static-token-0123456789abcdef";
/// The call manager's own API token, which the admin forwards with.
const NODE_TOKEN: &str = "sdk-test-node-token-0123456789abcdef";
const REALM: &str = "sdk.local";
const SOON: Duration = Duration::from_secs(5);

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
fn run(name: &str, dir: &Path, tool: &str, args: &[&str], url: &str, call_id: &str) -> Option<()> {
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
        .env("FCP_SDK_CALL_ID", call_id)
        .output()
        .unwrap_or_else(|e| panic!("{name}: {tool} did not run: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the {name} SDK's tests failed against a live admin\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    // Every test ran, none skipped: node's runner prints `# skipped 0`,
    // pytest says nothing of skips when there are none.
    let text = format!("{stdout}{stderr}");
    let skipped = if name == "typescript" {
        !text.contains("# skipped 0")
    } else {
        text.contains(" skipped")
    };
    assert!(!skipped, "{name}: tests were skipped:\n{text}");
    Some(())
}

/// A call manager on the admin's database, its media through the node so
/// a call can be held.
async fn call_manager(db: &TestDatabase, dir: &Path) -> (Service, u16, SocketAddr) {
    let sip_port = free_port();
    let http = free_port();
    let rtp_start = allocate_port_pair().rtp_port;
    let config = format!(
        r#"
[service]
node_id = "sdk-node"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"
api_token = "{NODE_TOKEN}"

[b2bua.media_anchoring]
mode = "Always"

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/node-prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir),
        rtp_end = rtp_start + 40,
    );
    let service = Service::start(
        "call-manager",
        fcp_testkit::bins::path("fcp-call-manager", "fcp-call-manager"),
        &config,
        &loopback(http, "/health"),
    )
    .await;
    (service, http, SocketAddr::from(([127, 0, 0, 1], sip_port)))
}

/// Alice calls Bob and he answers: both ends, and the node's id for the
/// call.
async fn a_call(
    node_http: u16,
    alice: &Phone,
    bob: &Phone,
) -> (fcp_testkit::Call, fcp_testkit::Call, String) {
    let call = alice.call("1002").await.expect("Alice calls");
    let ringing = bob.incoming(SOON).await.expect("Bob's phone rings");
    let answered = ringing.answer().await.expect("Bob answers");
    call.answered(SOON).await.expect("the call is answered");
    let calls: serde_json::Value = reqwest::Client::new()
        .get(loopback(node_http, "/calls"))
        .bearer_auth(NODE_TOKEN)
        .send()
        .await
        .expect("the node lists its calls")
        .json()
        .await
        .expect("JSON");
    let id = calls["calls"][0]["call_id"]
        .as_str()
        .or_else(|| calls[0]["call_id"].as_str())
        .unwrap_or_else(|| panic!("a call on the node: {calls}"))
        .to_string();
    (call, answered, id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sdks_pass_their_tests_against_a_live_admin() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let seed = Seed::new(db.url(), REALM).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let bob = seed.user("bob", "bob-pw", "1002").await;
    let (node, node_http, node_sip) = call_manager(&db, dir.path()).await;
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

# The guide's call example acts on the node's call.
[call_manager]
url = "http://127.0.0.1:{node_http}"
auth_token = "{NODE_TOKEN}"

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
    let alice_phone = Phone::new(&alice.username, &alice.password, REALM, node_sip)
        .await
        .expect("Alice's phone");
    let bob_phone = Phone::new(&bob.username, &bob.password, REALM, node_sip)
        .await
        .expect("Bob's phone");
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let suites: [(&str, &str, &[&str]); 2] = [
        ("typescript", "npm", &["test"]),
        ("python", "uv", &["run", "--extra", "test", "pytest", "-q"]),
    ];
    let mut ran = 0;
    for (name, tool, args) in suites {
        // A call for this SDK's live-call example to end.
        let (alice_call, bob_call, call_id) = a_call(node_http, &alice_phone, &bob_phone).await;
        let (dir, url, args): (PathBuf, String, Vec<String>) = (
            sdk.join(name),
            url.clone(),
            args.iter().map(|a| a.to_string()).collect(),
        );
        let outcome = tokio::task::spawn_blocking(move || {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            run(name, &dir, tool, &args, &url, &call_id)
        })
        .await
        .unwrap();
        if outcome.is_some() {
            ran += 1;
            assert!(
                alice_call.ended(SOON).await && bob_call.ended(SOON).await,
                "{name}: the guide's call example did not end the call"
            );
        } else {
            alice_call.hangup().await.ok();
            let _ = bob_call.ended(SOON).await;
        }
    }

    service.stop().await;
    node.stop().await;
    db.drop_db().await;
    if ran == 0 {
        eprintln!("no SDK toolchain here; nothing ran");
    }
}

/// The developer guide quotes the examples CI runs, exactly: each
/// `<!-- example: <path> -->` is followed by the file's text in a fenced
/// block, and every example file is quoted.
#[test]
fn the_guide_quotes_the_examples_as_they_are() {
    let guide = std::fs::read_to_string(repo().join("docs/DEVELOPER_GUIDE.md")).unwrap();
    let mut quoted = Vec::new();
    for (at, _) in guide.match_indices("<!-- example: ") {
        let rest = &guide[at + "<!-- example: ".len()..];
        let path = rest[..rest.find(" -->").expect("a closed marker")].to_string();
        let after = &rest[rest.find("```").expect("a fenced block after the marker")..];
        let body_start = after.find('\n').unwrap() + 1;
        let body_end = after[body_start..].find("\n```").expect("a closed block") + body_start;
        let block = &after[body_start..body_end];
        let file = std::fs::read_to_string(repo().join(&path))
            .unwrap_or_else(|e| panic!("the guide quotes {path}, which cannot be read: {e}"));
        assert_eq!(
            block,
            file.trim_end_matches('\n'),
            "docs/DEVELOPER_GUIDE.md quotes {path} differently from the file"
        );
        quoted.push(path);
    }
    for dir in ["sdk/typescript/examples", "sdk/python/examples"] {
        for entry in std::fs::read_dir(repo().join(dir)).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if name.ends_with(".js") || name.ends_with(".py") {
                let path = format!("{dir}/{name}");
                assert!(quoted.contains(&path), "the guide does not quote {path}");
            }
        }
    }
    assert_eq!(quoted.len(), 8, "{quoted:?}");
}
