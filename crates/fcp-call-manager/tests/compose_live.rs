//! The shipped stack, live (`docs/NIGHTLY_STACK.md` phase 2): the
//! images the Images workflow built, brought up by `docker compose` with
//! the shipped `.env`, and used as a phone would. Unlike `compose_stack.rs`
//! this test starts nothing: it expects the stack on the compose file's
//! ports and is gated on `FCP_LIVE_STACK`, which the workflow's `stack`
//! job sets — `fresh` for a stack that has never been used, when the
//! test makes the tenant, the users and their extensions through the
//! admin API; `restored` for the same stack after `backup.sh`,
//! `down -v` and `restore.sh`, when they must all still be there. In
//! both, every service answers its health check, two phones register at
//! the call manager (one over SIPS with the stack's own CA, given as
//! `FCP_LIVE_CA`), one calls the other and a tone is heard through the
//! container's media path, the admin's node list has the call manager
//! with its release and no skew, and the call's record reaches the
//! database through the job worker. Skipped, with a notice, when
//! `FCP_LIVE_STACK` is unset, so `cargo test --all-targets` never waits
//! on a stack.

use fcp_testkit::compose::ComposeStack;
use fcp_testkit::smoke::{http_request, loopback, trust_ca};
use fcp_testkit::Phone;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

const REALM: &str = "fcp.local"; // the compose call manager's realm
const TENANT: &str = "live";
/// One the admin API's password policy accepts.
const PASSWORD: &str = "Alice-lives-2026!";
const A_WHILE: Duration = Duration::from_secs(20);
/// The compose call manager's node id.
const CALL_MANAGER_NODE: &str = "cm-docker-01";

/// Every service the compose file runs by default, by the health URL the
/// testkit read from it.
const SERVICES: [&str; 12] = [
    "fcp-events-server",
    "fcp-admin",
    "fcp-api-gateway",
    "fcp-call-manager",
    "fcp-conference",
    "fcp-ivr",
    "fcp-voicemail",
    "fcp-presence",
    "fcp-sbc",
    "fcp-media",
    "fcp-webrtc-gw",
    "fcp-jobs",
];

#[derive(PartialEq)]
enum Mode {
    Fresh,
    Restored,
}

struct Admin {
    token: String,
}

impl Admin {
    async fn call(
        &self,
        method: &str,
        path: &str,
        tenant: Option<&str>,
        body: Option<&str>,
    ) -> (u16, serde_json::Value) {
        let bearer = format!("Bearer {}", self.token);
        let mut headers = vec![
            ("Authorization", bearer.as_str()),
            ("Content-Type", "application/json"),
        ];
        if let Some(tenant) = tenant {
            headers.push(("x-fcp-tenant-id", tenant));
        }
        let url = loopback(8080, path);
        let (code, text) = http_request(method, &url, &headers, body)
            .await
            .unwrap_or_else(|e| panic!("{method} {url}: {e}"));
        (
            code,
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
        )
    }

    /// The live tenant's id: made in a fresh stack, found in a restored one.
    async fn tenant(&self, mode: &Mode) -> String {
        if *mode == Mode::Fresh {
            let (code, made) = self
                .call(
                    "POST",
                    "/api/v1/tenants",
                    None,
                    Some(&serde_json::json!({ "name": TENANT }).to_string()),
                )
                .await;
            assert_eq!(code, 201, "{made}");
            let id = made["id"].as_str().expect("a tenant id").to_string();
            // The realm the phones register in is this tenant's.
            let (code, updated) = self
                .call(
                    "PUT",
                    &format!("/api/v1/tenants/{id}"),
                    None,
                    Some(&serde_json::json!({ "sip_domains": [REALM] }).to_string()),
                )
                .await;
            assert_eq!(code, 200, "{updated}");
            return id;
        }
        let (code, listed) = self.call("GET", "/api/v1/tenants", None, None).await;
        assert_eq!(code, 200, "{listed}");
        let tenants = listed
            .as_array()
            .cloned()
            .or_else(|| listed["tenants"].as_array().cloned())
            .unwrap_or_default();
        tenants
            .iter()
            .find(|t| t["name"] == TENANT)
            .and_then(|t| t["id"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| panic!("the restored stack has the tenant {TENANT}: {listed}"))
    }

    /// A user with an extension that rings them.
    async fn user(&self, tenant: &str, username: &str, extension: &str) {
        let (code, made) = self
            .call(
                "POST",
                "/api/v1/users",
                Some(tenant),
                Some(
                    &serde_json::json!({
                        "username": username,
                        "password": PASSWORD,
                        "display_name": username,
                        "tenant_id": tenant,
                    })
                    .to_string(),
                ),
            )
            .await;
        assert_eq!(code, 201, "{made}");
        let id = made["id"].as_str().expect("a user id");
        let (code, ext) = self
            .call(
                "POST",
                "/api/v1/extensions",
                Some(tenant),
                Some(
                    &serde_json::json!({
                        "number": extension,
                        "target": { "type": "user", "id": id },
                        "tenant_id": tenant,
                    })
                    .to_string(),
                ),
            )
            .await;
        assert_eq!(code, 201, "{ext}");
    }

    /// The tenant's call records once at least `n` are stored.
    async fn call_records(&self, tenant: &str, n: usize) -> Vec<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + A_WHILE;
        loop {
            let (code, cdrs) = self.call("GET", "/api/v1/cdrs", Some(tenant), None).await;
            assert_eq!(code, 200, "{cdrs}");
            let calls: Vec<serde_json::Value> = cdrs["cdrs"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter(|r| r["kind"] == "call")
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            if calls.len() >= n {
                return calls;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} of {n} call records reached the database: {cdrs}",
                calls.len()
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_shipped_stack_carries_a_call() {
    let mode = match std::env::var("FCP_LIVE_STACK").as_deref() {
        Ok("fresh") => Mode::Fresh,
        Ok("restored") => Mode::Restored,
        Ok(other) => panic!("FCP_LIVE_STACK is {other:?}; it is `fresh` or `restored`"),
        Err(_) => {
            eprintln!("FCP_LIVE_STACK not set; skipping the live stack test");
            return;
        }
    };
    let stack = ComposeStack::load();
    let admin = Admin {
        token: stack.dotenv("FCP_ADMIN_AUTH_TOKEN"),
    };
    let cm_token = stack.dotenv("FCP_CALL_MANAGER_API_TOKEN");
    let ca = std::env::var("FCP_LIVE_CA").ok();
    if let Some(ca) = &ca {
        trust_ca(Path::new(ca));
    }

    // Every service answers the health check the compose file gave it.
    for name in SERVICES {
        let url = &stack.service(name).health_url;
        let (code, body) = http_request("GET", url, &[], None)
            .await
            .unwrap_or_else(|e| panic!("{name} {url}: {e}"));
        assert_eq!(code, 200, "{name} {url}: {body}");
    }

    // The tenant and its users: made once, expected thereafter.
    let tenant = admin.tenant(&mode).await;
    if mode == Mode::Fresh {
        admin.user(&tenant, "alice", "1001").await;
        admin.user(&tenant, "bob", "1002").await;
    }

    // Two phones register with the sealed passwords, one of them over
    // SIPS with the CA the certs service made.
    let cm_sip: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let alice = Phone::new("alice", PASSWORD, REALM, cm_sip)
        .await
        .expect("phone binds");
    let bob = Phone::new("bob", PASSWORD, REALM, cm_sip)
        .await
        .expect("phone binds");
    assert_eq!(alice.register().await.expect("REGISTER"), 200);
    assert_eq!(bob.register().await.expect("REGISTER"), 200);
    if let Some(ca) = &ca {
        let cm_sips: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let secure = Phone::new_tls("alice", PASSWORD, REALM, cm_sips, Path::new(ca))
            .await
            .expect("TLS phone binds");
        assert_eq!(
            secure.register().await.expect("REGISTER over SIPS"),
            200,
            "the call manager's certificate is the certs service's, under its CA"
        );
    }

    // Alice calls Bob; a tone crosses the container's media path.
    let call = alice.call("1002").await.expect("INVITE sent");
    let Some(incoming) = bob.incoming(A_WHILE).await else {
        panic!("Bob's phone did not ring");
    };
    let bob_call = incoming.answer().await.expect("Bob answers");
    call.answered(A_WHILE)
        .await
        .expect("Alice hears the answer");
    call.media().play_tone(440.0, 4000);
    assert!(
        bob_call.media().heard(440, A_WHILE).await,
        "Bob hears Alice's tone through the stack"
    );
    assert_eq!(call.hangup().await.expect("BYE"), 200);
    assert!(bob_call.ended(A_WHILE).await);

    // The admin's node list has the call manager with its release, and the
    // cluster runs one release; the call manager's own status agrees.
    let (code, nodes) = admin.call("GET", "/api/v1/cluster/nodes", None, None).await;
    assert_eq!(code, 200, "{nodes}");
    let cm = nodes["nodes"]
        .as_array()
        .and_then(|list| list.iter().find(|n| n["node_id"] == CALL_MANAGER_NODE))
        .unwrap_or_else(|| panic!("the call manager on the node list: {nodes}"));
    let version = cm["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| panic!("the node's release: {cm}"))
        .to_string();
    let (code, stats) = admin.call("GET", "/api/v1/cluster/stats", None, None).await;
    assert_eq!(code, 200, "{stats}");
    assert_eq!(stats["stats"]["version_skew"], false, "{stats}");
    let bearer = format!("Bearer {cm_token}");
    let (code, text) = http_request(
        "GET",
        &loopback(8084, "/status"),
        &[("Authorization", &bearer)],
        None,
    )
    .await
    .expect("the call manager's status");
    assert_eq!(code, 200, "{text}");
    let status: serde_json::Value = serde_json::from_str(&text).expect("status is JSON");
    assert_eq!(status["cluster_versions"]["skew"], false, "{status}");
    assert!(
        status["cluster_versions"]["versions"][&version]
            .as_array()
            .is_some_and(|nodes| nodes.iter().any(|n| n == CALL_MANAGER_NODE)),
        "the call manager groups itself under {version}: {status}"
    );

    // The call's record crossed the stream to the job worker and the
    // database: one in a fresh stack, and in a restored one the fresh
    // stack's record beside it — the dump carried it.
    let expected = if mode == Mode::Fresh { 1 } else { 2 };
    let records = admin.call_records(&tenant, expected).await;
    assert!(
        records.iter().all(|r| r["node_id"] == CALL_MANAGER_NODE),
        "every record is this call manager's: {records:?}"
    );
    println!(
        "live stack ({}): {} on the node list, {} call record(s), release {version}",
        if mode == Mode::Fresh {
            "fresh"
        } else {
            "restored"
        },
        CALL_MANAGER_NODE,
        records.len()
    );
}
