//! The shipped site, live (`docs/SITE_SETUP.md`, hybrid phase 7): the
//! site stack under `deploy/docker-compose/site`, brought up by
//! `docker compose` from the nightly's images beside the cloud stack on
//! the same host, enrolled to it with a token the workflow took from the
//! cloud's admin API. This test starts nothing: it expects both stacks on
//! their compose files' ports and is gated on `FCP_LIVE_SITE`, the id of
//! the site the workflow made for the tenant `live`. Then: the cloud's
//! site record says enrolled with the link up and the site's call manager
//! on it; a user made in the cloud reaches the site's database down the
//! link and registers at the site's call manager; the site's link trunk
//! is bound at the cloud's SBC; the site's own admin refuses to make a
//! user, naming the cloud; and the site is no further behind the cloud
//! than nothing. Skipped, with a notice, when `FCP_LIVE_SITE` is unset.

use fcp_testkit::compose::ComposeStack;
use fcp_testkit::smoke::{http_request, loopback, trust_ca};
use fcp_testkit::Phone;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

const REALM: &str = "fcp.local";
const TENANT: &str = "live";
/// One the admin API's password policy accepts.
const PASSWORD: &str = "Carol-lives-2026!";
/// The site's call manager, as `site/configs/fcp-call-manager.toml` names it.
const SITE_CALL_MANAGER_NODE: &str = "site-cm-01";
/// How long the site may take: a snapshot, a trunk refresh, a heartbeat.
const A_WHILE: Duration = Duration::from_secs(90);

struct Admin {
    port: u16,
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
        let url = loopback(self.port, path);
        let (code, text) = http_request(method, &url, &headers, body)
            .await
            .unwrap_or_else(|e| panic!("{method} {url}: {e}"));
        (
            code,
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
        )
    }
}

/// Poll until `check` holds of what `path` returns, or fail with the last answer.
async fn until<F: Fn(&serde_json::Value) -> bool>(
    what: &str,
    admin: &Admin,
    path: &str,
    tenant: Option<&str>,
    check: F,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let (code, body) = admin.call("GET", path, tenant, None).await;
        if code == 200 && check(&body) {
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: last {code} {body}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_shipped_site_is_enrolled_and_in_step() {
    let Ok(site_id) = std::env::var("FCP_LIVE_SITE") else {
        eprintln!("FCP_LIVE_SITE not set; skipping the live site test");
        return;
    };
    let cloud = ComposeStack::load();
    let site = ComposeStack::load_site();
    if let Ok(ca) = std::env::var("FCP_LIVE_CA") {
        trust_ca(Path::new(&ca));
    }
    let cloud_admin = Admin {
        port: 8080,
        token: cloud.dotenv("FCP_ADMIN_AUTH_TOKEN"),
    };
    let site_admin = Admin {
        port: 18080,
        token: site.dotenv("FCP_ADMIN_AUTH_TOKEN"),
    };
    let sbc_token = cloud.dotenv("FCP_SBC_HEALTH_API_TOKEN");

    // Every service of the site answers the health check its compose
    // file gave it.
    for name in ["fcp-admin", "fcp-call-manager", "fcp-link"] {
        let url = &site.service(name).health_url;
        let (code, body) = http_request("GET", url, &[], None)
            .await
            .unwrap_or_else(|e| panic!("{name} {url}: {e}"));
        assert_eq!(code, 200, "{name} {url}: {body}");
    }

    // The live tenant, and its site as the cloud sees it: enrolled, the
    // link up, the site's call manager on it with its release.
    let (code, listed) = cloud_admin.call("GET", "/api/v1/tenants", None, None).await;
    assert_eq!(code, 200, "{listed}");
    let tenant = listed
        .as_array()
        .cloned()
        .or_else(|| listed["tenants"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .find(|t| t["name"] == TENANT)
        .and_then(|t| t["id"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the cloud has the tenant {TENANT}: {listed}"));
    let site_path = format!("/api/v1/sites/{site_id}");
    let record = until(
        "the site enrolled with its link up and its call manager on it",
        &cloud_admin,
        &site_path,
        Some(&tenant),
        |s| {
            s["status"] == "enrolled"
                && s["link_up"] == true
                && s["nodes"]
                    .as_array()
                    .is_some_and(|n| n.iter().any(|n| n["node_id"] == SITE_CALL_MANAGER_NODE))
        },
    )
    .await;
    let cm_release = record["nodes"]
        .as_array()
        .and_then(|n| n.iter().find(|n| n["node_id"] == SITE_CALL_MANAGER_NODE))
        .and_then(|n| n["version"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        !cm_release.is_empty(),
        "the site's call manager's release: {record}"
    );
    assert!(
        record["trunk_id"].as_str().is_some_and(|t| !t.is_empty()),
        "the site got a link trunk when it enrolled: {record}"
    );

    // A user made in the cloud reaches the site: she registers at the
    // site's call manager with the password the cloud sealed and the
    // link brought down.
    let (code, made) = cloud_admin
        .call(
            "POST",
            "/api/v1/users",
            Some(&tenant),
            Some(
                &serde_json::json!({
                    "username": "carol",
                    "password": PASSWORD,
                    "display_name": "carol",
                    "tenant_id": tenant,
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(code, 201, "{made}");
    let site_sip: SocketAddr = "127.0.0.1:5090".parse().unwrap();
    let carol = Phone::new("carol", PASSWORD, REALM, site_sip)
        .await
        .expect("phone binds");
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let code = carol.register().await.expect("REGISTER at the site");
        if code == 200 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let (_, status) = cloud_admin
                .call("GET", &site_path, Some(&tenant), None)
                .await;
            panic!("carol never registered at the site (last {code}); the site: {status}");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // The site's link trunk is bound at the cloud's SBC: calls for the
    // site go down that flow.
    let bearer = format!("Bearer {sbc_token}");
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let (code, text) = http_request(
            "GET",
            &loopback(8094, "/admin/sites"),
            &[("Authorization", &bearer)],
            None,
        )
        .await
        .expect("the SBC's sites");
        assert_eq!(code, 200, "{text}");
        let bound: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
        if bound.iter().any(|s| s["site_id"] == site_id.as_str()) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the site never bound at the SBC: {text}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // The site's own admin has her too, and refuses to make another:
    // users are the cloud's, and it says where.
    let (code, users) = site_admin
        .call("GET", "/api/v1/users", Some(&tenant), None)
        .await;
    assert_eq!(code, 200, "{users}");
    let has_carol = users
        .as_array()
        .cloned()
        .or_else(|| users["users"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .any(|u| u["username"] == "carol");
    assert!(has_carol, "carol at the site's admin: {users}");
    let (code, refused) = site_admin
        .call(
            "POST",
            "/api/v1/users",
            Some(&tenant),
            Some(
                &serde_json::json!({
                    "username": "dave",
                    "password": PASSWORD,
                    "display_name": "dave",
                    "tenant_id": tenant,
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(code, 403, "{refused}");
    assert!(
        refused.to_string().contains("managed by the cloud"),
        "{refused}"
    );

    // And the site is in step: nothing behind, no sync error.
    let record = until(
        "the site caught up with the cloud",
        &cloud_admin,
        &site_path,
        Some(&tenant),
        |s| s["sync_behind"] == 0 && s["sync_error"].is_null(),
    )
    .await;
    println!(
        "live site {site_id}: enrolled, link up, {SITE_CALL_MANAGER_NODE} on release {cm_release}, carol registered, bound at the SBC, sync cursor {}",
        record["sync_cursor"]
    );
}
